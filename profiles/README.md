# Desktop Profiles

ここには、`Waybroker` 上で選択可能な GUI profile manifest を置きます。考え方は「GUI は broker 本体の一部ではなく、ユーザーが選んで載せる session profile」というものです。

## 初期方針

- まずは `X11` 先行で profile を定義する
- `sessiond` が profile を列挙し、選択状態を管理する
- `displayd` / `waylandd` / `lockd` / `watchdog` は profile 非依存で残す
- `xfce`, `openbox`, `mate` などは将来ここへ追加する

## 現在の profile

- `demo-x11.json`
- `xfce-x11.json`
- `openbox-x11.json`

- `demo-x11`
  - repo 内の mock component を使う launch / watchdog 検証用 profile
- `xfce-x11`, `openbox-x11`
  - 実際の package 導入を前提にする rootless `X11` profile
