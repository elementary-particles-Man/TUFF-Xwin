# TUFF-Xwin Screenshot Phase2 Checkpoint

## Checkpoint Baseline

- Current main HEAD: `b995f33f2d0c0edb23ca94f7c13bf94865cd496e` (Phase 2-N)
- Historical checkpoint main HEAD: `2bfbbfbfde58b5e570ea97175ecc05a629e4ba5c` (Phase 2-A~I Baseline)
- この文書は Phase2 の checkpoint であり、開発履歴と現行の固定化された境界状態を管理するものです。

## Core Purpose Reaffirmed

- TUFF-Xwin の目的は KDE級DEを作ることではない
- 目的は GUI panic と kernel panic を連動させないこと
- GUI / compositor / screenshot app / browser client は部品であり、壊れても局所停止すべきである
- 表示系の失敗を kernel / 他アプリ / 入力 / filesystem / GPU境界へ波及させない

## Historical Milestones (Phase 2-A to Phase 2-I)

以下のフェーズは、開発初期から対話的画面共有の初期経路構築にいたる歴史的チェックポイントです。一部の機能（PipeWire ingestion など）は後続のフェーズで実装され、現行状態へとアップデートされています。

### Phase2-A Fixed: Artifact Root Hardening
- ArtifactRoot は canonical root を保持する
- root は absolute / existing directory / `/run/user`非配下に制限する
- artifact path は相対名のみ許可する
- `..` / `.` / absolute artifact path を拒否する
- symlinkを含む artifact path を拒否する
- Unix では artifact open/read を openat / O_NOFOLLOW / O_DIRECTORY / O_CLOEXEC で harden 済みである
- expected RGBA byte length を checked arithmetic で算出する
- metadata length と read後 length の両方で mismatch を拒否する
- TOCTOU の kernel-level 完全証明は後続phaseに残すが、既知の symlink 差し替えと親ディレクトリ差し替えは安全側へ倒している

### Phase2-B Fixed: Test Harness Displayd
- repo内 test-only harness displayd を追加済み
- dev-only harness binary `xwin-screenshot-harness-displayd` を `--features dev-harness` 付きで起動できる
- tempdir socket のみ使用する
- tempdir artifact root のみ使用する
- CaptureOutput を受けて OutputCaptured または Rejected を返す
- raw RGBA8888 artifact を生成する
- DisplaydUnixSocketTransport / DisplaydArtifactCaptureClient / FileCaptureArtifactReader / ScreenshotApp のE2Eを固定済み
- 本物displayd processは起動しない
- 実displayd.sockには接続しない

### Phase2-C Fixed: Config File Support
- `--config <path>` を追加済み
- config file は TOML
- 自動探索なし
- 環境変数の暗黙読取なし
- `XDG_CONFIG_HOME` / `HOME` / `XDG_RUNTIME_DIR` を読まない
- default -> config file -> CLI explicit override の順で確定する
- fake backend config flow を確認済み
- isolated-displayd backend config flow を確認済み
- 実displayd.sockには接続しない

### Phase2-D Fixed: Isolated VM Plan and Runbook
- 隔離VM/専用テスト機計画を追加済み
- 隔離VM/専用テスト機runbookを追加済み
- repo-local manifest runner を `scripts/run-xwin-screenshot-isolated-manifest.sh` として追加済み
- 現用OSでは実施しない
- CUI/SSH recovery path を先に確保する
- snapshotまたは復元手順を先に確保する
- 検証用ユーザーと検証用runtime directoryを分離する
- runbookは repo checkout / cargo検証 / fake backend / isolated-displayd harness / config flow / negative tests を順序化する
- runbook作成時点では VM起動も実displayd.sock接続も行っていない

### Phase2-E Fixed: Real Displayd Explicit Socket Preflight
- プロダクション用 `displayd` バイナリへの明示ソケット接続準備を完了。
- `displayd` に `--socket <PATH>` オプションを追加し、環境変数に頼らずソケットパスを指定可能にした。
- `scripts/run-xwin-screenshot-real-displayd-preflight.sh` を追加。
- 自動ランタイム探索を回避し、repo 内の隔離されたパスでのみ実バイナリ間通信を確認する手順を固定。
- 失敗時のログ回収と停止手順を定義済み。

### Phase2-F Fixed: Real Capture Backend Scaffold
- `displayd` において、`FakeCaptureBackend` 固定の状態から、実画面取得 backend を選択可能な構造へ分離。
- `--capture-backend <fake|real>` および保護フラグ `--allow-real-capture` を導入。
- `real` を選択する場合、両方のフラグが必須（二重の明示的 opt-in）であり、欠落している場合は起動を拒否する。
- 現段階の `RealCaptureBackendStub` は NotImplemented として fail-closed し、DRM/KMS/PipeWire/Wayland 等への自動接触を防止。
- 未実装時に `fake` へ黙って fallback しないことを保証。

