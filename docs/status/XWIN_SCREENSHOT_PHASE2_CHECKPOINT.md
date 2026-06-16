# TUFF-Xwin Screenshot Phase2 Checkpoint

## Checkpoint Baseline

- main HEAD: `2bfbbfbfde58b5e570ea97175ecc05a629e4ba5c`
- この文書は Phase2-A〜D の checkpoint であり、release ではない
- この文書は実OS統合前の安全境界固定であり、実装ではない
- この文書作成時点では VM起動・実displayd.sock接続・実Wayland session接続を行わない

## Core Purpose Reaffirmed

- TUFF-Xwin の目的は KDE級DEを作ることではない
- 目的は GUI panic と kernel panic を連動させないこと
- GUI / compositor / screenshot app / browser client は部品であり、壊れても局所停止すべきである
- 表示系の失敗を kernel / 他アプリ / 入力 / filesystem / GPU境界へ波及させない

## Phase2-A Fixed: Artifact Root Hardening

- ArtifactRoot は canonical root を保持する
- root は absolute / existing directory / `/run/user`非配下に制限する
- artifact path は相対名のみ許可する
- `..` / `.` / absolute artifact path を拒否する
- symlinkを含む artifact path を拒否する
- Unix では artifact open/read を openat / O_NOFOLLOW / O_DIRECTORY / O_CLOEXEC で harden 済みである
- expected RGBA byte length を checked arithmetic で算出する
- metadata length と read後 length の両方で mismatch を拒否する
- TOCTOU の kernel-level 完全証明は後続phaseに残すが、既知の symlink 差し替えと親ディレクトリ差し替えは安全側へ倒している

## Phase2-B Fixed: Test Harness Displayd

- repo内 test-only harness displayd を追加済み
- dev-only harness binary `xwin-screenshot-harness-displayd` を `--features dev-harness` 付きで起動できる
- tempdir socket のみ使用する
- tempdir artifact root のみ使用する
- CaptureOutput を受けて OutputCaptured または Rejected を返す
- raw RGBA8888 artifact を生成する
- DisplaydUnixSocketTransport / DisplaydArtifactCaptureClient / FileCaptureArtifactReader / ScreenshotApp のE2Eを固定済み
- 本物displayd processは起動しない
- 実displayd.sockには接続しない

## Phase2-C Fixed: Config File Support

- `--config <path>` を追加済み
- config file は TOML
- 自動探索なし
- 環境変数の暗黙読取なし
- `XDG_CONFIG_HOME` / `HOME` / `XDG_RUNTIME_DIR` を読まない
- default -> config file -> CLI explicit override の順で確定する
- fake backend config flow を確認済み
- isolated-displayd backend config flow を確認済み
- 実displayd.sockには接続しない

## Phase2-D Fixed: Isolated VM Plan and Runbook

- 隔離VM/専用テスト機計画を追加済み
- 隔離VM/専用テスト機runbookを追加済み
- repo-local manifest runner を `scripts/run-xwin-screenshot-isolated-manifest.sh` として追加済み
- 現用OSでは実施しない
- CUI/SSH recovery path を先に確保する
- snapshotまたは復元手順を先に確保する
- 検証用ユーザーと検証用runtime directoryを分離する
- runbookは repo checkout / cargo検証 / fake backend / isolated-displayd harness / config flow / negative tests を順序化する
- runbook作成時点では VM起動も実displayd.sock接続も行っていない

## Phase2-E Fixed: Real Displayd Explicit Socket Preflight

- プロダクション用 `displayd` バイナリへの明示ソケット接続準備を完了。
- `displayd` に `--socket <PATH>` オプションを追加し、環境変数に頼らずソケットパスを指定可能にした。
- `scripts/run-xwin-screenshot-real-displayd-preflight.sh` を追加。
- 自動ランタイム探索を回避し、repo 内の隔離されたパスでのみ実バイナリ間通信を確認する手順を固定。
- 失敗時のログ回収と停止手順を定義済み。

## Phase2-F Fixed: Real Capture Backend Scaffold

- `displayd` において、`FakeCaptureBackend` 固定の状態から、実画面取得 backend を選択可能な構造へ分離。
- `--capture-backend <fake|real>` および保護フラグ `--allow-real-capture` を導入。
- `real` を選択する場合、両方のフラグが必須（二重の明示的 opt-in）であり、欠落している場合は起動を拒否する。
- 現段階の `RealCaptureBackendStub` は NotImplemented として fail-closed し、DRM/KMS/PipeWire/Wayland 等への自動接触を防止。
- 未実装時に `fake` へ黙って fallback しないことを保証。

