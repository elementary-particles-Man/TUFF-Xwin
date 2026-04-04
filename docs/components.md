# Components

## displayd

### 役割

- `DRM/KMS` の所有
- output mode 管理
- `libinput` の受信
- `seat` と `VT` の制御
- secure display path の最低限の提供

### 持つべきもの

- KMS state
- connector / crtc / plane 情報
- 入力デバイス列挙
- 最後に成功した frame state

### 持つべきでないもの

- ウィンドウ配置ポリシー
- デスクトップ UI
- lockscreen の認証ロジック

### 死んだらどうなるか

表示は失うが、kernel と他の process は生きる。`systemd --user`、VT、SSH から復旧可能であることを目標にする。

## waylandd

### 役割

- Wayland client 接続終端
- global object の提供
- surface と buffer の寿命追跡
- clipboard / selection / DnD の基盤

### 設計意図

`compositor` が死んでも、client 接続そのものは維持しやすくする。`KWin` や `Mutter` を一度落としたら全 client が道連れになる現状を避ける。

現時点の試作では、`waylandd` は `surface registry snapshot` を IPC で返せます。snapshot には clipboard / primary selection owner も含みます。`compd` restart 後は `displayd` の last scene とこの registry を突き合わせ、生きている surface だけで scene を再構成し、dangling owner があれば focus へ handoff した結果を `waylandd` へ返します。

## compd

### 役割

- scene graph の構築
- focus 管理
- stacking order
- decoration と effect
- window rule と placement

### 位置づけ

これは `KWin-core` や `Mutter-core` に近い存在です。見た目や UX の大半はここに載りますが、特権は持たせません。

### 死んだらどうなるか

`watchdog` が再起動する。`waylandd` と `displayd` が残っていれば、接続維持、最終画面保持、clipboard / selection owner の再整列ができます。

## lockd

### 役割

- ロック画面の表示
- 認証 UI
- PAM 連携
- unlock 成功の通知

### 分離理由

lockscreen を `compositor` 本体に抱え込むと、「席を外した時の認証」の失敗が「画面系すべての停止」に化けやすいからです。

## sessiond

### 役割

- lid close / open
- suspend / resume の要求
- idle policy
- `polkit` 連携
- power key などの session policy
- desktop profile の選択と active state 管理

### 分離理由

`PowerDevil` や `gnome-settings-daemon` 的な都合を display server の心臓部に持ち込まないためです。

同時に、`xfce` や軽量 WM のような GUI profile 選択を broker 本体へ焼き込まず、user が入れ替えられるようにするためでもあります。

## Xwayland

### 役割

- 既存 X11 アプリの収容
- rootless window 提供
- X selection と Wayland clipboard の橋渡し

### 方針

`Xwayland` が死んでも、Wayland native app は死なない構成を保つことが重要です。

現時点の repository では、その compatibility island の試作置き場として `LeyerX11/` を追加し、rootless window state を `displayd` へ commit する最小 bridge を置いています。

## watchdog

### 役割

- `compd`、`lockd`、必要なら `waylandd` の監視
- crash loop 抑制
- degraded mode への切り替え

### 注意点

watchdog は権限を持ちすぎてはいけません。`PID 1` の代用品にせず、display stack 専用の supervisor として扱います。

## 境界のまとめ

```text
displayd = hardware broker
waylandd = protocol broker
compd    = policy and composition
lockd    = auth UI
sessiond = power and session policy
Xwayland = compatibility island
watchdog = recovery control
```
