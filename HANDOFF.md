# HANDOFF

更新日: `2026-03-20`  
対象 repository: `/media/flux/THPDOC/Develop/TUFF-Xwin`

## 前提

- ユーザーは日本語しか読めません。
- 説明、設計メモ、進捗整理は日本語で書いてください。
- push 時は `../ssh` の鍵を使ってください。
  - private key: `/media/flux/THPDOC/Develop/ssh/id_ed25519`
  - known_hosts: `/media/flux/THPDOC/Develop/ssh/known_hosts`

## 現在の git 状態

- branch: `main`
- remote: `origin = git@github.com:elementary-particles-Man/TUFF-Xwin.git`
- 状態: `main...origin/main` で clean

## ここまでの進捗

### 1. Repository 初期化

- git repository 作成
- GitHub remote 設定
- author は `elementary-particles-Man <flux5963@gmail.com>` に設定済み

### 2. Rust workspace 骨格

- root workspace:
  - `Cargo.toml`
  - `.cargo/config.toml`
  - `rustfmt.toml`
- crates:
  - `waybroker-common`
  - `displayd`
  - `waylandd`
  - `compd`
  - `lockd`
  - `sessiond`
  - `watchdog`

### 3. Documentation

- 基本設計:
  - `docs/architecture.md`
  - `docs/components.md`
  - `docs/failure-model.md`
  - `docs/plasma-integration.md`
  - `docs/roadmap.md`
- 補助設計:
  - `docs/design-memo.md`
  - `docs/repo-layout.md`
  - `docs/api-boundary.md`
  - `docs/sequence-resume.md`
  - `docs/ipc-format.md`
  - `docs/crash-loop-policy.md`

### 4. Project metadata

- `LICENSE-MIT`
- `LICENSE-APACHE`
- `CONTRIBUTING.md`
- GitHub issue templates
- pull request template

### 5. 実行補助

- `./scripts/dev-check.sh`
  - `cargo fmt --all --check`
  - `cargo check --workspace`
  - `cargo test --workspace`
- `./scripts/run-stack.sh`
  - 現在の stub services を順番に起動確認する

## 現在のコード上の要点

### 共有型

- `crates/waybroker-common/src/lib.rs`
  - `ServiceRole`
  - `ServiceBanner`
- `crates/waybroker-common/src/ipc.rs`
  - `IpcEnvelope`
  - `MessageKind`
  - `DisplayCommand`
  - `LockCommand`
  - `SessionCommand`
  - `WatchdogCommand`
  - `HealthState`
  - `OutputMode`
  - `SurfaceSnapshot`
  - `SurfacePlacement`

### 現在の stub binary

各 binary はまだ本処理を持っておらず、service 名と責務を表示する程度です。

- `displayd`
- `waylandd`
- `compd`
- `lockd`
- `sessiond`
- `watchdog`

## 直近のコミット

- `0010c70` Add initial IPC model types
- `e9ca84a` Add boundary and resume design seeds
- `bdf45d2` Add licensing and contribution templates
- `39f4419` Initial commit

## 次にやるなら

優先順はこのあたりです。

1. `examples/resume-failure/` を追加して失敗系 state も固定する
2. `scripts/run-degraded-mode.sh` を追加して watchdog 観点の実行導線を作る
3. `waylandd` と `displayd` の間に Unix socket の最小 stub 通信を入れる
4. `watchdog` に health report と restart request の最小実装を生やす

## 注意点

- repo は `CIFS` 上にあるため、build artifact は `.cargo/config.toml` 経由で `/home/flux/.cache/tuff-xwin-target` に逃がしてある
- この設定を戻すと build script 実行でこける可能性が高い
- `target/` は repo 直下にも残っているが、無視してよい
