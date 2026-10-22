# guss Tools — Technical Reference

Architecture, implementation, and development guide for the `guss` CLI.

---

## Architecture

### One Binary, One Dispatch Point

```
guss (cli/src/main.rs)
├── net:        cluster connection + branch management
├── build:      app builds + publishing
├── deploy:     service images + rollout (including ECS)
├── docs:       API documentation
├── setup:      install, update, release, login
├── project:    git-flow branching
└── guss:       interactive shell
```

Every task is its own crate under `crates/`. They share:

- `common/` — subprocess, paths, config, auth, git, ssh, docker, UI
- `cli/src/main.rs` — the **only binary**, wiring + single auth gate

No global state, no singletons. Everything is pure functions passed config values.

### Directory Layout

```
guss/
├── Cargo.toml                workspace: deps in [workspace.dependencies], pinned once
├── rust-toolchain.toml       1.97.0 pinned; auto-applies
├── cli/src/main.rs           ONLY binary entry point
└── crates/
    ├── common/src/
    │   ├── proc.rs           all subprocess calls route here
    │   ├── paths.rs          all filesystem paths (repo + config)
    │   ├── config.rs         ~/.config/guss/config.toml (settings)
    │   ├── git.rs            git operations
    │   ├── docker.rs         docker build/push/run
    │   ├── clusters.rs       cluster definitions and resolution
    │   ├── ssh/ssm.rs        SSH + AWS Systems Manager
    │   ├── auth.rs           permission gate (fails open)
    │   ├── ui.rs             headings, colors, confirm()
    │   ├── error.rs          error types
    │   └── db.rs             local JSON persistence
    ├── net/src/lib.rs        cluster connection, branch switching
    ├── build/src/lib.rs      app builds, publishing
    ├── deploy/src/lib.rs     service images, rollout (including ECS)
    ├── docs/src/lib.rs       API documentation
    ├── setup/src/            install, update, release
    ├── project/src/          git-flow branching
    ├── guss/src/             interactive shell
    └── ...
```

---

## Configuration & State

### Settings: `~/.config/guss/config.toml`

Located **outside any checkout** so a machine can support multiple checkouts.

```toml
root                   = "/path/to/project-guss"    # primary checkout
vapi_root              = "/path/to/project-vapi"   # optional
mode                   = "dev|test|prod"           # deployment mode
ssh_key                = "id_ed25519"              # optional (default)
tracked_repos          = ["~/repo1", "~/repo2"]   # optional: repos to track with mode

[auth]                                             # optional: enable permissions
login_host = "..."
server_key = "..."

[release]                                          # optional: releases
base_url = "..."
ssh_host = "..."
remote_dir = "..."
public_key = "..."
```

Settings are **never cached** — `setup init` writes the file, then the same process reads it immediately. Cached settings break when initialization writes them mid-run.

### Key Directories

| What | Location | Writable | Purpose |
|------|----------|----------|---------|
| Config | `~/.config/guss/` | Yes | Settings, keys, tokens |
| Device key | `~/.config/guss/device.key` (0600) | Yes | ed25519 pair (auth login) |
| Session token | `~/.config/guss/token.toml` (0600) | Yes | Cached 30-day token |
| Release key | `~/.config/guss/release.key` (0600) | Yes | Signing key (publish only) |
| SSH keys | `~/.ssh/` | Yes | Git + Docker SSH (assumed to exist) |

---

## Deployment Modes & Branch Management

### Mode System

Three modes, each with a corresponding branch:

| Mode | Branch | Use |
|------|--------|-----|
| **dev** | `dev` | development, frequent rebuilds |
| **prod** | `prod` | production, requires version tag |
| **test** | flexible | testing, any branch except `prod` |

### Mode Switching: `guss net mode <mode>`

When mode changes, the following repos are checked out and pulled:

1. **Automatic (from project root):** webserver, network, applications
2. **Configured:** any repos in `tracked_repos` from config

For **dev/prod modes:** exact branch match required.  
For **test mode:** allows any branch except `prod` (which switches to `dev`).

After checkout, `git pull` fetches latest changes.

### Build & Deployment Flow

#### Dev/Test Builds

```
build all dev/test  →  (no git commits)  →  deploy
```

Only the image changes; code commits are skipped.

#### Prod Builds

```
build all prod <version>  →  deploy to prod  →  git commits:
  - applications repo: commit build changes
  - applications repo: create/merge v<version> branch (if new, create; if exists, merge)
  - applications repo: switch back to prod
  - webserver repo: commit publish changes
  - webserver repo: push to prod
```

Version branches are created off the commit that was deployed. If a version branch already exists, new changes are merged into it.

