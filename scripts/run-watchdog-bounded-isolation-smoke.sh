#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/waybroker-watchdog-isolation.XXXXXX")"
export WAYBROKER_RUNTIME_DIR="$runtime_dir"
watchdog_pid=""

cleanup() {
  if [[ -n "$watchdog_pid" ]] && kill -0 "$watchdog_pid" 2>/dev/null; then
    kill "$watchdog_pid" 2>/dev/null || true
    wait "$watchdog_pid" 2>/dev/null || true
  fi
  rm -rf "$runtime_dir"
}
trap cleanup EXIT

target_dir="$(cargo metadata --format-version 1 --no-deps --quiet | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')/debug"
cargo build -p watchdog >/dev/null

echo "==> Starting watchdog bounded isolation smoke"
"$target_dir/watchdog" --serve-ipc >"$runtime_dir/watchdog.log" 2>&1 &
watchdog_pid=$!

for _ in $(seq 1 100); do
  [[ -S "$runtime_dir/watchdog.sock" ]] && break
  sleep 0.05
done
[[ -S "$runtime_dir/watchdog.sock" ]]

request() {
  python3 - "$runtime_dir/watchdog.sock" "$1" "$2" "$3" <<'PY'
import json
import socket
import sys

sock_path, source, destination, payload = sys.argv[1:]
message = {
    "source": source,
    "destination": destination,
    "kind": {"kind": "watchdog-command", "payload": json.loads(payload)},
}
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.settimeout(3)
sock.connect(sock_path)
sock.sendall(json.dumps(message).encode() + b"\n")
data = b""
while b"\n" not in data:
    chunk = sock.recv(65536)
    if not chunk:
        raise SystemExit("watchdog closed connection before response")
    data += chunk
sock.close()
response = json.loads(data.split(b"\n", 1)[0])
print(response["kind"]["payload"]["op"])
if response["kind"]["payload"]["op"] == "escalate":
    print(response["kind"]["payload"].get("reason", ""))
PY
}

restart_payload() {
  python3 - "$1" "$2" "$3" <<'PY'
import json
import sys
print(json.dumps({
    "op": "restart",
    "role": sys.argv[1],
    "session_instance_id": sys.argv[2],
    "reason": sys.argv[3],
}))
PY
}

state_payload() {
  python3 - "$1" "$2" "$3" "$4" "$5" <<'PY'
import json
import sys

session, generation, sequence, replace, display = sys.argv[1:]
generation = int(generation)
sequence = int(sequence)
replace = replace == "true"
print(json.dumps({
    "op": "inspect-launch-state" if not replace else "update-launch-state",
    "state": {
        "profile_id": "demo",
        "session_instance_id": session,
        "display_name": display,
        "protocol": "layer-x11",
        "broker_services": ["sessiond", "watchdog"],
        "generation": generation,
        "sequence": sequence,
        "components": [{
            "id": "wm",
            "role": "window-manager",
            "critical": True,
            "command": ["wm"],
            "resolved_command": "/usr/bin/wm",
            "state": "spawned",
            "pid": 1,
            "restart_count": 0,
            "last_exit_status": None,
        }],
        "unix_timestamp": 1,
        "service_component_bindings": [],
        "service_recovery_execution_policies": [],
    }
}))
PY
}

delta_payload() {
  python3 - "$1" "$2" "$3" "$4" "$5" <<'PY'
import json
import sys

session, generation, sequence, replace, display = sys.argv[1:]
print(json.dumps({
    "op": "update-launch-state",
    "delta": {
        "profile_id": "demo",
        "session_instance_id": session,
        "display_name": display,
        "protocol": "layer-x11",
        "broker_services": ["sessiond", "watchdog"],
        "generation": int(generation),
        "sequence": int(sequence),
        "replace": replace == "true",
        "components": [{
            "id": "wm",
            "role": "window-manager",
            "critical": True,
            "command": ["wm"],
            "resolved_command": "/usr/bin/wm",
            "state": "failed",
            "pid": None,
            "restart_count": 1,
            "last_exit_status": 1,
        }],
        "unix_timestamp": 2,
        "service_component_bindings": [],
        "service_recovery_execution_policies": [],
    }
}))
PY
}

