# Wayland Standard Feature Compatibility Table

This table compares standard Wayland features (as seen in major compositors like KDE Plasma or wlroots-based ones) with the implementation status in TUFF-Xwin.

| Feature Category | Standard Protocol / Feature | TUFF-Xwin Status | Implementation Detail |
| :--- | :--- | :--- | :--- |
| **Screenshot** | `wlr-screencopy-v1`, `ext-image-copy-capture` | **Complete (Boundary)** | `CaptureBackend` implemented. `FakeCaptureBackend` used for testing. Production logic isolated behind feature flags. |
| **Screen Recording** | `wlr-screencopy-v1` + PipeWire | **Complete (Boundary)** | `RecordBackend` implemented. `FakeRecordBackend` used for testing. |
| **Clipboard** | `wl_data_device` | **Complete (State)** | `DataPayloadRegistry` implemented. Support for `WriteData` / `ReadData` and offer lifecycle. |
| **Primary Selection** | `zwp_primary_selection_v1` | **Complete (State)** | Similar to clipboard. Tracks offers and owners across handoffs. |
| **Drag and Drop** | `wl_data_device` | **Complete (State)** | `DnDState` machine tracks enter/motion/drop/leave/cancel. |
| **Layer Shell** | `wlr-layer-shell-v1` | **Complete (Logic)** | Role-based layout using real `OutputMode` geometry. Supports anchor, margins, and exclusive zone. |
| **Idle Inhibition** | `idle-inhibit-v1` | **Complete (IPC)** | `InhibitIdle` / `ReleaseIdle` tracking in `sessiond`. |
| **Input Method / IME** | `text-input-v3`, `input-method-v2` | **Complete (State)** | `ImeRuntimeState` tracks focus, preedit, commit, and cursor rect. `ImeBackend` boundary defined. |
| **Pointer Constraints** | `wp_pointer_constraints_v1` | **Complete (State)** | `DisplayState` tracks locked/confined regions per output. |
| **Relative Pointer** | `zwp_relative_pointer_v1` | **Complete (Event)** | `RelativePointerMotion` events and `InjectRelativePointerMotion` for testing. |
| **Gamma Control** | `wlr-gamma-control-v1` | **Complete (Boundary)** | `DisplayBackend` handles gamma LUT validation. |
| **Output Management** | `wlr-output-management-v1` | **Complete (Boundary)** | `DisplayBackend` abstracts output inventory and mode setting. |
| **Presentation Time** | `wp_presentation` | **Complete (Logic)** | `PresentationClock` trait and feedback tracking via `FramePresented` events. |
| **Foreign Toplevel** | `ext-foreign-toplevel-list-v1` | **Complete (State)** | `ForeignToplevelHandle` registry in `SurfaceRegistrySnapshot`. |

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
