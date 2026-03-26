# Desktop Profiles

## Goal

`Waybroker` の目的は、新しい固定 GUI を押し付けることではありません。目的は `display/input/session` を broker 化し、その上に `XFCE`、`KDE Plasma`、`GNOME`、軽量 WM 群をユーザーが自由に選んで載せられるようにすることです。

## Core Split

固定で残るもの:

- `displayd`
- `waylandd`
- `compd`
- `lockd`
- `sessiond`
- `watchdog`

ユーザーが選ぶもの:

- desktop shell
- panel
- applet
- window manager
- compatibility layer

つまり、`Waybroker` は GUI そのものではなく、GUI を載せ替えるための最小 display/session manager です。

## X11 First Strategy

最初は `X11` profile を先に作ります。理由は次です。

- 既存 desktop 環境の選択肢が多い
- window / panel / settings daemon の分割が明確
- rootless island として切り出しやすい

この段階では `LeyerX11` を下位互換層として扱い、`x11bridge` が rootless `X11` scene を broker 側へ渡します。

## sessiond の役割

`sessiond` は power policy だけでなく、desktop profile manager も兼ねます。

- profile manifest を列挙する
- user が選んだ profile を active state として保持する
- どの broker service と GUI component が必要かを launch plan として出す
- command の解決状態を launch state として記録する
- `watchdog` report を見て、必要なら degraded fallback profile へ切り替える

launch state は次の用途に使います。

- GUI package が未導入かどうかを切り分ける
- critical component が欠けている profile を boot 前に検出する
- 将来 `watchdog` が「どの GUI component を監視するか」を知る

初期実装では `sessiond` が `active-profile.json` と `launch-state-<profile>.json` を runtime dir へ書きます。

`watchdog` は `launch-state-<profile>.json` を読み、各 component を `healthy / unhealthy / inactive` で分類できます。加えて、常駐 `sessiond` supervisor が launch-state 更新ごとに watchdog へ stream すれば、pull ではなく event-driven に同じ判定を返せます。これにより、`xfwm4` が落ちたのか、単に未導入なのか、まだ起動していないだけなのかを分けられます。

`sessiond` の supervisor stub は critical component に restart counter を持ちます。`watchdog` はその値を見て、`restart-component` で済む段階か、`degraded-profile` へ落とす段階かを判断します。

profile manifest は `degraded_profile_id` を持てます。`watchdog` は launch state から `watchdog-report-<profile>.json` を作るだけでなく、Unix socket server として `sessiond` から launch-state snapshot を受け取れます。`sessiond --serve-ipc --spawn-components --manage-active --notify-watchdog` で動かしていれば、active profile の component を常駐で poll し、その更新を watchdog へ stream し、返ってきた report に `degraded-profile` action が含まれていれば `active-profile.json` を fallback profile に差し替えます。結果は `profile-transition-<from>-to-<to>.json` と新しい `launch-state-<profile>.json` に記録されます。

## Failure Boundary

- `xfce4-panel` が死んでも kernel は死なない
- `xfwm4` が死んでも `displayd` は死なない
- `x11bridge` が死んでも broker 本体は残る
- `displayd` が死んでも user は VT / SSH から回復する

ここで重要なのは、「GUI 選択の自由」と「障害半径の小ささ」を同時に成立させることです。
