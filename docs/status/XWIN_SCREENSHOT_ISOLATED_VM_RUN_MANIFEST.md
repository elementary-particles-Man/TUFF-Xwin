# TUFF-Xwin Screenshot Isolated VM Run Manifest

## Manifest Baseline

- main HEAD: `7ff095bcc346d0b6e949fd5b45a4516c64bd11d2`
- この文書は隔離VM/専用テスト機で実行する検証コマンドのmanifestであり、実装ではない
- この文書作成時点では VM起動・QEMU起動・実displayd.sock接続・実Wayland session接続を行わない
- 前提文書: [XWIN_SCREENSHOT_PHASE2_CHECKPOINT.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_PHASE2_CHECKPOINT.md)
- 前提文書: [XWIN_SCREENSHOT_ISOLATED_VM_RUNBOOK.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_ISOLATED_VM_RUNBOOK.md)

## Manifest Purpose

- runbookを実行可能な検証単位へ分解する
- 隔離環境で叩くコマンド列を事前固定する
- stdout/stderr/log/artifactの保存名を固定する
- pass/fail記録形式を固定する
- 思いつきで実displayd.sockや現用runtimeへ触ることを防ぐ

## Execution Environment Assumptions

- 対象はVMまたは専用テスト機のみ
- 現用OSでは実行しない
- CUI/SSH recovery path確保済み
- snapshotまたは復元手順確保済み
- 検証用ユーザー分離済み
- 検証用workspaceは clean
- origin/main同期済み
- runtime自動探索は禁止
- すべての一時socket/artifact/outputはtempdir配下

## Manifest Variables

- `REPO_DIR`: TUFF-Xwin checkout root
- `RUN_ROOT`: isolated test run root
- `TMP_ROOT`: `RUN_ROOT/tmp`
- `SOCKET_PATH`: `TMP_ROOT/xwin-displayd-harness.sock`
- `ARTIFACT_ROOT`: `TMP_ROOT/artifacts`
- `OUT_DIR`: `TMP_ROOT/out`
- `CONFIG_DIR`: `TMP_ROOT/config`
- `LOG_DIR`: `RUN_ROOT/logs`
- `REPORT_FILE`: `RUN_ROOT/XWIN_SCREENSHOT_RUN_REPORT.md`

## Forbidden Variables

- `XDG_RUNTIME_DIR` を使わない
- `DISPLAY` を使わない
- `WAYLAND_DISPLAY` を使わない
- `XDG_CONFIG_HOME` 自動探索を使わない
- `HOME` 自動探索を使わない
- `/run/user` 配下pathを使わない

## Manifest Step 0: Identity and Workspace

- command: `pwd`
- command: `git rev-parse --show-toplevel`
- command: `git rev-parse HEAD`
- command: `git status --short --branch`
- pass: HEAD が `7ff095bcc346d0b6e949fd5b45a4516c64bd11d2` または実行時に明示指定された検証対象SHAである
- pass: workspace clean
- fail: cleanでない場合は以後の検証へ進まない

## Manifest Step 1: Repo Validation

- command: `cargo fmt --check`
- command: `cargo check --workspace`
- command: `cargo test --workspace`
- command: `git diff --check`
- pass: すべて PASS
- fail: 1つでも失敗した場合は CLI 検証へ進まない
- log: `LOG_DIR/step1-cargo-fmt.log`
- log: `LOG_DIR/step1-cargo-check.log`
- log: `LOG_DIR/step1-cargo-test.log`
- log: `LOG_DIR/step1-git-diff-check.log`

## Manifest Step 2: Fake Backend CLI PNG

- command: `xwin-screenshot --backend fake --format png --save-dir OUT_DIR`
- pass: `OUT_DIR` 配下に PNG が生成される
- pass: 実displayd.sockへ接続しない
- pass: 実Wayland sessionへ接続しない
- log: `LOG_DIR/step2-fake-png.log`
- record: generated PNG path and file size

## Manifest Step 3: Fake Backend CLI JPEG

- command: `xwin-screenshot --backend fake --format jpeg --save-dir OUT_DIR`
- pass: `OUT_DIR` 配下に JPEG が生成される
- pass: 実displayd.sockへ接続しない
- pass: 実Wayland sessionへ接続しない
- log: `LOG_DIR/step3-fake-jpeg.log`
- record: generated JPEG path and file size

## Manifest Step 4: Dev Harness Displayd PNG

- command: `cargo run -p xwin-screenshot --features dev-harness --bin xwin-screenshot-harness-displayd -- --socket SOCKET_PATH --artifact-root ARTIFACT_ROOT --width 2 --height 2 --serve-once`
- pass: dev-only binary は production displayd ではない
- pass: `SOCKET_PATH` は tempdir配下
- pass: `ARTIFACT_ROOT` は tempdir配下
- pass: `/run/user` path を使わない
- pass: 実displayd.sock を使わない
- log: `LOG_DIR/step4-harness-displayd-png.log`
- record: harness readiness, artifact path, byte size

## Manifest Step 5: Isolated Displayd Harness PNG