## Phase2-G Fixed: Explicit X11 Real Capture Backend

- `displayd` において、X11 root window を対象とした実画面取得 backend を実装。
- `--capture-method <stub|x11>` および `--x11-display <DISPLAY>` を導入。
- X11 キャプチャは、`--capture-backend real`, `--allow-real-capture`, `--capture-method x11`, `--x11-display` の 4 つが揃った場合のみ有効化。
- `DISPLAY` 環境変数の自動参照を禁止し、予期せぬセッションへの接続を防止。
- 接続失敗、root window 取得失敗、フォーマット変換失敗は全て fail-closed とし、`fake` への fallback を排除。
- X11 以外の Wayland / PipeWire / DRM-KMS / Portal は引き続き未実装として明示的に分離。

## Current Verified Boundaries

- 実displayd.sock 非接続
- 実Wayland session 非接続
- 実DRM/KMS/PipeWire/input device 非接触
- 実global hotkey 非登録
- 実system tray 非登録
- 本物displayd process 非起動 (Preflight runnerによる一時起動を除く)
- browser surface boundary は main に固定済み
- AgeAssuranceBrowserSurfaceBoundary (PR #4) は main に統合済み
- `displayd` は `--socket <PATH>` による明示バインドをサポート済み
- `displayd` は `--capture-backend` / `--capture-method` による明示手法選択をサポート
- real capture は二重または四重の明示的 opt-in でのみ許可
- X11 接続時の `DISPLAY` 環境変数自動参照なし
- runtime自動探索なし
- `/run/user` path 拒否
- policy deny は transport 前に停止
- artifact contract mismatch は保存前に拒否
- Unix artifact open/read は openat / O_NOFOLLOW 系で harden 済み
- failure は panic ではなく Result error に寄せる

## Current xwin-screenshot Capabilities

- fake backend で PNG/JPEG 保存が可能
- isolated-displayd backend を明示socket pathとartifact rootで構成可能
- config file から fake / isolated-displayd backend を構成可能
- CLI override で config file 値を上書き可能
- repo-local manifest runner で fake / dev-harness / isolated-displayd / config flow / negative checks をまとめて再現できる
- real displayd explicit socket preflight runner により、プロダクションバイナリ間の明示接続、手法選択、X11実接続（opt-in時のみ）を再現できる
- dev-harness feature は CI でも `cargo check/test --workspace --features dev-harness` で検証される
- test harness displayd と dev-only harness binary で CaptureOutput -> OutputCaptured -> RGBA8888 artifact -> ingest -> PNG/JPEG encode を repo内E2E確認可能
- browser hostile clientの screen capture は explicit visible grant なしでは拒否される

## Still Not Done

- 実displayd.sock接続
- 実Wayland session統合
- 実global hotkey登録
- 実system tray登録
- 実DRM/KMS/PipeWire/input device接続
- 実runtime artifact読み込み
- 本物displayd process起動 (Preflight段階であり、常駐起動は未実施)
- openat/no-follow による kernel-level TOCTOU 完全証明
- 実Chromeプロセス判定
- Chrome/V8 exploit検出
- ブラウザsandbox再実装
- 実OS policy enforcement
- release作成

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
- 実行コマンド列と記録形式は [XWIN_SCREENSHOT_ISOLATED_VM_RUN_MANIFEST.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_ISOLATED_VM_RUN_MANIFEST.md) に分離する

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
- `cargo check --workspace --features real-x11` PASS
- `cargo test --workspace --features real-x11` PASS
- `cargo test -p xwin-sec --test browser_surface_boundary` PASS
- `cargo test -p xwin-sec --test age_assurance_browser_surface_boundary` PASS
- `bash -n scripts/run-xwin-screenshot-real-displayd-preflight.sh` PASS
- `scripts/run-xwin-screenshot-real-displayd-preflight.sh` SUCCESS (including X11 backend selection safety)
- `git diff --check` PASS
- origin/main同期
- workspace clean
- 稼働中OS非干渉
- VM未起動
- 実displayd.sock未接続 (自動探索・システム接続なし)
- 本物displayd process未起動 (Preflight runnerによる一時起動を除く)
- Real Capture Backend 選択は fail-closed である
- X11 Real Capture は明示 opt-in 時のみ動作する
