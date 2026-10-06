# VHP Tools Distribution — TODO

Current state: **Design + code framework exists, infrastructure missing.**

---

## Phase 1: Release Infrastructure

- [ ] **Set up release server** (`dev.vhpportal.office/tools`)
  - [ ] Create `/dev` and `/prod` channel directories
  - [ ] Serve static files (manifest, tarballs, signatures)
  - [ ] Configure HTTP (or HTTPS) access
  - [ ] Set up SSH access for `vhp setup publish` uploads (user: `devteam`)

- [ ] **Generate signing keys**
  - [ ] Run `vhp setup keygen` on a secure publishing machine
  - [ ] Store private key securely (offline or in a vault)
  - [ ] Distribute public key to team in `[release] public_key` config

- [ ] **Test the publish flow**
  - [ ] `vhp setup publish dev --dry-run` to inspect commands
  - [ ] Actual `vhp setup publish dev` to push to server
  - [ ] Verify manifest.toml and signatures are correct
  - [ ] Test `vhp setup update --channel dev` from a client machine

---

## Phase 2: Authentication & Permissions (Design → Code)

These are planned but not implemented. Depends on Phase 1 completing.

- [ ] **Central login server** (Postgres-backed)
  - [ ] Endpoint: `dev.vhpportal.office` (or configurable in future)
  - [ ] Accepts device-key challenges (ed25519 signed nonces)
  - [ ] Issues 30-day signed tokens
  - [ ] Stores user public keys + per-device allow-lists

- [ ] **Device-key login in CLI**
  - [ ] Generate user keypair on first login (stored in `~/.config/vhp/`)
  - [ ] Implement device-key challenge flow in `crates/setup/src/lib.rs`
  - [ ] Cache tokens locally (30 days offline capability)
  - [ ] Auto-refresh tokens before expiry

- [ ] **Permission enforcement**
  - [ ] Single dispatch-point check in `cli/src/main.rs`
  - [ ] Implicit-deny allow-list (module + optional subcommand granularity)
  - [ ] Bypass auth entirely for main/central server

- [ ] **Token refresh**
  - [ ] Implement `vhp setup login` (initial device auth)
  - [ ] Implement `vhp setup logout` (clear local token)
  - [ ] Auto-refresh on `vhp setup update`

---

## Phase 3: CI/CD Integration

- [ ] **Tag-to-release automation**
  - [ ] On git tag matching `v*`, trigger release build
  - [ ] Verify working tree is clean + HEAD is the tag
  - [ ] Auto-call `vhp setup publish prod` (or gated approval step)
  - [ ] Publish to both `dev` and `prod` channels

- [ ] **GitHub integration** (optional, lower priority)
  - [ ] Create release notes on GitHub from git tag
  - [ ] Link to release server tarballs (not GitHub assets)

---

## Phase 4: Documentation & Rollout

- [ ] **Update README.md**
  - [ ] Document `vhp setup login` flow
  - [ ] Show examples of permission errors + how to request access
  - [ ] Add troubleshooting for auth failures

- [ ] **Onboard team**
  - [ ] Distribute public key to all machines
  - [ ] First person runs `vhp setup login` to test flow
  - [ ] Set up allow-lists for team roles

- [ ] **Monitor & rotate keys**
  - [ ] Plan key rotation procedure (requires client update)
  - [ ] Set up alerts for failed release publishes

---

## Current Blockers

1. **Release server doesn't exist** — `dev.vhpportal.office` infrastructure
2. **Auth server not built** — Postgres + login endpoint (design done, code TBD)
3. **No signing keys generated** — `vhp setup keygen` never run in prod context
4. **Version pinned at 0.1.0** — decide versioning scheme before first real release

---

## Notes

- **Static musl target only** (`x86_64-unknown-linux-musl`) — one binary for all glibc versions
- **Trust-on-first-use:** `install.sh` cannot verify first signature (no key yet), only checksum
- **Plain HTTP is safe:** ed25519 signatures prevent tampering + rollback
- **Offline-capable:** tokens valid 30 days, no network needed between refreshes
- **Main server bypassed:** central deployment machine does not need login (implicit trust)

See [TECHNICAL.md](TECHNICAL.md#releasing) for publish/update flows.
