# TUFF Xwin Developer Tool Surface Boundary

## Coverage

- Added Xwin security boundary for developer-tool surfaces.
- Focuses on desktop/browser/IDE surface permissions that overlap with untrusted developer-source execution.
- Constrains clipboard, file picker, drag/drop, terminal launch, browser download/upload, and extension install prompts when untrusted developer context is present.
- Consumes supplied PAL/KAIRO markers to identify untrusted developer projects and sensitive data access.

## Boundary Logic

- **Allow**: No developer tool surface or trusted context without risky request.
- **Observe**: Untrusted developer context opened but no execution-adjacent surface requested.
- **Constrain**: Untrusted developer context plus low-risk surface request without secret/network/persistence overlap.
- **Quarantine**:
    - Clipboard, file-picker, drag-drop, terminal launch, dialogs, or capture requested from untrusted developer context.
    - Propagated KAIRO quarantine.
- **Fail Closed**:
    - External protocol launch requested from untrusted developer context.
    - Secret namespace context overlaps with terminal, file picker, or extension install.
    - Persistence write context overlaps with terminal or extension install.
    - Propagated KAIRO fail-closed.
    - Unknown untrusted developer context with risky surface request.

## Non-Goals & Scope Boundaries

- Surface boundary only. No terminal launch, browser launch, or script execution.
- No reading of clipboard contents, files, browser cookies, or terminal buffers.
- No mutation of browser profiles, extensions, or OS state.
- No global developer-tool blocking; constraints apply only when untrusted context and risky surface capability overlap.
