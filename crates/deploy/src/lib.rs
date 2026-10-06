//! `deploy` command: build and run containers locally or deploy them.
//!
//! Clusters and their containers are data-driven from JSON config files.
//! `deploy <cluster> [container...]` builds and runs those containers based on the
//! cluster's environment type (local, net, or ecs).

use clap::{Args, ValueEnum};
use std::thread;
use vhp_common::error::{Result, anyhow};
use vhp_common::{clusters, config, docker, git, paths, proc, ui, version};
use aws::{self, ssm};

#[derive(ValueEnum, Clone, Debug)]
pub enum ToolsAction {
    /// Bring up the docker-compose stack.
    Up,
    /// Bring down the docker-compose stack.
    Down,
    /// Stop containers without removing them.
    Stop,
    /// Start previously stopped containers.
    Start,
    /// Show logs from the compose stack.
    Logs,
    /// Show container status.
    Ps,
    /// Check docker and compose file.
    Doctor,
    /// List available clusters.
    List,
    /// Validate AWS credentials and show active region/account (ECS only).
    Init,
}

#[derive(Args, Clone)]
pub struct DeployArgs {
    /// Cluster name from [deploy] clusters in config.toml.
    pub cluster: String,
    /// Container name(s); empty = every bulk container in the cluster.
    pub container: Vec<String>,
    /// Cluster-specific management commands: up/down/logs/ps (local), logs (net), init (ecs), list.
    #[arg(long, value_name = "ACTION")]
    pub tools: Option<ToolsAction>,
    /// Number of log lines to show before following (used with --tools logs).
    #[arg(short, long)]
    pub tail: Option<usize>,
    /// Skip confirmation prompts (for automation).
    #[arg(short, long)]
    pub yes: bool,
    /// Print commands without executing them.
    #[arg(long)]
    pub dry_run: bool,
    /// Skip docker build cache (force rebuild all layers).
    #[arg(long)]
    pub no_cache: bool,
    /// Show version history and deployments for this cluster/container.
    #[arg(long)]
    pub details: bool,
}

fn tools_cmd(
    cfg: &clusters::ClusterConfig,
    cluster: &str,
    action: ToolsAction,
    container: &[String],
    tail: Option<usize>,
    dry_run: bool,
) -> Result<()> {
    // Handle list for all environments
    if matches!(action, ToolsAction::List) {
        match cfg.environment.as_str() {
            "ecs" => {
                ui::heading("ecs list");
                let region = ssm::get_region()?;

                ui::info("ECR Repositories:");
                list_ecr_repos(&region, false)?;

                ui::info("ECS Clusters:");
                list_ecs_clusters(&region, false)?;

                ui::info("ECS Task Definitions:");
                list_task_definitions(&region, false)?;
            }
            "net" => {
                ui::heading(&format!("cluster: {}", cluster));
                ui::info(&format!("  environment -> net"));
                let containers: Vec<&str> = cfg.containers.keys().map(|s| s.as_str()).collect();
                ui::info(&format!("  containers -> {}", containers.join(", ")));
            }
            "local" => {
                ui::heading(&format!("cluster: {}", cluster));
                ui::info(&format!("  environment -> local"));
                let root = paths::project_vhp()?;
                if let Some(compose) = cfg.compose_path(&root) {
                    ui::info(&format!("  compose -> {}", compose.display()));
                }
                let containers: Vec<&str> = cfg.containers.keys().map(|s| s.as_str()).collect();
                ui::info(&format!("  containers -> {}", containers.join(", ")));
            }
            _ => {}
        }
        return Ok(());
    }

    // Handle init for ecs clusters (doesn't need compose)
    if matches!(action, ToolsAction::Init) {
        ui::heading("init");
        let region = ssm::get_region()?;
        ui::good(&format!("AWS region: {}", region));
        let account = aws::account_id()?;
        ui::good(&format!("AWS account: {}", account));
        ui::good("AWS credentials and region confirmed");
        return Ok(());
    }

    // For net clusters, some tools work via SSM
    if cfg.environment == "net" {
        return tools_cmd_net(cfg, cluster, action, container, tail, dry_run);
    }

    // Local clusters use docker-compose
    let root = paths::project_vhp()?;
    let compose = cfg.compose_path(&root).ok_or_else(|| anyhow!("cluster '{}' has no compose file", cluster))?;
    let project = format!("deploy-{}", cluster);

    match action {
        ToolsAction::Up => {
            ui::heading(&format!("Deploy {} up", cluster));
            docker::compose_up(&compose, &project, dry_run)?;
            ui::good(&format!("brought up {}", cluster));
            ui::heading(&format!("Deploy {} logs", cluster));
            docker::compose_logs(&compose, &project, Some(50), dry_run)?;
        }
        ToolsAction::Down => {
            ui::heading(&format!("Deploy {} down", cluster));
            docker::compose_down(&compose, &project, dry_run)?;
            ui::good(&format!("stopped {}", cluster));
        }
        ToolsAction::Stop => {
            ui::heading(&format!("Deploy {} stop", cluster));
            docker::compose_stop(&compose, &project, dry_run)?;
            ui::good(&format!("paused {}", cluster));
        }
        ToolsAction::Start => {
            ui::heading(&format!("Deploy {} start", cluster));
            docker::compose_start(&compose, &project, dry_run)?;
            ui::good(&format!("resumed {}", cluster));
        }
        ToolsAction::Logs => {
            ui::heading(&format!("Deploy {} logs", cluster));
            docker::compose_logs(&compose, &project, tail, dry_run)?;
        }
        ToolsAction::Ps => {
            ui::heading(&format!("Deploy {} ps", cluster));
            docker::compose_ps(&compose, &project, dry_run)?;
        }
        ToolsAction::Doctor => {
            ui::heading(&format!("Deploy {} check", cluster));
            let compose_str = compose.to_str().ok_or_else(|| anyhow!("invalid compose path"))?;
            ui::info(&format!("  compose -> {}", compose_str));
            if !dry_run && compose.exists() {
                ui::good("  ✓ compose file exists");
            } else if dry_run {
                println!("  (dry-run: would check {})", compose_str);
            } else {
                return Err(anyhow!("compose file not found: {}", compose_str));
            }
            ui::info(&format!("  project -> {}", project));
            ui::good("  ✓ ready to deploy");
        }
        ToolsAction::List | ToolsAction::Init => {
            // Handled above before compose path resolution
            unreachable!()
        }
    }

    Ok(())
}

