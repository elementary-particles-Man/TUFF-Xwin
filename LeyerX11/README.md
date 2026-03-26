# LeyerX11

`LeyerX11` は、`Waybroker` 本体の外側に載る最小 `X11` 互換レイヤの実験置き場です。ここでの狙いは「巨大な `X server` を再発明すること」ではなく、古い `X11` アプリを rootless window として収容し、`displayd` / `waylandd` / `compd` の責務を汚さずに互換性だけを島として切り出すことです。

## 方針

- `Waybroker` 本体には入れない optional layer として扱う
- local desktop 専用
- rootless window のみ
- 初期段階では network transparency, GLX, window manager 機能は持たない
- `X11` window state は `Waybroker` の scene に変換して submit する

## 置いてあるもの

- `crates/layerx11-common`
  - rootless `X11` scene の共通型
  - `Waybroker` scene への変換
- `crates/x11bridge`
  - sample scene を読み、`displayd` へ `CommitScene` する最小 bridge
- `docs/boundary.md`
  - `LeyerX11` の責務境界
- `examples/minimal-rootless-scene.json`
  - 最小 rootless scene の fixture
- `scripts/run-rootless-demo.sh`
  - `displayd` と `x11bridge` の往復デモ

## なぜ別ツリーか

ユーザーが `GNOME` / `KDE Plasma` / 軽量 shell を自由に入れ替えられる構成を作るには、互換レイヤまで broker 本体へ焼き込まない方がよいからです。`LeyerX11` は、必要な人だけが追加する compatibility island として扱います。
