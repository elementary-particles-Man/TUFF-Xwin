# wl_region Chrome実機追試結果

実行日: 2026-11-17 JST
基準main: `ecb0a80e79b9c8bd172a3da0994eea25b8bc94d5`

- Rust実装: `wl_region` destroy/add/subtract、i32 LE 16-byte payload厳密検証、正寸法検証、i64幾何、非重複union、subtract分割、4096矩形上限を実装。
- Surface: opaque/input regionのsnapshotをpendingへ保存し、commitでcurrentへ反映。region破棄後もsnapshotを保持。
- Rust検証: fmt/check/test/clippy、dev-check、指定5 smokeはPASS。
- user-space install: PASS。installed `waylandd` SHA-256は `region-20261115/installed-waylandd.sha256`。
- TUFF current-session: 6 unit active、専用socket readiness exit 0。
- Chrome接続: PASS（専用Wayland endpointへclient_connected）。exit code 0。
- wl_region protocol error: 0件。
- 次の未実装要求: `xdg_toplevel` opcode 8、payload size 16、object id 30。対象外のため追加実装していない。
- surface commit/presentation: 該当一次イベントは確認できず、GUI描画/PRESENTATIONはNOT_RUN/未達。
- KWin/Plasma: PID継続。primary worktreeは開始・終了一致。
- cleanup: 今回のTUFF unit/runtimeのみ実施。

## 判定

`wl_region`修正とChrome接続継続はPASS。Chrome GUI描画、presentation、DRM/pageflip、物理入力、portal、hotplugは未達。