/// Stream command output via SSM (polls and prints output in real-time).
fn logs_with_search(mode: config::Mode, cluster: &str, command: &str) -> Result<()> {
    let instance_name = format!("{}-{}", mode.docker_tag(), cluster);

    ui::info(&format!("  SSM → {instance_name}"));
    ui::info(&format!("  $ {command}"));
    println!();

    // Pull the logs
    let logs = ssm::run_command(mode, cluster, command)?;

    vhp_common::viewer::show(&logs)
}

fn tools_cmd_net(
    _cfg: &clusters::ClusterConfig,
    cluster: &str,
    action: ToolsAction,
    container: &[String],
    _tail: Option<usize>,
    dry_run: bool,
) -> Result<()> {
    let settings = config::Settings::load()?;
    let mode = settings.get_mode()?.for_deployment();

    match action {
        ToolsAction::Logs => {
            let project = format!("{cluster}-cluster");
            let compose = format!("/etc/docker/compose/{cluster}-cluster/docker-compose.yml");
            let tail_count = _tail.unwrap_or(100);

            if container.is_empty() {
                // Show logs from all services with interactive search
                ui::heading(&format!("Deploy {} logs (all services, last {})", cluster, tail_count));
                let cmd = format!(
                    "sudo docker-compose -p {project} -f {compose} logs --tail {tail_count}"
                );

                if dry_run {
                    println!("  $ {cmd} (on {mode}-{cluster})");
                } else {
                    logs_with_search(mode, cluster, &cmd)?;
                }
            } else {
                // Show logs from specific service with interactive search
                let service_name = &container[0];
                ui::heading(&format!("Deploy {} logs ({}, last {})", cluster, service_name, tail_count));

                let cmd = format!(
                    "sudo docker-compose -p {project} -f {compose} logs --tail {tail_count} {service_name}"
                );

                if dry_run {
                    println!("  $ {cmd} (on {mode}-{cluster})");
                } else {
                    logs_with_search(mode, cluster, &cmd)?;
                }
            }
            Ok(())
        }
        _ => {
            Err(anyhow!(
                "tool action '{:?}' not supported for net clusters (only logs is supported via SSM)",
                action
            ))
        }
    }
}

