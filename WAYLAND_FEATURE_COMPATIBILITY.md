# Wayland Standard Feature Compatibility Table

This table compares standard Wayland features (as seen in major compositors like KDE Plasma or wlroots-based ones) with the implementation status in TUFF-Xwin.

| Feature Category | Standard Protocol / Feature | TUFF-Xwin Status | Implementation Detail |
| :--- | :--- | :--- | :--- |
| **Screenshot** | `wlr-screencopy-v1`, `ext-image-copy-capture` | **Complete (Wire)** | `CaptureBackend` implemented. `FakeCaptureBackend` used for testing. Production logic isolated behind feature flags. |
| **Screen Recording** | `wlr-screencopy-v1` + PipeWire | **Complete (Wire)** | `RecordBackend` implemented. `FakeRecordBackend` used for testing. |
| **Clipboard** | `wl_data_device` | **Complete (Wire)**  | `DataPayloadRegistry` implemented. Support for `WriteData` / `ReadData` and offer lifecycle. |
| **Primary Selection** | `zwp_primary_selection_v1` | **Complete (State)** | Similar to clipboard. Tracks offers and owners across handoffs. |
| **Drag and Drop** | `wl_data_device` | **Complete (Wire)**  | `DnDState` machine tracks enter/motion/drop/leave/cancel. |
| **Layer Shell** | `wlr-layer-shell-v1` | **Complete (Wire)** | Repository-local wire state machine with configure/ack lifecycle, role validation, and placement calculation over fake output geometry. |
| **Idle Inhibition** | `idle-inhibit-v1` | **Complete (Wire)** | Repository-local wire state machine with fake idle backend bookkeeping only; no host idle settings are touched. |
| **Input Method / IME** | `text-input-v3`, `input-method-v2` | **Complete (State)** | `ImeRuntimeState` tracks focus, preedit, commit, and cursor rect. `ImeBackend` boundary defined. |
| **Pointer Constraints** | `wp_pointer_constraints_v1` | **Complete (Wire)** | Repository-local wire state machine tracks lock/confine, region updates, and cleanup on destroy/release. |
| **Relative Pointer** | `zwp_relative_pointer_v1` | **Complete (Wire)** | Repository-local wire state machine emits relative motion events from fake input injection only. |
| **Gamma Control** | `wlr-gamma-control-v1` | **Complete (Boundary)** | `DisplayBackend` handles gamma LUT validation. |
| **Output Management** | `wlr-output-management-v1`, `xdg-output` | **Complete (Wire)** | `DisplayBackend` abstracts output inventory and mode setting. |
| **Presentation Time** | `wp_presentation` | **Complete (Wire)** | `PresentationClock` trait and feedback tracking via `FramePresented` events. |
| **Foreign Toplevel** | `ext-foreign-toplevel-list-v1` | **Complete (State)** | `ForeignToplevelHandle` registry in `SurfaceRegistrySnapshot`. |
| **Popups** | `xdg_popup` | **Complete (Wire)** | Wire state machine for transient surfaces and positioner logic. |
| **Viewporter** | `wp_viewporter` | **Complete (Wire)** | Wire state machine for viewport source/destination rects. |
| **Fractional Scale** | `wp_fractional_scale_v1` | **Complete (Wire)** | Wire state machine for 120-based preferred scaling. |
| **Window Decoration** | `xdg_decoration` | **Complete (Wire)** | ServerSide/ClientSide decoration mode negotiation. |
| **Subsurfaces** | `wl_subsurface` | **Complete (Wire)** | Wire-level parent-child surface hierarchy and sync commit support. |
| **Input Method** | `zwp_input_method_v2` | **Complete (Wire)** | Wire-level IME protocol and fake backend integration. |
| **Text Input** | `zwp_text_input_v3` | **Complete (Wire)** | Wire state machine for client-side text editing state. |

## Current Parity Status

1. **Architectural Parity Baseline**: Complete. Core broker architecture and trait boundaries are established.
2. **Wire Protocol Parity (P1-P5)**: Underway. 
   - P1: Headless wire protocol core (`wayland-wire`) added.
   - P2: `wl_compositor`, `wl_surface`, `wl_shm` state machines added.
   - P3: Repository-local XML protocol spec parser and metadata validation integrated.
   - P4: Isolated Unix socket harness for handshake verification.
   - P4b: Surface commit E2E verified over wire (registry -> bind -> surface -> shm -> commit).
   - P5: Libwayland client compatibility harness added. Handshake and surface creation verified with real C client.
   - P5b: SCM_RIGHTS (FD passing) and SHM pool/buffer lifecycle verified with real C client over isolated socket.
   - P6: XDG Shell and Seat/Input state machines verified over isolated wire (handshake -> configure -> ack -> input events).
   - P7: Full XDG Shell lifecycle verified with real C libwayland-client using repository-local C stubs (avoiding external wayland-scanner).
   - P13: Layer shell, idle-inhibit, relative-pointer, and pointer-constraints wire state machines added with isolated-socket-only fake backends.

## Implementation Progress (2026-05-27)

All Wayland parity features have reached **Architectural Completion**. The core broker logic is now decoupled from OS-specific implementations through clean trait boundaries and state machines.

### Key Tests Added
- `test_ime_state_transitions`: Verifies IME focus and editing lifecycle.
- `test_dnd_and_data_transfer_lifecycle`: Verifies clipboard and DnD data paths.
- `test_layer_shell_layout_logic`: Verifies precise positioning based on output geometry and metadata.
- `test_handle_capture_output`: Verifies screenshot capture and presentation feedback queries.
- `test_relative_pointer_motion`: Verifies raw input event routing.

### Backend Abstractions
- `CaptureBackend`: Isolates frame capture from IPC handling.
- `RecordBackend`: Isolates video encoding/recording lifecycle.
- `DisplayBackend`: Isolates DRM/KMS operations.
- `PresentationClock`: Provides monotonic timestamps for frame timing.
- `ImeBackend`: Isolates IME bridge communication.
