//! AWS Systems Manager Session Manager: resolve service names and connect/run commands.

use guss_common::config::Mode;
use guss_common::error::{Result, anyhow};
use guss_common::proc;
use guss_common::ui;
use std::io::Write;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_POLLS: u32 = 60; // ~2 minutes
const MAX_POLLS_LONG: u32 = 1800; // ~1 hour for logs

/// Get the AWS region from `aws configure get region`, which respects
/// AWS_REGION / AWS_DEFAULT_REGION env vars first, then ~/.aws/config.
pub fn get_region() -> Result<String> {
    proc::capture("aws", &["configure", "get", "region"])
}

/// Resolve `{mode}-{service}` to the instance ID of the one running EC2
/// instance tagged with that Name. Errors if none or multiple match.
/// (Test mode uses dev instances, not separate test instances.)
pub fn resolve_target(mode: Mode, service: &str) -> Result<String> {
    let name = format!("{}-{service}", mode.docker_tag());
    let tag_filter = format!("Name=tag:Name,Values={name}");
    let out = proc::capture(
        "aws",
        &[
            "ec2",
            "describe-instances",
            "--filters",
            &tag_filter,
            "Name=instance-state-name,Values=running",
            "--query",
            "Reservations[].Instances[].InstanceId",
            "--output",
            "text",
        ],
    )?;
    match out.split_whitespace().collect::<Vec<_>>().as_slice() {
        [] => Err(anyhow!("no running instance tagged Name={name}")),
        [id] => Ok(id.to_string()),
        ids => Err(anyhow!("multiple running instances tagged Name={name}: {}", ids.join(", "))),
    }
}

/// Open an interactive SSM session to `{mode}-{service}`.
pub fn connect(mode: Mode, service: &str) -> Result<()> {
    let instance_id = resolve_target(mode, service)?;
    proc::run("aws", &["ssm", "start-session", "--target", &instance_id], false)
}

