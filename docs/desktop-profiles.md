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

## Failure Boundary

- `xfce4-panel` が死んでも kernel は死なない
- `xfwm4` が死んでも `displayd` は死なない
- `x11bridge` が死んでも broker 本体は残る
- `displayd` が死んでも user は VT / SSH から回復する

ここで重要なのは、「GUI 選択の自由」と「障害半径の小ささ」を同時に成立させることです。
