# Wayland Standard Feature Compatibility Table

This table compares standard Wayland features (as seen in major compositors like KDE Plasma or wlroots-based ones) with the current implementation status in TUFF-Xwin.

| Feature Category | Standard Protocol / Feature | TUFF-Xwin Status | Implementation Detail |
| :--- | :--- | :--- | :--- |
| **Screenshot** | `wlr-screencopy-v1`, `ext-image-copy-capture` | **Partial (Mock)** | Custom IPC `CaptureOutput`. Currently returns **mock pixels** via `displayd` (not real frame capture). |
| **Screen Recording** | `wlr-screencopy-v1` + PipeWire | **Partial (Mock/IPC)** | Added `StartRecord` / `StopRecord` IPC. Mock frame capture logic in `displayd`. **No real PipeWire/video encoding yet.** |
| **Clipboard** | `wl_data_device` | **Minimal** | Basic metadata tracking in `waylandd`. Full data transfer between clients is **unimplemented**. |
| **Primary Selection** | `zwp_primary_selection_v1` | **Minimal** | Similar status to clipboard. |
| **Drag and Drop** | `wl_data_device` | **Missing** | Not currently implemented in the IPC or state tracking. |
| **Layer Shell** | `wlr-layer-shell-v1` | **Partial (Stub)** | Added basic role-based layout logic (Background/Layer) in `compd`. **Hardcoded geometry (1920x1080 / height 36).** |
| **Idle Inhibition** | `idle-inhibit-v1` | **Partial (IPC)** | Added `InhibitIdle` / `ReleaseIdle` IPC in `sessiond`. Tracks inhibitors in memory. |
| **Input Method / IME** | `text-input-v3`, `input-method-v2` | **Missing** | No virtual keyboard or international input support. |
| **Pointer Constraints** | `wp_pointer_constraints_v1` | **Partial (Log-only)** | Added `SetPointerConstraints` IPC in `displayd`. **Diagnostic log only; no real input device locking.** |
| **Relative Pointer** | `zwp_relative_pointer_v1` | **Missing** | Required for unaccelerated relative motion (e.g., in FPS games). |
| **Gamma Control** | `wlr-gamma-control-v1` | **Partial (Log-only)** | Added `SetGamma` IPC in `displayd`. **Diagnostic log only; no real hardware LUT modification.** |
| **Output Management** | `wlr-output-management-v1` | **Partial** | Internal IPC `EnumerateOutputs` / `SetMode` exists in `displayd`. |
| **Presentation Time** | `wp_presentation` | **Missing** | Precise frame timing feedback for clients. |
| **Foreign Toplevel** | `ext-foreign-toplevel-list-v1` | **Missing** | No standard way for panels/taskbars to list all windows. |

## Implementation Progress (2026-05-27)

1. **Screen Recording (IPC/Mock)**: 
   - Added `StartRecord` and `StopRecord` commands to `DisplayCommand` and `WaylandCommand`.
   - Implemented tracking of active recording sessions in `displayd`.
   - **Note**: Currently produces `mock-video-data` files. Future integration with PipeWire required.
2. **Idle Inhibition (IPC)**: 
   - Added `InhibitIdle` and `ReleaseIdle` commands to `SessionCommand`.
   - Implemented tracking of inhibitors in `sessiond` (within `SessionSupervisor`).
3. **Gamma Control & Pointer Constraints (Diagnostic)**:
   - Added `SetGamma` and `SetPointerConstraints` to `DisplayCommand`.
   - Implemented handlers in `displayd` for log-only verification.
4. **Layer Shell (Basic Layout Stub)**:
   - Added `apply_role_based_layout` in `compd` to automatically position `Background` and `Layer` (panel) surfaces during scene reconciliation.
   - **Note**: Uses hardcoded values for screen size and panel height.
5. **Wayland Display Listener (Diagnostic)**:
   - `waylandd` includes a minimal listener for `client_connected` observation. This is **not** a full Wayland protocol server.
