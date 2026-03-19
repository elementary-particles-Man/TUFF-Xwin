# Plasma Integration

## 前提

狙いは `Plasma Desktop` を捨てることではありません。ユーザーが見ているシェル、パネル、通知、設定 UI はできるだけ維持しつつ、`KWin` の責務だけを再分割します。

## どう切るか

### KWin から displayd に出すもの

- `DRM/KMS` 直叩き
- `libinput` の直接受信
- `seat` と `VT` の所有
- output hotplug の低レベル処理

### KWin から waylandd に出すもの

- Wayland client 接続終端
- global object 管理
- surface/buffer の寿命追跡
- clipboard/DnD の基本面

### KWin に残すもの

- フォーカス
- タイリングや配置
- 装飾
- effect
- stacking order
- shortcut policy の上位判断

ここに残るものを暫定的に `compd` と呼びます。

## Plasma Shell との関係

`plasmashell` は通常の privileged でない client として扱うべきです。

- パネル
- デスクトップ
- 通知
- ウィジェット

は `compd` から特別扱いを受けてもよいですが、`displayd` の内部には入れません。

## KScreenLocker の扱い

現行では `KScreenLocker` は `KWin` と密接に結びつきがちです。`Waybroker` ではこれを `lockd` として別 service にします。

必要な条件:

- secure input path
- 認証成功後の明示的 state transition
- compd crash 中でも lock state を保持できること

## PowerDevil の扱い

`PowerDevil` は UX レイヤであるべきで、display server の中枢ではありません。したがって、

- idle policy
- lid close action
- suspend request
- screen dimming policy

は `sessiond` に委譲するか、少なくとも `compd` の外へ出します。

## polkit と PAM

この系統は display stack と近すぎると壊れ方が悪くなります。

- `PAM` は `lockd` や session login に限定する
- `polkit` は `sessiond` が対話要求を broker する
- `compd` は認証実装を持たない

## Xwayland 互換

`KDE Plasma` を現実に使うなら、当面 `Xwayland` は必須です。

そのため、

- `Xwayland` は rootless
- 死んでも Wayland native client を巻き込まない
- clipboard と selection の橋渡しは `waylandd` が吸収する

という前提で設計します。

## 段階導入

現実には全面改造は無理なので、段階導入が必要です。

1. `KWin` のログと復旧境界を明示化する
2. `lockscreen` を分離する
3. `power/session policy` を分離する
4. `DRM/input` broker を外出しする
5. `Wayland endpoint` を切り出す

いきなり全部やるのではなく、「まず一番壊れ方が悪いところ」から切るべきです。