### Phase2-G Fixed: Explicit X11 Real Capture Backend
- `displayd` において、X11 root window を対象とした実画面取得 backend を実装。
- `--capture-method <stub|x11>` および `--x11-display <DISPLAY>` を導入。
- X11 キャプチャは、`--capture-backend real`, `--allow-real-capture`, `--capture-method x11`, `--x11-display` の 4 つが揃った場合のみ有効化。
- `DISPLAY` 環境変数の自動参照を禁止し、予期せぬセッションへの接続を防止。
- 接続失敗、root window 取得失敗、フォーマット変換失敗は全て fail-closed とし、`fake` への fallback を排除。
- X11 以外の Wayland / PipeWire / DRM-KMS / Portal は引き続き未実装として明示的に分離。

### Phase2-H Fixed: PipeWire Portal Capture Scaffold
- Wayland 環境での実画面取得に向け、PipeWire / xdg-desktop-portal 経由のキャプチャ手法を選択可能にする scaffold を導入。
- `portal` 手法の明示選択をサポートし、保護フラグ `--allow-portal-capture` を必須化（四重の明示的 opt-in）。
- 初期段階の `PortalCaptureBackendStub` は NotImplemented/Unsupported として fail-closed し、ダイアログの自動承認や未許可の D-Bus 接続を防止。
- Xwayland環境における X11 `BadMatch` を期待される fail-closed (Case 7 SUCCESS) として整理。

### Phase2-I Fixed: Explicit Portal User-Mediated Capture
- xdg-desktop-portal を用いた Wayland 実画面キャプチャの対話的検証経路を実装。
- Portal 接続に四重 opt-in（`--capture-backend real`, `--allow-real-capture`, `--capture-method portal`, `--allow-portal-capture`）に加えて対話許可フラグ（`--allow-portal-dialog`）を強制。
- `PortalCaptureBackend` にて、セッション作成 -> 画面選択ダイアログ表示 -> PipeWire FD 取得までのロジックを実装。
- PipeWire フレーム自体の取得は、環境依存（libpipewire等）を考慮し、セッション成立と PipeWire リモートオープンを確認した上で明示的に fail-closed 扱いとする（`FAIL-CLOSED (Session established, ingestion stubbed - SUCCESS)`）。
- キャプチャの成否に関わらず、`fake` への自動 fallback は行われない。
- 検証スクリプト `run-xwin-screenshot-real-displayd-preflight.sh` に対話的テスト Case 11 を追加し、明示フラグがない場合は SKIP される構造を保証。

## Current Fixed State (Phase 2-J to Phase 2-M)

以下のマイルストーンは、実フレームの取り込み成功から、機能の堅牢化、およびドッグフード適用のための境界ロックとクリーンアップが完了した現在の固定化された状態です。

### Phase 2-J Fixed: Portal/PipeWire Real Frame Ingestion Success
- **English**: Established real PipeWire streams using file descriptors returned by the xdg-desktop-portal. Ingestion of raw RGBA pixel buffers into the displayd memory space has been fully implemented and verified.
- **日本語**: xdg-desktop-portal から返されたファイル記述子（FD）を利用して実際の PipeWire ストリームを確立。生RGBAピクセルバッファを displayd のメモリ空間へ直接取り込む（Ingestion）本番経路を実装・検証しました。

### Phase 2-K Fixed: PipeWire Frame Ingestion Hardening
- **English**: Hardened the PipeWire frame loop against unexpected disconnections, stride/dimension mismatches, and buffer starvation. Enforced strict fail-closed validations to prevent processing corrupted frames.
- **日本語**: 予期せぬ切断、ストライドや解像度の不一致、バッファ不足などの異常状態に対して PipeWire フレームループを堅牢化。破損フレームの混入を防ぐため、厳密な fail-closed バリデーションを適用しました。

### Phase 2-L Fixed: Dogfood Capture Boundary Lock
- **English**: Locked the application behaviors for direct dogfooding. Fixed the default save path to `$HOME/Pictures/TUFF-Xwin` (with custom `--save-dir` support), applied millisecond-precision timestamps to filename templates to avoid rapid-click overwrites, detected portal cancellations gracefully, and enforced manifest-based backups with marker-gated cleanup for keybindings.
- **日本語**: ドッグフード適用のために挙動を固定しました。デフォルトの保存先を `$HOME/Pictures/TUFF-Xwin` にロック（`--save-dir` にも対応）し、ミリ秒精度のタイムスタンプ付きユニークファイル名による連打競合を防止。KDE ポータルのキャンセル検知や、マニフェストと専用マーカー（`marker-gated cleanup`）を用いたホットキーバインドの安全な退避・復元を確立しました。

