# Xwin Screenshot App

`xwin-screenshot` は TUFF-Xwin の純正スクリーンショットアプリの最小実装である。

## 目的

- GUI 障害を小さく閉じ込める。
- `displayd` との境界を fake client で検証しやすくする。
- 保存先、保存形式、ホットキー、トレイ、UI を独立した小さな責務に分ける。

## 非目的

- KDE 級の巨大デスクトップ環境を作ることではない。
- 実 Wayland session へ直接接続することではない。
- 実 global hotkey 登録を行うことではない。
- 実 system tray 登録を行うことではない。
- 実 DRM / KMS / input device を直接触ることではない。
- xwin-screenshot app本体が直接 PipeWire を触ることではない（displayd が portal から取得した FD を経由して扱うのみ）。

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
- `xwin-screenshot-harness-displayd` は dev-harness feature で起動する dev-only helper であり、production displayd ではない。
- dev-harness feature は default build には含めず、CI でのみ `cargo check/test --workspace --features dev-harness` を追加検証する。
- `scripts/run-xwin-screenshot-isolated-manifest.sh` は dev-harness / fake / isolated-displayd / config flow / negative checks を repo-local `target/xsm/` 配下の run root で再現する補助 runner であり、AgeAssurance boundary の regression も併せて確認する。production displayd ではない。
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
- dev-only harness binary は production displayd ではなく、明示 socket / artifact root の tempdir 検証だけを担う。
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
- repo-local runner はこの CLI / config / harness 経路を手打ちせず再現するための補助であって、production displayd を置き換えるものではない。

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

## Phase 2-E

- プロダクション用の `displayd` バイナリへ明示的なソケットパス経由で接続する Preflight runner を追加した。
- `displayd` は `--socket <PATH>` 引数により、環境変数や自動探索を介さず指定されたパスでサービスを開始できる。
- `scripts/run-xwin-screenshot-real-displayd-preflight.sh` により、repo 内の隔離されたパスで実バイナリ間通信（CaptureOutput契約）を確認する。
- 常駐起動やシステムへのインストールは行わず、一時的なプロセスとして検証を行う。

## Phase 2-F

- `displayd` において、`FakeCaptureBackend` 以外の実画面取得 backend を安全に追加するための足場（Scaffold）を導入した。
- 実画面取得は、デフォルトの `fake` を維持しつつ、`--capture-backend real` と保護フラグ `--allow-real-capture` の両方を明示した場合のみ有効になる（二重の opt-in）。
- 現時点の `RealCaptureBackendStub` は NotImplemented / Unsupported として fail-closed し、DRM/KMS/PipeWire/Wayland 等への自動接触を防止している。
- 実画面取得の成功はこの段階では要求されず、安全な構造分離と拒否の動作を優先している。

## Phase 2-G

- `displayd` において、X11 root window を対象とした実画面取得 backend を実装した。
- X11 キャプチャは、`--capture-backend real`, `--allow-real-capture`, `--capture-method x11`, `--x11-display <DISPLAY>` の 4 つが揃った場合のみ有効になる。
- `DISPLAY` 環境変数の自動参照は行わず、明示的なディスプレイ指定を必須としている。
- 接続失敗、取得失敗、フォーマット変換失敗は全て fail-closed となり、`fake` への自動 fallback は行われない。
- Wayland / PipeWire / Portal 手法は引き続き未実装であり、別フェーズでの対応を予定している。

## Phase 2-H

- Wayland 環境での実画面取得に向け、PipeWire / xdg-desktop-portal 経由のキャプチャ手法を選択可能にする足場（Scaffold）を導入した。
- Portal キャプチャは、`--capture-backend real`, `--allow-real-capture`, `--capture-method portal`, `--allow-portal-capture` の 4 つが揃った場合のみ有効になる（四重 of opt-in）。
- 現時点の `PortalCaptureBackendStub` は NotImplemented / Unsupported として fail-closed し、ダイアログの自動承認や未許可の接続を防止している。
- Xwayland 環境での X11 `BadMatch` を「正しく拒否された状態（fail-closed）」として定義し、予期せぬ fallback がないことを保証している。

## Phase 2-I

- `xdg-desktop-portal` を用いた Wayland 実画面キャプチャの対話的検証経路を実装した。
- ポータル接続の保護のため、従来の四重 opt-in に加えて、GUIダイアログ表示を許可する明示的フラグ `--allow-portal-dialog` の指定を必須とした。
- `PortalCaptureBackend` により、セッション確立からポータル経由での `open_pipe_wire_remote` (PipeWire FD取得) までの接続シーケンスを実装。
- PipeWire 自体のフレーム取得ライブラリ（libpipewire等）がない環境でも安全に動作させるため、セッション成立後に明示的に fail-closed (Ingestion stubbed) とする動作を固定。
- キャプチャの成否によらず `fake` への自動 fallback は行わず、安全に処理を打ち切る。
- 対話的 preflight テスト（Case 11）により、ユーザーの対話操作に基づく検証シーケンスを実証可能にした。
