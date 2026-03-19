# Crash Loop Policy

## Goal

`Waybroker` は「死なないこと」を前提にしません。その代わり、どこで再起動をやめて degraded mode に落ちるかを明文化します。

## Scope

この文書の対象は `watchdog` が監視する service です。

- `compd`
- `lockd`
- `sessiond`
- `waylandd`
- 必要に応じて `displayd`

## Policy

### compd

- 30 秒以内に 1 回落ちた: 即再起動
- 30 秒以内に 3 回落ちた: effect 無効化を要求
- 30 秒以内に 5 回落ちた: Xwayland 連携を一時停止して最小 scene に落とす
- 30 秒以内に 7 回落ちた: user に degraded desktop を通知し、通常 session を維持したまま復旧待ちへ入る

### lockd

- 30 秒以内に 1 回落ちた: 即再起動
- 30 秒以内に 3 回落ちた: `blank-only` へ降格
- 30 秒以内に 5 回落ちた: auth UI を止め、sessiond に policy failure として返す

### sessiond

- 30 秒以内に 1 回落ちた: 即再起動
- 30 秒以内に 3 回落ちた: suspend / lid policy を conservative no-op に落とす
- 30 秒以内に 5 回落ちた: power policy failure を通知し、表示系は維持する

### waylandd

- 30 秒以内に 1 回落ちた: 即再起動
- 30 秒以内に 2 回落ちた: incident として記録し、client 接続維持不能ケースとして最優先で扱う

### displayd

- 30 秒以内に 1 回落ちた: watchdog が原因分類を要求
- `kernel` / `driver` 由来が疑われる時は loop restart しない
- user には VT / SSH fallback を優先表示する

## General Rules

- 再起動は service 単位で行う
- いきなり full session restart へ飛ばない
- crash loop counter は process restart ごとではなく role ごとに持つ
- degraded mode は escalation しても、復旧したら戻せる設計にする

## Logging

watchdog は最低限次を残す必要があります。

- `role`
- `pid`
- `timestamp`
- `reason`
- `crash_loop_count`
- `action`

例:

```text
watchdog role=compd crash_loop_count=3 action=disable-effects reason=segfault
```

## Rust Mapping

この policy を表す最初の command は [ipc.rs](/media/flux/THPDOC/Develop/TUFF-Xwin/crates/waybroker-common/src/ipc.rs) の `WatchdogCommand` と `HealthState` で持ちます。
