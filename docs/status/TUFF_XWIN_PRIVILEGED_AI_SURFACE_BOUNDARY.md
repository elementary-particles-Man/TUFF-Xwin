# TUFF Xwin Privileged AI Surface Boundary

## Coverage

- Added Xwin security boundary for privileged AI browser/desktop surfaces.
- Focuses on surface permission requests: clipboard, file picker, drag/drop, screen/window capture, browser profile surface, native messaging, external protocol, and download/upload dialogs.
- Consumes supplied PAL/KAIRO markers to identify privileged AI context and sensitive data presence.

## Boundary Logic

- **Allow**: No privileged AI surface or clean display-only context.
- **Observe**: Privileged AI surface present with low-risk display-only context.
- **Constrain**: Low/medium risk surface request without sensitive context overlap.
- **Quarantine**:
    - Clipboard, file-picker, drag-drop, screen/window capture, or dialogs requested while sensitive or external-input privileged AI context is present.
    - Propagated KAIRO quarantine.
- **Fail Closed**:
    - Native messaging, external protocol launch, or browser profile surface requested with privileged AI sensitive context.
    - Propagated KAIRO fail-closed.
    - Unknown risky surface permission requested in privileged AI context.

## Non-Goals & Scope Boundaries

- Surface boundary only. No browser launch, URL opening, or JS execution.
- No webdriver/chromedriver/playwright/selenium usage.
- No reading of cookies, profiles, files, clipboard contents, or screenshots.
- No mutation of browser profiles, extensions, or sandbox.
- No global AI blocking; constraints apply only when privileged context and risky surface capability overlap.
