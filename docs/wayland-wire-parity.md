# Wayland Wire Protocol Parity

## Overview
This document tracks the progress of achieving runtime wire protocol parity in TUFF-Xwin. This phase follows the **Architectural Parity Baseline** (main branch) and aims to provide a low-level execution core for standard Wayland client communication.

## Implementation Policy (Headless / Repository Only)
The implementation is strictly limited to the TUFF-Xwin repository. 

- **NO** interaction with the running OS or production Wayland sessions.
- **NO** binding to `XDG_RUNTIME_DIR` sockets.
- **Verification**: Done via in-memory byte streams and temporary Unix sockets within unit tests.

## Phase 1: Wire Protocol Core (Current)
We have added the `wayland-wire` crate which handles:
- **Message Framing**: Encoding and decoding of Wayland wire messages (8-byte header + variable payload).
- **Object Registry**: Management of object/resource IDs and interface names.
- **Core Dispatcher**: Minimal handling for `wl_display` and `wl_registry`.
- **Global Advertisement**: Ability to advertise standard globals like `wl_compositor`, `wl_shm`, and `wl_seat`.

## Current Status
- [x] **Headless Wire Core**: Base crate `wayland-wire` added to workspace.
- [x] **Codec**: Support for encoding/decoding object ID, opcode, and size.
- [x] **ID Management**: Registry for tracking client and server allocated IDs.
- [x] **Bootstrap**: Support for `get_registry` and `sync` requests.
- [ ] **Protocol Generation**: `wayland-scanner` equivalent is **unimplemented**.
- [ ] **Full Interop**: Compatibility with standard libwayland-based clients is **unimplemented**.

## Next Steps
1. Expand the dispatcher to handle more core interfaces (`wl_shm_pool`, `wl_surface`).
2. Implement protocol XML parsing and code generation to avoid manual payload manipulation.
3. Add a headless compositor test harness that can run simple Wayland clients.
