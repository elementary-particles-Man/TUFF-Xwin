#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

packages=(
  waylandd
  compd
  displayd
  lockd
  sessiond
  watchdog
)

echo "Running Waybroker stack stubs from $repo_root"

for pkg in "${packages[@]}"; do
  echo
  echo "==> cargo run -p $pkg"
  cargo run -p "$pkg"
done
