#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

runtime_dir="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker}"
mkdir -p "$runtime_dir"

cleanup() {
  echo "Cleaning up..."
  pkill -f "displayd" || true
  pkill -f "compd" || true
  pkill -f "lockd" || true
}

trap cleanup EXIT

echo "Starting services for resume demo..."

# Start displayd
cargo run -p displayd &
sleep 1

# Start compd with IPC server
cargo run -p compd -- --serve-ipc &
sleep 1

# Start lockd with IPC server
cargo run -p lockd -- --serve-ipc &
sleep 1

echo "Starting resume sequence from sessiond..."
cargo run -p sessiond -- --resume-demo

echo "Resume demo FINISHED"
