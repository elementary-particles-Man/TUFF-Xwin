#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin Integration Smoke Test
# This script verifies the minimal interaction between all Waybroker services.
# It is designed to be fast, deterministic, and suitable for CI.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-integration-smoke}"
rm -rf "$WAYBROKER_RUNTIME_DIR"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

echo "==> Running Waybroker Integration Smoke Test"
echo "==> Runtime directory: $WAYBROKER_RUNTIME_DIR"

cleanup() {
  echo "==> Cleaning up background processes..."
  pkill -P $$ || true
}

trap cleanup EXIT

# Helper to wait for a socket
wait_for_socket() {
  local socket=$1
  local timeout=5
  local count=0
  echo "Waiting for socket $socket..."
  while [[ ! -S "$socket" ]]; do
    if [[ $count -ge $((timeout * 10)) ]]; then
      echo "Error: Timeout waiting for socket $socket" >&2
      ls -la "$(dirname "$socket")"
      return 1
    fi
    sleep 0.1
    ((count++))
  done
  echo "Socket $socket found."
  sleep 0.1
}

echo "==> Pre-building all packages..."
cargo build --workspace

target_dir="/home/flux/.cache/tuff-xwin-target/debug"

# 1. Hardware Broker Baseline (displayd)
echo "==> [1/6] Testing displayd output enumeration"
"$target_dir/displayd" > "$WAYBROKER_RUNTIME_DIR/displayd-init.log" 2>&1 &
displayd_pid=$!
if ! wait_for_socket "$WAYBROKER_RUNTIME_DIR/displayd.sock"; then
  echo "displayd failed to start. Log:"
  cat "$WAYBROKER_RUNTIME_DIR/displayd-init.log"
  exit 1
fi
"$target_dir/waylandd" --require-displayd

# 2. Composition Policy (compd)
echo "==> [2/6] Testing compd scene commit to displayd"
"$target_dir/compd" --commit-demo --require-displayd
kill "$displayd_pid" || true
wait "$displayd_pid" 2>/dev/null || true
rm -f "$WAYBROKER_RUNTIME_DIR/displayd.sock"
sleep 0.5

# 3. Security & Auth (lockd)
echo "==> [3/6] Testing lockd state transition"
"$target_dir/lockd" --serve-ipc --once > "$WAYBROKER_RUNTIME_DIR/lockd-smoke.log" 2>&1 &
lockd_pid=$!
if ! wait_for_socket "$WAYBROKER_RUNTIME_DIR/lockd.sock"; then
  echo "lockd failed to start. Log:"
  cat "$WAYBROKER_RUNTIME_DIR/lockd-smoke.log"
  exit 1
fi
# Use python to send a raw IPC message
python3 -c 'import socket; s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect("'"$WAYBROKER_RUNTIME_DIR"'/lockd.sock"); s.sendall(b"{\"source\":\"sessiond\",\"destination\":\"lockd\",\"kind\":{\"kind\":\"lock-command\",\"payload\":{\"op\":\"set-lock-state\",\"state\":\"locked\"}}}\n")'

# 4. Session & Profile Management (sessiond)
echo "==> [4/6] Testing sessiond profile loading and selection"
"$target_dir/sessiond" --list-profiles > /dev/null
"$target_dir/sessiond" --select-profile demo-x11 --write-selection --launch-active

# 5. Recovery & Health (watchdog)
echo "==> [5/6] Testing watchdog report generation"
"$target_dir/watchdog" --profile-id demo-x11 --write-reports
if [[ ! -f "$WAYBROKER_RUNTIME_DIR/watchdog-report-demo-x11.json" ]]; then
  echo "Error: Watchdog report not generated" >&2
  exit 1
fi

# 6. Full Orchestration (Resume Sequence)
echo "==> [6/6] Testing resume orchestration (displayd <-> compd <-> lockd <-> sessiond)"
# Final thorough cleanup before orchestration
pkill -P $$ || true
sleep 0.5
rm -f "$WAYBROKER_RUNTIME_DIR"/*.sock

# Start long-running stubs
"$target_dir/displayd" > "$WAYBROKER_RUNTIME_DIR/displayd-resume.log" 2>&1 &
if ! wait_for_socket "$WAYBROKER_RUNTIME_DIR/displayd.sock"; then
  echo "displayd failed to start for orchestration. Log:"
  cat "$WAYBROKER_RUNTIME_DIR/displayd-resume.log"
  exit 1
fi

"$target_dir/compd" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/compd-resume.log" 2>&1 &
if ! wait_for_socket "$WAYBROKER_RUNTIME_DIR/compd.sock"; then
  echo "compd failed to start for orchestration. Log:"
  cat "$WAYBROKER_RUNTIME_DIR/compd-resume.log"
  exit 1
fi

"$target_dir/lockd" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/lockd-resume.log" 2>&1 &
if ! wait_for_socket "$WAYBROKER_RUNTIME_DIR/lockd.sock"; then
  echo "lockd failed to start for orchestration. Log:"
  cat "$WAYBROKER_RUNTIME_DIR/lockd-resume.log"
  exit 1
fi

# Run resume demo
"$target_dir/sessiond" --resume-demo

echo "==> INTEGRATION SMOKE TEST PASSED"
