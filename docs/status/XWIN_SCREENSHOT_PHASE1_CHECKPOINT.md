# TUFF-Xwin Screenshot Phase1 Checkpoint

## Checkpoint

- main HEAD: `9d4ca3da31a8d1979e9fd39da03afdf324c82288`
- この文書は xwin-screenshot Phase1 系の到達点固定であり、release ではない
- 実OS統合前の repo内 checkpoint である

## Core Purpose

- TUFF-Xwin の目的は KDE級DEを作ることではない
- 目的は GUI panic と kernel panic を連動させないこと
- X-window / compositor / browser / screenshot app は部品であり、壊れても局所停止すべきである
- UNIX哲学に基づき、小型・分離・明示的例外処理・ログ/原因保持を優先する

## Implemented Milestones

- `security/pre-screenshot-hardening`: IPC過大入力、path sanitization、capture buffer検証、malformed wire拒否
- `feature/xwin-screenshot-app`: xwin-screenshot app scaffold、PNG/JPEG encode、fake capture/hotkey/tray
- `security/xwin-browser-boundary`: xwin-sec crate、browser hostile-client policy
- `feature/xwin-screenshot-displayd-ipc`: `DisplayCommand::CaptureOutput` / `DisplayEvent::OutputCaptured` 境界
- `feature/xwin-screenshot-artifact-ingest`: `DisplaydCaptureArtifact` ingest、`allowed_root`、RGBA size validation
- `feature/xwin-screenshot-displayd-artifact-contract`: RGBA8888 format validation と OutputCaptured contract 固定
- `feature/displayd-rgba8888-artifact-writer`: displayd writer を native u32 dump から明示 RGBA8888 bytes へ変更
- `feature/xwin-screenshot-isolated-displayd-transport`: tempdir isolated Unix socket transport
- `feature/xwin-screenshot-cli-backend-selection`: fake / isolated-displayd backend のCLI選択

## Current xwin-screenshot Capabilities

- fake backend で PNG/JPEG 保存が可能
- isolated-displayd backend を明示CLI指定で組み立て可能
- `DisplaydUnixSocketTransport` は明示 socket path のみ使用
- `ArtifactRoot` 配下の raw RGBA8888 artifact を `CapturedFrame` 化可能
- PNG compression level と JPEG quality をCLI指定可能
- `CaptureTarget` は fullscreen / active-window を扱う
- policy deny の場合は transport へ進まない

## Security Boundaries Fixed

- 実 `displayd.sock` には接続しない
- `/run/user` 配下 socket は拒否
- `XDG_RUNTIME_DIR` / `WAYLAND_DISPLAY` / `DISPLAY` を読まない
- 実 Wayland session に接続しない
- 実 DRM/KMS/PipeWire/input device に触らない
- 実 global hotkey / tray 登録はしない
- Browser client は hostile default
- Screen capture は `xwin-sec` policy hook の対象
- artifact path は `allowed_root` 配下に限定
- raw RGBA8888 は `width * height * 4` を満たす場合のみ受理

## Display Artifact Contract

- `DisplayEvent::OutputCaptured` は `width` / `height` / `format` / `artifact_path` を持つ
- format は RGBA8888 のみ受理
- artifact body は PNG/JPEG ではなく raw RGBA8888 bytes
- displayd は pixel `0xAARRGGBB` を `[R, G, B, A]` bytes に明示変換して保存
- native endian memory dump には依存しない
- xwin-screenshot は ingest 後に PNG/JPEG へ変換する

## Explicit Non-Goals at This Checkpoint

- 実 `displayd.sock` 接続
- 実 `XDG_RUNTIME_DIR` 探索
- 実 Wayland session 統合
- 実 global hotkey 登録
- 実 system tray 登録
- 実 DRM/KMS/PipeWire/input device 接続
- symlink race / TOCTOU 完全対策
- 本物の displayd process 起動
- Chrome 実プロセス判定
- Chrome/V8 exploit検出
- ブラウザsandbox再実装
- 実OS policy enforcement

## Next Phase Candidates

- isolated-displayd CLI flow のさらに薄いE2E整備
- symlink / TOCTOU を含む artifact root hardening
- 実displayd processではなく test harness displayd との契約テスト
- CLI config file 読み込み。ただし自動runtime探索は禁止
- 隔離環境での実displayd.sock接続検討
- hotkey/tray production実装はまだ後続

## Permanent Development Rules Learned

- repo root確認が通るまで作業開始ではない
- `pwd` / `git rev-parse --show-toplevel` / `git status --short --branch` が失敗したら即停止
- dirty file処理のためにrepo移動・削除をしない
- commit前に `git status --short --branch` で現在ブランチを確認する
- 読めないWeb/PDF/一次資料は未読として止まる。補完しない
