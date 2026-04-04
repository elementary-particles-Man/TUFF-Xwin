# Privacy & Artifacts Policy

## Information Stored (Runtime Artifacts)
During the operation of the TUFF-Xwin display architecture, certain runtime artifacts are temporarily stored:
- Scene snapshots and tree states.
- Surface registry dumps.
- Watchdog and session component logs / artifacts.
- Specific metadata (e.g., selection owner, payload ID, clipboard serial numbers).

## Information NOT Stored
- **We do not persist the actual content of the clipboard.**
- The system is not designed to, and does not aim to, permanently store user input text or actual screen contents.

## Storage Location
- **Primary:** `XDG_RUNTIME_DIR/waybroker` (Recommended and preferred).
- **Fallback:** System temporary directory (`/tmp/waybroker` or equivalent).

## Retention Period
These files are **runtime artifacts only**. They are not intended for permanent storage and exist solely for the duration of the active session or for immediate recovery purposes.

## Cleanup
Users or integrators are responsible for clearing the runtime directory after session termination if the system does not automatically purge `XDG_RUNTIME_DIR` upon logout.
