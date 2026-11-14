# 2026-11-14 wl_seat / IPC 修正追試

対象PR: [#35](https://github.com/elementary-particles-Man/TUFF-Xwin/pull/35)

修正commit: `9a4f1550901a9cc4aee9dc3168180ec8f95b7adc`

## 実装

- `wl_seat.get_touch`、`wl_seat.release`、`wl_touch.release`、`wl_keyboard.release`を追加。
- touch objectの最小登録・解除・ID重複検証を追加。
- 未実装の物理タッチ入力は広告せず、seat capabilitiesを`pointer|keyboard=3`へ統一。
- `scripts/run-waylandd-ipc-isolation-smoke.sh`の5秒上限付き再接続処理はPR先端に既存実装済みで、今回のコード差分は不要だった。

## 実測

- `cargo fmt --all --check`: PASS
- `RUSTFLAGS=-D warnings cargo check --all-targets`: PASS
- `cargo test --all-targets`: PASS
- `cargo clippy --all-targets --all-features -- -D warnings`: PASS
- `bash scripts/dev-check.sh`: PASS
- IPC isolation smoke 10連続: PASS（全10回exit 0）
- live topology / canonical replay / displayd reconnect / integration smoke: PASS
- `scripts/install-user.sh`: PASS。修正commit基準のrelease binaryを`/home/flux/.local/share/tuff-xwin`へ配置。

## Chrome専用endpoint

専用runtime `/run/user/1000/waybroker`、socket `wayland-tuff`、専用一時profile、sandbox有効で実Chromeを接続した。

- `client_connected`: PASS
- `wl_seat opcode=2`: 再発なし
- Chrome終了: exit 0（試験後のTERMによる制御終了）
- 表面描画/portal/DRM pageflip: NOT_VERIFIED
- 次の未対応要求として`wl_region opcode=1`が記録され、ChromeはBroken pipeのログを出した。今回のwl_seat修正の範囲外であり、追加実装は行っていない。

## 保全

KWin/Plasmaは停止・kill・takeoverしていない。primary worktreeは作業前後とも既存のdirty/untracked状態を保持した。今回の専用current-sessionのみを使用した。
