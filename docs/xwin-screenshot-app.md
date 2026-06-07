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

## Phase 3

- `DisplaydCaptureArtifact` を安全に ingest して `CapturedFrame` へ変換する。
- artifact path は明示した `allowed_root` 配下に限定する。
- raw RGBA artifact は `width * height * 4` と一致した場合のみ受理する。
- `DisplaydArtifactCaptureClient` により `DisplaydIpcCaptureClient` の返却物を encode 経路へ接続する。
- 実 `displayd.sock` にはまだ接続しない。
- symlink / TOCTOU / 実 runtime artifact は後続 phase の扱いとする。
