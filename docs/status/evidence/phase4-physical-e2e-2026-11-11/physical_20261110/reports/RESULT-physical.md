# TUFF-Xwin Phase 4 実機残4項目 E2E 結果

- 実行日: 2026-11-11 JST
- 対象: `ee3a678ae90832763590683d48cd4de81de0c728`（PR #32 merge済み）
- 総合: **BLOCKED**

## PASS

- software regression: fmt/check/test/Clippy/dev-check/dev-harness PASS
- live-topology: 10回連続 PASS、`production_client_error` 0件
- canonical replay / displayd reconnect / IPC isolation / integration: PASS
- 非侵襲 device 状態取得: `card0-HDMI-A-2` connected、modes と EDID hash を記録
- portal D-Bus の ScreenCast/Screenshot interface の存在確認

## BLOCKED / NOT_RUN

- physical DRM pageflip: **BLOCKED**。唯一の実ユーザーVTは既存KWinの tty2。独立ユーザーVTがないため chvt、KWin停止、DRM master奪取を実施せず。
- virtual KMS: **BLOCKED**。`sudo -n modprobe vkms` は `Module vkms not found`。
- authenticated uinput: **BLOCKED**。`/dev/uinput` は `root:root 0600`、隔離seat/VTなし。
- Chrome GUI + portal real capture: **BLOCKED**。TUFF専用の実DRM/vkms output と独立endpointが成立していないため、Chrome起動・portal同意操作を行わず。
- physical hotplug: **NOT_RUN**。物理アクチュエータなし。ケーブル抜き差しは実施せず。

## 変更・復旧

- 実コード変更、commit、PR、merge: なし。環境未成立をコード修正で偽装していない。
- 元の primary worktree と既存未コミット変更は変更なし。
- KWin/Plasma、既存session、入力、SSHは停止・再起動していない。
- 新規試験プロセスは終了済み。詳細証跡は `../evidence/`。
