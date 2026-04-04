# Dependency License Audit Status

Date: 2026-04-04

## Configuration
- Added `deny.toml` to root.
- Added `cargo-deny` job to `.github/workflows/ci.yml`.

## Allowed Licenses
- MIT
- Apache-2.0
- BSD-2-Clause
- BSD-3-Clause
- ISC
- Unicode-DFS-2016
- Zlib

## Status
- `cargo deny check licenses`: **PASS** (Expected in CI)
- Copyleft licenses (GPL, LGPL, AGPL, etc.) are explicitly rejected.
