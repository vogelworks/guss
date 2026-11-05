# GUSS
One Rust CLI, **`guss`**, for building, deploying, cluster access, docs search, and
git-flow branching.

## Goals
The project spawned from getting tired of using sparse bash scripts to managed developement environements, build apps/services, manage clusters, and connect to servers. Rust was not an obvious choice, but I really enjoy strong types and have enjoyed most of the languages features. Switching accomplished the following immediatly:
- brought what was a 5-10 minutes process to deploy all services to under a minute mostly through multi-treading.
- Allowed for configurable developement environments to suite individual needs
- Provided more control pipelines through use of "modes" 

## Resources
- [docs/Help/guss-cli.md](../docs/Help/guss-cli.md) - Usage & command reference 
- [SETUP.md](SETUP.md) - get a dev build running
- [DEVELOPING.md](DEVELOPING.md) - workflow, testing, house conventions
- [TECHNICAL.md](TECHNICAL.md) - architecture and internals
- [crates/project/DESIGN.md](crates/project/DESIGN.md) - `guss project` design

## Road Map

- Continue AWS integration to move from deployment through docker compose to ECS
- Basic logging first with connection and visual logs (currently possible just not kind), and ending with log analysis
- Teraform integration
- Generalize documentation | currently there is specific documentation for specific setups. Want to move to declarative functions to document with structure around what gets included and what chars to recognize (depending on project/file).
- 

## Prerequisites

- **Rust 1.97.0+** — install from [rustup.rs](https://rustup.rs)
  - The workspace pins 1.97.0 in `rust-toolchain.toml`; rustup handles it automatically
- **Git** with SSH keys set up for GitHub
- **Docker** (if building/running containerized services)
- **AWS CLI** configured with credentials (for deployment workflows)

## Initial Setup

### 1. Clone the repo with submodules

```bash
git clone --recurse-submodules git@github.com:guss1946/project-guss.git
cd project-guss/guss
```

If you cloned without `--recurse-submodules`, run:
```bash
git submodule update --init --recursive
```

### 2. Build and verify

```bash
cargo build --workspace
cargo test --workspace
```

All tests should pass. If you see dependency issues, run `cargo update` to refresh the lock file.

### 3. Configure the CLI (set deployment mode)

Most workflows need a deployment mode. Run:
```bash
guss net init
# or to set manually:
guss net mode dev    # or prod / test
```

This writes `~/.config/guss/config.toml` with the mode and other settings.

**Config file reference** (`~/.config/guss/config.toml`):
```toml
root = "/path/to/project-guss"      # repo location (auto-detected if absent)
mode = "dev"                        # deployment mode: dev, prod, or test
vapi_root = "/path/to/project-vapi" # vapi services location (optional)
ssh_key = "id_ed25519"              # SSH key for docker builds (defaults to id_ed25519)
```

Most fields are optional. The permission gate is off until `[auth]` is set.

### 4. Set up shell aliases (optional but recommended)

```bash
# Add guss-dev shortcuts to ~/.bashrc or ~/.zshrc:
# g <args>         → build and run (always current)
# gg <args>        → run prebuilt binary (instant, stale until rebuild)
# gb               → cargo build --workspace
# gt               → cargo test --workspace
```

See [DEVELOPING.md](DEVELOPING.md) for the full setup block to copy.

## Working on guss

### Building and testing

```bash
cargo build --workspace              # build everything
cargo test --workspace               # run all tests
cargo test -p <crate> --lib         # test one crate (e.g., -p deploy)
```

### Running the CLI

From any directory:
```bash
cargo run --manifest-path=~/path/to/project-guss/guss/Cargo.toml -- <command>
# Or with the v alias:
v <command>
```

### Before committing

1. Run `cargo build && cargo test` — all tests green
2. Check [DEVELOPING.md](DEVELOPING.md) for the pre-commit checklist
3. Verify you haven't broken first-run experience (see "Isolating from your real machine" in DEVELOPING.md)

## Code Structure

- **cli/** — single binary entry point (`main.rs`), dispatch only
- **crates/** — one crate per task (net, build, deploy, setup, project, guss, etc.)
  - **common/** — shared code: proc wrappers, paths, config, auth, ui, db, clusters
  - **aws/** — AWS operations (SSM, ECR, ECS, etc.)
  - Each task crate has `lib.rs` with `Args`, `Cmd` enum, and `run()` function

See [TECHNICAL.md](TECHNICAL.md) and [DEVELOPING.md](DEVELOPING.md) for architecture details and conventions.

