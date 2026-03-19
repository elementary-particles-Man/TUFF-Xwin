# Failure Model

## 目的

この文書は「何が壊れた時に、どこまで巻き込むか」を定義します。平常時の美しさより、障害半径を制御することが主眼です。

## 障害分類

### 1. compd crash

症状:

- ウィンドウ配置や effect が止まる
- 新規 redraw が止まる

期待動作:

- `waylandd` は client 接続を維持する
- `displayd` は最後の安定 frame を保つ
- `watchdog` が `compd` を再起動する
- 復帰後に scene を再構成する

### 2. lockd crash

症状:

- ロック画面だけが壊れる

期待動作:

- 通常 session を巻き込まない
- lock state が有効なら、`displayd` 側で最低限の blank 画面に落とせる
- `lockd` を再起動する

### 3. Xwayland crash

症状:

- X11 アプリだけ消える

期待動作:

- Wayland native app はそのまま動く
- `Xwayland` を再起動しても全 session は死なない

### 4. sessiond crash

症状:

- lid、idle、power key policy が止まる

期待動作:

- 画面表示は維持する
- suspend 不能や policy 欠落は許容しても、desktop の心臓部は落とさない

### 5. displayd crash

症状:

- 画面出力と入力受信を失う

期待動作:

- kernel は生きる
- 他 process も生きる
- VT または SSH で再起動可能
- worst case でも「強制電源断しかない」は避ける

### 6. kernel driver stall

症状:

- `DRM` や `iwlwifi` などの driver が stall する
- scheduler や RCU が詰まる場合もある

期待動作:

- userspace だけでは完全救済できない
- ただし display stack の複合故障に見えないよう、ログと隔離を明確にする

ここは `Waybroker` の限界です。kernel deadlock は kernel 側で直す必要があります。

## degraded mode

正常復旧できない場合は、段階的に機能を落とします。

1. effect 無効
2. animation 無効
3. Xwayland 再起動
4. lockd 再起動
5. compd 再起動
6. displayd 再起動
7. VT 退避を促す

重要なのは「いきなりブラックアウトして何もできない」にしないことです。

## suspend/resume の扱い

今回のような `resume` 起因の問題では、少なくとも次を満たす必要があります。

- `sessiond` が resume state machine を持つ
- `displayd` は output 再確立に専念する
- `compd` は resume 完了前に通常描画へ戻らない
- lock 要求は resume path の成否と独立に扱う

ロック画面を resume path の途中に埋め込むと、障害解析不能な複合故障になります。

## ログ方針

各プロセスは別ログにします。

- `displayd.log`
- `waylandd.log`
- `compd.log`
- `lockd.log`
- `sessiond.log`

これにより、「何が最初に壊れたか」をあとから追えるようにします。
