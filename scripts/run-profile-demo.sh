#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

runtime_dir="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker}"
launch_state="$runtime_dir/launch-state-demo-x11.json"

cleanup() {
  if [[ -f "$launch_state" ]]; then
    while read -r pid; do
      kill "$pid" 2>/dev/null || true
    done < <(rg -o '"pid":\s*[0-9]+' "$launch_state" | rg -o '[0-9]+')
  fi
}

trap cleanup EXIT

echo "Running desktop profile demo from $repo_root"

echo
echo "==> cargo run -p sessiond -- --list-profiles"
cargo run -p sessiond -- --list-profiles

echo
echo "==> cargo run -p sessiond -- --select-profile demo-x11 --print-launch-plan --write-selection"
cargo run -p sessiond -- --select-profile demo-x11 --print-launch-plan --write-selection

echo
echo "==> cargo run -p sessiond -- --launch-active --spawn-components"
cargo run -p sessiond -- --launch-active --spawn-components

echo
echo "==> cargo run -p watchdog -- --profile-id demo-x11 --write-reports"
cargo run -p watchdog -- --profile-id demo-x11 --write-reports
