# TUFF-Xwin Phase-4 実機E2E結果

- 総合: **BLOCKED**（物理E2E未完了）
- 対象SHA: `3ca6eb418e9dd1e4c7f0addbedbb5973cc9ea410`

## 成功

- software validation: fmt/check/test/Clippy/dev-check/dev-harness PASS
- canonical replay / displayd reconnect / waylandd IPC isolation / multi-session / integration PASS
- portal D-Bus introspection PASS
- Chrome headless/Wayland smoke PASS
- 接続出力: `card0-HDMI-A-2`

## 未達・停止理由

- live topology smokeは`production_client_error`を再現。ログ: `logs/smoke.log`、失敗runtimeの詳細は実行時ログに出力。
- DRM modeset/pageflipは、KWinが現行DRM masterであり、scene commitを伴う独立セッションを安全に起動できないため未確認。VT切替・KWin停止は未実施。
- 物理HIDは`/dev/uinput`がroot専用で、displayd→waylanddのsystemd cgroup認証を満たす専用サービスE2Eを起動していないため未確認。
- Chrome portal GUIは、portal自体は稼働しているがTUFF-Xwin専用Wayland endpointへ接続した実GUI操作を成立させていないため未確認。
- hotplugは物理ケーブル操作が必要なため未実施。

source cloneはclean、追加commit/push/PR/mergeなし。
