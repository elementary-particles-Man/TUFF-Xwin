#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin lockd durable-state smoke test.
# Verifies session-scoped persistence, restart continuity, malformed-client isolation,
# unauthorized mutation rejection, and fail-closed recovery from a corrupt state artifact.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-lockd-state-persistence}"
rm -rf "$WAYBROKER_RUNTIME_DIR"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

target_dir="$(
  cargo metadata --format-version 1 --no-deps --quiet |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])'
)/debug"

lockd_pid=""

cleanup() {
  if [[ -n "${lockd_pid:-}" ]]; then
    kill "$lockd_pid" 2>/dev/null || true
    wait "$lockd_pid" 2>/dev/null || true
  fi
  rm -f "$WAYBROKER_RUNTIME_DIR/lockd.sock"
}
trap cleanup EXIT

wait_for_socket() {
  local socket=$1
  local timeout=${2:-5}
  local count=0
  while [[ ! -S "$socket" ]]; do
    if [[ $count -ge $((timeout * 10)) ]]; then
      echo "FAILED: timeout waiting for socket $socket" >&2
      return 1
    fi
    sleep 0.1
    count=$((count + 1))
  done
}

wait_for_file() {
  local path=$1
  local timeout=${2:-5}
  local count=0
  while [[ ! -f "$path" ]]; do
    if [[ $count -ge $((timeout * 10)) ]]; then
      echo "FAILED: timeout waiting for file $path" >&2
      return 1
    fi
    sleep 0.1
    count=$((count + 1))
  done
}

start_lockd() {
  local session_instance_id=$1
  local log_path=$2
  shift 2

  cleanup
  "$target_dir/lockd" \
    --serve-ipc \
    --session-instance-id "$session_instance_id" \
    "$@" \
    > "$log_path" 2>&1 &
  lockd_pid=$!
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/lockd.sock" 5
}

stop_lockd() {
  if [[ -n "${lockd_pid:-}" ]]; then
    kill "$lockd_pid" 2>/dev/null || true
    wait "$lockd_pid" 2>/dev/null || true
    lockd_pid=""
  fi
  rm -f "$WAYBROKER_RUNTIME_DIR/lockd.sock"
}

lockd_request() {
  local source=$1
  local payload_json=$2

  python3 - "$WAYBROKER_RUNTIME_DIR/lockd.sock" "$source" "$payload_json" <<'PY'
import json
import socket
import sys

socket_path, source, payload_json = sys.argv[1:4]
payload = json.loads(payload_json)
envelope = {
    "source": source,
    "destination": "lockd",
    "kind": {
        "kind": "lock-command",
        "payload": payload,
    },
}

with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
    client.settimeout(3.0)
    client.connect(socket_path)
    client.sendall(json.dumps(envelope, separators=(",", ":")).encode() + b"\n")
    data = b""
    while not data.endswith(b"\n"):
        chunk = client.recv(65536)
        if not chunk:
            raise SystemExit("lockd closed connection before response")
        data += chunk

decoded = json.loads(data)
print(json.dumps(decoded["kind"]["payload"], separators=(",", ":")))
PY
}

assert_lock_state() {
  local response=$1
  local expected_state=$2
  local expected_generation=$3
  local expected_session=$4

  python3 - "$response" "$expected_state" "$expected_generation" "$expected_session" <<'PY'
import json
import sys

payload = json.loads(sys.argv[1])
expected_state = sys.argv[2]
expected_generation = int(sys.argv[3])
expected_session = sys.argv[4]

assert payload["op"] == "state", payload
assert payload["state"] == expected_state, payload
assert payload["generation"] == expected_generation, payload
assert payload["session_instance_id"] == expected_session, payload
PY
}

assert_op() {
  local response=$1
  local expected_op=$2

  python3 - "$response" "$expected_op" <<'PY'
import json
import sys

payload = json.loads(sys.argv[1])
assert payload["op"] == sys.argv[2], payload
PY
}

echo "==> Pre-building lockd"
cargo build -p lockd

session_a="lockd-state-a"
session_b="lockd-state-b"
state_a="$WAYBROKER_RUNTIME_DIR/session-${session_a}-lockd-state.json"
state_b="$WAYBROKER_RUNTIME_DIR/session-${session_b}-lockd-state.json"

echo "==> Fresh session starts unlocked"
start_lockd "$session_a" "$WAYBROKER_RUNTIME_DIR/lockd-a-first.log"
wait_for_file "$state_a" 5
response="$(lockd_request sessiond '{"op":"get-state"}')"
assert_lock_state "$response" unlocked 1 "$session_a"

echo "==> Sessiond locks and state is durably committed"
response="$(lockd_request sessiond '{"op":"set-lock-state","state":"locked"}')"
assert_op "$response" set-lock-state
response="$(lockd_request watchdog '{"op":"get-state"}')"
assert_lock_state "$response" locked 2 "$session_a"

echo "==> Restart preserves locked state and generation"
stop_lockd
start_lockd "$session_a" "$WAYBROKER_RUNTIME_DIR/lockd-a-restart.log"
response="$(lockd_request sessiond '{"op":"get-state"}')"
assert_lock_state "$response" locked 2 "$session_a"

echo "==> Malformed client cannot terminate lockd"
python3 - "$WAYBROKER_RUNTIME_DIR/lockd.sock" <<'PY'
import socket
import sys

with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
    client.connect(sys.argv[1])
    client.sendall(b"{not-json}\n")
PY
response="$(lockd_request sessiond '{"op":"get-state"}')"
assert_lock_state "$response" locked 2 "$session_a"

echo "==> Unauthorized source cannot unlock"
response="$(lockd_request compd '{"op":"set-lock-state","state":"unlocked"}')"
assert_op "$response" rejected
response="$(lockd_request sessiond '{"op":"get-state"}')"
assert_lock_state "$response" locked 2 "$session_a"

echo "==> Separate session gets separate durable state"
stop_lockd
start_lockd "$session_b" "$WAYBROKER_RUNTIME_DIR/lockd-b.log"
wait_for_file "$state_b" 5
response="$(lockd_request sessiond '{"op":"get-state"}')"
assert_lock_state "$response" unlocked 1 "$session_b"

echo "==> Corrupt persisted state fails closed"
stop_lockd
printf '{not-json\n' > "$state_a"
start_lockd "$session_a" "$WAYBROKER_RUNTIME_DIR/lockd-a-corrupt.log"
response="$(lockd_request sessiond '{"op":"get-state"}')"
assert_lock_state "$response" blank-only 1 "$session_a"
grep -Fq 'service=lockd op=state_restore event=fail_closed' "$WAYBROKER_RUNTIME_DIR/lockd-a-corrupt.log"

echo "==> LOCKD STATE PERSISTENCE SMOKE TEST PASSED"
