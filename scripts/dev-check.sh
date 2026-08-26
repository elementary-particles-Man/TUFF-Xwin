#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ "${TUFF_XWIN_RUN_REAL_CAPTURE_TESTS:-}" == "1" ]]; then
  echo "dev-check refuses real portal capture opt-in; run real capture manually instead." >&2
  exit 1
fi

cargo fmt --all --check
cargo check --workspace
cargo test --workspace
