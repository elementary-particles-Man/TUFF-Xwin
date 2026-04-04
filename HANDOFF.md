# HANDOFF

更新日: `2026-04-04`  
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
- 状態: local 変更あり（`compd` broker recovery / `Wayland native` demo 追加中）

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
- `./scripts/run-watchdog-resync-demo.sh`
  - `watchdog` を再起動しても `sessiond` が full state を再送し、監視と degraded fallback が継続することを確認する
- `./scripts/run-scene-recovery-demo.sh`
  - `displayd` が最後に commit された scene を保持し、再起動後も `compd` が再取得できることを確認する
- `./scripts/run-compd-broker-recovery.sh`
  - `watchdog -> sessiond` recovery execution で `compd` を再起動し、`displayd + waylandd` snapshot から scene rebuild して再commit できることを確認する

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

### 12. sessiond -> watchdog health stream

- `WatchdogCommand`
  - `InspectLaunchState`
  - `UpdateLaunchState`
  - `ResyncLaunchState`
  - `InspectionResult`
- `watchdog`
  - `--serve-ipc [--once]` で Unix socket server として sessiond から full launch-state / delta update を受け取る
- `sessiond`
  - `--notify-watchdog` で managed active profile の launch-state 更新を watchdog へ stream
  - 初回は full launch-state、以後は component 差分だけを送る
  - 各 update に `generation` と `sequence` を持たせ、profile 切替時に generation を進める
  - watchdog が cache miss した場合と sequence gap を検出した場合は `ResyncLaunchState` を受け、full launch-state を再送する
- watchdog の応答 report をその場で評価し、degraded fallback を自前で適用
- `scripts/run-degraded-mode.sh`
  - `watchdog` を background server として起動し、manual pull なしで degraded switch と fallback health report 収束まで確認する
- `scripts/run-watchdog-resync-demo.sh`
  - `watchdog` 再起動直後の cache miss に対して `sessiond` が `ResyncLaunchState` を受けて full launch-state を再送することを確認する

### 13. displayd authoritative scene snapshot

- `DisplayCommand`
  - `GetSceneSnapshot { output }` を追加
- `DisplayEvent`
  - `SceneCommitted` に `commit_id` を追加
  - `SceneSnapshot { snapshot }` を追加
- `CommittedSceneState`
  - `source` / `target` / `focus` / `surfaces` / `commit_id` / `unix_timestamp` を持つ restart-safe scene snapshot 型を追加
- `displayd`
  - `CommitScene` 成功時に `WAYBROKER_RUNTIME_DIR/displayd-last-scene.json` へ snapshot を書き出す
  - 起動時に既存 snapshot を再読込し、`GetSceneSnapshot` へ応答できる
- `compd`
  - `--restore-from-displayd` で `displayd` の最後の committed scene を再取得し、内部 scene として復元できる
- `scripts/run-scene-recovery-demo.sh`
  - scene commit -> `displayd` 再起動 -> `compd` restore の流れを確認する導線を追加

### 14. waylandd surface registry snapshot

- `WaylandCommand`
  - `GetSurfaceRegistry` を追加
- `WaylandEvent`
  - `SurfaceRegistry { snapshot }` を追加
- `SurfaceRegistrySnapshot`
  - `generation` / `surfaces` / `unix_timestamp` を持つ wayland lifecycle snapshot 型を追加
- `WaylandSurfaceState`
  - `id` / `app_id` / `role` / `mapped` / `buffer_attached` を持つ
- `waylandd`
  - `--serve-ipc` で Unix socket server として surface registry を返せる
  - `--registry PATH` で fixture から registry を読み込める
- `compd`
  - `--reconcile-waylandd [--require-waylandd]` で `displayd` last-scene を `waylandd` registry と突き合わせて rebuild できる
  - 消えた surface を drop し、必要なら focus を再選定する
- `examples/minimal-scene/surface-registry.json`
  - `panel-1` が inactive、`terminal-1` だけ生存している fixture
- `scripts/run-scene-recovery-demo.sh`
  - `displayd` 再起動後に `waylandd` registry も使って `compd` の broker rebuild まで確認する

### 15. compd broker recovery execution

- `ServiceRecoveryExecutionPolicy`
  - `restart_command_args` を追加
  - supervisor restart 時だけ recovery 専用引数を追記できる
- `sessiond`
  - `watchdog-action-execution-<role>.json` に `recovery_command_args` を記録する
  - recovery 実行時は通常 launch command に `restart_command_args` を追加して spawn する
- `compd`
  - `--serve-ipc --restore-from-displayd --reconcile-waylandd` で待受前に startup rebuild を実行する
  - rebuild 成功後は `displayd` へ再commit し、その scene を authoritative snapshot として更新する
- `profiles/demo-wayland-compd-recovery.json`
  - `Wayland native` の最小 skeleton
  - repo 内 `compd` binary を session component として supervisor 管理し、recovery 時だけ rebuild 引数を追加する
- `scripts/run-compd-broker-recovery.sh`
  - `compd-trouble` resume failure から `watchdog` restart request、`sessiond` recovery execution、`displayd-last-scene.json` 更新までを確認する

## 現在のコード上の要点

### 共有型

- `crates/waybroker-common/src/lib.rs`
  - `ServiceRole`
  - `ServiceBanner`
- `crates/waybroker-common/src/ipc.rs`
  - `IpcEnvelope`
  - `MessageKind`
  - `DisplayCommand`
  - `DisplayEvent`
  - `CommittedSceneState`
  - `WaylandCommand`
  - `WaylandEvent`
  - `SurfaceRegistrySnapshot`
  - `WaylandSurfaceState`
  - `LockCommand`
  - `SessionCommand`
  - `WatchdogCommand`
  - `HealthState`
  - `OutputMode`
  - `SurfaceSnapshot`
  - `SurfacePlacement`

### 現在の stub binary

各 binary はまだ本処理を持っていませんが、`displayd` と `waylandd` は最小 IPC 往復、`displayd` は last-scene snapshot 保持、`waylandd` は surface-registry snapshot 応答、`compd` はその broker rebuild、`x11bridge` は rootless `X11` scene の commit デモまで実装済みです。

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

1. `waylandd` surface registry を clipboard / selection owner と結びつけ、`compd` rebuild 後の focus/selection handoff を作る
2. `sessiond/watchdog` stream に `source_id` か session instance id を足し、multi-session supervisor 化でも cache key が衝突しないようにする
3. `Wayland native` profile に shell / panel の最小 skeleton を追加し、mock lock UI 依存を減らす
4. `LeyerX11` に clipboard / selection の最小橋渡しを足す
5. degraded profile 切替後の component 再起動と state 収束を `watchdog` / `sessiond` 間で自動化する

## 注意点

- repo は `CIFS` 上にあるため、build artifact は `.cargo/config.toml` 経由で `/home/flux/.cache/tuff-xwin-target` に逃がしてある
- この設定を戻すと build script 実行でこける可能性が高い
- `target/` は repo 直下にも残っているが、無視してよい
