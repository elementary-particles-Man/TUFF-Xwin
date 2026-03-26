#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

echo "Running desktop profile demo from $repo_root"

echo
echo "==> cargo run -p sessiond -- --list-profiles"
cargo run -p sessiond -- --list-profiles

echo
echo "==> cargo run -p sessiond -- --select-profile xfce-x11 --print-launch-plan --write-selection"
cargo run -p sessiond -- --select-profile xfce-x11 --print-launch-plan --write-selection
