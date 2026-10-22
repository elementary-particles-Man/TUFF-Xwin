#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin sessiond authority/isolation smoke test.
# Verifies session identity, monotonic authority generations, source authorization,
# malformed-client isolation, cross-session watchdog rejection, and recovery-artifact authority.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-sessiond-authority-isolation}"
rm -rf "$WAYBROKER_RUNTIME_DIR"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

session_instance_id="sessiond-authority-a"
other_session_instance_id="sessiond-authority-b"

target_dir="$(
  cargo metadata --format-version 1 --no-deps --quiet |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])'
)/debug"

sessiond_pid=""

cleanup() {
  if [[ -n "${sessiond_pid:-}" ]]; then
    kill "$sessiond_pid" 2>/dev/null || true
    wait "$sessiond_pid" 2>/dev/null || true
    sessiond_pid=""
  fi
  rm -f "$WAYBROKER_RUNTIME_DIR/sessiond.sock"
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

wait_for_absence() {
  local path=$1
  local timeout=${2:-5}
  local count=0
  while [[ -e "$path" ]]; do
    if [[ $count -ge $((timeout * 10)) ]]; then
      echo "FAILED: timeout waiting for removal of $path" >&2
      return 1
    fi
    sleep 0.1
    count=$((count + 1))
  done
}

json_field() {
  local path=$1
  local field=$2
  python3 - "$path" "$field" <<'PY'
import json
import sys
with open(sys.argv[1], "r", encoding="utf-8") as handle:
    value = json.load(handle)
for key in sys.argv[2].split('.'):
    value = value[key]
print(value)
PY
}

sessiond_request() {
  local source=$1
  local payload_json=$2
  python3 - "$WAYBROKER_RUNTIME_DIR/sessiond.sock" "$source" "$payload_json" <<'PY'
import json
import socket
import sys
socket_path, source, payload_json = sys.argv[1:4]
envelope = {
    "source": source,
    "destination": "sessiond",
    "kind": {
        "kind": "session-command",
        "payload": json.loads(payload_json),
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
            raise SystemExit("sessiond closed connection before response")
        data += chunk
response = json.loads(data)
print(json.dumps(response["kind"]["payload"], separators=(",", ":")))
PY
}

assert_payload_op() {
  local response=$1
  local expected=$2
  python3 - "$response" "$expected" <<'PY'
import json
import sys
payload = json.loads(sys.argv[1])
assert payload["op"] == sys.argv[2], payload
PY
}

assert_reason_contains() {
  local response=$1
  local expected=$2
  python3 - "$response" "$expected" <<'PY'
import json
import sys
payload = json.loads(sys.argv[1])
assert sys.argv[2] in payload.get("reason", ""), payload
PY
}

start_managed_sessiond() {
  local log_path=$1
  cleanup
  "$target_dir/sessiond" \
    --serve-ipc \
    --manage-active \
    --session-instance-id "$session_instance_id" \
    > "$log_path" 2>&1 &
  sessiond_pid=$!
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/sessiond.sock" 5
}

stop_managed_sessiond() {
  cleanup
}

echo "==> Pre-building workspace"
cargo build --workspace

active_profile="$WAYBROKER_RUNTIME_DIR/session-${session_instance_id}-active-profile.json"
launch_state="$WAYBROKER_RUNTIME_DIR/session-${session_instance_id}-launch-state.json"
authority_state="$WAYBROKER_RUNTIME_DIR/session-${session_instance_id}-sessiond-state.json"

# Establish a deterministic session/profile artifact.
"$target_dir/sessiond" \
  --select-profile demo-x11 \
  --write-selection \
  --session-instance-id "$session_instance_id" \
  > "$WAYBROKER_RUNTIME_DIR/select.log" 2>&1
wait_for_file "$active_profile" 5

# A one-shot launch must use the requested session identity and create authority generation 1.
"$target_dir/sessiond" \
  --launch-active \
  --session-instance-id "$session_instance_id" \
  > "$WAYBROKER_RUNTIME_DIR/launch.log" 2>&1
wait_for_file "$launch_state" 5
wait_for_file "$authority_state" 5
[[ "$(json_field "$launch_state" session_instance_id)" == "$session_instance_id" ]]
[[ "$(json_field "$launch_state" generation)" == "1" ]]
[[ "$(json_field "$authority_state" stream_generation)" == "1" ]]

echo "==> Starting managed sessiond"
start_managed_sessiond "$WAYBROKER_RUNTIME_DIR/sessiond-first.log"
wait_for_file "$authority_state" 5
first_generation="$(json_field "$authority_state" stream_generation)"
[[ "$first_generation" -gt 1 ]]
[[ "$(json_field "$launch_state" generation)" == "$first_generation" ]]
[[ "$(json_field "$launch_state" session_instance_id)" == "$session_instance_id" ]]

# A malformed client must not kill the daemon.
python3 - "$WAYBROKER_RUNTIME_DIR/sessiond.sock" <<'PY'
import socket
import sys
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
    client.connect(sys.argv[1])
    client.sendall(b"{not-json}\n")
PY

response="$(sessiond_request waylandd '{"op":"inhibit-idle","reason":"authority-smoke"}')"
assert_payload_op "$response" inhibit-idle

# A non-watchdog source cannot inject a watchdog report.
profile_payload="$(python3 - "$active_profile" "$session_instance_id" <<'PY'
import json
import sys
profile_path, session_id = sys.argv[1:3]
with open(profile_path, "r", encoding="utf-8") as handle:
    profile = json.load(handle)
components = []
for component in profile.get("session_components", []):
    components.append({
        "id": component["id"],
        "role": component["role"],
        "critical": component["critical"],
        "status": "inactive",
        "pid": None,
        "crash_loop_count": 0,
        "action": "none",
        "reason": "authority smoke",
    })
report = {
    "profile_id": profile["id"],
    "session_instance_id": session_id,
    "display_name": profile["display_name"],
    "protocol": profile["protocol"],
    "healthy_components": 0,
    "unhealthy_components": 0,
    "inactive_components": len(components),
    "components": components,
    "unix_timestamp": 0,
}
print(json.dumps({"op":"apply-watchdog-report","report":report}, separators=(",", ":")))
PY
)"
response="$(sessiond_request compd "$profile_payload")"
assert_payload_op "$response" profile-unchanged
assert_reason_contains "$response" "not authorized"

# A legitimate watchdog source still cannot cross the managed-session boundary.
cross_session_payload="$(python3 - "$profile_payload" "$other_session_instance_id" <<'PY'
import json
import sys
payload = json.loads(sys.argv[1])
payload["report"]["session_instance_id"] = sys.argv[2]
print(json.dumps(payload, separators=(",", ":")))
PY
)"
response="$(sessiond_request watchdog "$cross_session_payload")"
assert_payload_op "$response" profile-unchanged
assert_reason_contains "$response" "does not match managed session"

# Recovery artifacts are accepted only when issued by sessiond through watchdog.
recovery_artifact="$WAYBROKER_RUNTIME_DIR/session-${session_instance_id}-watchdog-recovery-compd.json"
execution_artifact="$WAYBROKER_RUNTIME_DIR/session-${session_instance_id}-watchdog-action-execution-compd.json"
rm -f "$execution_artifact"
cat > "$recovery_artifact" <<EOF_JSON
{
  "role": "compd",
  "reason": "unauthorized recovery artifact",
  "requested_by": "compd",
  "unix_timestamp": 1,
  "action": "restart-request-accepted",
  "status": "pending"
}
EOF_JSON
wait_for_absence "$recovery_artifact" 5
[[ ! -e "$execution_artifact" ]]

# Restarting the authority for the same session must advance generation.
echo "==> Restarting managed sessiond"
stop_managed_sessiond
start_managed_sessiond "$WAYBROKER_RUNTIME_DIR/sessiond-second.log"
second_generation="$(json_field "$authority_state" stream_generation)"
[[ "$second_generation" -gt "$first_generation" ]]
[[ "$(json_field "$launch_state" generation)" == "$second_generation" ]]
[[ "$(json_field "$authority_state" stream_sequence)" == "1" ]]

# Corrupt durable authority must fail closed rather than restarting at generation 1.
stop_managed_sessiond
printf '{not-json\n' > "$authority_state"
set +e
timeout 3 "$target_dir/sessiond" \
  --serve-ipc \
  --manage-active \
  --session-instance-id "$session_instance_id" \
  > "$WAYBROKER_RUNTIME_DIR/sessiond-corrupt.log" 2>&1
corrupt_rc=$?
set -e
if [[ "$corrupt_rc" -eq 0 || "$corrupt_rc" -eq 124 ]]; then
  echo "FAILED: corrupt session authority did not fail closed (rc=$corrupt_rc)" >&2
  cat "$WAYBROKER_RUNTIME_DIR/sessiond-corrupt.log" >&2 || true
  exit 1
fi
grep -Fq 'failed to decode session authority state' "$WAYBROKER_RUNTIME_DIR/sessiond-corrupt.log"

echo "==> SESSIOND AUTHORITY AND ISOLATION SMOKE TEST PASSED"
