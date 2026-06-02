# Wayland Wire Protocol Parity

## Overview
This document tracks the progress of achieving runtime wire protocol parity in TUFF-Xwin. This phase follows the **Architectural Parity Baseline** (main branch) and aims to provide a low-level execution core for standard Wayland client communication.

## Implementation Policy (Headless / Repository Only)
The implementation is strictly limited to the TUFF-Xwin repository. 

- **NO** interaction with the running OS or production Wayland sessions.
- **NO** binding to `XDG_RUNTIME_DIR` sockets.
- **Global Advertisement**: Ability to advertise standard globals like `wl_compositor`, `wl_shm`, and `wl_seat`.

## Phase 2: Core State Machines
- **Surface Lifecycle**: Implemented pending/current state for `wl_surface`.
- **SHM Management**: Added `wl_shm` pool and buffer tracking.
- **Regions**: Added `wl_region` support.

## Phase 3: Protocol XML and Metadata (Current)
- **Repo-local XML**: Core protocol spec saved in `protocols/core/wayland-core.xml`.
- **XML Parser**: Added `wayland-wire` capability to parse protocol XML into metadata.
- **Signature Validation**: Ability to generate signature strings and validate `WireArg` counts/types against XML spec.
- **Metadata-driven Dispatch**: Hand-written dispatchers now validate incoming messages against the parsed metadata.

## Phase 4: Isolated Socket Harness (Current)
- **Isolated Server**: Added `WireServer` in `wayland-wire` that binds to a caller-specified Unix socket (strictly outside `XDG_RUNTIME_DIR`).
- **Fake Client**: Added `WireFakeClient` to simulate client requests over the wire.
- **E2E Testing**: Established end-to-end tests that perform a full Wayland handshake and surface commit over a temporary Unix socket.
- **Strict Path Validation**: Both `wayland-wire` and `waylandd` reject socket paths in runtime directories to ensure isolation.
- **Isolated Socket Harness**: Server and Fake Client for byte-stream verification.

## Phase 4b: Surface Commit E2E (Current)
- **Extended E2E**: Verified the full sequence (`registry` -> `bind` -> `create_surface` -> `create_pool` -> `attach` -> `commit` -> `callback.done`) over a temporary Unix socket.
- **State Split**: Confirmed that `pending` state remains isolated until `commit` is received.
- **Resource Management**: Verified SHM pool and buffer lifecycle, including bounds checking.
- **E2E Validation**: Full sequence from connection to `xdg_toplevel` setup and input focus is verified over isolated sockets.

## Phase 7: Libwayland XDG Shell (Current)
- **Real Client XDG**: Verified the full `xdg-shell` lifecycle (`get_xdg_surface` -> `get_toplevel` -> `configure` -> `ack_configure` -> `commit`) using the official C `libwayland-client` library.
- **Protocol Stubs**: Added repository-local C stubs for `xdg-shell` to avoid external `wayland-scanner` dependency during testing.
- **Metadata Verification**: Confirmed that window titles, app IDs, and configure serials are correctly tracked and synchronized between the Rust server and the C client.
- **Ping/Pong**: Implemented and verified the `xdg_wm_base.ping` / `pong` mechanism.

## Current Status
- [x] **Headless Wire Core**: Base crate `wayland-wire` added to workspace.
- [x] **Codec**: Support for encoding/decoding object ID, opcode, and size.
- [x] **ID Management**: Registry for tracking client and server allocated IDs.
- [x] **Bootstrap**: Support for `get_registry` and `sync` requests.
- [x] **Protocol XML Parser**: Local parser for `.xml` protocol definitions.
- [x] **Metadata Validation**: Automated verification of opcode and arguments.
- [x] **Isolated Socket Harness**: Server and Fake Client for byte-stream verification.
- [x] **Surface Commit E2E**: Full protocol sequence verified over wire.
- [x] **Libwayland Compatibility**: Verified against real C client requests.
- [x] **FD Passing / SHM**: Full support for SCM_RIGHTS and memory sharing.
- [x] **XDG Shell & Input**: Handshake and event routing state machines implemented.
- [x] **Real Client XDG Lifecycle**: Verified with `libwayland-client`.
- [ ] **Full Interop**: Compatibility with standard libwayland-based clients is **High** (all core desktop protocols verified).

## Next Steps
1. **Phase 8: Data Device and Subcompositor**: Implement `wl_data_device` (clipboard/DnD) and `wl_subcompositor` wire state machines.
2. Integrate with `compd` and `waylandd` for real-world window management policy testing.


2. Implement protocol XML parsing and code generation to avoid manual payload manipulation.
3. Add a headless compositor test harness that can run simple Wayland clients.

## Phase 8: Data Device (Clipboard & DnD)
- **wl_data_device_manager**: Implemented factory for data sources and data devices.
- **wl_data_source / wl_data_offer**: Lifecycle management for clipboard and Drag and Drop payloads.
- **Selection Flow**: Implemented full wire-level handshake for clipboard selection (set_selection -> data_offer/selection events -> receive -> send).
- **DnD State Machine**: Implemented `start_drag` with automatic pointer focus integration (simulated).
- **FD Transfer**: Support for transferring payload FDs via isolated `SCM_RIGHTS` ancillary messages.
- **Strict Validation**: Added reject-tests for invalid serials, destroyed sources/offers, and missing FDs.

## Phase 9: Popup and Subsurface
- **xdg_popup / xdg_positioner**: Implemented wire state machine for popups, including positioner configuration and layout negotiation.
- **wl_subcompositor / wl_subsurface**: Implemented parent-child surface relationships at the wire level, supporting position offsets and sync/desync commit modes.
- **Validation**: Added E2E tests for popup lifecycle and subsurface hierarchical state management in isolated sockets.
- **Strict Protocol Compliance**: Enforced parent surface existence and positioner validity for popup creation.

## Phase 10: Text Input and Input Method
- **zwp_text_input_v3**: Implemented wire state machine for client-side text input, including surrounding text, content type, and cursor rectangle management.
- **zwp_input_method_v2**: Implemented wire-level IME protocol support, allowing a fake IME backend to coordinate with text-input clients.
- **FakeImeBackend**: Added a repository-local fake IME for testing preedit and commit string flows without real IME processes.
- **Validation**: Verified full round-trip from text-input enable to IME commit-string response in isolated sockets.
- **Strict Checks**: Added rejections for duplicate input-method binding and focus-less text-input operations.

## Phase 11: Display Scale, Viewporter, and Decoration
- **wp_viewporter**: Implemented wire state machine for `wp_viewport` objects, mapping source rect and destination size to the surface's pending state, applied on commit.
- **wp_fractional_scale_v1**: Implemented `preferred_scale` notification based on the output's fractional scale factor.
- **zxdg_decoration_manager_v1**: Implemented ServerSide/ClientSide decoration mode negotiation.
- **wp_presentation**: Connected `wp_presentation_feedback` to surface commits, dispatching `presented` events with fake clock timestamps, and `discarded` events on surface destruction.
- **Validation**: Added E2E tests verifying the complete lifecycle of viewports, fractional scales, decorations, and presentation feedback over isolated sockets.
- **Strict Protocol Compliance**: Added rejections for double viewports, invalid bounds, and operations on destroyed objects.
