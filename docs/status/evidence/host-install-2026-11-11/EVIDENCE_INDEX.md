# TUFF-Xwin host install 一次証跡

対象main: `954ffd1f5c421c24d4648c8e7fe9369d6386310e`
収録対象: user-space install、current-session起動、Chrome接続、software smoke、KDEログ抽出、cleanup

全収録ファイルは [MANIFEST.json](MANIFEST.json) に元相対パス、SHA-256、bytes、分類を記録している。秘密値の置換はなく [REDACTIONS.json](REDACTIONS.json) は0件。

## 入口

- [01-baseline.txt](01-baseline.txt): primary tree、既存導入、toolchain、baseline確認
- [02-install-and-prestart.txt](02-install-and-prestart.txt): install後の実体、session.env、KDE PID
- [install.command.txt](install.command.txt), [install.stdout](install.stdout), [install.stderr](install.stderr): release installの実行記録
- [03-runtime-status-2.txt](03-runtime-status-2.txt): 6 unit active、socket、readiness exit 0、KDE PID
- [04-chrome-summary.txt](04-chrome-summary.txt): Chrome専用endpoint接続とexit 1の実証
- [07-cleanup.txt](07-cleanup.txt): cleanup前後、KWin/Plasma継続確認

## 2026-11-14 追試

- [retest-20261114/](retest-20261114/): 独立runtime再起動、正しいunit状態、readiness、Chrome接続、5 smoke、KDE抽出、cleanupの一次ログ。
- `retest-20261114/smokes/OVERALL.status` は `overall=0`。Chromeは`retest-20261114/04-waylandd-journal.txt`に`wl_seat opcode 2`エラーを記録。

## software

- [software/](software/): fmt、check、test、Clippy、dev-checkの各command/stdout/stderr/status
- [smokes/](smokes/): live-topology、canonical replay、reconnect、IPC isolation、integrationの各command/stdout/stderr/status

全5 smokeの終了コードは0。

## KDE障害情報

- [kde/kde-user-filter.stdout](kde/kde-user-filter.stdout): 7日分のKWin/Plasma/org.kde/crash/segfault/watchdog/oom抽出
- [kde/kde-kernel-filter.stdout](kde/kde-kernel-filter.stdout): kernel DRM/amdgpu/GPU reset/OOM/I/O error抽出（exit 1、該当なし）
- [kde/kwin-coredumps.stdout](kde/kwin-coredumps.stdout): KWin/Plasma/Xwayland coredump抽出
- [kde/FACTS.txt](kde/FACTS.txt): 抽出行の事実表。原因断定はしていない

観測事実には `kwin_wayland_drm: The main thread was hanging temporarily!`、plasmashellのQML binding loop、Qt Wayland警告、過去のplasmashell SIGABRTが含まれる。

## 未収録

未加工user journal全文（約91MB）はリポジトリ肥大化を避けて収録せず、filtered stdoutを一次参照として収録した。バイナリbackup、core本体、Chrome profile、EDID本体は収録していない。
