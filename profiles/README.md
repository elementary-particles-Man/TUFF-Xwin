# Desktop Profiles

ここには、`Waybroker` 上で選択可能な GUI profile manifest を置きます。考え方は「GUI は broker 本体の一部ではなく、ユーザーが選んで載せる session profile」というものです。

## 初期方針

- まずは `X11` 先行で profile を定義する
- `sessiond` が profile を列挙し、選択状態を管理する
- `displayd` / `waylandd` / `lockd` / `watchdog` は profile 非依存で残す
- `xfce`, `openbox`, `mate` などは将来ここへ追加する

## 現在の profile

- `demo-x11.json`
- `demo-x11-crashy.json`
- `demo-x11-degraded.json`
- `demo-wayland-compd-recovery.json`
- `xfce-x11.json`
- `openbox-x11.json`

- `demo-x11`
  - repo 内の mock component を使う launch / watchdog 検証用 profile
- `demo-x11-crashy`
  - critical component の再起動と crash-loop 判定を確認する profile
- `demo-x11-degraded`
  - crash-loop 後に切り替える最小 fallback profile
- `demo-wayland-compd-recovery`
  - `sessiond/watchdog` 経由で `compd` broker を restart し、`displayd + waylandd` snapshot から scene を rebuild する `Wayland native` demo profile
- `xfce-x11`, `openbox-x11`
  - 実際の package 導入を前提にする rootless `X11` profile

各 profile は必要なら `degraded_profile_id` を持てます。これは critical component の crash-loop や未導入を `watchdog` が検知した時に、`sessiond` が次に切り替える fallback profile を指します。
