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

## Current Status
- [x] **Headless Wire Core**: Base crate `wayland-wire` added to workspace.
- [x] **Codec**: Support for encoding/decoding object ID, opcode, and size.
- [x] **ID Management**: Registry for tracking client and server allocated IDs.
- [x] **Bootstrap**: Support for `get_registry` and `sync` requests.
- [x] **Protocol XML Parser**: Local parser for `.xml` protocol definitions.
- [x] **Metadata Validation**: Automated verification of opcode and arguments.
- [ ] **Protocol Generation**: `wayland-scanner` equivalent is **unimplemented**.
- [ ] **Full Interop**: Compatibility with standard libwayland-based clients is **unimplemented**.


## Next Steps
1. Expand the dispatcher to handle more core interfaces (`wl_shm_pool`, `wl_surface`).
2. Implement protocol XML parsing and code generation to avoid manual payload manipulation.
3. Add a headless compositor test harness that can run simple Wayland clients.
