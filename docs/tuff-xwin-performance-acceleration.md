# TUFF-Xwin Performance Acceleration Notes

## 目的

この文書は、P15 完了後の性能リファイン方針をまとめたものです。
次の最短目標は TUFF-Xwin 標準アクティブウィンドウキャプチャに耐える低 CPU / 低メモリ基盤を整えることです。

## 重点

- Vulkan は利用可能な場合の標準経路として使います。`displayd` と `compd` は起動時に GPU 初期化を試みます。`waylandd` は buffer import 経路が実装されるまで protocol/state 管理に専念します。
- `TUFF_XWIN_DISABLE_VULKAN=1` で Vulkan を完全無効化できます。
- `TUFF_XWIN_DISABLE_ACCEL=1` で Vulkan / SIMD をまとめて無効化できます。
- `TUFF_XWIN_DISABLE_SIMD=1` で SIMD 経路のみ無効化できます。
- portable fallback は常に残します。
- `--no-vulkan` を指定した個別サービスは CPU 経路を強制できます。
- GPU 初期化失敗時はサービスを停止させず、CPU fallback へ切り替えます。
- 実 Wayland session、実 DRM/KMS、実 PipeWire、実 input device には触りません。

## 実装境界

- `crates/vulkan-backend`: GPU 支援と AVX2 pixel refine の共通バックエンド
- `crates/waybroker-common/src/accel.rs`: 環境変数と feature の加速ポリシー
- `crates/wayland-wire/src/pixel_ops.rs`: pixel 変換の portable / SIMD 切替
- `crates/wayland-wire/src/screencopy.rs`: snapshot 用 scratch buffer
- `crates/wayland-wire/src/image_copy_capture.rs`: capture frame の scratch 再利用

## 期待される効果

- 小さい wire メッセージと pixel buffer の再確保を減らす
- capture/screencopy 系の変換を将来の active-window capture に流用しやすくする
- feature 無効環境でも `cargo check/test` が通る
