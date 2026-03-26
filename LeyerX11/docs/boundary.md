# LeyerX11 Boundary

## Purpose

`LeyerX11` は `X11` アプリ互換のための橋であって、`Waybroker` の新しい中核ではありません。ここでは「どこまでを `LeyerX11` の責務にし、どこから先を `Waybroker` 側の責務にするか」を固定します。

## LeyerX11 が持つもの

- `X11` window id と property の互換表現
- rootless window の geometry
- `WM_CLASS`、title、window type の最小解釈
- `Waybroker` scene へ落とすための変換

## LeyerX11 が持たないもの

- `DRM/KMS`、input device、seat、VT
- scene policy の最終決定
- desktop shell の UI policy
- `PAM` や power policy

## 初期非目標

- full `Xorg` 互換
- remote display
- `GLX`
- compositing manager の再実装
- 古い ICCCM/EWMH の完全実装

## 最初の wire

最初の実装では、`LeyerX11` は rootless `X11` window state を `SurfaceSnapshot` へ変換し、`displayd` へ `CommitScene` を送るだけに留めます。

```text
X11 apps
   |
[ LeyerX11 / x11bridge ]
   |
CommitScene over Unix socket
   |
[ displayd ]
```

ここで重要なのは、「`X11` 互換の重さ」を broker 本体へ逆流させないことです。
