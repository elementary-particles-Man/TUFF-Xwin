#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin Watchdog Auto-Recovery Wiring Smoke Test
# This script verifies that a restart-request from sessiond is correctly 
# accepted and logged by watchdog during a resume failure.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-watchdog-recovery}"
rm -rf "$WAYBROKER_RUNTIME_DIR"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

echo "==> Running Watchdog Auto-Recovery Wiring Smoke Test"
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
"$target_dir/displayd" > "$WAYBROKER_RUNTIME_DIR/displayd.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/displayd.sock"

"$target_dir/compd" --serve-ipc --fail-resume > "$WAYBROKER_RUNTIME_DIR/compd.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/compd.sock"

"$target_dir/lockd" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/lockd.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/lockd.sock"

"$target_dir/watchdog" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/watchdog.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/watchdog.sock"

# Run sessiond scenario
echo "==> Executing resume scenario: compd-trouble"
"$target_dir/sessiond" --resume-scenario "compd-trouble" > "$WAYBROKER_RUNTIME_DIR/sessiond.log" 2>&1

# Verify artifacts
echo "==> Verifying artifacts..."

# 1. Watchdog recovery artifact
recovery_artifact="$WAYBROKER_RUNTIME_DIR/watchdog-recovery-compd.json"
if [[ ! -f "$recovery_artifact" ]]; then
  echo "FAILED: Watchdog recovery artifact not found at $recovery_artifact"
  exit 1
fi
echo "Watchdog recovery artifact found."
grep '"action": "restart-request-accepted"' "$recovery_artifact" > /dev/null
echo "Watchdog artifact content verified (action accepted)."

# 2. Sessiond resume trace
resume_trace="$WAYBROKER_RUNTIME_DIR/resume-trace-compd-trouble.json"
if [[ ! -f "$resume_trace" ]]; then
  echo "FAILED: Resume trace not found at $resume_trace"
  exit 1
fi
echo "Resume trace found."
grep '"name": "watchdog_restart_request"' "$resume_trace" > /dev/null
grep '"outcome": "accepted"' "$resume_trace" > /dev/null
echo "Resume trace content verified (watchdog accepted step found)."

echo "==> WATCHDOG AUTO-RECOVERY WIRING SMOKE TEST PASSED"
