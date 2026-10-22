# project — design reference

A git-flow controller, wired in as `guss project ...` (formerly the standalone
`gust` binary).

---

## Concepts

`project` models git-flow branching: long-lived `main` + `develop`, short-lived
`feature/*` and `release/*` branches. (No hotfix flow yet — add one when
needed; the `prefix.hotfix` config key is kept for it.)

| Branch type | Branches from | Merges into        | Tagged |
|-------------|---------------|--------------------|--------|
| feature     | develop       | develop            | no     |
| release     | develop       | main + develop     | yes    |

---

## Config

Stored as TOML in `.gitflow` at the repo root.

```toml
[branch]
main    = "main"
develop = "develop"

[prefix]
feature = "feature/"
release = "release/"
hotfix  = "hotfix/"
tag     = ""          # e.g. "v" → tags like "v1.2.0"

[pr]
enabled = true        # auto-open PR via `gh` after publish

[[hooks]]
event  = "release.publish"   # dot-namespaced action name, see Hooks below
branch = "release/*"         # optional glob; omit to match every branch
run    = "guss deploy portal webserver dev --yes"
```

`Config::load()` errors if `.gitflow` is missing. `guss project init` writes defaults.

---

## Module layout

```
src/
  lib.rs               ProjectArgs + Subcommand dispatch, called from `cli`'s main.rs
  config.rs            load/save .gitflow, branch name helpers, HookConfig
  git.rs               thin wrapper around std::process::Command for git
  gh.rs                shells out to `gh pr create --fill`
  hooks.rs             match + run configured [[hooks]] after an action
  commands/
    mod.rs
    init.rs            guss project init
    feature.rs         start / finish / publish / list
    release.rs         start / finish / publish
```

---

## Command behaviour

### feature

- **start** — checkout develop, create `feature/<name>`
- **finish** — merge `--no-ff` into develop, delete branch
- **publish** — push to remote, optionally open PR → develop

### release

- **start** — checkout develop, create `release/<version>`
- **finish** — merge `--no-ff` into main, tag, merge into develop, delete branch
- **publish** — push to remote, optionally open PR → main

### status *(TODO)*

Show current branch, indicate which flow type it is, and list all active
feature/release branches.

---

## Hooks

`[[hooks]]` entries in `.gitflow` run a shell command after a lifecycle
action completes successfully. They only fire through `guss project`
commands — there's no real git hook installed, so a bare `git push`/`git
pull` in the repo bypasses them entirely. This is deliberately per-repo:
which hooks exist depends entirely on which repo's `.gitflow` you're
standing in.

- `event` — dot-namespaced action name matching the command that ran:
  `feature.start` / `feature.finish` / `feature.publish`,
  `release.start` / `release.finish` / `release.publish`.
  Roughly: `.publish` events are the "push" side, `.finish` events are the
  "pull"/integration side.
- `branch` — optional glob (`*` wildcard only) narrowing which branch the
  hook applies to. Omit to match every branch for that event.
- `run` — a shell command string, executed via `sh -c`.

Multiple hooks can match the same event; they run in config order. A hook
that exits non-zero stops the chain and the whole `guss project` command
returns that error — but only after the git action itself already
succeeded, since the hook is a post-action step, not a gate on it.

Implemented in `hooks.rs`; wired into every non-`list` command in
`feature.rs`/`release.rs` (not `init.rs`, since a freshly-created default config has no hooks to fire
yet).

---

## Permissions

`guss project` commands run behind the CLI's login/permission gate (see the
"Login & permissions" section of `docs/Setup/guss-cli-setup.md`) — enforced in
`cli/src/main.rs` via `ProjectArgs::permission_key()`, not inside this crate.
Most commands are gated under the flat `"project"` key; `release` is
qualified separately (`"project:release"`) since it touches shared
branches/tags.

> **Known issue:** `release start`/`finish` take
> a positional `version` argument that currently collides with clap's
> auto-generated `--version`/`-V` flag once this crate runs under the `guss`
> umbrella binary (`propagate_version` on the top-level `Cli`) — both panic
> at parse time. Not yet fixed; unrelated to the permission gate itself.

---

## Error handling

Uses `guss_common::Result` (an `anyhow`-backed alias shared by every task crate).

All commands propagate errors up to `run()`, which `cli`'s `main` prints to
stderr and exits 1 on. No `unwrap` in command logic. `gh` unavailability is a
warning, not an error.

---

## Dependencies

| Crate       | Use                                    |
|-------------|-----------------------------------------|
| guss-common  | shared Result/error type, proc/git/gh helpers |
| clap        | CLI arg parsing + --help                |
| serde       | derive Serialize/Deserialize            |
| toml        | .gitflow read/write                     |
| colored     | terminal output colouring               |

`clap` (derive feature) handles arg parsing and `--help` generation.
