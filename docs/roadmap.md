# Roadmap

## 全体像

`Waybroker` は置き換え型プロジェクトではなく、段階的に故障分離を強めるプロジェクトとして進めるべきです。最初から「完成した新 display server」を狙うと失敗します。

## Phase 0: 要件固定

期間:

- 1 か月

成果物:

- 失敗例の収集
- 対象範囲の固定
- 非目標の明文化

制約:

- `Intel` / `AMD`
- 単一 seat
- local desktop
- `KDE Plasma` 優先

## Phase 1: 監視とログ分離

期間:

- 2 か月

成果物:

- process ごとのログ分離
- crash taxonomy
- simple watchdog
- 復旧不能ケースの洗い出し

目的:

まず「何が最初に死んだか分からない」状態を終わらせる。

## Phase 2: lock と power の分離

期間:

- 3 から 4 か月

成果物:

- `lockd` 試作
- `sessiond` 試作
- `KScreenLocker` / `PowerDevil` の責務整理

目的:

UX 補助機能の障害が display 全体停止へ化けないようにする。

## Phase 3: displayd 試作

期間:

- 6 から 9 か月

成果物:

- `DRM/KMS` broker
- `libinput` broker
- 最終 frame 保持
- basic VT recovery

目的:

特権を display hardware の直近に閉じ込める。

## Phase 4: compd 分離

期間:

- 6 から 12 か月

成果物:

- `KWin-core` の無特権化
- scene 再構築
- compd restart 実験

目的:

一番大きい単一障害点を切り離す。

## Phase 5: waylandd 分離

期間:

- 9 から 15 か月

成果物:

- Wayland endpoint の外出し
- client connection 維持
- clipboard / DnD / selection の再設計

目的:

client 接続寿命と compositor 寿命を分ける。

## 工数感

### 強い試作

- 3 から 5 人
- 9 から 15 か月
- 3 から 6 人年

### 自分たちで常用

- 5 から 8 人
- 18 から 30 か月
- 8 から 18 人年

### 他人に配れる品質

- 8 から 15 人
- 3 から 5 年
- 25 から 50 人年

## 成功条件

- `compd` crash 後に session が維持される
- `Xwayland` crash が X11 app に閉じる
- `lockd` crash が lock 機能だけに閉じる
- `displayd` crash 後に VT か SSH で復旧できる
- `resume` 失敗時に原因層が判別できる

## 失敗しやすい点

- scope が膨らみ、実質的な全部置換になる
- `displayd` に policy を積みすぎる
- `compd` の再起動時 state handoff を甘く見る
- `clipboard`、`DnD`、`IME` を後回しにして詰む

## 最初の一歩

本気で始めるなら、最初にやるべきことは次です。

1. 既存 `KWin` の責務を表にする
2. `lockscreen` と `power policy` の依存関係を洗い出す
3. `displayd` が持つべき最小 API を定義する
4. crash/restart のシーケンス図を作る

ここまでができれば、構想は「愚痴」ではなく「設計」に変わります。
