#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin Role-Scoped Recovery Execution Smoke Test
# Delegate to the proven compd broker recovery smoke so CI exercises the same
# recovery-execution artifact path without duplicating stack orchestration here.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

exec "$repo_root/scripts/run-compd-broker-recovery.sh"
