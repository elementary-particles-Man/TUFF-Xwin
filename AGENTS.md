# AGENTS.md (TUFF-Xwin)

## User language
- The user reads Japanese only.
- Write all explanations, handoff notes, and design summaries in Japanese unless explicitly asked otherwise.

## First files to read
1. `/media/flux/THPDOC/Develop/TUFF-Xwin/HANDOFF.md`
2. `/media/flux/THPDOC/Develop/TUFF-Xwin/docs/README.md`
3. `/media/flux/THPDOC/Develop/TUFF-Xwin/crates/waybroker-common/src/ipc.rs`

## Repository identity
- Repo path: `/media/flux/THPDOC/Develop/TUFF-Xwin`
- Remote: `git@github.com:elementary-particles-Man/TUFF-Xwin.git`
- Main branch: `main`

## Build and runtime note
- This repository lives on a `CIFS` share.
- Do not remove `.cargo/config.toml`; it redirects `cargo target-dir` to `/home/flux/.cache/tuff-xwin-target` because build scripts cannot reliably execute from the share itself.
- Normal verification commands:
  - `./scripts/dev-check.sh`
  - `./scripts/run-stack.sh`

## Push policy
- When pushing from this repository, use the SSH key in `../ssh` relative to the repo root.
- Concretely:
  - private key: `/media/flux/THPDOC/Develop/ssh/id_ed25519`
  - known_hosts: `/media/flux/THPDOC/Develop/ssh/known_hosts`
- Typical pattern:
  - `GIT_SSH_COMMAND='ssh -i /media/flux/THPDOC/Develop/ssh/id_ed25519 -o IdentitiesOnly=yes -o UserKnownHostsFile=/media/flux/THPDOC/Develop/ssh/known_hosts -o StrictHostKeyChecking=yes' git push`

## Current implementation seeds
- `docs/` contains architecture, boundary, resume, IPC, and crash-loop policy documents.
- `crates/waybroker-common/src/ipc.rs` defines the current message envelope and initial command enums.
- The service binaries are still stubs; they currently expose service identity and responsibility only.

## Immediate next candidates
- Add `examples/resume-failure/`
- Add `scripts/run-degraded-mode.sh`
- Start Unix socket stub communication between `waylandd` and `displayd`
- Add a simple health report path for `watchdog`
