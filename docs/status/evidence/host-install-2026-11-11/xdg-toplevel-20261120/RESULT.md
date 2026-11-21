# xdg_toplevel v6 Chrome実機追試

実行日: 2026-11-22 JST
基準main: `867b2726ff81b6a5ff7d399f0d9543b19b19a59d`

- `xdg_toplevel` opcode 0–13、toplevel lifecycle、親循環、seat/serial、resize edge、min/max、fullscreen outputを実装。
- v5以上の`wm_capabilities`を初回configure前に送信。
- release install、TUFF 6 unit、Wayland readiness: PASS。
- Chrome専用endpoint接続: PASS、Chrome exit code: 0。
- `wl_surface event 4`クラッシュと`xdg_toplevel`未実装要求: 再発なし。
- Waylandd観測: client_connected後、`presentation rejected: no presentation obligation`およびstale scene rejection。surface commit/presentation完了は確認不能。
- Chrome GUI描画: NOT_VERIFIED。DRM/pageflip、portal、物理入力、hotplug: NOT_RUN。
- KWin/Plasma: 継続稼働。primary worktree: 開始・終了状態一致。

## 判定

xdg_toplevel protocol接続継続はPASS。実surface描画とpresentationは未達。