---

## Command Patterns & Conventions

### Always Use `proc::` for Subprocesses

Never call `std::process::Command` directly. All subprocess calls go through `proc::run`, `proc::run_in`, `proc::capture`, etc.:

```rust
// Echoes command, honors dry-run, turns non-zero into Err
proc::run_in(dir, "git", &["status"], dry_run)?;

// Optional capture (returns stdout)
let output = proc::capture(dir, "git", &["rev-parse", "HEAD"], dry_run)?;
```

Benefits:
- Commands are echoed for transparency
- `--dry-run` is honored in one place
- Non-zero exits become `Err`
- Secrets in output are masked (for logs/CI)

### Always Use `paths::` for Filesystem Paths

Never hardcode or build paths from `$HOME`. Use functions from `paths::`:

```rust
let root = paths::project_guss()?;        // via config or probe
let webserver = paths::webserver_dir()?; // <root>/webserver
let network = paths::network_dir()?;     // <root>/network
```

Why: older code hardcoded `~/project-guss` and silently operated on a different tree when a checkout was elsewhere. `paths::` centralizes the resolution logic.

### `dry_run: bool` is Always Last

Thread the `--dry-run` flag from clap through to `proc::` as the last parameter:

```rust
fn deploy(..., dry_run: bool) -> Result<()> {
    proc::run_in(..., dry_run)?;  // last arg
}
```

Read operations (`git::is_clean`, `git::current_branch`) always execute; only mutations respect dry-run.

### Confirmations via `ui::require_confirm`

Destructive operations go through `require_confirm`:

```rust
ui::require_confirm("Deploy service?", yes)?;
```

Behavior:
- With `--yes`: auto-confirms (automation-safe)
- With TTY: prompts the user
- Non-TTY + no `--yes`: **aborts** (never hangs)

### Help Text from Doc Comments

Write `///` doc comments on enum variants and struct fields; clap pulls them for `--help`:

```rust
#[derive(Args)]
pub struct BuildArgs {
    /// Release tag (dev/prod/test).
    pub tag: Tag,
    
    #[arg(short, long)]
    /// Skip prompts (for automation).
    pub yes: bool,
}
```

Don't write `about = "..."` — let the comment be the source of truth.

---

## Git Operations

### Branch Checking

The `git` module provides safe reads:

```rust
let current = git::current_branch(dir)?;           // what branch are we on
let clean = git::is_clean(dir);                    // any uncommitted changes
let exists = git::branch_exists(dir, "prod");      // local branch exists
```

### Branch Switching

Checkout and pull are explicit steps:

```rust
git::run(dir, &["checkout", "prod"], dry_run)?;   // switch
git::run(dir, &["pull"], dry_run)?;               // fetch + merge
```

Pulling after checkout ensures the branch is up-to-date before operations.

### Committing (Prod Only)

```rust
proc::run_in(dir, "git", &["add", "."], dry_run)?;
proc::run_in(dir, "git", &["commit", "-m", &msg], dry_run)?;
proc::run_in(dir, "git", &["push", "origin", branch], dry_run)?;
```

Non-prod builds skip this entirely. Prod builds commit **after** deployment succeeds (build task, not deploy task).

---

## Authentication & Permissions

### How It Works

1. **Device key** (`~/.config/guss/device.key`, generated once per machine):
   - Ed25519 keypair for challenge signing
   - Device identifies itself to the login server by signing a challenge
   - No password involved

2. **Session token** (`~/.config/guss/token.toml`):
   - Valid **30 days offline**
   - Cached after successful login
   - Checked once per invocation (at dispatch, before subcommand logic runs)

3. **Permission checks** (in `cli/src/main.rs`):
   - At the single dispatch point, before subcommand runs
   - Granularity: per-task (e.g., `docs`, `vapi`) or finer (e.g., `deploy:dev`, `deploy:prod`)
   - If not in the token's `allowed` list: command fails with "permission denied"

4. **Gate off until configured**:
   - No `[auth]` section in config = gate is off
   - Adding `[auth]` enables permissions
   - Deliberately: no "main server bypass" based on the existence of other files

### Login Flow

```bash
guss setup login <user>       # challenge-response via device key
                             # caches token for 30 days
guss setup status             # who's logged in, what's allowed
guss setup logout             # clear the token
```

---

## Building from Source

### Prerequisites

```bash
# Rust (pinned by rust-toolchain.toml)
rustup install 1.97.0

# Optional: GitHub CLI (for PR creation in guss project)
sudo apt install gh

# For publishing releases (cross-compile to musl)
rustup target add x86_64-unknown-linux-musl
sudo apt install musl-tools
```

### Build Commands

