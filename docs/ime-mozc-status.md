# IME / Mozc Support Status

## Overview
This document tracks the implementation status of Input Method (IME) and Mozc support in TUFF-Xwin.

## Implementation Policy (Repository Only)
As of 2026-05-27, IME support is in the **Scaffold** phase. The implementation is strictly limited to the TUFF-Xwin repository code to avoid any interference with the host OS or running IME processes (Mozc, Fcitx, IBus).

- **NO** interaction with running processes.
- **NO** modification of host environment variables or system files.
- **NO** `systemctl`, `pkill`, or `ps` commands used during development.

## Current Progress
- [x] **IPC Definition**: Added `ImeCommand`, `ImeEvent`, `ImeStatus`, and `ImeBridgeMode` to `waybroker-common`.
- [x] **State Management**: Added `ImeRuntimeState` to `waylandd` to track bridge modes and text surface focus.
- [x] **Diagnostic Stubs**: Implemented handlers in `waylandd` to respond to IME status queries.
- [ ] **Protocol Support**: `text-input-v3` and `input-method-v2` are **unimplemented**.
- [ ] **Real Bridge**: Connection to a real Mozc or Fcitx instance is **unimplemented**.

## Usage (Internal Testing Only)
The current implementation allows testing IPC message flow and state transitions within the TUFF-Xwin stack:

1. `GetImeStatus`: Retrieve current memory-only state.
2. `SetImeBridgeMode`: Test switching between `Disabled`, `PassthroughExternal`, and `ProtocolStub`.
3. `FocusTextSurface` / `ClearTextFocus`: Simulate text input focus changes.

## Future Roadmap
1. Implement standard Wayland IME protocols within `waylandd`.
2. Create a secure IME bridge component that isolates Mozc communication.
3. Integrate with `compd` for candidate window placement.
