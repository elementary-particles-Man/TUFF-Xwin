# minimal-scene

`Waybroker` が最初に扱うべき最小 scene のイメージです。ここでは本物の protocol 実装ではなく、必要な state の最小集合を固定することだけを目的にします。

## Scenario

- output は 1 枚
- client は 2 つ
- foreground window は 1 つ
- lock state は off
- animation は無し

## Files

- [scene.toml](/media/flux/THPDOC/Develop/TUFF-Xwin/examples/minimal-scene/scene.toml)

## Why It Exists

最小 scene が決まっていないと、`displayd` と `compd` の境界がすぐ曖昧になります。まずは「何を commit できれば 1 フレーム描けたことにするか」を固定します。
