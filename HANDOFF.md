# HANDOFF

更新日: `2026-03-27`  
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
  - `displayd` と `waylandd` の Unix socket stub 通信を含めて起動確認する

### 6. 最小 IPC transport

- `crates/waybroker-common/src/transport.rs`
  - runtime dir 解決
  - service socket path 解決
  - Unix socket bind/connect
  - 1 行 JSON framing helper

### 7. displayd / waylandd stub 通信

- `displayd`
  - Unix socket server として待受
  - `DisplayCommand` を受信
  - `DisplayEvent` を返す
- `waylandd`
  - startup 時に `displayd` へ `EnumerateOutputs` を送る
  - `OutputInventory` を受けて表示する

### 8. LeyerX11 最小互換レイヤ

- `LeyerX11/`
  - optional な `X11` compatibility island の実験ツリー
- `LeyerX11/crates/layerx11-common`
  - rootless `X11` scene 型
  - `Waybroker` surface への変換
- `LeyerX11/crates/x11bridge`
  - sample rootless scene を読み、`displayd` へ `CommitScene` を送る
- `LeyerX11/scripts/run-rootless-demo.sh`
  - `displayd` と `x11bridge` の往復確認

### 9. Desktop profile manager

- `profiles/`
  - GUI profile manifest 置き場
- `profiles/xfce-x11.json`
  - `LeyerX11` 上の `XFCE` profile
- `profiles/openbox-x11.json`
  - `LeyerX11` 上の最小 `Openbox` profile
- `crates/sessiond`
  - profile の列挙
  - profile 選択
  - launch plan の表示
  - active profile の JSON 書き出し
- `scripts/run-profile-demo.sh`
  - profile manager の確認導線

### 10. Degraded fallback switching

- `DesktopProfile`
  - `degraded_profile_id` で fallback profile を宣言可能
- `sessiond`
  - `watchdog-report-<profile>.json` を読み、`--apply-watchdog-active` で active profile を fallback へ切替
  - `profile-transition-<from>-to-<to>.json` を runtime dir へ記録
- `profiles/demo-x11-degraded.json`
  - crash-loop 後の最小 fallback demo profile
- `scripts/run-degraded-mode.sh`
  - crash-loop 検知から degraded profile 切替まで確認する導線

### 11. watchdog -> sessiond IPC

- `SessionCommand`
  - `ApplyWatchdogReport`
  - `ProfileTransition`
  - `ProfileUnchanged`
- `sessiond`
  - `--serve-ipc [--once]` で Unix socket server として待受
  - `--manage-active` で active profile runtime を常駐 supervisor として保持
- `watchdog`
  - `--notify-sessiond` で report を IPC 送信し、切替結果を応答として受ける
- `scripts/run-degraded-mode.sh`
  - file 経由ではなく `watchdog -> sessiond` IPC で degraded fallback を自動適用し、そのまま fallback component 起動まで確認する

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

各 binary はまだ本処理を持っていませんが、`displayd` と `waylandd` は最小 IPC 往復、`x11bridge` は rootless `X11` scene の commit デモまで実装済みです。

- `displayd`
- `waylandd`
- `compd`
- `lockd`
- `sessiond`
- `watchdog`
- `x11bridge`

## 直近のコミット

- `0010c70` Add initial IPC model types
- `e9ca84a` Add boundary and resume design seeds
- `bdf45d2` Add licensing and contribution templates
- `39f4419` Initial commit

## 次にやるなら

優先順はこのあたりです。

1. `sessiond` から実際に selected profile を起動する launcher stub を足す
2. `compd` と `displayd` の scene commit stub を足して policy と hardware broker を分ける
3. `watchdog` に health report と restart request の最小実装を生やす
4. `LeyerX11` に clipboard / selection の最小橋渡しを足す
5. degraded profile 切替後の component 再起動と state 収束を `watchdog` / `sessiond` 間で自動化する

## 注意点

- repo は `CIFS` 上にあるため、build artifact は `.cargo/config.toml` 経由で `/home/flux/.cache/tuff-xwin-target` に逃がしてある
- この設定を戻すと build script 実行でこける可能性が高い
- `target/` は repo 直下にも残っているが、無視してよい