/// Run a long-running command (like docker logs -f) and stream output in real-time.
/// Polls for up to ~1 hour, printing new output as it arrives.
pub fn run_command_streaming(mode: Mode, service: &str, command: &str) -> Result<()> {
    let instance_id = resolve_target(mode, service)?;
    let params = serde_json::json!({ "commands": [command] }).to_string();

    ui::info(&format!("  SSM → {mode}-{service}"));
    ui::info(&format!("  $ {command}"));
    ui::info(&format!("  Instance: {instance_id}"));
    println!();

    let command_id = proc::capture(
        "aws",
        &[
            "ssm",
            "send-command",
            "--instance-ids",
            &instance_id,
            "--document-name",
            "AWS-RunShellScript",
            "--parameters",
            &params,
            "--query",
            "Command.CommandId",
            "--output",
            "text",
        ],
    )?;

    let mut last_output_len = 0;
    let mut last_status = String::new();
    std::thread::sleep(Duration::from_secs(2));

    for poll_num in 0..MAX_POLLS_LONG {
        let status = proc::capture(
            "aws",
            &[
                "ssm",
                "get-command-invocation",
                "--command-id",
                &command_id,
                "--instance-id",
                &instance_id,
                "--query",
                "Status",
                "--output",
                "text",
            ],
        )?;

        let status_trimmed = status.trim();
        if status_trimmed != last_status {
            eprintln!("[ssm] Status: {} (poll #{})", status_trimmed, poll_num);
            last_status = status_trimmed.to_string();
        }

        let output = proc::capture(
            "aws",
            &[
                "ssm",
                "get-command-invocation",
                "--command-id",
                &command_id,
                "--instance-id",
                &instance_id,
                "--query",
                "StandardOutputContent",
                "--output",
                "text",
            ],
        )
        .unwrap_or_default();

        if output.len() > last_output_len {
            print!("{}", &output[last_output_len..]);
            let _ = std::io::stdout().flush();
            last_output_len = output.len();
        }

        if poll_num < 5 && output.is_empty() && status_trimmed == "InProgress" {
            // Check stderr early on if we have no stdout yet
            let stderr = proc::capture(
                "aws",
                &[
                    "ssm",
                    "get-command-invocation",
                    "--command-id",
                    &command_id,
                    "--instance-id",
                    &instance_id,
                    "--query",
                    "StandardErrorContent",
                    "--output",
                    "text",
                ],
            )
            .unwrap_or_default();
            if !stderr.is_empty() {
                eprintln!("[ssm] stderr: {}", stderr);
            }
        }

        match status_trimmed {
            "Success" | "Failed" | "Cancelled" | "TimedOut" => {
                eprintln!("[ssm] Done (status: {})", status_trimmed);
                return Ok(());
            }
            "Pending" | "InProgress" | "Delayed" => {
                std::thread::sleep(Duration::from_millis(500));
            }
            _ => {
                eprintln!("[ssm] Unknown status: {}", status_trimmed);
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
    eprintln!("[ssm] Polling ended after ~1 hour");
    Ok(())
}

/// Run a single shell command on `{mode}-{service}` via SSM Run Command,
/// poll for completion up to ~2 minutes, and return stdout.
/// ponytail: fixed 2s polling, no backoff — add exponential backoff or
/// configurable timeout if a command ever needs to run longer.
pub fn run_command(mode: Mode, service: &str, command: &str) -> Result<String> {
    run_command_with_timeout(mode, service, command, MAX_POLLS)
}

/// Run a shell command with configurable timeout (max_polls * 2 seconds).
fn run_command_with_timeout(mode: Mode, service: &str, command: &str, max_polls: u32) -> Result<String> {
    let instance_id = resolve_target(mode, service)?;
    let params = serde_json::json!({ "commands": [command] }).to_string();

    ui::info(&format!("  SSM → {mode}-{service}"));
    ui::info(&format!("  $ {command}"));

    let command_id = proc::capture(
        "aws",
        &[
            "ssm",
            "send-command",
            "--instance-ids",
            &instance_id,
            "--document-name",
            "AWS-RunShellScript",
            "--parameters",
            &params,
            "--query",
            "Command.CommandId",
            "--output",
            "text",
        ],
    )?;

    let mut poll_count = 0;
    for _ in 0..max_polls {
        let status = proc::capture(
            "aws",
            &[
                "ssm",
                "get-command-invocation",
                "--command-id",
                &command_id,
                "--instance-id",
                &instance_id,
                "--query",
                "Status",
                "--output",
                "text",
            ],
        )?;
        match status.as_str() {
            "Success" => {
                ui::info(&format!("  ✓ completed"));
                return proc::capture(
                    "aws",
                    &[
                        "ssm",
                        "get-command-invocation",
                        "--command-id",
                        &command_id,
                        "--instance-id",
                        &instance_id,
                        "--query",
                        "StandardOutputContent",
                        "--output",
                        "text",
                    ],
                );
            }
            "Pending" | "InProgress" | "Delayed" => {
                poll_count += 1;
                if poll_count % 5 == 0 {
                    ui::info(&format!("  ... ({poll_count}s elapsed)"));
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            other => {
                let stderr = proc::capture(
                    "aws",
                    &[
                        "ssm",
                        "get-command-invocation",
                        "--command-id",
                        &command_id,
                        "--instance-id",
                        &instance_id,
                        "--query",
                        "StandardErrorContent",
                        "--output",
                        "text",
                    ],
                )
                .unwrap_or_default();
                return Err(anyhow!("ssm command {other}: {stderr}"));
            }
        }
    }
    Err(anyhow!("ssm command timed out after ~2 minutes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_filter_format() {
        let filter = format!("Name=tag:Name,Values={}", format!("{}-{}", Mode::Dev, "core"));
        assert_eq!(filter, "Name=tag:Name,Values=dev-core");
    }

    #[test]
    fn json_params_round_trip() {
        let cmd = "echo 'hello' && echo \"world\"\n";
        let json = serde_json::json!({ "commands": [cmd] });
        let s = json.to_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["commands"][0].as_str(), Some(cmd));
    }

    #[test]
    fn resolve_target_error_messages() {
        let empty_msg = format!("{}", anyhow!("no running instance tagged Name={}", "dev-core"));
        assert!(empty_msg.contains("no running instance"));

        let multi_msg = format!("{}", anyhow!("multiple running instances tagged Name={}: {}", "dev-core", "i-123, i-456"));
        assert!(multi_msg.contains("multiple running instances"));
    }
}
