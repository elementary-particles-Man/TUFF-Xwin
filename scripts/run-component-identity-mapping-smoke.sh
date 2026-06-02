#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin Component Identity Mapping Smoke Test
# This script verifies that recovery targets are resolved via explicit bindings
# defined in the profile, rather than ambiguous role inference.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-identity-smoke}"
rm -rf "$WAYBROKER_RUNTIME_DIR"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

echo "==> Running Component Identity Mapping Smoke Test"
echo "==> Runtime directory: $WAYBROKER_RUNTIME_DIR"

target_dir="/home/flux/.cache/tuff-xwin-target/debug"

cleanup() {
  echo "==> Cleaning up..."
  pkill -P $$ || true
  sleep 0.5
  rm -f "$WAYBROKER_RUNTIME_DIR"/*.sock
}

trap cleanup EXIT

wait_for_socket() {
  local socket=$1
  local timeout=5
  local count=0
  echo "Waiting for socket $socket..."
  while [[ ! -S "$socket" ]]; do
    if [[ $count -ge $((timeout * 10)) ]]; then
      echo "Error: Timeout waiting for socket $socket" >&2
      return 1
    fi
    sleep 0.1
    count=$((count + 1))
  done
  echo "Socket $socket found."
  sleep 0.1
}

# Pre-build
echo "==> Pre-building all packages..."
cargo build --workspace

# Start minimal stack
"$target_dir/displayd" > "$WAYBROKER_RUNTIME_DIR/displayd.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/displayd.sock"

"$target_dir/watchdog" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/watchdog.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/watchdog.sock"

"$target_dir/lockd" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/lockd.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/lockd.sock"

# Select demo-x11 (which now has explicit bindings)
"$target_dir/sessiond" --select-profile demo-x11 --write-selection > /dev/null

# Start sessiond in manage-active mode
"$target_dir/sessiond" --serve-ipc --manage-active --spawn-components --notify-watchdog > "$WAYBROKER_RUNTIME_DIR/sessiond-managed.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/sessiond.sock"

# Inject faulty compd
pkill -f "compd" || true
sleep 0.5
rm -f "$WAYBROKER_RUNTIME_DIR/compd.sock"
"$target_dir/compd" --serve-ipc --fail-resume > "$WAYBROKER_RUNTIME_DIR/compd-faulty.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/compd.sock"

# Trigger resume failure
echo "==> Triggering compd resume failure..."
"$target_dir/sessiond" --resume-scenario "compd-trouble" > "$WAYBROKER_RUNTIME_DIR/resume-trigger.log" 2>&1

# Wait for recovery execution
echo "==> Waiting for recovery execution..."
timeout=10
count=0
execution_artifact="$WAYBROKER_RUNTIME_DIR/watchdog-action-execution-compd.json"
while [[ ! -f "$execution_artifact" ]]; do
  if [[ $count -ge $((timeout * 10)) ]]; then
    echo "FAILED: Recovery execution artifact not found"
    cat "$WAYBROKER_RUNTIME_DIR/sessiond-managed.log"
    exit 1
  fi
  sleep 0.1
  count=$((count + 1))
done

# Verify explicit resolution
echo "==> Verifying explicit identity mapping..."
grep '"resolution_source": "explicit"' "$execution_artifact" > /dev/null
grep '"bound_component_id": "demo-wm"' "$execution_artifact" > /dev/null
grep '"result": "succeeded"' "$execution_artifact" > /dev/null

echo "Artifact verification: PASSED"
echo "Evidence: $(grep "bound_component_id" "$execution_artifact")"

echo "==> COMPONENT IDENTITY MAPPING SMOKE TEST PASSED"
