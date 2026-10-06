# Developing `vhp`

A cheatsheet for working on the CLI itself.

**For using `vhp`:** [docs/Help/vhp-cli.md](../docs/Help/vhp-cli.md) (setup: [docs/Setup/vhp-cli-setup.md](../docs/Setup/vhp-cli-setup.md)).  
**For architecture & deeper details:** [TECHNICAL.md](TECHNICAL.md).

---

## Orientation

```
guss/
├── Cargo.toml            workspace root — deps pinned once in [workspace.dependencies]
├── rust-toolchain.toml   pins 1.97.0 + the musl target; applies automatically here
├── cli/src/main.rs       the ONLY binary (`vhp`). Wiring + the single auth gate.
└── crates/
    ├── common/           shared: proc, paths, config, auth, git, ssh, docker, ui, db
    ├── aws/              AWS operations (SSM, ECR, ECS, etc.)
    ├── net build deploy docs setup project guss    one crate per task
```

One binary, one dispatch point. Task logic lives in its own crate; `cli/src/main.rs`
does nothing but forward.

| Where | What |
|---|---|
| `common/src/proc.rs` | **all** subprocess calls go through here |
| `common/src/paths.rs` | every filesystem path; repo paths vs config paths |
| `common/src/config.rs` | `~/.config/vhp/config.toml` — settings, outside any checkout |
| `common/src/ui.rs` | headings, colored output, `require_confirm` |
| `common/src/auth.rs` | the permission gate (fails open until `[auth]` is set) |

---

## Shortcuts

Installed in `~/.bashrc` under the `# vhp-dev` marker (delete that block to undo):

| Alias | Does |
|---|---|
| `v <args>` | build-and-run — always current, works from any directory |
| `vv <args>` | run the prebuilt `target/debug/vhp` — instant, stale until `vb` |
| `vb` | `cargo build --workspace` |
| `vt` | `cargo test --workspace` |
| `vfresh <args>` | run against a throwaway `HOME` (`/tmp/vhp-fresh`) |

`v` uses `--manifest-path` rather than `cd`, so **your working directory is
preserved** — which matters, because some commands detect their context by walking
up from the cwd.

```bash
v setup status
v deploy local --help
```

---

## Testing

```bash
vt                                  # everything
cargo test -p deploy --lib          # one crate
cargo test -p setup --lib -- --nocapture
vt require_confirm                  # by name substring
```

### Isolating from your real machine

Three overrides, in increasing order of blast radius:

```bash
VHP_ROOT=/tmp/nope v jump a                    # pretend the checkout is elsewhere
XDG_CONFIG_HOME=/tmp/alt/.config v stack list  # different config, same home
vfresh setup init --root ~/vhp/project-vhp --no-pull --yes   # fully separate HOME
```

`vfresh` is the one to reach for when testing the install/config path. Config, keys,
**and** the rc file all land under `/tmp/vhp-fresh`, so it exercises the real
first-run experience without touching your setup. Check what it touched with
`find /tmp/vhp-fresh -type f`.

### Dry-run first

Every mutating subcommand takes `--dry-run` (print the command, change nothing) and
`--yes` (skip prompts). Secrets are masked in the echoed line, so the output is safe
to paste.

```bash
v deploy local --tools up --dry-run
v setup publish --dry-run --yes
```

---

## House conventions

Follow these and a new command will look like the existing ones.

**Never call `std::process::Command` directly.** Use `proc::run` / `run_in` /
`run_env` / `capture` / `capture_optional` / `succeeds`. They echo the command,
turn a non-zero exit into an `Err`, and honor dry-run in one place.

**Never build paths from `$HOME`.** Use `paths::*`. `jump` hardcoded
`~/project-vhp` and silently walked to a different tree than every other task on a
machine whose checkout lived elsewhere — that is the bug this rule prevents.

**`dry_run: bool` is the last parameter**, threaded from the clap flag down to
`proc`. Reads (`capture`, `git::is_clean`) always execute; only mutations are
skipped.

**Destructive steps go through `ui::require_confirm(prompt, yes)`.** It auto-yeses
with `--yes`, prompts on a TTY, and *aborts* when there is no TTY and no `--yes` —
so a piped or cron invocation can never silently do something irreversible.

**Help text comes from `///` doc comments** on the enum variant or field. Don't
write `about = "..."` except on the top-level `#[command]`.

### Adding a subcommand to an existing task

1. Add a variant to that crate's `enum XxxCmd` with a `///` comment.
2. Add the match arm in its `run`.
3. Implement a private fn taking `..., dry_run: bool` last.

### Adding a whole task crate

1. `crates/<name>/Cargo.toml` — `version/edition/authors.workspace = true`,
   `[lib] name = "<name>"`, deps from `[workspace.dependencies]` only.
2. Add `"crates/<name>"` to `members` in `guss/Cargo.toml`.
3. `src/lib.rs`: `//!` module doc, `#[derive(Args)] pub struct NameArgs`,
   `#[derive(Subcommand)] enum NameCmd`, `pub fn run(args) -> Result<()>`.
4. Add the dep to `cli/Cargo.toml`, then **three** edits in `cli/src/main.rs`:
   the enum variant, the match arm, and an entry in `permission_key()`. Forgetting
   the third is the easy mistake.
5. Optionally: a line in `setup`'s `print_help()` and a README row.

Single-file `lib.rs` is the norm; split into `src/*.rs` once it grows (see
`stack` and `project`).

---

## Writing testable code

The workspace has no `tempfile` dep and no `tests/` directories — everything is
inline `#[cfg(test)] mod tests`, with temp dirs hand-rolled from
`std::env::temp_dir()` + nanos.

**`std::env::set_var` is `unsafe` in edition 2024** and would race sibling tests in
the same binary. So don't test by mutating the environment — **split the pure
decision out and pass the ambient inputs in**. That pattern is used throughout:

| Public fn (reads the world) | Pure core (tested) |
|---|---|
| `paths::project_vhp()` | `resolve_root(home)` |
| `config::config_dir()` | `config_dir_from(xdg, home)` |
| `auth::enforce(key)` | `decide(auth, token, key)` |
| `config::resolve_mode()` | `resolve_mode_from(path)` |
| `proc::run(cmd, ..)` | pure command building helpers |

If something is hard to test, it usually wants this split rather than a mock.

---

## Known rough edges

| Thing | Status |
|---|---|
| `setup publish` | needs `sudo apt install musl-tools` (`ring` compiles C for musl) |
| `setup publish` local test | needs sshd — it uploads with `scp`, and `ssh localhost` is refused here |
| `gh` not installed | `vhp project feature publish` (PR creation) won't work until it is |

---

## Before you commit

```bash
vb && vt                            # build clean, all tests green
v setup status                      # gate reports "not configured"
v deploy local --help               # config still resolves
vfresh setup init --root ~/vhp/project-vhp --no-pull --yes   # first-run path intact
```

The last one is worth the ten seconds: most of what breaks in this tool breaks for
someone whose machine isn't laid out like yours.
