# Architecture / アーキテクチャ

## Problem Definition / 問題設定

### English
The current architecture of `KDE Plasma on Wayland` typically manifests as a monolithic structure:

```text
Wayland Apps   Xwayland   Plasma Shell   KScreenLocker
      \            |             |              /
       \           |             |             /
                    [ KWin ]
          compositor + WM + display server
          + input/focus + lock coordination
                      |
            logind / libinput / DRM/KMS
                      |
             [ Linux Kernel ]
```

In this model, `KWin` is too heavy. If `KWin` gets blocked or crashes, it takes down display outputs, inputs, window management, lock screen handling, and potentially the entire user session.

### 日本語
現行の `KDE Plasma on Wayland` は、概念的には次のような太い構造になりがちです。

```text
Wayland Apps   Xwayland   Plasma Shell   KScreenLocker
      \            |             |              /
       \           |             |             /
                    [ KWin ]
          compositor + WM + display server
          + input/focus + lock coordination
                      |
            logind / libinput / DRM/KMS
                      |
             [ Linux Kernel ]
```

この構造では `KWin` が大きすぎます。`KWin` が詰まると、表示、入力、ウィンドウ管理、ロック画面、場合によってはセッション管理まで巻き込みます。

---

## Basic Structure of Waybroker / Waybroker の基本構造

### English
`Waybroker` segments these components into decoupled roles:

```text
  Wayland Apps      Plasma Shell      Xwayland
       \                 |               /
        \                |              /
                 [ waylandd ]
       client socket / object lifetime / clipboard
                     |
               scene & policy IPC
                     |
                  [ compd ]
         layout / focus / effects / decoration
                     |
              output submit / input req
                     |
                 [ displayd ]
      DRM/KMS / libinput / seat / VT / lease
             |                     |
             |                     +---- [ lockd ]
             |
             +-------------------------- [ sessiond ]
                     |
              [ Linux Kernel ]
```

### 日本語
`Waybroker` は、これを次のように分けます。

```text
  Wayland Apps      Plasma Shell      Xwayland
       \                 |               /
        \                |              /
                 [ waylandd ]
       client socket / object lifetime / clipboard
                     |
               scene & policy IPC
                     |
                  [ compd ]
         layout / focus / effects / decoration
                     |
              output submit / input req
                     |
                 [ displayd ]
      DRM/KMS / libinput / seat / VT / lease
             |                     |
             |                     +---- [ lockd ]
             |
             +-------------------------- [ sessiond ]
                     |
              [ Linux Kernel ]
```

---

## Core Principles / 重要な原則

### 1. Least Privilege / 最小特権
* **EN**: Operations requiring `DRM master`, input device access, or seat ownership are confined strictly to `displayd`. The compositor (`compd`) is entirely unprivileged.
* **JA**: `DRM master`、`input device access`、`seat ownership` を必要とする処理は `displayd` に閉じ込めます。`compd` は原則として無特権です。

### 2. Distributed State Management / 状態の分散管理
* **EN**:
  - Client connections and surface lifetimes are handled by `waylandd`.
  - Stacking layout, decoration, and scene graph generation are handled by `compd`.
  - Final commits to the hardware output are managed by `displayd`.
  - Lockscreen UI and authentication are restricted to `lockd`.
  - Power management and high-level session rules are kept inside `sessiond`.
* **JA**:
  - client 接続と surface の寿命は `waylandd`
  - 表示方針と scene graph は `compd`
  - 出力への commit は `displayd`
  - ロック UI と認証状態は `lockd`
  - 電源とセッション方針は `sessiond`

### 3. Crash-Resiliency (Designed to Restart) / 再起動前提
* **EN**: Processes like `compd` or `lockd` are not assumed to never crash. If they crash, the watchdog restarts them, pulling back the minimal required state from `waylandd` and `displayd` without severing client connections.
* **JA**: `compd` や `lockd` は「死なないこと」を前提にしません。死んだら再起動し、最小限の状態を `waylandd` と `displayd` から取り戻す前提にします。

### 4. No Custom Kernel Code / kernel を増やさない
* **EN**: The battleground is in the user-space display stack responsibility split, not the kernel itself. Writing custom Wayland-centric kernels only increases the complexity of hardware management without eliminating the true single points of failure.
* **JA**: 問題の主戦場は `kernel` ではありません。主戦場は `userspace display stack` の責務分割です。`Wayland 専用 kernel` を増やしても、GPU と input を二重管理する複雑性が増えるだけで、本質的な単一障害点は消えません。

---

## Data Flows / データフロー

### Rendering / 描画
1. **EN**: Client connects to `waylandd`. / **JA**: client は `waylandd` に接続する。
2. **EN**: `waylandd` tracks surface and buffer lifetimes. / **JA**: `waylandd` は surface と buffer の寿命を管理する。
3. **EN**: `compd` builds the layout scene graph, assigning surfaces to physical outputs. / **JA**: `compd` は scene を組み、どの surface をどの output に載せるか決める。
4. **EN**: `displayd` submits the atomic KMS configuration. / **JA**: `displayd` は atomic commit を行う。

### Input Routing / 入力
1. **EN**: `displayd` receives physical input events via `libinput`. / **JA**: `displayd` が `libinput` からイベントを受ける。
2. **EN**: `compd` evaluates window focus and routing rules. / **JA**: `compd` が focus と routing を判断する。
3. **EN**: `waylandd` delivers events to the focused client. / **JA**: `waylandd` が対象 client に配送する。
4. **EN**: Global shortcuts or secure inputs are handled in coordination with `sessiond`. / **JA**: グローバルショートカットや secure input の扱いは `sessiond` と連携する。

### Lockscreen / ロック
1. **EN**: `sessiond` triggers a lock request. / **JA**: `sessiond` がロック要求を発行する。
2. **EN**: `lockd` starts its authentication interface. / **JA**: `lockd` が専用 UI を持つ。
3. **EN**: `compd` observes the lock state and hides standard surfaces. / **JA**: `compd` は lock state を見て、通常 surface を隠す。
4. **EN**: After authentication success, `lockd` signals to unlock. / **JA**: 認証成功後に `lockd` が解除通知を送る。

---

## Relationship with X11 / X11 との関係

* **EN**: `Waybroker` is not about resurrecting X11. However, it inherits the separation advantages of the X11 era:
  - `waylandd` acts like the X socket endpoint.
  - `compd` operates like a window manager/compositor.
  - `displayd` runs like the hardware-specific driver interface.
  We slice the monolithic compositor back into these modular, restartable user-space processes.
* **JA**: `Waybroker` は `X11` を復活させる案ではありません。しかし故障分離という意味では `X11` の長所を継承します。
  - `X server` 的な接続口は `waylandd`
  - `window manager/compositor` 的な制御面は `compd`
  - `display hardware` 直結部分は `displayd`
  つまり「単一の巨大 compositor」ではなく、「落ちても再起動できる複数の役割」に戻します。

---

## Scope / 対応範囲

* **EN**: Initially, we constraint the scope to avoid infinite expansion:
  - Intel / AMD GPUs only (No proprietary Nvidia complexity initially).
  - Single seat configurations.
  - Local desktop display only.
  - Rootless `Xwayland` integration.
  - Prioritize `KDE Plasma` target desktop stack.
* **JA**: 初期段階では次に絞るべきです。
  - `Intel` / `AMD` のみ
  - 単一 `seat`
  - local desktop のみ
  - `Xwayland` は rootless 前提
  - `KDE Plasma` 優先
  この制限を入れないと、構想ではなく未完成の巨大置換計画に化けます。
