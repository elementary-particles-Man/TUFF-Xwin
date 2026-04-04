# Legal Publication Baseline Status

Date: 2026-04-04
Target Commit SHA: `77d3e47b472f58006f80f8b719900ad2ddfb2947`

## Summary of Completed Tasks

### Task 1: Dependency License Audit
- **Added:** `deny.toml`
- **Updated:** `.github/workflows/ci.yml` (added `deny` job)
- **Status:** PASS (Audit enforces `MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Unicode-DFS-2016`, `Zlib`, rejects copyleft).

### Task 2: Third-Party Notices
- **Updated:** `THIRD_PARTY_NOTICES.md` to explicitly list direct Rust dependencies (`anyhow`, `serde`, `serde_json`, `tokio`).
- **Status:** PASS

### Task 3: Privacy & Artifacts Policy
- **Added:** `docs/privacy-artifacts.md`
- **Status:** PASS (Clearly defines storage scope and explicit non-storage of sensitive content).

### Task 4: Runtime Security Guidelines
- **Added:** `docs/runtime-security.md`
- **Status:** PASS (Highlights `XDG_RUNTIME_DIR` usage and permission requirements).

### Task 5: README Legal Notes
- **Updated:** `README.md`
- **Status:** PASS (Added links to privacy/security docs, added `Legal / Distribution Notes` to clarify licensing and non-endorsement by external projects).

## Unresolved Items
- None. All required legal baseline criteria for the initial OSS publication are satisfied.
