# TUFF-Xwin user-space 導入・KDE並走実運用結果

実行日: 2026-11-11 JST
対象main: `954ffd1f5c421c24d4648c8e7fe9369d6386310e`

追試日: 2026-11-14 JST（同一の永続source worktree）

修正追試日: 2026-11-14 JST（IPC readiness待機修正）

## 結果

- user-space release install: **PASS**。`~/.local/share/tuff-xwin` と `~/.local/bin` に実体を配置。
- TUFF current-session 並走起動: **PASS**。専用runtime `/run/user/1000/tuff-xwin-ops-20261111`、6 unit active、専用 `wayland-tuff` socket、readiness exit 0。
- KDE並走: **PASS**。KWin/Plasma/Xwaylandは停止・killせず、開始前後のPIDを確認。
- Chrome専用endpoint接続: **FAIL**。Chromeは専用socketへ接続し、waylanddが `client_connected` を記録したが、`wl_seat` opcode 2 の未処理 protocol errorでexit 1。Chrome GUI PASSとは扱わない。
- 指定software smoke: **PASS**。live-topology、canonical replay、displayd reconnect、IPC isolation、integration は全てexit 0。
- KDE障害情報: 事実を採取。KWin main-thread hang、plasmashell QML binding loop / Qt警告、過去のplasmashell SIGABRTを確認。因果関係は断定していない。
- cleanup: **PASS**。今回作成したTUFF current-sessionのみ停止・cleanup。KWin/Plasmaは継続稼働。

## 2026-11-14 追試

- 独立runtimeの6 unit、専用`wayland-tuff` socket、readinessは再度PASS。正しい`tuff-xwin-current-session.target`で確認した。
- Chromeは専用endpointへ接続し`client_connected`を記録したが、`wl_seat opcode 2`未処理protocol errorでexit 1。ネイティブGUI PASSとは扱わない。
- 指定5 smokeは全てexit 0。追試後はTUFF current-sessionのみ停止・runtime削除し、KWin/PlasmaのPID継続を確認。
- KDEの7日分抽出とkernel/coredump照合を追試証跡へ追加。原因の断定はしていない。

## 2026-11-14 IPC待機修正

- `scripts/run-waylandd-ipc-isolation-smoke.sh` の通常起動と`--once`起動に、PID生存確認、`ECONNREFUSED`/`ENOENT`再接続、最大5秒の共通待機を適用。
- 失敗時はcleanup前に`waylandd.log`/`waylandd-once.log`末尾をstderrへ出す。
- IPC isolation smokeは10連続PASS。全指定Rust検証、dev-check、追加smokeもexit 0。
- 実機再導入後、初回はdisplayd socket生成前の起動順競合でwaylanddが終了したが、TUFF専用targetのみ再起動して再成立。Chromeは再び接続後`wl_seat opcode 2`でexit 1。
- 修正追試の詳細は [fix-20261114/](evidence/host-install-2026-11-11/fix-20261114/) に収録。

## 実コード変更

許可範囲内で `scripts/tuff-xwin-current-session.sh` のみ修正した。

- `TUFF_XWIN_REPO_ROOT` がsession.envにあれば永続sourceを使用。
- `profiles/host-wayland.json` がない場合はfail-closed。
- systemd unitのWayland socket pathとsession IDを生成時に正しく固定。

この修正により、インストール済みlauncherが過去の一時sourceを参照せず、専用runtimeのWayland endpointを起動できることを実測した。

## 未達

- 実DRM pageflip: 実施せず。KWinのDRM masterを奪取しない。
- 物理入力/uinput: 実施せず。`/dev/uinput`権限変更・root常駐を行わない。
- portal実キャプチャ: 同意UIを自動承認せず、実キャプチャPASSにはしていない。
- KDEの根因確定: ログ証跡のみで、設定変更・再起動・ドライバ変更は行っていない。

## 2026-11-14 wl_seat修正追試

- `9a4f1550901a9cc4aee9dc3168180ec8f95b7adc`で`wl_seat.get_touch`、各release、touch状態管理、capabilities=3を実装。
- 修正版release binaryを`install-user.sh`で再導入し、専用current-sessionと`/run/user/1000/waybroker/wayland-tuff`を起動。
- 実Chromeは専用endpointへ接続し、`wl_seat opcode=2`は再発しなかった。次に`wl_region opcode=1`が未対応として観測され、今回の対象外のためChrome GUI PASSとは扱わない。
- IPC isolation smokeは10連続PASS。詳細は [fix-20261114/](evidence/host-install-2026-11-11/fix-20261114/) を参照。

## 一次証跡

実行コマンド、終了コード、stdout/stderr、サービス状態、Chrome接続、KDE抽出は [EVIDENCE_INDEX.md](evidence/host-install-2026-11-11/EVIDENCE_INDEX.md) から参照できる。収録ファイルのSHA-256とサイズは [MANIFEST.json](evidence/host-install-2026-11-11/MANIFEST.json) に記録した。

- 収録対象: 76ファイル
- redaction: 0件
- 未収録: 91MBの未加工user journal全文のみ。関連KDE抽出は `evidence/kde/kde-user-filter.stdout` として収録。

追試の一次証跡は [retest-20261114/](evidence/host-install-2026-11-11/retest-20261114/) に収録した。
