# Repository Layout

## Goals

- 文書と実装骨格を同じ repository に置く
- `Waybroker` の責務分離を、そのまま crate 分割に反映する
- まだ remote を切っていない段階でも、すぐ `cargo check` と `git status` が通る形にする

## Top-Level Directories

- `docs/`: 設計文書、障害モデル、統合メモ
- `profiles/`: ユーザーが選択する GUI profile manifest
- `crates/`: Rust workspace members
- `examples/`: 将来の動作検証や playground
- `scripts/`: 開発補助スクリプト
- `.github/workflows/`: CI skeleton

## Crates

- `crates/waybroker-common`
  - 共通型
  - service metadata
  - 共通出力関数
- `crates/displayd`
  - hardware broker の実装入口
- `crates/waylandd`
  - protocol broker の実装入口
- `crates/compd`
  - composition policy の実装入口
- `crates/lockd`
  - lock service の実装入口
- `crates/sessiond`
  - session policy の実装入口
- `crates/watchdog`
  - recovery control の実装入口

## Current Seeds

- `docs/api-boundary.md`
- `docs/sequence-resume.md`
- `examples/minimal-scene/`
- `scripts/run-stack.sh`

## Next Files To Add

- `examples/resume-failure/`
- `scripts/run-degraded-mode.sh`

## Code Seeds

- `crates/waybroker-common/src/ipc.rs`
  - 初期 envelope
  - service 間 command enum
  - crash loop や resume hint の共有型
