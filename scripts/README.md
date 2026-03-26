# Scripts

開発補助スクリプトを置く場所です。

- `dev-check.sh`: format, check, test をまとめて走らせる
- `run-stack.sh`: `displayd` と `waylandd` の Unix socket stub 通信を含めて、各 service の起動確認を行う
- `run-profile-demo.sh`: `sessiond` と `watchdog` で GUI profile の選択、launch state、監視結果を確認する
- `run-crash-loop-demo.sh`: critical GUI component の再起動と crash-loop 判定を確認する
- `run-degraded-mode.sh`: 常駐 `sessiond` supervisor が active profile を監視し、`sessiond -> watchdog` の full state + delta health stream、必要時の resync、自動 degraded 切替で fallback profile を起動する
- `mock-desktop-component.sh`: profile launcher / watchdog の検証用 mock GUI component
