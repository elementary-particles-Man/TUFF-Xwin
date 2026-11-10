# TUFF-Xwin Phase-4 不具合修正再テスト

## 修正・統合

- 原因: 最終callback後のsmoke client即時closeにより、未読wire eventを伴うRSTが発生していた。
- 修正: clientをhalf-close後にEOFまでdrainし、waylanddはEOF確認後のみ正常終了として扱う。
- 失敗runtime保存: `TUFF_XWIN_KEEP_FAILED_TOPOLOGY_LOGS=1`
- 修正commit: `955c0ced10e0b3769a4b5fb6c76c4669fb857268`
- PR: #32
- merge: `ee3a678ae90832763590683d48cd4de81de0c728`

## 検証

- live topology: 10回連続PASS、`production_client_error` 0件
- fmt/check/test/Clippy/dev-check/dev-harness: PASS
- canonical replay / compd outage / displayd reconnect / IPC isolation / integration: PASS
- PR由来 authoritative validation / deny / xwin-screenshot / GitGuardian: PASS

## 未達

- DRM modeset/pageflip: BLOCKED（KWinのDRM masterを奪取しない）
- authenticated evdev/uinput E2E: BLOCKED（隔離済みsystemd service/cgroup経路未成立）
- TUFF専用Wayland endpoint上のChrome GUI + portal実request: BLOCKED
- 物理hotplug: NOT_RUN（物理アクチュエータなし）

したがってPhase-4全体は物理カテゴリ未達のためBLOCKEDです。KWin/Plasma/現行セッションは停止していません。
