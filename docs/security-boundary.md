# TUFF-Xwin Security Boundary

TUFF-Xwin は KDE 級の完全なデスクトップ環境を目標にしない。
最終目的は、GUI 側の panic や broker 側の失敗が kernel panic や実セッション障害へ連動しないように封じ込めることにある。

browser-as-hostile-client 境界の詳細は [browser-security-boundary.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/browser-security-boundary.md) に分離している。

## 方針

- 実 Wayland session には接続しない。
- 実 DRM / KMS / PipeWire / input device には触らない。
- IPC は repo 内の tempdir / isolated socket / fake backend で検証する。
- screenshot app は hardening 後に実装する。
- 小型 Xwin アプリは broker 境界検証を兼ねる実用品として扱う。
- browser client は hostile とみなし、clipboard / file picker / drag-and-drop / screen capture / IME / GPU / compositor 境界は明示 grant でのみ跨がせる。
- browser exploit の検出や web page 内容の監視は Xwin の責務にしない。

## Screenshot Phase 2

- `xwin-screenshot` は `DisplayCommand::CaptureOutput` / `DisplayEvent::OutputCaptured` までの protocol-backed fake transport 境界を持つ。
- 実 `displayd.sock` にはまだ接続しない。
- 実 artifact 読み込み、実 raw 変換、実 global hotkey、実 tray は後続 phase に分ける。
- screen capture 要求は `xwin-sec` の policy hook で事前判定する。

## Screenshot Phase 2-B

- 本物の `displayd` process ではなく repo 内の test harness displayd を使う。
- tempdir socket と tempdir artifact root だけを使い、`CaptureOutput` -> `OutputCaptured` -> RGBA8888 artifact -> ingest -> PNG/JPEG encode の契約 E2E を固定する。
- 実 `displayd.sock` には接続しない。
- 実 Wayland session / DRM-KMS / PipeWire / input device には触らない。
- 実 OS 統合は後続の隔離環境 phase まで行わない。

## Screenshot Phase 3

- `DisplaydCaptureArtifact` は Phase2-A の方針として相対 artifact 名のみ ingest し、`allowed_root` 配下に join された結果だけを読む。
- `displayd` の `OutputCaptured` は `width` / `height` / `format` / `artifact_path` を返し、現時点の format 契約は `RGBA8888` とする。
- `displayd` 側 writer は native `u32` dump ではなく明示的な RGBA8888 byte stream を書く。
- artifact 本体は PNG/JPEG ではなく raw RGBA8888 bytes として扱う。
- raw RGBA artifact は `width * height * 4` の整合性を通してから `CapturedFrame` 化する。
- Unix では artifact open/read を openat / O_NOFOLLOW / O_DIRECTORY / O_CLOEXEC で harden する。
- `DisplaydArtifactCaptureClient` は `DisplaydIpcCaptureClient` の返却物を encode 経路へ渡す。
- 実 `displayd.sock` にはまだ接続しない。
- symlink は Phase2-A で拒否し、kernel-level の完全証明と実 runtime artifact は後続 phase として残す。

## Screenshot Phase2-A

- `ArtifactRoot` は canonical root を保持する。
- root は absolute かつ既存ディレクトリでなければならない。
- `/run/user` 配下の root は拒否する。
- artifact path は相対名のみを許可し、`..` / `.` / absolute path は拒否する。
- symlink を含む artifact path は Phase2-A では拒否する。
- TOCTOU の完全対策は後続 phase に残す。

## Screenshot CLI backend selection

- `xwin-screenshot` は `fake` と `isolated-displayd` を明示選択できる。
- `--config PATH` で明示 config file を読む。
- `isolated-displayd` は `--displayd-socket` と `--artifact-root` を必須とする。
- 実 `displayd.sock` の自動探索は行わない。
- `/run/user`、`XDG_RUNTIME_DIR`、実 Wayland session は読まない。
- CLI backend selection でも policy hook は transport より前に走る。

## Screenshot Phase2-C

- config file は自動探索しない。
- `XDG_CONFIG_HOME` / `HOME` / `XDG_RUNTIME_DIR` / `WAYLAND_DISPLAY` / `DISPLAY` は読まない。
- 設定確定順は default -> config file -> CLI override である。
- isolated-displayd backend では `displayd_socket` と `artifact_root` が必須である。
- 実 `displayd.sock` には接続しない。
- 実 OS 統合はまだ行わない。

## Screenshot Phase2-D

- 実 displayd.sock / 実 Wayland session / 実 input / 実 DRM-KMS へ進む前に、隔離VMまたはテスト機の手順を固定する。
- この段階の文書は [XWIN_SCREENSHOT_ISOLATED_VM_PLAN.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_ISOLATED_VM_PLAN.md) に分離する。
- 実行手順は [XWIN_SCREENSHOT_ISOLATED_VM_RUNBOOK.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_ISOLATED_VM_RUNBOOK.md) に分離する。
- checkpoint は [XWIN_SCREENSHOT_PHASE2_CHECKPOINT.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_PHASE2_CHECKPOINT.md) に固定する。
- 実行コマンド列と記録形式は [XWIN_SCREENSHOT_ISOLATED_VM_RUN_MANIFEST.md](/mnt/thpdoc/Develop/TUFF-Xwin/docs/status/XWIN_SCREENSHOT_ISOLATED_VM_RUN_MANIFEST.md) に分離する。
- 現用OSの displayd.sock / Wayland session / XDG_RUNTIME_DIR / DISPLAY / WAYLAND_DISPLAY は参照しない。
- 失敗しても VM またはテスト機だけで閉じることを前提にする。

## 入力境界

- IPC の JSON line と wire payload は明示的な上限で制限する。
- malformed input は panic ではなく Result error で返す。
- runtime path は session id と artifact name をサニタイズする。
- path traversal や absolute path の注入は拒否する。

## 失敗封じ込め

- capture backend failure は panic にしない。
- artifact 保存前に buffer サイズと寸法の整合性を検証する。
- watchdog / session instance id の不一致は拒否する。
