# Wayland Wire Compatibility Matrix

## 目的

この文書は、TUFF-Xwin の Wayland wire parity を RC 前に機械可読で整理するための互換性マトリクスです。

- ここでの `Wire` は、repo-local の `wayland-wire` 実装と isolated socket テストを意味します。
- `Real Wayland session` は対象外です。
- `wl_display_connect(NULL)` は使いません。
- 実 input device、実 display/output、実 Wayland session には触れません。

## 判定ラベル

- `Complete(Wire)`: wire state machine と isolated socket 検証が揃っている。
- `Partial(Wire)`: wire 実装はあるが、対象範囲が限定される。
- `HarnessOnly`: optional real-client isolated harness での確認範囲。
- `FakeBackendOnly`: fake backend のみで検証する。
- `NotRealSession`: 実 Wayland session 非依存のまま維持する。

## Wire Coverage

| 領域 | 主な protocol | 判定 | 備考 |
| :--- | :--- | :--- | :--- |
| Core / SHM / Region | `wl_display`, `wl_registry`, `wl_compositor`, `wl_surface`, `wl_shm`, `wl_region` | `Complete(Wire)` | isolated socket の基盤。 |
| XDG Shell / Popup / Subsurface | `xdg_wm_base`, `xdg_surface`, `xdg_toplevel`, `xdg_popup`, `wl_subcompositor`, `wl_subsurface` | `Complete(Wire)` | configure/ack, popup, parent-child commit を wire で扱う。 |
| Seat Input | `wl_seat`, `wl_pointer`, `wl_keyboard` | `Partial(Wire)` | fake input injection と pointer release までを repo-local で扱う。 |
| Data Device / Clipboard / DnD | `wl_data_device_manager`, `wl_data_source`, `wl_data_device`, `wl_data_offer`, `zwp_primary_selection_v1` | `Complete(Wire)` | clipboard / DnD の wire state machine を保持。 |
| Text Input / IME | `zwp_text_input_v3`, `zwp_input_method_v2`, `zwp_input_popup_surface_v2` | `Complete(Wire)` | isolated socket で IME roundtrip を検証。 |
| Output / Presentation / Viewporter / Decoration | `zxdg_output_manager_v1`, `zwlr_output_manager_v1`, `zwlr_screencopy_manager_v1`, `ext_image_copy_capture_manager_v1`, `wp_viewporter`, `wp_presentation`, `wp_fractional_scale_v1`, `zxdg_decoration_manager_v1` | `Complete(Wire)` | output / screencopy / presentation / scale / decoration を wire 化。 |
| P13 Layer/Input Control | `zwlr_layer_shell_v1`, `zwp_idle_inhibit_manager_v1`, `zwp_relative_pointer_manager_v1`, `zwp_pointer_constraints_v1` | `Complete(Wire)` | P13 で layer-shell / idle-inhibit / relative-pointer / pointer-constraints を追加。 |
| Fake Backend Controls | idle backend, fake input injection, fake capture backend | `FakeBackendOnly` | 実 OS の idle / input / capture には触れない。 |

## Optional Real-Client Isolated Harness

`crates/wayland-wire-harness` の optional probe は、`libwayland-client` が利用できる環境でのみ動作します。

| 範囲 | 判定 | 備考 |
| :--- | :--- | :--- |
| `wl_display` / `wl_registry` | `HarnessOnly` | tempdir 配下の isolated socket を使う。 |
| `wl_compositor` / `wl_shm` / `wl_surface` | `HarnessOnly` | surface commit までを確認する。 |
| `xdg-shell` | `HarnessOnly` | `xdg_surface` / `xdg_toplevel` の最小ライフサイクルを確認する。 |
| `seat` / `data-device` / `viewporter` / `presentation` / `layer-shell` | `NotRealSession` | 現時点では real-client probe の対象外。 |

## P14 到達点

- `crates/wayland-wire/tests/protocol_matrix.rs` で protocol metadata snapshot を検証する。
- `docs/wayland-wire-parity.md` と `WAYLAND_FEATURE_COMPATIBILITY.md` に RC 前 matrix を追記する。
- optional isolated harness は temp socket のみを使い、実 Wayland session には接続しない。
