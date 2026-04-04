#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin Multi-Session Recovery Isolation Smoke Test
# Verifies that recovery requests are strictly scoped by session_instance_id.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-multi-session-smoke}"
rm -rf "$WAYBROKER_RUNTIME_DIR"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

echo "==> Running Multi-Session Recovery Isolation Smoke Test"
echo "==> Runtime directory: $WAYBROKER_RUNTIME_DIR"

target_dir="/home/flux/.cache/tuff-xwin-target/debug"

# 1. Start Watchdog in IPC mode
# Watchdog handles multiple sessions by caching their states separately.
echo "==> Starting Watchdog (IPC server mode)..."
"$target_dir/watchdog" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/watchdog.log" 2>&1 &
watchdog_pid=$!

# Helper to wait for a socket
wait_for_socket() {
  local socket=$1
  while [[ ! -S "$socket" ]]; do sleep 0.1; done
}

wait_for_socket "$WAYBROKER_RUNTIME_DIR/watchdog.sock"

# 2. Simulate Session Alpha (id: alpha) and Session Beta (id: beta)
# We send a WatchdogCommand::Restart for 'alpha' only.
echo "==> Sending recovery request for session-alpha..."
python3 -c '
import socket, json
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect("'"$WAYBROKER_RUNTIME_DIR"'/watchdog.sock")
msg = {
    "source": "sessiond",
    "destination": "watchdog",
    "kind": {
        "kind": "watchdog-command",
        "payload": {
            "op": "restart",
            "role": "compd",
            "session_instance_id": "alpha",
            "reason": "simulated failure for alpha"
        }
    }
}
s.sendall(json.dumps(msg).encode() + b"\n")
# Wait for response to avoid breaking pipe on watchdog side
s.recv(1024)
s.close()
'

sleep 0.5

# 3. Verify Artifacts
artifact_alpha="$WAYBROKER_RUNTIME_DIR/session-alpha-watchdog-recovery-compd.json"
artifact_beta="$WAYBROKER_RUNTIME_DIR/session-beta-watchdog-recovery-compd.json"

echo "==> Checking for artifacts..."

if [[ -f "$artifact_alpha" ]]; then
  echo "PASS: Artifact for session-alpha found: $artifact_alpha"
  cat "$artifact_alpha"
else
  echo "FAIL: Artifact for session-alpha MISSING"
  exit 1
fi

if [[ ! -f "$artifact_beta" ]]; then
  echo "PASS: Artifact for session-beta NOT found (Isolation verified)"
else
  echo "FAIL: Artifact for session-beta found! (Isolation BREACHED)"
  exit 1
fi

# 4. Verify Path Safety with evil ID
echo "==> Testing path safety with unsafe ID..."
python3 -c '
import socket, json
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect("'"$WAYBROKER_RUNTIME_DIR"'/watchdog.sock")
msg = {
    "source": "sessiond",
    "destination": "watchdog",
    "kind": {
        "kind": "watchdog-command",
        "payload": {
            "op": "restart",
            "role": "compd",
            "session_instance_id": "../evil",
            "reason": "injection attempt"
        }
    }
}
s.sendall(json.dumps(msg).encode() + b"\n")
# Wait for response to avoid breaking pipe on watchdog side
s.recv(1024)
s.close()
'

sleep 0.5

# Expect the ID to be sanitized to ".._evil"
artifact_sanitized="$WAYBROKER_RUNTIME_DIR/session-.._evil-watchdog-recovery-compd.json"

if [[ -f "$artifact_sanitized" ]]; then
  echo "PASS: Sanitized artifact found: $artifact_sanitized"
else
  echo "FAIL: Sanitized artifact MISSING. Expected path-safe file."
  exit 1
fi

# Verify no directory traversal occurred
if ls "$WAYBROKER_RUNTIME_DIR"/session-*-../evil-watchdog-recovery-compd.json > /dev/null 2>&1; then
  echo "FAIL: Directory traversal detected!"
  exit 1
fi

echo "==> MULTI-SESSION RECOVERY ISOLATION SMOKE TEST PASSED"

kill "$watchdog_pid" || true