/// Parallel fan-out for multi-container runs.
fn run_parallel(
    resolved: Vec<clusters::ResolvedContainer>,
    cfg: &clusters::ClusterConfig,
    cluster: &str,
    is_full_bulk: bool,
    mode: Option<config::Mode>,
    yes: bool,
    dry_run: bool,
    no_cache: bool,
    root: &std::path::Path,
    versions_seen: &std::collections::HashMap<String, String>,
) -> Result<()> {
    for r in &resolved {
        ui::heading(&format!("{} status", r.name));
        if git::is_repo(&r.source) {
            println!("{}", git::run_optional(&r.source, &["status", "--short", "--branch"]));
        }
    }

    ui::heading("Ready to Deploy");
    let names: Vec<&str> = resolved.iter().map(|r| r.name.as_str()).collect();
    println!("  containers -> {}", names.join(", "));
    ui::require_confirm("Proceed with build?", yes)?;

    ui::heading("Building images");
    let mut build_handles = vec![];
    for r in resolved.iter().cloned() {
        let name = r.name.clone();
        let m = mode;
        let handle = thread::spawn(move || build_container(&r, m, dry_run, no_cache));
        build_handles.push((name, handle));
    }
    let mut build_errors = vec![];
    for (name, handle) in build_handles {
        match handle.join() {
            Ok(Ok(())) => ui::good(&format!("  ✓ {name}")),
            Ok(Err(e)) => {
                build_errors.push((name.clone(), e));
                ui::warn(&format!("  ✗ {name}"));
            }
            Err(_) => {
                build_errors.push((name.clone(), anyhow!("thread panicked")));
                ui::warn(&format!("  ✗ {name} (thread panic)"));
            }
        }
    }
    if !build_errors.is_empty() {
        let errors = build_errors
            .iter()
            .map(|(n, e)| format!("  {n}: {e}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(anyhow!("Build failed:\n{errors}"));
    }

    // Only push/deploy if mode is set (i.e., --net was specified)
    if mode.is_none() {
        ui::good("images built and ready (use --net to deploy remotely)");
        return Ok(());
    }

    if !ui::confirm("Push the images?", yes)? {
        ui::warn("images built but NOT pushed");
        return Ok(());
    }

    // For ECS clusters, handle push/deploy differently
    if cfg.environment == "ecs" {
        if !ui::confirm("Push and deploy to ECS?", yes)? {
            ui::warn("containers were NOT pushed or deployed");
            return Ok(());
        }

        ui::heading("Pushing to ECR and deploying");
        let mut ecs_handles = vec![];
        for r in resolved.iter().cloned() {
            let name = r.name.clone();
            let m = mode;
            let c = cfg.clone();
            let handle = thread::spawn(move || ecs_deploy_container(&r, &c, m, yes, dry_run));
            ecs_handles.push((name, handle));
        }
        let mut ecs_errors = vec![];
        for (name, handle) in ecs_handles {
            match handle.join() {
                Ok(Ok(())) => {
                    ui::good(&format!("  ✓ {name}"));
                    let target = format!("deploy:{}/{}", cluster, &name);
                    let mode_str = mode.map(|m| m.to_string()).unwrap_or_else(|| "unknown".to_string());
                    if let Some(v) = versions_seen.get(&name) {
                        if let Some(m) = mode {
                            if m == config::Mode::Prod {
                                version::record(root, &target, v, "prod", dry_run)?;
                            }
                        }
                        version::log_deployment(root, &target, Some(v), &mode_str, dry_run)?;
                    } else {
                        // Log deployment even for unversioned containers
                        version::log_deployment(root, &target, None, &mode_str, dry_run)?;
                    }
                }
                Ok(Err(e)) => {
                    ecs_errors.push((name.clone(), e));
                    ui::warn(&format!("  ✗ {name}"));
                }
                Err(_) => {
                    ecs_errors.push((name.clone(), anyhow!("thread panicked")));
                    ui::warn(&format!("  ✗ {name} (thread panic)"));
                }
            }
        }
        if !ecs_errors.is_empty() {
            let errors = ecs_errors
                .iter()
                .map(|(n, e)| format!("  {n}: {e}"))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(anyhow!("ECS deployment failed:\n{errors}"));
        }
    } else {
        // Net clusters: push to Docker Hub and deploy via SSM
        ui::heading("Pushing images");
        let mut push_handles = vec![];
        for r in resolved.iter().cloned() {
            let name = r.name.clone();
            let m = mode;
            let handle = thread::spawn(move || push_container(&r, m, dry_run));
            push_handles.push((name, handle));
        }
        let mut push_errors = vec![];
        for (name, handle) in push_handles {
            match handle.join() {
                Ok(Ok(())) => ui::good(&format!("  ✓ {name}")),
                Ok(Err(e)) => {
                    push_errors.push((name.clone(), e));
                    ui::warn(&format!("  ✗ {name}"));
                }
                Err(_) => {
                    push_errors.push((name.clone(), anyhow!("thread panicked")));
                    ui::warn(&format!("  ✗ {name} (thread panic)"));
                }
            }
        }
        if !push_errors.is_empty() {
            let errors = push_errors
                .iter()
                .map(|(n, e)| format!("  {n}: {e}"))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(anyhow!("Push failed:\n{errors}"));
        }

        if !ui::confirm("Deploy the containers?", yes)? {
            ui::warn("containers were NOT deployed");
            return Ok(());
        }

        ui::heading("Deploying containers");
        let mut deploy_handles = vec![];
        for r in resolved.iter().cloned() {
            let name = r.name.clone();
            let m = mode;
            let c = cluster.to_string();
            let handle = thread::spawn(move || deploy_container(&r, m, &c, yes, dry_run));
            deploy_handles.push((name, handle));
        }
        let mut deploy_errors = vec![];
        for (name, handle) in deploy_handles {
            match handle.join() {
                Ok(Ok(())) => {
                    ui::good(&format!("  ✓ {name}"));
                    let target = format!("deploy:{}/{}", cluster, &name);
                    let mode_str = mode.map(|m| m.to_string()).unwrap_or_else(|| "unknown".to_string());
                    if let Some(v) = versions_seen.get(&name) {
                        if let Some(m) = mode {
                            if m == config::Mode::Prod {
                                version::record(root, &target, v, "prod", dry_run)?;
                            }
                        }
                        version::log_deployment(root, &target, Some(v), &mode_str, dry_run)?;
                    } else {
                        // Log deployment even for unversioned containers
                        version::log_deployment(root, &target, None, &mode_str, dry_run)?;
                    }
                }
                Ok(Err(e)) => {
                    deploy_errors.push((name.clone(), e));
                    ui::warn(&format!("  ✗ {name}"));
                }
                Err(_) => {
                    deploy_errors.push((name.clone(), anyhow!("thread panicked")));
                    ui::warn(&format!("  ✗ {name} (thread panic)"));
                }
            }
        }
        if !deploy_errors.is_empty() {
            let errors = deploy_errors
                .iter()
                .map(|(n, e)| format!("  {n}: {e}"))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(anyhow!("Deploy failed:\n{errors}"));
        }
    }

    if is_full_bulk {
        ui::heading("Cleaning up");
        docker::prune(dry_run)?;
    }

    Ok(())
}

fn build_container(r: &clusters::ResolvedContainer, mode: Option<config::Mode>, dry_run: bool, no_cache: bool) -> Result<()> {
    let image = if let Some(m) = mode {
        docker::image_name(&r.name, m.docker_tag())
    } else {
        format!("vapi-{}:latest", r.name)
    };

    // For ECS containers, use the configured dockerfile; for others use default.
    let dockerfile = if let Some(ecs) = &r.ecs {
        ecs.dockerfile.as_str()
    } else {
        "Dockerfile"
    };

    // Build CONFIG_PATH arg if configured
    let config_arg = r.config.as_ref().and_then(|config_path| {
        let context_root = r.source.join(&r.build_context).canonicalize().ok()?;
        let config_abs = config_path.canonicalize().ok()?;
        pathdiff::diff_paths(&config_abs, &context_root)
            .map(|rel| format!("CONFIG_PATH={}", rel.display()))
    });

    // Combine CONFIG_PATH (if present) with ECS build_args (if present)
    let mut build_args: Vec<String> = vec![];
    if let Some(config) = config_arg {
        build_args.push(config);
    }
    if let Some(ecs) = &r.ecs {
        build_args.extend(ecs.build_args.clone());
    }

    let build_args_str: Vec<&str> = build_args.iter().map(|s| s.as_str()).collect();
    docker::build_with_dockerfile_and_args(&r.source, &image, &r.build_context, dockerfile, &build_args_str, dry_run, no_cache)
}

fn push_container(r: &clusters::ResolvedContainer, mode: Option<config::Mode>, dry_run: bool) -> Result<()> {
    let image = if let Some(m) = mode {
        docker::image_name(&r.name, m.docker_tag())
    } else {
        format!("vapi-{}:latest", r.name)
    };
    docker::push(&image, dry_run)
}

fn deploy_container(r: &clusters::ResolvedContainer, mode: Option<config::Mode>, cluster: &str, yes: bool, dry_run: bool) -> Result<()> {
    let m = mode.ok_or_else(|| anyhow!("mode required for deployment"))?.for_deployment();
    let image = docker::image_name(&r.name, m.docker_tag());
    aws::docker_remote::pull(m, cluster, &image, yes, dry_run)?;
    aws::docker_remote::compose_up(m, cluster, cluster, &r.name, yes, dry_run)?;
    aws::docker_remote::prune(m, cluster, yes, dry_run)
}

fn ecs_deploy_container(
    r: &clusters::ResolvedContainer,
    cfg: &clusters::ClusterConfig,
    mode: Option<config::Mode>,
    _yes: bool,
    dry_run: bool,
) -> Result<()> {
    let ecs_cfg = cfg.ecs.as_ref().ok_or_else(|| anyhow!("ECS cluster missing ecs configuration"))?;
    let container_ecs = r.ecs.as_ref().ok_or_else(|| anyhow!("container '{}' missing ecs configuration", r.name))?;

    // Determine region
    let region = if let Some(r) = &ecs_cfg.region {
        r.clone()
    } else {
        ssm::get_region()?
    };

    // Determine account ID
    let account = if let Some(a) = &ecs_cfg.account {
        a.clone()
    } else {
        aws::account_id()?
    };

    // Build ECR registry URL
    let registry = aws::ecr_registry(&account, &region);

    // Build image tag for local build
    let m = mode.ok_or_else(|| anyhow!("mode required for ECS deployment"))?.for_deployment();
    let local_image = docker::image_name(&r.name, m.docker_tag());

    // Determine ECR repo and service names (default to container name)
    let ecr_repo = container_ecs.ecr_repo.as_deref().unwrap_or(&r.name);
    let service = container_ecs.service.as_deref().unwrap_or(&r.name);

    // Build ECR image name
    let ecr_image = format!("{}/{}", registry, ecr_repo);
    let ecr_image_tagged = format!("{}:{}", ecr_image, m.docker_tag());

    // ECR login
    aws::ecr_login(&registry, &region, dry_run)?;

    // Tag the local image with ECR registry
    docker::tag(&local_image, &ecr_image_tagged, dry_run)?;

    // Push to ECR
    docker::push(&ecr_image_tagged, dry_run)?;

    // Resolve cluster name with mode template
    let cluster_name = ecs_cfg.cluster.as_str();
    let cluster_name = if cluster_name.is_empty() {
        format!("vhp-cluster-{}", m.docker_tag())
    } else {
        cluster_name.replace("{mode}", m.docker_tag())
    };

    // Force new ECS deployment
    aws::ecs_force_deploy(&cluster_name, service, &region, dry_run)?;

    Ok(())
}

pub fn run(args: DeployArgs) -> Result<()> {
    let DeployArgs {
        cluster,
        container,
        tools,
        tail,
        yes,
        dry_run,
        no_cache,
        details,
    } = args;

    let cluster_configs = clusters::load()?;

    let cfg = clusters::find(&cluster_configs, &cluster)?;

    // Validate that the cluster environment is recognized
    if cfg.environment != "local" && cfg.environment != "net" && cfg.environment != "ecs" {
        return Err(anyhow!(
            "cluster '{}' has unknown environment '{}' (expected: local, net, or ecs)",
            cluster, cfg.environment
        ));
    }

    if details {
        return show_deploy_details(&cluster, &container);
    }

    if let Some(action) = tools {
        match (&action, cfg.environment.as_str()) {
            (ToolsAction::List, "net") | (ToolsAction::List, "local") => {
                // list shows config info for net/local clusters
                return tools_cmd(&cfg, &cluster, action, &container, tail, dry_run);
            }
            (ToolsAction::Logs, "net") => {
                // logs works for net via SSM
                return tools_cmd(&cfg, &cluster, action, &container, tail, dry_run);
            }
            (ToolsAction::Init, "ecs") | (ToolsAction::List, "ecs") => {
                // init and list work for ecs clusters
                return tools_cmd(&cfg, &cluster, action, &container, tail, dry_run);
            }
            (_, "local") => {
                // all tools work for local
                return tools_cmd(&cfg, &cluster, action, &container, tail, dry_run);
            }
            _ => {
                return Err(anyhow!(
                    "--tools {:?} not supported for {} clusters",
                    action, cfg.environment
                ));
            }
        }
    }

    let root = paths::project_vhp()?;
    let vapi_root = paths::project_vapi()?;
    let settings = config::Settings::load()?;
    let mode = settings.get_mode()?;

    let is_full_bulk = container.is_empty();
    let targets: Vec<String> = if is_full_bulk {
        cfg.bulk_containers().map(String::from).collect()
    } else {
        container.clone()
    };

    if targets.is_empty() {
        return Err(anyhow!(
            "no containers to build in cluster '{}' (all marked bulk: false)",
            cluster
        ));
    }

    ui::heading(&format!("Deploy {}", cluster));
    println!("  containers -> {}", targets.join(", "));
    println!("       mode -> {}", cfg.environment);

    // Resolve all containers upfront
    let resolved: Vec<clusters::ResolvedContainer> = targets
        .iter()
        .map(|name| cfg.resolve(name, &root, &vapi_root))
        .collect::<Result<_>>()?;

    // Read versions from containers for logging/checking (all environments/modes)
    let mut versions_seen = std::collections::HashMap::new();
    let mut unversioned = vec![];

    if cfg.environment == "net" || cfg.environment == "ecs" {
        for r in &resolved {
            match version::pkg_version(&r.source) {
                Ok(v) => {
                    // Check versions only in prod mode
                    if mode == config::Mode::Prod {
                        version::check_table_only(&root, &format!("deploy:{}/{}", cluster, r.name), &v)?;
                    }
                    versions_seen.insert(r.name.clone(), v);
                }
                Err(_) => {
                    unversioned.push(&r.name);
                }
            }
        }
    }

    // Warn about unversioned containers
    if !unversioned.is_empty() && (cfg.environment == "net" || cfg.environment == "ecs") {
        let names: Vec<String> = unversioned.into_iter().map(|s| s.to_string()).collect();
        ui::warn(&format!("  containers without package.json (cannot be versioned): {}", names.join(", ")));
        ui::require_confirm("Proceed without version gating for these containers?", yes)?;
    }

    // Multi-container: use parallel path
    if resolved.len() > 1 {
        let mode = if cfg.environment == "net" || cfg.environment == "ecs" {
            Some(settings.get_mode()?.for_deployment())
        } else {
            None
        };
        if cfg.environment == "net" || cfg.environment == "ecs" {
            for r in &resolved {
                git::require_mode_branch(&r.source, mode.unwrap())?;
            }
        }
        return run_parallel(resolved, &cfg, &cluster, is_full_bulk, mode, yes, dry_run, no_cache, &root, &versions_seen);
    }

    // Single container: sequential path
    for r in &resolved {
        // Determine image name and build dockerfile/args based on environment
        let dockerfile = if let Some(ecs) = &r.ecs {
            ecs.dockerfile.as_str()
        } else {
            "Dockerfile"
        };

        let image = if cfg.environment == "net" || cfg.environment == "ecs" {
            let mode = settings.get_mode()?.for_deployment();
            docker::image_name(&r.name, mode.docker_tag())
        } else {
            format!("vapi-{}:latest", r.name)
        };

        ui::heading(&format!("Building {}", r.name));

        // Build CONFIG_PATH arg if configured
        let config_arg = r.config.as_ref().and_then(|config_path| {
            let context_root = r.source.join(&r.build_context).canonicalize().ok()?;
            let config_abs = config_path.canonicalize().ok()?;
            pathdiff::diff_paths(&config_abs, &context_root)
                .map(|rel| format!("CONFIG_PATH={}", rel.display()))
        });

        // Combine CONFIG_PATH (if present) with ECS build_args (if present)
        let mut build_args: Vec<String> = vec![];
        if let Some(config) = config_arg {
            build_args.push(config);
        }
        if let Some(ecs) = &r.ecs {
            build_args.extend(ecs.build_args.clone());
        }

        let build_args_str: Vec<&str> = build_args.iter().map(|s| s.as_str()).collect();
        docker::build_with_dockerfile_and_args(&r.source, &image, &r.build_context, dockerfile, &build_args_str, dry_run, no_cache)?;

        if cfg.environment == "net" {
            let mode = settings.get_mode()?;
            let mode_str = mode.to_string();
            git::require_mode_branch(&r.source, mode)?;
            ui::require_confirm(&format!("push and deploy {}?", r.name), yes)?;
            docker::push(&image, dry_run)?;
            let deploy_mode = mode.for_deployment();
            aws::docker_remote::pull(deploy_mode, &cluster, &image, yes, dry_run)?;
            aws::docker_remote::compose_up(deploy_mode, &cluster, &cluster, &r.name, yes, dry_run)?;
            aws::docker_remote::prune(deploy_mode, &cluster, yes, dry_run)?;
            let target = format!("deploy:{}/{}", cluster, r.name);
            if let Some(v) = versions_seen.get(&r.name) {
                if mode == config::Mode::Prod {
                    version::record(&root, &target, v, "prod", dry_run)?;
                }
                version::log_deployment(&root, &target, Some(v), &mode_str, dry_run)?;
            } else {
                // Log deployment even for unversioned containers
                version::log_deployment(&root, &target, None, &mode_str, dry_run)?;
            }
            ui::good(&format!("deployed {}", r.name));
        } else if cfg.environment == "ecs" {
            let mode = settings.get_mode()?;
            let mode_str = mode.to_string();
            git::require_mode_branch(&r.source, mode)?;
            ui::require_confirm(&format!("push and deploy {} to ECS?", r.name), yes)?;
            ecs_deploy_container(&r, &cfg, Some(mode.for_deployment()), yes, dry_run)?;
            let target = format!("deploy:{}/{}", cluster, r.name);
            if let Some(v) = versions_seen.get(&r.name) {
                if mode == config::Mode::Prod {
                    version::record(&root, &target, v, "prod", dry_run)?;
                }
                version::log_deployment(&root, &target, Some(v), &mode_str, dry_run)?;
            } else {
                // Log deployment even for unversioned containers
                version::log_deployment(&root, &target, None, &mode_str, dry_run)?;
            }
            ui::good(&format!("deployed {} to ECS", r.name));
        } else {
            let compose = cfg.compose_path(&root).ok_or_else(|| anyhow!("cluster '{}' has no compose file", cluster))?;
            let compose_dir = compose.parent().ok_or_else(|| {
                anyhow!("compose path has no parent: {}", compose.display())
            })?;
            let project = format!("deploy-{}", cluster);

            proc::run_in(
                compose_dir,
                "sudo",
                &[
                    "docker",
                    "compose",
                    "-f",
                    compose.to_str().ok_or_else(|| anyhow!("invalid compose path"))?,
                    "-p",
                    &project,
                    "up",
                    "-d",
                    &r.name,
                ],
                dry_run,
            )?;

            ui::good(&format!("running {} locally", r.name));
        }
    }

    Ok(())
}

fn list_ecr_repos(region: &str, details: bool) -> Result<()> {
    let repos = proc::capture(
        "aws",
        &[
            "ecr",
            "describe-repositories",
            "--region",
            region,
            "--query",
            "repositories[]",
            "--output",
            "json",
        ],
    )?;

    if repos.trim() == "[]" {
        println!("  (none)");
        return Ok(());
    }

    let repos_json: serde_json::Value = serde_json::from_str(&repos)?;
    if let Some(arr) = repos_json.as_array() {
        for repo in arr {
            let name = repo["repositoryName"].as_str().unwrap_or("?");
            let uri = repo["repositoryUri"].as_str().unwrap_or("?");

            if details {
                let image_count = repo["imageScanningConfiguration"]["scanOnPush"]
                    .as_bool()
                    .unwrap_or(false);
                println!("  {}", name);
                println!("    URI: {}", uri);
                println!("    Scan on push: {}", image_count);
            } else {
                println!("  {}", name);
            }
        }
    }
    Ok(())
}

fn list_ecs_clusters(region: &str, details: bool) -> Result<()> {
    let clusters = proc::capture(
        "aws",
        &[
            "ecs",
            "list-clusters",
            "--region",
            region,
            "--query",
            "clusterArns[]",
            "--output",
            "json",
        ],
    )?;

    if clusters.trim() == "[]" {
        println!("  (none)");
        return Ok(());
    }

    let cluster_arns: Vec<String> = serde_json::from_str(&clusters)?;
    for arn in cluster_arns {
        if let Some(name) = arn.split('/').last() {
            if details {
                let stats = proc::capture(
                    "aws",
                    &[
                        "ecs",
                        "describe-clusters",
                        "--clusters",
                        name,
                        "--region",
                        region,
                        "--query",
                        "clusters[0]",
                        "--output",
                        "json",
                    ],
                );

                println!("  {}", name);
                if let Ok(stats_json_str) = stats {
                    if let Ok(stats_json) = serde_json::from_str::<serde_json::Value>(&stats_json_str) {
                        let running = stats_json["runningCount"].as_i64().unwrap_or(0);
                        let pending = stats_json["pendingCount"].as_i64().unwrap_or(0);
                        let registered = stats_json["registeredContainerInstancesCount"].as_i64().unwrap_or(0);
                        println!("    Running tasks: {}", running);
                        println!("    Pending tasks: {}", pending);
                        println!("    Container instances: {}", registered);
                    }
                }
            } else {
                println!("  {}", name);
            }
        }
    }
    Ok(())
}

fn list_task_definitions(region: &str, details: bool) -> Result<()> {
    let tasks = proc::capture(
        "aws",
        &[
            "ecs",
            "list-task-definitions",
            "--region",
            region,
            "--query",
            "taskDefinitionArns[]",
            "--output",
            "json",
        ],
    )?;

    if tasks.trim() == "[]" {
        println!("  (none)");
        return Ok(());
    }

    let task_arns: Vec<String> = serde_json::from_str(&tasks)?;
    for arn in task_arns {
        if let Some(full_name) = arn.split('/').last() {
            if details {
                let def = proc::capture(
                    "aws",
                    &[
                        "ecs",
                        "describe-task-definition",
                        "--task-definition",
                        full_name,
                        "--region",
                        region,
                        "--query",
                        "taskDefinition",
                        "--output",
                        "json",
                    ],
                );

                println!("  {}", full_name);
                if let Ok(def_json_str) = def {
                    if let Ok(def_json) = serde_json::from_str::<serde_json::Value>(&def_json_str) {
                        let cpu = def_json["cpu"].as_str().unwrap_or("?");
                        let memory = def_json["memory"].as_str().unwrap_or("?");
                        println!("    CPU: {}", cpu);
                        println!("    Memory: {}", memory);

                        if let Some(containers) = def_json["containerDefinitions"].as_array() {
                            for (i, container) in containers.iter().enumerate() {
                                let container_name = container["name"].as_str().unwrap_or("?");
                                let image = container["image"].as_str().unwrap_or("?");
                                println!("    Container {}: {}", i, container_name);
                                println!("      Image: {}", image);

                                if let Some(env) = container["environment"].as_array() {
                                    if !env.is_empty() {
                                        println!("      Environment:");
                                        for var in env {
                                            let key = var["name"].as_str().unwrap_or("?");
                                            let value = var["value"].as_str().unwrap_or("?");
                                            println!("        {} = {}", key, value);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                println!("  {}", full_name);
            }
        }
    }
    Ok(())
}

fn show_deploy_details(cluster: &str, container: &[String]) -> Result<()> {
    let root = paths::project_vhp()?;
    let deployments = version::get_deploy_details(&root, cluster, container)?;

    if deployments.is_empty() {
        ui::info(&format!("No deployments found for cluster '{}'", cluster));
        return Ok(());
    }

    ui::heading(&format!("Deployment History: {}", cluster));

    // Group by target
    let mut targets_map: std::collections::HashMap<String, Vec<_>> = std::collections::HashMap::new();
    for (target, date, version, mode) in deployments {
        targets_map.entry(target).or_insert_with(Vec::new).push((date, version, mode));
    }

    // Print grouped by target
    for (target, deploys) in targets_map {
        println!("\n{}:", target);
        for (date, version, mode) in deploys {
            let version_str = version.map(|v| format!(" v{}", v)).unwrap_or_default();
            println!("  {}@{}{}", mode, date, version_str);
        }
    }

    Ok(())
}
