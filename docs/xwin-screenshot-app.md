# Xwin Screenshot App

`xwin-screenshot` は TUFF-Xwin の純正スクリーンショットアプリの最小実装である。

## 目的

- GUI 障害を小さく閉じ込める。
- `displayd` との境界を fake client で検証しやすくする。
- 保存先、保存形式、ホットキー、トレイ、UI を独立した小さな責務に分ける。

## 非目的

- KDE 級の巨大デスクトップ環境を作ることではない。
- 実 Wayland session へ接続することではない。
- 実 global hotkey 登録を行うことではない。
- 実 system tray 登録を行うことではない。
- 実 DRM / KMS / PipeWire / input device を触ることではない。

## 設定

- `hotkey`: 既定は `PrintScreen`
- `capture_target`: `Fullscreen` / `ActiveWindow`
- `format`: `Png` / `Jpeg`
- `png_compression_level`: `0..=9`
- `jpeg_quality`: `1..=100`
- `save_dir`: 明示ディレクトリ
- `filename_template`: path separator を含まない単一ファイル名

## 保存形式

- PNG
- JPEG

PNG は圧縮レベルを持つ。JPEG は quality を持つ。

## 境界

- `CaptureClient` は fake 実装から始める。
- `TrayController` と `HotkeyController` も fake 実装を既定にする。
- 保存前に RGBA buffer のサイズ整合性を検証する。
- traversal、absolute filename、空 filename は拒否する。

## Phase 2

- `displayd` への境界は `DisplayCommand::CaptureOutput` と `DisplayEvent::OutputCaptured` までの protocol-backed fake transport へ進めた。
- 実 `displayd.sock` にはまだ接続しない。
- 実 artifact 読み込み、実 raw 変換、実 global hotkey、実 tray は後続 phase に回す。
- screen capture は `xwin-sec` の policy hook を通して判定する。

## Phase 2-B

- 本物の `displayd` process ではなく repo 内の test harness displayd を使う。
- tempdir socket と tempdir artifact root だけを使い、`CaptureOutput` -> `OutputCaptured` -> RGBA8888 artifact -> ingest -> PNG/JPEG encode の契約 E2E を固定する。
- 実 `displayd.sock` には接続しない。
- 実 Wayland session / DRM-KMS / PipeWire / input device には触らない。
- 実 OS 統合は後続の隔離環境 phase まで行わない。

## Phase 3

- `DisplaydCaptureArtifact` を安全に ingest して `CapturedFrame` へ変換する。
- artifact path は Phase2-A の方針として相対名のみを受け付け、`allowed_root` 配下に join した結果だけを読む。
- `displayd` の `OutputCaptured` は `width` / `height` / `format` / `artifact_path` を返し、現在の `format` 契約は `RGBA8888` である。
- artifact 本体は PNG/JPEG ではなく raw RGBA8888 bytes として扱う。
- `displayd` 側 writer は native `u32` dump ではなく明示的な RGBA8888 byte stream を書く。
- raw RGBA artifact は `width * height * 4` と一致した場合のみ受理する。
- Unix では artifact open/read を openat / O_NOFOLLOW / O_DIRECTORY / O_CLOEXEC で harden する。
- `DisplaydArtifactCaptureClient` により `DisplaydIpcCaptureClient` の返却物を encode 経路へ接続する。
- 実 `displayd.sock` にはまだ接続しない。
- symlink は Phase2-A で拒否し、kernel-level の完全証明と実 runtime artifact 読み込みは後続 phase の扱いとする。
- Phase2-B では test harness displayd により `CaptureOutput` / `OutputCaptured` / artifact ingest / encode の往復を repo 内で固定する。

## Phase 2-A

- `ArtifactRoot` は canonical root を保持する。
- root は absolute かつ既存ディレクトリでなければならない。
- `/run/user` 配下の root は拒否する。
- artifact path は相対名のみを許可し、`..` / `.` / absolute path は拒否する。
- symlink を含む artifact path は Phase2-A では拒否する。
- TOCTOU の完全対策は後続 phase に残す。

## CLI backend selection

- `xwin-screenshot` は CLI から backend を明示選択できる。
- `--config PATH` で明示 config file を読める。
- `--backend fake` は fake capture backend を使う。
- `--backend isolated-displayd` は tempdir などの明示 socket path を使う isolated Unix socket transport を使う。
- `isolated-displayd` は `--displayd-socket` と `--artifact-root` を必須とする。
- `--config` の設定は default -> config file -> CLI explicit override の順で確定する。
- 実 `displayd.sock` の自動探索は行わない。
- `/run/user`、`XDG_RUNTIME_DIR`、実 Wayland session は読まない。

## Phase 2-C

- `--config` は明示 path のみを読む。
- config file は自動探索しない。
- `XDG_CONFIG_HOME` / `HOME` / `XDG_RUNTIME_DIR` / `WAYLAND_DISPLAY` / `DISPLAY` は読まない。
- config file でも fake / isolated-displayd backend、target、format、compression、quality、save_dir、displayd_socket、artifact_root を扱える。
- isolated-displayd backend では `displayd_socket` と `artifact_root` が必須である。
- 実 `displayd.sock` には接続しない。
- 実 OS 統合はまだ行わない。

## Phase 2-D

- 実 displayd.sock / 実 Wayland session / 実 input / 実 DRM-KMS に進む前に、隔離VMまたはテスト機の手順を固定する。
- 詳細計画は [XWIN_SCREENSHOT_ISOLATED_VM_PLAN.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_ISOLATED_VM_PLAN.md) に分離する。
- 実行手順は [XWIN_SCREENSHOT_ISOLATED_VM_RUNBOOK.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_ISOLATED_VM_RUNBOOK.md) に分離する。
- checkpoint は [XWIN_SCREENSHOT_PHASE2_CHECKPOINT.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_PHASE2_CHECKPOINT.md) に固定する。
- 実行コマンド列と記録形式は [XWIN_SCREENSHOT_ISOLATED_VM_RUN_MANIFEST.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_ISOLATED_VM_RUN_MANIFEST.md) に分離する。
- 現用OSの displayd.sock / Wayland session / XDG_RUNTIME_DIR / DISPLAY / WAYLAND_DISPLAY は読まない。
- 失敗しても VM またはテスト機だけで閉じる。
