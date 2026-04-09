# Architecture

## 問題設定

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

## Waybroker の基本構造

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

## 重要な原則

### 1. 最小特権

`DRM master`、`input device access`、`seat ownership` を必要とする処理は `displayd` に閉じ込めます。`compd` は原則として無特権です。

### 2. 状態の分散管理

- client 接続と surface の寿命は `waylandd`
- 表示方針と scene graph は `compd`
- 出力への commit は `displayd`
- ロック UI と認証状態は `lockd`
- 電源とセッション方針は `sessiond`

こうすると、一つのプロセスが死んでも全状態を一緒に失わずに済みます。

### 3. 再起動前提

`compd` や `lockd` は「死なないこと」を前提にしません。死んだら再起動し、最小限の状態を `waylandd` と `displayd` から取り戻す前提にします。

### 4. kernel を増やさない

問題の主戦場は `kernel` ではありません。主戦場は `userspace display stack` の責務分割です。`Wayland 専用 kernel` を増やしても、GPU と input を二重管理する複雑性が増えるだけで、本質的な単一障害点は消えません。

## データフロー

### 描画

1. client は `waylandd` に接続する
2. `waylandd` は surface と buffer の寿命を管理する
3. `compd` は scene を組み、どの surface をどの output に載せるか決める
4. `displayd` は atomic commit を行う

### 入力

1. `displayd` が `libinput` からイベントを受ける
2. `compd` が focus と routing を判断する
3. `waylandd` が対象 client に配送する
4. グローバルショートカットや secure input の扱いは `sessiond` と連携する

### ロック

1. `sessiond` がロック要求を発行する
2. `lockd` が専用 UI を持つ
3. `compd` は lock state を見て、通常 surface を隠す
4. 認証成功後に `lockd` が解除通知を送る

## X11 との関係

`Waybroker` は `X11` を復活させる案ではありません。しかし故障分離という意味では `X11` の長所を継承します。

- `X server` 的な接続口は `waylandd`
- `window manager/compositor` 的な制御面は `compd`
- `display hardware` 直結部分は `displayd`

つまり「単一の巨大 compositor」ではなく、「落ちても再起動できる複数の役割」に戻します。

## 対応範囲

初期段階では次に絞るべきです。

- `Intel` / `AMD` のみ
- 単一 `seat`
- local desktop のみ
- `Xwayland` は rootless 前提
- `KDE Plasma` 優先

この制限を入れないと、構想ではなく未完成の巨大置換計画に化けます。
