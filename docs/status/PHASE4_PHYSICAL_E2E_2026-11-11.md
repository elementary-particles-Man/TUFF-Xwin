# TUFF-Xwin Phase 4 実機残項目 E2E 結果

実行日: 2026-11-11 JST
対象: `ee3a678ae90832763590683d48cd4de81de0c728`（PR #32 merge済み）

## 結果

総合判定: **BLOCKED**

### PASS

- fmt / check / test / Clippy / dev-check / dev-harness
- live-topology 10回連続 PASS
- `production_client_error`: 0件
- canonical replay / displayd reconnect / IPC isolation / integration
- `card0-HDMI-A-2` connected、mode一覧、EDID SHA-256を確認
- portal ScreenCast / Screenshot D-Bus interfaceを確認

### BLOCKED / NOT_RUN

- DRM pageflip: BLOCKED。独立ユーザーVTがなく、既存KWinがcard0を使用中。KWin停止・DRM master奪取・chvtは未実施。
- virtual KMS: BLOCKED。`sudo -n modprobe vkms` は `Module vkms not found`。
- authenticated uinput: BLOCKED。`/dev/uinput` は `root:root 0600` で、隔離seat/VTがない。
- TUFF endpoint Chrome GUI / portal実キャプチャ: BLOCKED。実DRM/vkms上の独立TUFF endpointが成立していないため未起動。
- 物理hotplug: NOT_RUN。物理アクチュエータがなく、ケーブル操作は実施していない。

## 変更と復旧

- この実機試験による製品コード変更はなし。
- KWin、Plasma、既存session、入力、SSHは停止・再起動していない。
- 元の作業ツリーの未コミット変更は保全した。
- 詳細な個人環境ログ・EDID本体・一時runtimeはリポジトリへ持ち込んでいない。

## 実行証跡

完全な実行ログはローカル試験run rootに保存した。リポジトリに残す本成果物は本ファイルであり、将来のエージェントが `docs/status/` から発見できる。
