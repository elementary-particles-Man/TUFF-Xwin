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
  - secure blank
- `LockCommand`
  - lock state transition
  - auth prompt
- `SessionCommand`
  - suspend request
  - resume hint
  - degraded mode hint
- `WatchdogCommand`
  - restart
  - escalate
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

境界を越えてよいのは「意図」と「最小 state snapshot」だけです。

## Versioning

初期段階では envelope 自体に version を持たせません。破壊的変更が出るまでは、repository の commit history を仕様履歴として扱います。

ただし、最初の wire break が出た時点で次を追加する想定です。

- `schema_version`
- capability negotiation
- unknown field policy

## Rust Mapping

初期の message type は [lib.rs](/media/flux/THPDOC/Develop/TUFF-Xwin/crates/waybroker-common/src/lib.rs) と [ipc.rs](/media/flux/THPDOC/Develop/TUFF-Xwin/crates/waybroker-common/src/ipc.rs) に置きます。
