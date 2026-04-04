# Scripts

開発補助スクリプトを置く場所です。

- `dev-check.sh`: format, check, test をまとめて走らせる
- `run-stack.sh`: `displayd` と `waylandd` の Unix socket stub 通信を含めて、各 service の起動確認を行う
- `run-profile-demo.sh`: `sessiond` と `watchdog` で GUI profile の選択、launch state、監視結果を確認する
- `run-crash-loop-demo.sh`: critical GUI component の再起動と crash-loop 判定を確認する
- `run-degraded-mode.sh`: 常駐 `sessiond` supervisor が active profile を監視し、`sessiond -> watchdog` の full state + delta health stream、必要時の resync、自動 degraded 切替で fallback profile を起動する
- `run-watchdog-resync-demo.sh`: `watchdog` を途中で再起動し、`sessiond` が resync を要求されても degraded fallback まで収束することを確認する
- `run-scene-recovery-demo.sh`: `displayd` の last-scene snapshot と `waylandd` の surface/selection registry を使って、`compd` が broker rebuild と selection handoff までできることを確認する
- `run-compd-broker-recovery.sh`: `watchdog -> sessiond` recovery execution で `compd` を restart し、startup 時に `displayd + waylandd` snapshot から scene rebuild、selection handoff、再commit までできることと、`Wayland native` profile が broker-owned `lockd` と `shell/panel/settings-daemon/applet` skeleton に分かれていることを確認する
- `mock-desktop-component.sh`: profile launcher / watchdog の検証用 mock GUI component
