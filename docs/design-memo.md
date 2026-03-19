# Design Memo

## Positioning

この repository は `TUFF-Xwin` という名前で管理し、アーキテクチャ名は `Waybroker` とします。

- `TUFF-Xwin`: repository / codename
- `Waybroker`: runtime architecture

## Core Claim

現代の `KWin` や `Mutter` の問題は、「kernel と近すぎること」ではなく、「display/input/session/policy を抱え込みすぎていること」です。したがって、対策は新 kernel ではなく、`userspace broker` と `restartable services` の導入です。

## First Build Target

最初に狙うべきは完成品ではなく、次の条件を満たす試作です。

1. `compd` crash が session 全体停止に直結しない
2. `Xwayland` crash が X11 app に閉じる
3. `lockd` crash が lock 機能に閉じる
4. `displayd` crash 後でも VT または SSH から回復できる

## Initial API Boundary

- `displayd`
  - output enumerate
  - output commit
  - input event stream
  - seat state
- `waylandd`
  - client lifecycle
  - surface registry
  - clipboard and selection core
- `compd`
  - scene update
  - focus update
  - placement and stacking
- `lockd`
  - lock state
  - auth conversation
- `sessiond`
  - suspend and resume intent
  - lid and idle policy

## Why This Repo Exists Now

この repository の役割は、構想を口頭の苛立ちで終わらせず、`docs`、`crate split`、`workspace layout` の形に固定することです。
