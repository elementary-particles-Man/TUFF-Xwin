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
  - `./scripts/run-degraded-mode.sh`
  - `./scripts/run-watchdog-resync-demo.sh`

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

## Current progress snapshot
- ここから下は日本語で維持すること。
- `displayd <-> waylandd` の最小 Unix socket IPC は実装済み。
  - output enumerate と inventory 応答まで通る。
- `LeyerX11/` に optional な最小 `X11` compatibility island を実装済み。
  - `layerx11-common` と `x11bridge` で rootless scene を broker 側へ commit できる。
- `sessiond` は desktop profile manager として動作する。
  - profile 列挙、選択、active profile 保存、launch state 保存、component spawn、restart limit、supervisor loop まで実装済み。
  - profile は `demo-x11` / `demo-x11-crashy` / `demo-x11-degraded` / `openbox-x11` / `xfce-x11` がある。
- `watchdog` は launch state inspection と recovery 判断を持つ。
  - `healthy / unhealthy / inactive` 判定、critical component の restart / degraded-profile 判断、report 書き出しまで実装済み。
- `watchdog -> sessiond` IPC による degraded fallback 適用は実装済み。
  - `sessiond --serve-ipc --manage-active` で active profile runtime を保持しつつ fallback へ切替できる。
- `sessiond -> watchdog` health stream は event-driven 化済み。
  - 初回は full launch-state、その後は component 差分だけを `UpdateLaunchState` で送る。
  - `ResyncLaunchState` により watchdog cache miss 後の full state 再送ができる。
  - launch stream には `generation` / `sequence` を持たせてあり、stale delta は watchdog 側で無視し、sequence gap のみ resync を要求する。
- 主要 demo script:
  - `./scripts/run-profile-demo.sh`
  - `./scripts/run-crash-loop-demo.sh`
  - `./scripts/run-degraded-mode.sh`
  - `./scripts/run-watchdog-resync-demo.sh`

## Immediate next candidates
- `sessiond/watchdog` stream に `source_id` か session instance id を足し、将来 multi-session supervisor 化しても cache key が衝突しないようにする
- `compd -> displayd` の scene commit をもう一段進め、policy broker と hardware broker の境界を強める
- `LeyerX11` に clipboard / selection / 最小 atom/EWMH を足して、実アプリ互換を広げる
- `Wayland native` profile の最小 skeleton を追加し、`LeyerX11 first` からの次段を作る