expect_op() {
  local expected=$1
  shift
  local response
  response="$("$@")"
  [[ "${response%%$'\n'*}" == "$expected" ]] || {
    echo "unexpected response: $response" >&2
    exit 1
  }
}

echo "==> Valid restart request"
expect_op restart request sessiond watchdog "$(restart_payload compd alpha 'alpha failure')"
artifact_alpha="$runtime_dir/session-alpha-watchdog-recovery-compd.json"
[[ -f "$artifact_alpha" ]]
python3 - "$artifact_alpha" <<'PY'
import json
import sys
artifact = json.load(open(sys.argv[1]))
assert artifact["requested_by"] == "sessiond"
assert artifact["action"] == "restart-request-accepted"
assert artifact["status"] == "pending"
PY

echo "==> Malformed and interrupted clients"
python3 - "$runtime_dir/watchdog.sock" <<'PY'
import socket
import sys
for payload in (b"{not-json}\n", b'{"source":'):
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(sys.argv[1])
    sock.sendall(payload)
    sock.close()
PY
sleep 0.1
kill -0 "$watchdog_pid"
expect_op restart request sessiond watchdog "$(restart_payload lockd alpha 'after malformed client')"

echo "==> Authorization and session validation"
expect_op escalate request sessiond sessiond "$(restart_payload compd denied 'wrong destination')"
expect_op escalate request compd watchdog "$(restart_payload compd denied 'wrong source')"
expect_op escalate request sessiond watchdog "$(restart_payload compd ../evil 'invalid session')"
expect_op escalate request sessiond watchdog "$(restart_payload displayd denied 'unsupported role')"
[[ ! -e "$runtime_dir/session-../evil-watchdog-recovery-compd.json" ]]
[[ ! -e "$runtime_dir/session-.._evil-watchdog-recovery-compd.json" ]]

echo "==> Full state, duplicate, conflict, and delta ordering"
full_payload="$(state_payload alpha 1 1 false initial)"
expect_op inspection-result request sessiond watchdog "$full_payload"
expect_op inspection-result request sessiond watchdog "$full_payload"
conflict_payload="$(state_payload alpha 1 1 false conflicting)"
expect_op resync-launch-state request sessiond watchdog "$conflict_payload"
expect_op inspection-result request sessiond watchdog "$(delta_payload alpha 1 2 false delta)"
expect_op inspection-result request sessiond watchdog "$(delta_payload alpha 1 2 false duplicate)"
expect_op resync-launch-state request sessiond watchdog "$(delta_payload alpha 1 4 false gap)"
expect_op resync-launch-state request sessiond watchdog "$(delta_payload alpha 2 2 false jump)"
expect_op inspection-result request sessiond watchdog "$(delta_payload alpha 2 1 true replace)"

echo "==> Restart and resync isolation"
kill "$watchdog_pid"
wait "$watchdog_pid" 2>/dev/null || true
watchdog_pid=""
"$target_dir/watchdog" --serve-ipc >"$runtime_dir/watchdog-restarted.log" 2>&1 &
watchdog_pid=$!
for _ in $(seq 1 100); do
  [[ -S "$runtime_dir/watchdog.sock" ]] && break
  sleep 0.05
done
expect_op resync-launch-state request sessiond watchdog "$(delta_payload alpha 2 2 false 'after restart')"
expect_op inspection-result request sessiond watchdog "$(state_payload alpha 2 1 false restored)"
expect_op inspection-result request sessiond watchdog "$(delta_payload alpha 2 2 false resumed)"

echo "==> Atomic artifact failure leaves no temporary artifact"
failure_target="$runtime_dir/session-atomic-failure-watchdog-recovery-compd.json"
mkdir "$failure_target"
if request sessiond watchdog "$(restart_payload compd atomic-failure 'atomic failure target')" >/dev/null 2>&1; then
  echo "atomic artifact failure unexpectedly returned a response" >&2
  exit 1
fi
[[ -d "$failure_target" ]]
! find "$runtime_dir" -maxdepth 1 -name '.session-atomic-failure-watchdog-recovery-compd.json.tmp-*' -print -quit | grep -q .

echo "==> WATCHDOG BOUNDED ISOLATION SMOKE TEST PASSED"
