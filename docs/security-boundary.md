# TUFF-Xwin Security Boundary

TUFF-Xwin は KDE 級の完全なデスクトップ環境を目標にしない。
最終目的は、GUI 側の panic や broker 側の失敗が kernel panic や実セッション障害へ連動しないように封じ込めることにある。

## 方針

- 実 Wayland session には接続しない。
- 実 DRM / KMS / PipeWire / input device には触らない。
- IPC は repo 内の tempdir / isolated socket / fake backend で検証する。
- screenshot app は hardening 後に実装する。
- 小型 Xwin アプリは broker 境界検証を兼ねる実用品として扱う。

## 入力境界

- IPC の JSON line と wire payload は明示的な上限で制限する。
- malformed input は panic ではなく Result error で返す。
- runtime path は session id と artifact name をサニタイズする。
- path traversal や absolute path の注入は拒否する。

## 失敗封じ込め

- capture backend failure は panic にしない。
- artifact 保存前に buffer サイズと寸法の整合性を検証する。
- watchdog / session instance id の不一致は拒否する。

