# Xwin Screenshot Real Displayd Explicit Socket Preflight

## Purpose

- `TUFF-Xwin` の `xwin-screenshot` を、隔離されたハーネスではなく、プロダクション用の `displayd` バイナリへ明示的なソケットパス経由で接続する準備を行う。
- 現用ランタイムの自動探索、`/run/user` 配下の既存ソケットへの自動接続、システムワイドなインストール、自動起動登録を一切行わずに検証可能な手順を確立する。
- 接続の前提条件、起動方法、失敗時の停止条件、ログ回収形式を事前に固定する。

## Baseline

- main HEAD: `111583d0ef5c7d63e94beb31a1a15cbc97bb824f`
- repo-local 実機検証 (isolated manifest runner) が全 PASS 済みであること。

## Connection Strategy: Explicit Socket Path

- `displayd` 起動時に `--socket <PATH>` (実装予定) または `WAYBROKER_RUNTIME_DIR` を用いて、ソケットパスを完全に制御する。
- `xwin-screenshot` は `--displayd-socket <PATH>` 引数を用い、環境変数や自動探索に頼らず指定されたパスへ直接接続する。
- 既存の `/run/user` / `XDG_RUNTIME_DIR` / `WAYLAND_DISPLAY` / `DISPLAY` は自動参照しない。

## Capture Backend Selection

- `displayd` は `--capture-backend <fake|real>` オプションにより、キャプチャの実装を選択できる。
- `real` を選択する場合、追加の保護フラグ `--allow-real-capture` が必須である（二重の明示的 opt-in）。
- `--capture-method <stub|x11|portal>` により、具体的な取得手法を指定する。既定は `stub`。
- `x11` 手法を用いる場合は `--x11-display <DISPLAY>` が必須であり、環境変数 `DISPLAY` は自動参照されない。
- `portal` 手法を用いる場合は、追加の保護フラグ `--allow-portal-capture` と対話許可フラグ `--allow-portal-dialog` が必須である（対話的実行のための明示的 opt-in 拡張）。
- 実装が未定義または環境が不適合な場合、`fake` に黙って fallback することはなく、明示的にエラーを返して fail-closed する。
- 注: Xwayland 環境では `x11` 手法は `BadMatch` により fail-closed することが期待される挙動である。

## Startup Procedure (Preflight Plan)

1. **Workspace Preparation**:
   - `git status` が clean であることを確認。
   - `cargo build --workspace --features real-x11,real-portal` で X11 および Portal サポートを有効にしてビルド。

2. **Real Displayd Startup**:
   - `target/debug/displayd` を起動。
   - ソケットパスは `target/xsm/preflight/displayd.sock` のように、repo 内の temp ディレクトリを指定する。
   - ログは `target/xsm/preflight/displayd.log` へリダイレクトする。
   - `fake` テスト時は `--capture-backend fake` を明示（既定）。
   - `x11` テスト時は `--capture-backend real --allow-real-capture --capture-method x11 --x11-display :0` 等を明示。
   - `portal` テスト時は `--capture-backend real --allow-real-capture --capture-method portal --allow-portal-capture --allow-portal-dialog` を明示。

3. **Screenshot App Connection**:
   - `target/debug/xwin-screenshot --backend isolated-displayd --displayd-socket <PATH> --artifact-root <PATH> ...` を実行。
   - `displayd` の設定に応じ、`fake` / `stub` / `x11` / `portal` のそれぞれの振る舞い（成功・拒否・取得）を確認する。

## Failure and Stop Conditions

- 指定されたソケットパスに `displayd` がバインドできない。
- `xwin-screenshot` がソケットへ接続できない（Connection Refused 等）。
- `displayd` が起動直後にクラッシュする。
- プロトコルエラー（JSON 変換失敗、メッセージ型不一致等）が発生する。
- 許可されていないパス（`/run/user` 等）へのアクセスが検知される。

## Log and Artifact Recovery

- `RUN_ROOT/logs/displayd.log`: `displayd` の標準出力/標準エラー。
- `RUN_ROOT/logs/screenshot.log`: `xwin-screenshot` の標準出力/標準エラー。
- `RUN_ROOT/XWIN_SCREENSHOT_PREFLIGHT_REPORT.md`: パス、成功・失敗、原因のサマリ。

## Non-Goals (Not handled in this phase)

- 実画面（DRM/KMS）のキャプチャ。
- `systemd` ユニットの作成・登録。
- `/usr/bin` 等へのインストール。
- グローバルホットキー、システムトレイの有効化。
- 実ブラウザプロセスとの連携。
- 自動起動の設定。

## Phase 2-I: Explicit Portal User-Mediated Capture Verification

- **Case 11: Real Portal Capture (User-Mediated)** を preflight スクリプトに追加。
- デフォルト実行時は SKIP され、`--allow-portal-real-capture` オプションが指定された時のみ対話的テストが動作する。
- ユーザーによるダイアログでのキャプチャ許可後、`PortalCaptureBackend` が以下を達成：
  - `Screencast` ポータルプロキシ作成成功
  - セッションの作成完了
  - 画面/ウィンドウ選択ソースの要求
  - `Screencast::start` 成功（セッション開始）
  - `open_pipe_wire_remote` による PipeWire FD のオープン成功
  - PipeWire フレーム取得は行わず、明示的エラーを返して fail-closed することを確認（`FAIL-CLOSED (Session established, ingestion stubbed - SUCCESS)`）。