### Phase 2-M Fixed: Cleanup and Fixed-Point Polish
- **English**: Removed unused legacy components (such as the legacy `BACKUP_FILE` variable in restore script) and cleaned up codebase comments. Aligned all test references to match the manifest-based rollback architecture, and verified a 100% clean check/test validation workspace.
- **日本語**: 復元スクリプト内の未使用の古い `BACKUP_FILE` 変数を削除し、コード内のコメントを整理。テスト内の参照をすべてマニフェストベースのロールバック構造と同期させ、ワークスペースの全テストおよびバリデーションの完全なパスを確認しました。

## Current Verified Boundaries (As of HEAD `b995f33f2d0c0edb23ca94f7c13bf94865cd496e`)

- **PipeWire Ingestion Status**: PipeWire frame ingestion is fully implemented and hardened (Phase 2-J/K). It is no longer stubbed.
- **KDE Shortcut / Path Restoration**: Shortcut configuration and systemd path variable modifications are safely backed up with timestamps and restored via `manifest.tsv` (Phase 2-L).
- **No Automatic Socket Binding**: `displayd` socket does not bind automatically; socket paths must be explicitly specified via `--socket <PATH>`.
- **Hostile Browser Protection**: Hostile browser clients are denied capture permissions unless explicit visible grants are established.
- **Marker-Gated Rollback**: New configurations (e.g. environment config and flameshot wrapper) created by TUFF-Xwin carry unique `# TUFF-Xwin` markers. The restore script only deletes files matching these markers, ensuring user customizations are never deleted (Phase 2-L/M).
- **Opt-in Portals**: Real display capturing is gated behind four-fold explicit opt-in arguments (`--capture-backend real`, `--allow-real-capture`, `--capture-method portal`, `--allow-portal-capture`).

## Current xwin-screenshot Capabilities

- **Real Frame Capture**: Captures real screen content in Wayland environments via PipeWire and xdg-desktop-portal.
- **Fail-Safe Dialogue Integration**: Interactive clipboard copy / save folder dialogue pops up on success; gracefully defaults to save on clipboard failure.
- **Safe Hotkey Installer / Rollback**: Script-driven shortcut bind installation with zero hardcoded values for restoring bindings.
- **Preflight and Isolated Validation**: Test runners can verify all configurations (fake, X11, portal, and fallback states) cleanly within local target paths.

## Still Not Done / Remaining Work

- **Systemd Service Integration**: Permanent background daemon integration (systemd unit registration) is not yet active.
- **DRM/KMS Direct Connection**: Direct framebuffer capture via DRM/KMS without compositor mediation is not implemented.
- **System Tray Integration**: Full-featured system tray notifications and menus.
- **Advanced Sandbox Hardening**: Chrome process authentication and sandbox protections remain under design.

## Conditions Before Any Real displayd.sock Phase

- workspace clean
- origin/main同期
- 隔離VMまたは専用テスト機のみ
- 現用OSでは実施しない
- CUI/SSH recovery path確保
- snapshotまたは復元手順確保
- 検証用ユーザー分離
- 検証用runtime directoryを明示pathで用意
- runtime自動探索を使わない
- 実displayd.sock pathは明示指定のみ
- policy hook が transport 前に走ることを再確認
- 失敗時ログ・artifact回収手順を先に固定
- 実行コマンド列と記録形式は [XWIN_SCREENSHOT_ISOLATED_VM_RUN_MANIFEST.md](XWIN_SCREENSHOT_ISOLATED_VM_RUN_MANIFEST.md) に分離する

## Recommended Next Choices

- 安全側: isolated VM run manifest を追加し、実際に隔離VMで叩くコマンド列とログ保存形式を固定する
- 実装側: openat/no-follow 系のTOCTOU hardening を別phaseで検討する
- 実行側: runbook を隔離VM/専用テスト機で実行する。ただしこの repo 作業では VM を起動しない
- 実OS接続側: runbook 全PASS後にのみ、別phaseで実 displayd.sock 明示path接続を検討する

## Workspace Hygiene Fixed

- 作業開始前に workspace clean を確認する
- 未コミット差分が出た場合は次作業へ進まず専用ブランチで回収する
- dirty file を理由に repo移動・repo削除をしない
- clean でない状態を放置して次フェーズへ進まない
- AGENTS.md のような運用差分も正式差分として扱う
- checkpoint / plan / runbook を実装完了扱いしない
- 読めない Web / PDF / 一次資料は未読として止まる。補完しない

## Acceptance State for This Checkpoint

- `cargo fmt --check` PASS
- `cargo check --workspace` PASS
- `cargo check -p displayd --features real-x11,real-portal` PASS
- `cargo test --workspace` PASS
- `bash -n scripts/run-xwin-screenshot-real-displayd-preflight.sh` PASS
- `bash -n scripts/run-xwin-screenshot-isolated-manifest.sh` PASS
- `git diff --check` PASS
- origin/main同期
- workspace clean
