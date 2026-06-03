# TUFF-Xwin Wayland Wire Parity P15 Final Audit

## 位置づけ

P15 は TUFF-Xwin の Wayland wire parity ラインにおける最終監査フェーズです。
このフェーズでは新しい Wayland protocol 機能は追加しません。

## 正式リポジトリ

`/mnt/thpdoc/Develop/TUFF-Xwin`

## 到達点の整理

- P1 から P13 までは repo-local の XML、isolated socket、fake backend による wire parity を実装済みです。
- P14 では互換性マトリクスと optional isolated harness の整備を行いました。
- P15 では RC 前の最終確認として、記述と検証の境界を固定します。

## バージョン鎖

- P12 baseline: `12876becc249e3d6a5f9fe847b0eca8f9aa261a5`
- P13 baseline: `a09fe8d4a33bfa599ff6121884edce50c9bb2a4b`
- P14 baseline: `d8431f2dfa1474f8b563b6c6f53da10be8e11ad3`
- P15 final: このブランチで作成し、後続で baseline ref として固定する

## 判定ラベル

- `Complete(Wire)`: repo-local wire 実装と isolated socket テストが揃っている。
- `Partial(Wire)`: wire 実装はあるが対象範囲が限定される。
- `FakeBackendOnly`: fake backend のみで検証する。
- `HarnessOnly`: optional isolated harness だけで確認する。
- `NotRealSession`: 実 Wayland session 非依存のまま維持する。

## 非対象

- 実 Wayland session への接続
- 実 input device の操作
- 実 display/output/DRM/KMS/PipeWire の操作
- 親 TUFF-OS リポジトリの操作
- `main` 統合
- P16 の作成

## 検証項目

- `cargo fmt --check`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo test --manifest-path crates/wayland-wire-harness/Cargo.toml`
- `git diff --check`

## 補足

optional isolated harness は `libwayland-client` が存在する場合にのみ使い、`wl_display_connect(NULL)` は使いません。
実行経路は tempdir 配下の isolated socket に限定します。
