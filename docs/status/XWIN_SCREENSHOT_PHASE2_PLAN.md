# TUFF-Xwin Screenshot Phase2 Plan

## Plan Baseline

- main HEAD: `eb0026bb9f9f6f7ef804041868942f2b0d4027ec`
- この文書は Phase2 計画であり、実装ではない
- Phase1 checkpoint: [docs/status/XWIN_SCREENSHOT_PHASE1_CHECKPOINT.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_PHASE1_CHECKPOINT.md)
- 実OS統合前の repo内計画文書である

## Phase1 Fixed State

- xwin-screenshot scaffold
- PNG/JPEG encode
- fake capture/hotkey/tray
- xwin-sec browser hostile boundary
- displayd IPC boundary
- DisplaydCaptureArtifact ingest
- RGBA8888 OutputCaptured contract
- displayd explicit RGBA8888 writer
- isolated displayd Unix socket transport
- CLI backend selection
- Phase1 checkpoint document

## Phase2 Core Rule

- Phase2でも実displayd.sockへは直ちに接続しない
- Phase2でも /run/user 自動探索はしない
- Phase2でも `XDG_RUNTIME_DIR` / `WAYLAND_DISPLAY` / `DISPLAY` は読まない
- Phase2でも実Wayland session / DRM-KMS / PipeWire / input device には触らない
- 実OS統合は隔離環境で明示的に切る

## Phase2-A Artifact Root Hardening

- 目的: `ArtifactRoot` の path安全性を強化する
- canonical root validation を固定する
- artifact path は相対名のみ許可する
- symlink を含む artifact path は拒否する
- open後検証方針を検討する
- `allowed_root` escape をさらに厳密化する
- 実runtime artifact はまだ扱わない
- 実OS統合は行わない

## Phase2-B Test Harness Displayd

- 目的: 本物displayd processではなく repo内 test harness displayd を作る
- `OutputCaptured` contract のE2Eを isolated socket で固定する
- `CaptureOutput` -> RGBA8888 artifact -> ingest -> PNG/JPEG encode を harness で通す
- tempdir socket と tempdir artifact root だけを使う
- 実displayd.sockには接続しない
- 実displayd processは起動しない
- Phase2-B の実装対象は `apps/screenshot/src/harness_displayd.rs` のような test-only ハーネスである
- negative mode として unknown format / empty artifact path / absolute artifact path / path traversal / byte length mismatch を検証する

## Phase2-C Config File Support

- 目的: CLI指定を設定ファイルへ移せるようにする
- `--config PATH` を追加する
- backend / save_dir / format / compression / quality / artifact_root / isolated socket path を設定可能にする
- 設定ファイルでも runtime自動探索は禁止
- 設定ファイルで `/run/user` socket を指定した場合は拒否
- 設定ファイルは明示pathのみ読む
- 環境変数からの暗黙設定は行わない
- 設定確定順は default -> config file -> CLI explicit override とする
- isolated-displayd backend では `displayd_socket` と `artifact_root` を必須にする

## Phase2-D Isolated VM or Test Machine Plan

- 目的: 実OS統合前の隔離環境手順を固定する
- VMまたはテスト機のみ対象
- 現用OS・現用Wayland session・現用displayd.sockには触らない
- 検証は復旧可能な環境で行う
- CUI/SSH/recovery pathを先に確保する
- 実displayd.sock接続はこの段階以降で検討する
- 詳細計画は [docs/status/XWIN_SCREENSHOT_ISOLATED_VM_PLAN.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_ISOLATED_VM_PLAN.md) に分離する

## Phase2-E Production Hotkey/Tray Deferred

- global hotkeyの実登録はまだ行わない
- system trayの実登録はまだ行わない
- hotkey/trayはGUI panic連鎖を起こしやすいため後続扱い
- fake hotkey/tray境界は維持する
- 実装する場合も隔離環境でのみ行う

## Explicit Phase2 Non-Goals

- 実displayd.sock接続
- 実XDG_RUNTIME_DIR探索
- 実Wayland session統合
- 実global hotkey登録
- 実system tray登録
- 実DRM/KMS/PipeWire/input device接続
- 本物displayd process起動
- Chrome実プロセス判定
- Chrome/V8 exploit検出
- ブラウザsandbox再実装
- 実OS policy enforcement

## Safety Acceptance Criteria

- cargo fmt --check PASS
- cargo check --workspace PASS
- cargo test --workspace PASS
- git diff --check PASS
- 実OS非干渉
- 実displayd.sock非接続
- runtime自動探索なし
- policy deny が transport 前に止まること
- artifact path escape を拒否すること
- failure は panic ではなく Result error に落とすこと

## Recommended Implementation Order

1. `docs/xwin-screenshot-phase2-plan` を main に固定する
2. `feature/xwin-screenshot-artifact-root-hardening`
3. `feature/xwin-screenshot-harness-displayd`
4. `feature/xwin-screenshot-config-file`
5. `docs/xwin-screenshot-isolated-vm-plan`
6. 隔離環境でのみ実displayd.sock接続検討

## Permanent Rules Reaffirmed

- repo root確認が通るまで作業開始ではない
- `pwd` / `git rev-parse --show-toplevel` / `git status --short --branch` が失敗したら即停止
- dirty file処理のためにrepo移動・削除をしない
- commit前に `git status --short --branch` で現在ブランチを確認する
- 読めない Web / PDF / 一次資料は未読として止まる。補完しない
- 実装と文書を混同しない
- checkpoint / plan / readiness を実装完了扱いしない
