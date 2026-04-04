# API Boundary

## Purpose

この文書は `Waybroker` の service 間境界を、責務だけでなく API の粒度として固定するためのメモです。ここで重要なのは「何ができるか」より、「どこまでしかできないか」です。

## Boundary Rules

- `displayd` は hardware ownership を持つが policy を持たない
- `waylandd` は client lifecycle を持つが scene policy を持たない
- `compd` は scene policy を持つが hardware ownership を持たない
- `lockd` は auth state を持つが通常 desktop policy を持たない
- `sessiond` は power/session policy を持つが surface life cycle を持たない
- `watchdog` は recovery orchestration を持つが domain logic を持たない

## Service Interfaces

### displayd

Inputs:

- output enumerate request
- output mode set request
- scene commit payload
- secure blank request
- input grab request

Outputs:

- output inventory
- frame commit result
- committed scene snapshot
- input event stream
- hotplug event
- VT / seat ownership event

Must not expose:

- window placement
- focus rules
- lockscreen UI logic

### waylandd

Inputs:

- client connect / disconnect
- protocol object create / destroy
- clipboard owner change
- scene ownership update from `compd`

Outputs:

- surface registry snapshot
- client lifecycle event
- clipboard / selection event
- protocol error

Must not expose:

- DRM/KMS details
- scheduling or power policy

### compd

Inputs:

- surface registry snapshot from `waylandd`
- committed scene snapshot from `displayd`
- input routing event from `displayd`
- lock state from `lockd`
- session hints from `sessiond`

Outputs:

- scene graph update
- focus target update
- animation / effect state
- damage region / output mapping

Must not expose:

- raw device control
- auth credential handling

### lockd

Inputs:

- lock request
- unlock credential exchange
- session state hint

Outputs:

- lock state transition
- auth prompt
- unlock success or failure

Must not expose:

- normal window policy
- direct hardware control

### sessiond

Inputs:

- lid events
- idle timers
- suspend / resume completion
- policy config reload

Outputs:

- lock request
- suspend request
- degraded-mode hint
- interactive auth request for privileged actions

Must not expose:

- scene graph mutation
- protocol object ownership

### watchdog

Inputs:

- process liveness
- health check result
- crash loop counters

Outputs:

- restart request
- degraded mode escalation
- incident log marker

Must not expose:

- desktop policy
- graphics resource ownership

## Initial IPC Shape

現時点の試作では、複雑な transport を先に決める必要はありません。初期段階は次で十分です。

- request/response: local Unix socket
- event stream: newline-delimited JSON or messagepack
- restart-safe state snapshot: file-backed or socket query

重要なのは serialization 形式ではなく、境界を越える state を最小化することです。

現時点の `displayd` は、最後に成功した scene を file-backed snapshot と socket query の両方で保持します。`compd` は restart 後にこの snapshot を読み戻して内部 scene graph を再構築し、hardware broker が保持する最終表示状態と policy service の寿命を切り離します。

同時に `waylandd` は、現在 still-alive な mapped surface の registry snapshot を保持します。`compd` rebuild では `displayd` snapshot を配置の真実、`waylandd` registry を lifecycle の真実として扱い、両者を交差させて scene を再構築します。

clipboard / primary selection owner も `waylandd` registry の一部として扱います。ただし owner が既に死んでいる場合だけは、そのまま dangling owner を残さず、`compd` が復元後 focus を使って handoff を提案し、`waylandd` が apply します。owner が `None` の場合や、まだ生きている owner を `compd` が勝手に上書きすることはしません。

将来 `Vulkan` や GPU submit path を足す場合も、この考え方は変えません。`/media/flux/THPDOC/Develop/TUFF-OS/docs/architecture/GPU_OFFLOAD_CONTRACT.md` と同様に、capability 判定、timeout、fallback、driver 差分吸収は hardware broker 側へ閉じ込め、`compd` や `waylandd` へ raw handle や driver 依存 state を漏らさない前提で進めます。

## Invariants

- client connection lifetime は `waylandd` が真実を持つ
- output ownership は `displayd` が真実を持つ
- focus と stacking は `compd` が真実を持つ
- lock state は `lockd` が真実を持つ
- lid / idle / suspend policy は `sessiond` が真実を持つ
- restart policy は `watchdog` が真実を持つ

同じ state に複数の authoritative owner を作らないことが最優先です。