```bash
cargo build --workspace              # debug build, all crates
cargo test --workspace               # run all tests
cargo build --release                # optimized binary
cargo build --release --locked \
  --target x86_64-unknown-linux-musl # static binary for releases
```

### Testing

```bash
cargo test --lib                     # all unit tests
cargo test -p deploy --lib           # one crate
cargo test -p setup --lib -- --nocapture  # with output
vt require_confirm                   # by name substring
```

For isolation:

```bash
guss_ROOT=/tmp/nope guss deploy --help                        # override project root
XDG_CONFIG_HOME=/tmp/alt/.config guss setup status           # alt config
vfresh setup init --root ~/guss/project-guss --no-pull --yes  # test first-run
```

(Shortcuts `v`, `vb`, `vt`, `vfresh` are installed in `~/.bashrc` by the tool itself.)

---

## Releasing

### Self-Hosted Releases

Not on GitHub. Releases are directories on a central HTTP server:

```
<remote_dir>/
├── dev/
│   ├── manifest.toml               # version, filename, sha256, target
│   ├── manifest.toml.sig           # detached ed25519 signature
│   └── guss-<version>-x86_64-...tar.gz
│       guss-<version>-x86_64-...tar.gz.sig
└── prod/
    ├── manifest.toml
    ├── manifest.toml.sig
    └── ...
```

### Publishing a Release

```bash
guss project release finish <version>  # tag the version
guss setup publish prod --dry-run      # inspect the build + upload sequence
guss setup publish prod                # build, sign, upload
```

Guards:
- Working tree must be clean
- HEAD must be tagged for the version in `Cargo.toml`
- The publishing machine has its own signing key (via `guss setup keygen`)

### Verification Order

Clients verify in this order:

1. Manifest signature (ed25519)
2. Download tarball
3. SHA256 checksum
4. Tarball signature (ed25519)
5. **Only then:** unpack

Nothing is extracted before it has been verified.

---

## Common Development Tasks

### Adding a Subcommand to an Existing Task

1. Add a variant to that crate's `enum XxxCmd`:
   ```rust
   #[derive(Subcommand)]
   enum NetCmd {
       Mode { new_mode: Option<String> },
       /// New subcommand description.
       NewSubcmd { arg: String },
   }
   ```

2. Add the match arm in its `run`:
   ```rust
   NetCmd::NewSubcmd { arg } => new_subcmd(arg, dry_run),
   ```

3. Implement the handler:
   ```rust
   fn new_subcmd(arg: &str, dry_run: bool) -> Result<()> { ... }
   ```

### Adding a New Task

1. Create `crates/<name>/Cargo.toml`:
   ```toml
   [package]
   name = "<name>"
   version.workspace = true
   edition.workspace = true
   authors.workspace = true

   [lib]
   name = "<name>"

   [dependencies]
   guss-common = { path = "../common" }
   # (deps from [workspace.dependencies] only)
   ```

2. Add to `guss/Cargo.toml` members list.

3. Create `crates/<name>/src/lib.rs`:
   ```rust
   //! Task description.
   
   use guss_common::error::Result;

   #[derive(clap::Args)]
   pub struct NameArgs { ... }

   #[derive(clap::Subcommand)]
   enum NameCmd { ... }

   pub fn run(args: NameArgs) -> Result<()> { ... }
   ```

4. Wire up in `cli/src/main.rs`:
   - Add `Cli` enum variant
   - Add match arm in `main()`
   - Add `permission_key()` entry (3/3 — easy to forget!)
   - Optional: add help line in `setup`'s `print_help()`

### Testing Best Practices

No `tempfile` or `tests/` directories. Use `#[cfg(test)] mod tests` inline.

**Split the pure logic from world reads:**

| Reads the world | Pure logic (testable) |
|---|---|
| `paths::project_guss()` | `resolve_root(home)` |
| `config::config_dir()` | `config_dir_from(xdg, home)` |
| `auth::enforce(key)` | `decide(auth, token, key)` |

Test the pure function with fixed inputs. The I/O-dependent function is thin and harder to break.

---

## Known Issues & Rough Edges

| Issue | Status | Workaround |
|---|---|---|
| `setup publish` needs musl tools | C compiler for `ring` | `sudo apt install musl-tools` |
| `setup publish` local test needs sshd | Test uploads via `scp` | Run in your own terminal, or use `--dry-run` |

---

## Before Committing

```bash
vb && vt                            # build clean, tests green
v setup status                      # gate reports "not configured"
v deploy local --help               # config resolves
vfresh setup init --root ~/guss/project-guss --no-pull --yes
```

The last one tests the first-run experience without touching your setup.

