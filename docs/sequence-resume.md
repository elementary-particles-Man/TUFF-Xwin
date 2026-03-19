# Resume Sequence

## Goal

この文書は、suspend/resume 復帰時に `lockscreen` と `compositor` と `hardware recovery` を混線させないための基準シーケンスを定義します。

## Normal Resume

```text
kernel      sessiond      displayd      waylandd      compd      lockd
  |             |             |             |           |          |
  |---resume--->|             |             |           |          |
  |             |---begin---->|             |           |          |
  |             |             |--reprobe--->|           |          |
  |             |             |<--outputs----           |          |
  |             |---hint------------------------------->|          |
  |             |             |-------------snapshot--->|          |
  |             |             |<--------commit request--|          |
  |             |             |----commit ok----------->|          |
  |             |---lock?--------------------------------------->|
  |             |<---------------------------------------state---|
  |             |---resume complete--------------------->|        |
```

ポイント:

- `displayd` が先に output を回復する
- `compd` は output が戻る前に通常描画へ進まない
- lock 要求は resume path に埋め込まず、`sessiond -> lockd` の独立遷移にする

## Resume With displayd Trouble

```text
kernel      sessiond      displayd      watchdog      compd      lockd
  |             |             |             |           |          |
  |---resume--->|             |             |           |          |
  |             |---begin---->|             |           |          |
  |             |             X  output recovery fails  |          |
  |             |--------------------------alert------->|          |
  |             |<----------------------degraded mode---|          |
  |             |---blank or hold------------------------------->|
  |             |--------------------------restart----->|        |
```

ポイント:

- `compd` を先に責めない
- `lockd` を巻き込まず、最悪でも blank / hold に落とす
- root cause が `displayd` なのか `kernel/driver` なのかをログに分ける

## Resume With compd Crash

```text
kernel      sessiond      displayd      waylandd      watchdog      compd
  |             |             |             |             |           |
  |---resume--->|             |             |             |           |
  |             |---begin---->|             |             |           |
  |             |             |--ok-------->|             |           |
  |             |             |             |---state---->|           |
  |             |             |             |             X crash     |
  |             |             |             |<--disconnect-|           |
  |             |             |--------------------------->| restart   |
  |             |             |-------------snapshot----------------->|
  |             |---------------------------------------------resume->|
```

ポイント:

- client 接続は `waylandd` が握り続ける
- `displayd` は最後の安定 frame を維持する
- `watchdog` が `compd` だけを再起動する

## Rules

- resume path の成否と lock state を同一 state machine にしない
- output recovery と auth prompt を同時に始めない
- `kernel` / `driver` 由来障害と `userspace` 障害を同じログに混ぜない
- 「復帰できなかったので全部再起動」は最後の手段にする