- command: `xwin-screenshot --backend isolated-displayd --displayd-socket SOCKET_PATH --artifact-root ARTIFACT_ROOT --format png --save-dir OUT_DIR`
- pass: `SOCKET_PATH` は tempdir配下
- pass: `ARTIFACT_ROOT` は tempdir配下
- pass: `OUT_DIR` 配下に PNG が生成される
- pass: `/run/user` path を使わない
- pass: 本物displayd process を起動しない
- log: `LOG_DIR/step5-isolated-displayd-png.log`
- record: generated artifact path, PNG path, file size

## Manifest Step 6: Dev Harness Displayd JPEG

- command: `cargo run -p xwin-screenshot --features dev-harness --bin xwin-screenshot-harness-displayd -- --socket SOCKET_PATH --artifact-root ARTIFACT_ROOT --width 2 --height 2 --serve-once`
- pass: dev-only binary は production displayd ではない
- pass: `SOCKET_PATH` は tempdir配下
- pass: `ARTIFACT_ROOT` は tempdir配下
- pass: `/run/user` path を使わない
- pass: 実displayd.sock を使わない
- log: `LOG_DIR/step6-harness-displayd-jpeg.log`
- record: harness readiness, artifact path, byte size

## Manifest Step 7: Isolated Displayd Harness JPEG

- command: `xwin-screenshot --backend isolated-displayd --displayd-socket SOCKET_PATH --artifact-root ARTIFACT_ROOT --format jpeg --save-dir OUT_DIR`
- pass: `SOCKET_PATH` は tempdir配下
- pass: `ARTIFACT_ROOT` は tempdir配下
- pass: `OUT_DIR` 配下に JPEG が生成される
- pass: `/run/user` path を使わない
- pass: 本物displayd process を起動しない
- log: `LOG_DIR/step7-isolated-displayd-jpeg.log`
- record: generated artifact path, JPEG path, file size

## Manifest Step 8: Config File Fake Flow

- create: `CONFIG_DIR/fake-png.toml`
- create: `CONFIG_DIR/fake-jpeg.toml`
- command: `xwin-screenshot --config CONFIG_DIR/fake-png.toml`
- command: `xwin-screenshot --config CONFIG_DIR/fake-jpeg.toml`
- pass: config fileは明示pathのみ
- pass: `XDG_CONFIG_HOME` / `HOME` 自動探索なし
- pass: `OUT_DIR` 配下に PNG/JPEG が生成される
- log: `LOG_DIR/step8-config-fake.log`

## Manifest Step 9: Config File Isolated Displayd Flow

- prep: `cargo run -p xwin-screenshot --features dev-harness --bin xwin-screenshot-harness-displayd -- --socket SOCKET_PATH --artifact-root ARTIFACT_ROOT --width 2 --height 2 --serve-once`
- create: `CONFIG_DIR/isolated-displayd-png.toml`
- create: `CONFIG_DIR/isolated-displayd-jpeg.toml`
- command: `xwin-screenshot --config CONFIG_DIR/isolated-displayd-png.toml`
- command: `xwin-screenshot --config CONFIG_DIR/isolated-displayd-jpeg.toml`
- pass: `displayd_socket` と `artifact_root` は tempdir配下
- pass: `/run/user` path を使わない
- pass: `OUT_DIR` 配下に PNG/JPEG が生成される
- log: `LOG_DIR/step9-harness-displayd.log`
- log: `LOG_DIR/step9-config-isolated-displayd.log`

## Manifest Step 10: Negative Contract Checks

- check: policy deny stops before socket transport
- check: unknown format rejected
- check: empty artifact_path rejected
- check: absolute artifact_path rejected
- check: path traversal artifact_path rejected
- check: byte length mismatch rejected
- check: `DisplayEvent::Rejected` が `Result error` に変換される
- pass: すべて拒否される
- log: `LOG_DIR/step10-negative-contract.log`

## Report Format

- `REPORT_FILE` に各 step の PASS/FAIL を記録する
- 各 step の command を記録する
- 各 step の stdout/stderr log path を記録する
- 生成された PNG/JPEG/artifact path を記録する
- 生成物の byte size を記録する
- panic が出た場合は panic 有無と backtrace 有無を記録する
- kernel panic または VM 停止が起きた場合は最終結果を `FAIL-KERNEL` として記録する

## Stop Conditions

- workspace cleanでない
- `cargo fmt` / `cargo check` / `cargo test` のいずれかが fail
- `/run/user` path が使われた
- `XDG_RUNTIME_DIR` / `DISPLAY` / `WAYLAND_DISPLAY` が参照された
- 実displayd.sockへ接続しようとした
- 実Wayland sessionへ接続しようとした
- 実input / DRM-KMS / PipeWireへ触れようとした
- panicが発生した
- VM GUIが壊れてCUI/SSHで回収できない
- kernel panicが発生した

## Success Criteria

- Step 0 PASS
- Step 1 PASS
- Step 2 PASS
- Step 3 PASS
- Step 4 PASS
- Step 5 PASS
- Step 6 PASS
- Step 7 PASS
- Step 8 PASS
- workspace clean維持
- 現用OS非干渉
- 実displayd.sock非接続
- 本物displayd process非起動

## Non-Goals

- この文書で実装しない
- この文書でVMを起動しない
- この文書でQEMUを起動しない
- この文書でOSイメージを取得しない
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
