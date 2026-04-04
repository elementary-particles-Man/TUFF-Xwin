# IPC Format

## Goal

`Waybroker` の初期 IPC は、賢い transport よりも restart-safe な境界を優先します。この文書では「何をメッセージに載せるか」と「何を載せないか」を決めます。

## Format Choice

試作段階では次を前提にします。

- transport: local Unix socket
- encoding: JSON
- framing: 1 message per line

理由:

- 目視確認しやすい
- ログへ落としやすい
- crash 後の再現に使いやすい

高速化や binary encoding は、境界が固まってから考えれば足ります。

## Envelope

すべてのメッセージは共通 envelope を持ちます。

```json
{
  "source": "compd",
  "destination": "displayd",
  "kind": {
    "kind": "display-command",
    "payload": {
      "op": "commit-scene",
      "target": { "type": "output", "name": "eDP-1" },
      "focus": { "type": "surface", "id": "terminal-1" },
      "surfaces": []
    }
  }
}
```

## Why This Shape

- `source` と `destination` を message 自体に入れる
  - routing と監査を簡単にするため
- operation 名は enum で固定する
  - stringly typed protocol 化を防ぐため
- payload は service ごとに閉じる
  - `displayd` が `lockd` の内部 state を理解しなくて済むようにするため

## Message Families

- `DisplayCommand`
  - output enumerate
  - mode set
  - scene commit
  - scene snapshot query
  - secure blank
- `DisplayEvent`
  - output inventory
  - mode applied
  - scene committed
  - scene snapshot
  - secure blank applied
  - rejected
- `WaylandCommand`
  - surface registry query
  - selection handoff apply
- `WaylandEvent`
  - surface registry snapshot
  - selection handoff applied
  - rejected
- `LockCommand`
  - lock state transition
  - auth prompt
- `SessionCommand`
  - suspend request
  - resume hint
  - degraded mode hint
  - watchdog report apply
  - profile transition / unchanged response
- `WatchdogCommand`
  - restart
  - escalate
  - launch-state inspect
  - launch-state delta update
  - launch-state resync request
  - inspection result
- `HealthState`
  - healthy
  - unhealthy

## What Stays Out Of Band

次は message に載せません。

- large buffer contents
- GPU object handles
- raw input device fd
- PAM conversation secrets
- kernel / driver internal state

境界を越えてよいのは「意図」と「最小 state snapshot / delta」だけです。

`displayd` は `CommitScene` を受けた後、最後に成功した scene を `WAYBROKER_RUNTIME_DIR/displayd-last-scene.json` へ書き出します。`compd` は restart 後に `get-scene-snapshot` を使ってこの snapshot を再取得し、scene policy の再構築を始めます。これにより、hardware broker が保持する最後の表示状態と、policy service の再起動寿命を分離します。

`waylandd` は別に `surface registry snapshot` を持ち、`get-surface-registry` で応答します。snapshot には mapped surface だけでなく clipboard / primary selection owner と、その owner が最後に publish した `payload_id` / `source_serial` も含めます。`compd` の restart 後 rebuild では、`displayd` の last-scene snapshot をそのまま信じ込まず、`waylandd` registry にまだ存在する mapped surface だけを残します。これにより、既に死んだ client surface を復元 scene に混ぜるのを避けます。

復元後に selection owner が死んでいた場合だけ、`compd` は `apply-selection-handoff` を `waylandd` へ送り、再計算後 focus へ handoff します。ここで `compd` は dead owner の `payload_id` / `source_serial` までは引き継がず、owner だけを新しい focus へ寄せ、payload metadata は clear します。`waylandd` は validate 後に registry を更新し、`selection-handoff-applied` を返します。これにより、復元直後に clipboard / primary selection が dangling owner を指したままになるのを避けつつ、owner id だけで誤って stale payload を再送することも避けます。

`sessiond -> watchdog` の launch-state stream では、各 message に `session_instance_id` と `generation` と `sequence` を持たせます。`session_instance_id` は supervisor instance 単位の識別子、`generation` はその instance 上の active profile runtime 世代、`sequence` はその世代内の更新順序です。`watchdog` は `profile_id + session_instance_id` ごとに cache を分離し、これを使って stale delta を無視し、欠番が出た場合だけ `resync-launch-state` を返して同じ instance からの full state 再送を要求します。

## Versioning

初期段階では envelope 自体に version を持たせません。破壊的変更が出るまでは、repository の commit history を仕様履歴として扱います。

ただし、最初の wire break が出た時点で次を追加する想定です。

- `schema_version`
- capability negotiation
- unknown field policy

## Rust Mapping

初期の message type は [lib.rs](/media/flux/THPDOC/Develop/TUFF-Xwin/crates/waybroker-common/src/lib.rs) と [ipc.rs](/media/flux/THPDOC/Develop/TUFF-Xwin/crates/waybroker-common/src/ipc.rs) に置きます。

初期の transport helper は [transport.rs](/media/flux/THPDOC/Develop/TUFF-Xwin/crates/waybroker-common/src/transport.rs) に置き、`displayd <-> waylandd` と `sessiond <-> watchdog` の間で 1 行 1 message の Unix socket 通信を行います。`sessiond` は `--manage-active` 時に supervisor instance ごとの `session_instance_id` を払い出して active profile runtime を持ち続け、最初の health stream では full launch-state を送り、その後は変更された component だけを delta として watchdog へ stream します。profile 切替時は同じ `session_instance_id` を維持したまま `generation` を進め、`sequence` は 1 から振り直します。watchdog は `profile_id + session_instance_id` ごとの cached launch-state に merge したうえで inspection を返し、stale delta は無視し、cache を失った場合や `sequence` 欠番を検出した場合だけ `resync-launch-state` を返して同じ session instance から full state の再送を要求します。必要なら degraded fallback の launch-state 更新と component 起動まで連続で行います。
