#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin Role-Scoped Recovery Execution Smoke Test
# This script verifies that a restart-request accepted by watchdog is 
# actually executed by a manage-active sessiond supervisor.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-recovery-execution}"
rm -rf "$WAYBROKER_RUNTIME_DIR"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

echo "==> Running Role-Scoped Recovery Execution Smoke Test"
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

# Start services
# Note: we need displayd for waylandd/compd to not crash immediately
"$target_dir/displayd" > "$WAYBROKER_RUNTIME_DIR/displayd.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/displayd.sock"

"$target_dir/watchdog" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/watchdog.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/watchdog.sock"

"$target_dir/lockd" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/lockd.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/lockd.sock"

# We need a profile selection for manage-active sessiond
"$target_dir/sessiond" --select-profile demo-x11 --write-selection > /dev/null

# Start sessiond in manage-active mode
"$target_dir/sessiond" --serve-ipc --manage-active --spawn-components --notify-watchdog > "$WAYBROKER_RUNTIME_DIR/sessiond-managed.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/sessiond.sock"

# Inject faulty compd. 
# sessiond already spawned one, let's kill it and start our own faulty one.
# We need to make sure the faulty one uses the same socket path.
pkill -f "compd" || true
sleep 0.5
rm -f "$WAYBROKER_RUNTIME_DIR/compd.sock"

"$target_dir/compd" --serve-ipc --fail-resume > "$WAYBROKER_RUNTIME_DIR/compd-faulty.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/compd.sock"

# Run a second sessiond just to trigger the resume scenario against the running stack
echo "==> Executing resume scenario: compd-trouble"
"$target_dir/sessiond" --resume-scenario "compd-trouble" > "$WAYBROKER_RUNTIME_DIR/resume-trigger.log" 2>&1

# Wait for supervisor to detect and execute recovery
echo "==> Waiting for recovery execution..."
timeout=10
count=0
execution_artifact="$WAYBROKER_RUNTIME_DIR/watchdog-action-execution-compd.json"
while [[ ! -f "$execution_artifact" ]]; do
  if [[ $count -ge $((timeout * 10)) ]]; then
    echo "FAILED: Recovery execution artifact not found after timeout"
    echo "--- sessiond log ---"
    cat "$WAYBROKER_RUNTIME_DIR/sessiond-managed.log"
    exit 1
  fi
  sleep 0.1
  count=$((count + 1))
done

echo "Recovery execution artifact found."
grep '"result": "succeeded"' "$execution_artifact" > /dev/null
echo "Recovery artifact content verified (result succeeded)."

# Verify that sessiond log shows the execution
grep "op=recovery_execution event=finished role=compd result=succeeded" "$WAYBROKER_RUNTIME_DIR/sessiond-managed.log" > /dev/null
echo "Sessiond structured log verified."

echo "==> ROLE-SCOPED RECOVERY EXECUTION SMOKE TEST PASSED"
