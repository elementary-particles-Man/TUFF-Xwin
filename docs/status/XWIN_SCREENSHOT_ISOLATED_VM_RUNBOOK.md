# TUFF-Xwin Screenshot Isolated VM Runbook

## Baseline

- main HEAD: `92d607c4c5a1c5fa0541c09636b12894d7b728e8`
- この文書は隔離VM/専用テスト機で実行するためのrunbookであり、実装ではない
- この文書作成時点では VM起動・QEMU起動・実displayd.sock接続・実Wayland session接続を行わない
- 前提文書: [XWIN_SCREENSHOT_ISOLATED_VM_PLAN.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_ISOLATED_VM_PLAN.md)
- Phase2 checkpoint: [XWIN_SCREENSHOT_PHASE2_CHECKPOINT.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_PHASE2_CHECKPOINT.md)
- 実行コマンド列と記録形式は [XWIN_SCREENSHOT_ISOLATED_VM_RUN_MANIFEST.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_ISOLATED_VM_RUN_MANIFEST.md) に分離する

## Runner Usage

- primary execution path: `scripts/run-xwin-screenshot-isolated-manifest.sh`
- runner は repo 直下の `target/xwin-screenshot-isolated-manifest/<run-id>` 配下に `RUN_ROOT` を作る
- runner は step ごとのログ、report、artifact inventory を `RUN_ROOT` に保存する
- 手打ちの command 列は manifest と runbook の参考情報として残す

## Runbook Purpose

- 隔離環境での作業を手順化し、思いつき実行を防ぐ
- 現用OS・現用Wayland session・現用displayd.sockを保護する
- GUI panic と kernel panic を連動させないため、検証手順にも復旧導線を組み込む
- 失敗してもVMまたは専用テスト機だけで閉じる検証にする

## Entry Conditions

- workspace clean
- origin/main 同期済み
- 対象環境はVMまたは専用テスト機のみ
- 現用OSでは実施しない
- CUIまたはSSH recovery pathが確保済み
- snapshotまたは復元手順が確保済み
- 検証用ユーザーを分離済み
- 検証用runtime directoryを明示pathで用意する
- runtime自動探索を使わない

## Host Machine Non-Interference

- 現用OSの XDG_RUNTIME_DIR を読まない
- 現用OSの DISPLAY を読まない
- 現用OSの WAYLAND_DISPLAY を読まない
- 現用OSの displayd.sock に接続しない
- 現用OSの input device / DRM-KMS / PipeWire に触らない
- 現用OSで global hotkey / tray 登録をしない

## Runbook Step 0: Repo Checkout

- 隔離環境内で repo clone または checkout を行う
- checkout 後に `git rev-parse HEAD` を確認する
- 想定HEADと異なる場合は停止する
- `git status --short --branch` が clean であることを確認する
- cleanでない場合は検証へ進まない

## Runbook Step 1: Workspace Validation

- `cargo fmt --check`
- `cargo check --workspace`
- `cargo test -p xwin-sec --test browser_surface_boundary`
- `cargo test --workspace`
- `git diff --check`
- `git status --short --branch`
- いずれかが失敗した場合は以後のCLI検証へ進まない

## Runbook Step 2: Fake Backend CLI

- `xwin-screenshot --backend fake --format png --save-dir <tempdir>`
- `xwin-screenshot --backend fake --format jpeg --save-dir <tempdir>`
- 出力先は必ずtempdir配下にする
- 実displayd.sockは使わない
- 実Wayland sessionは使わない
- 失敗時はログとstderrを保存して停止する

## Runbook Step 3: Isolated Displayd Harness CLI

- tempdir socket path を明示する
- tempdir artifact root を明示する
- dev-only binary `xwin-screenshot-harness-displayd` を `--features dev-harness` 付きで起動できるようにする
- `cargo run -p xwin-screenshot --features dev-harness --bin xwin-screenshot-harness-displayd -- --socket <tempdir/socket> --artifact-root <tempdir/artifacts> --width 2 --height 2 --serve-once`
- `xwin-screenshot --backend isolated-displayd --displayd-socket <tempdir/socket> --artifact-root <tempdir/artifacts> --format png --save-dir <tempdir/out>`
- jpeg でも同等確認を行う
- `/run/user` 配下pathは使わない
- runtime自動探索は使わない
- 本物displayd processは起動しない

## Runbook Step 4: Config File Flow

- 明示 `--config <tempdir/config.toml>` だけを使う
- `XDG_CONFIG_HOME` / `HOME` 自動探索は使わない
- fake backend config flow を確認する
- isolated-displayd backend config flow を確認する
- config内の `displayd_socket` / `artifact_root` はtempdir配下に限定する
- `/run/user` path が混ざったら停止する

## Runbook Step 5: Negative Tests

- policy deny が socket接続前に止まること
- unknown format が拒否されること
- path traversal artifact_path が拒否されること
- absolute artifact_path が拒否されること
- byte length mismatch が拒否されること
- `DisplayEvent::Rejected` が `Result error` に変換されること

## Log and Artifact Collection

- 各stepの stdout/stderr を保存する
- 失敗時の config file を保存する
- 失敗時の artifact root 内容を保存する
- panic が出た場合は backtrace 有無を記録する
- kernel panic または VM 停止が起きた場合は検証停止し、実OS統合へ進まない
- 成功時も生成された PNG/JPEG の path と size を記録する

## Stop Conditions

- workspace が clean でない
- `cargo fmt` / `cargo check` / `cargo test` のいずれかが fail
- `/run/user` path が使われた
- `XDG_RUNTIME_DIR` / `DISPLAY` / `WAYLAND_DISPLAY` が参照された
- 実displayd.sockへ接続しようとした
- 実Wayland sessionへ接続しようとした
- 実input / DRM-KMS / PipeWireへ触れようとした
- panic が発生した
- VM GUI が壊れ、CUI/SSHで回収できない
- kernel panic が発生した

## Success Criteria

- `cargo fmt --check` PASS
- `cargo check --workspace` PASS
- `cargo test -p xwin-sec --test browser_surface_boundary` PASS
- `cargo test --workspace` PASS
- fake backend PNG/JPEG PASS
- isolated-displayd harness PNG/JPEG PASS
- config file fake flow PASS
- config file isolated-displayd flow PASS
- negative tests PASS
- workspace clean維持
- 現用OS非干渉
- 実displayd.sock非接続

## Escalation After Runbook Pass

- runbookが隔離環境で全PASSした後にのみ、別phaseで実displayd.sock接続を検討する
- 実displayd.sock接続phaseでも明示pathのみを使う
- runtime自動探索は継続禁止
- CUI/SSH recovery path と snapshot は必須
- 失敗時ログ・artifact回収手順を先に固定する

## Non-Goals

- この文書で実装を行わない
- この文書でVMを起動しない
- この文書でQEMUを起動しない
- この文書でFedora/Ubuntu等のイメージを取得しない
- この文書で実displayd.sockへ接続しない
- この文書で実Wayland sessionへ接続しない
- この文書で実global hotkey/trayを実装しない
- この文書でDRM/KMS/PipeWire/input deviceへ触らない

## Workspace Hygiene

- 作業開始前に workspace clean を確認する
- 未コミット差分が出た場合は次作業へ進まず専用ブランチで回収する
- dirty file を理由に repo移動・repo削除をしない
- cleanではない状態を放置して次フェーズへ進まない
- AGENTS.md のような運用差分も正式差分として扱う
