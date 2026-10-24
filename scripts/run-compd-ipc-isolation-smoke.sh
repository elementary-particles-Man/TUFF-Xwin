#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/waybroker-compd-isolation.XXXXXX")"
export WAYBROKER_RUNTIME_DIR="$runtime_dir"
displayd_pid=""
compd_pid=""
cleanup() {
  for pid in "$compd_pid" "$displayd_pid"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
  rm -rf "$runtime_dir"
}
trap cleanup EXIT
target_dir="$(cargo metadata --format-version 1 --no-deps --quiet | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')/debug"
cargo build -p displayd -p compd >/dev/null
start_displayd() {
  TUFF_XWIN_DISPLAY_BACKEND=mock "$target_dir/displayd" --no-vulkan >"$runtime_dir/displayd.log" 2>&1 &
  displayd_pid=$!
  for _ in $(seq 1 100); do [[ -S "$runtime_dir/displayd.sock" ]] && break; sleep 0.05; done
  [[ -S "$runtime_dir/displayd.sock" ]] && kill -0 "$displayd_pid"
}
start_compd() {
  "$target_dir/compd" --serve-ipc --no-vulkan >"$runtime_dir/compd.log" 2>&1 &
  compd_pid=$!
  for _ in $(seq 1 100); do [[ -S "$runtime_dir/compd.sock" ]] && break; sleep 0.05; done
  [[ -S "$runtime_dir/compd.sock" ]] && kill -0 "$compd_pid"
}
start_displayd
start_compd
python3 - "$runtime_dir/compd.sock" "$compd_pid" <<'PY'
import json, os, socket, sys, time
path, pid = sys.argv[1:]
pid = int(pid)
def request(payload, source="sessiond", destination="compd"):
    message = {"source": source, "destination": destination, "kind": {"kind": "display-command", "payload": payload}}
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); sock.settimeout(8); sock.connect(path)
    sock.sendall(json.dumps(message).encode() + b"\n"); data = b""
    while b"\n" not in data:
        chunk = sock.recv(65536)
        if not chunk: raise RuntimeError("compd closed connection before response")
        data += chunk
    sock.close(); return json.loads(data.split(b"\n", 1)[0])
def assert_liveness(response):
    if response["kind"]["kind"] != "display-event" or response["kind"]["payload"]["op"] != "liveness": raise RuntimeError(response)
assert_liveness(request({"op": "get-liveness"}))
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); sock.settimeout(2); sock.connect(path); sock.sendall(b"not-json\n"); sock.close(); time.sleep(.05); os.kill(pid, 0)
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); sock.settimeout(2); sock.connect(path); sock.close(); time.sleep(.05); os.kill(pid, 0)
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); sock.settimeout(2); sock.connect(path); time.sleep(6)
if sock.recv(1): raise RuntimeError("idle client unexpectedly received a response")
sock.close(); os.kill(pid, 0)
lock = {"source": "lockd", "destination": "displayd", "kind": {"kind": "lock-command", "payload": {"op": "set-lock-state", "state": "locked"}}}
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); sock.settimeout(5); sock.connect(path); sock.sendall(json.dumps(lock).encode() + b"\n"); data = sock.recv(65536); sock.close()
response = json.loads(data.split(b"\n", 1)[0])
if response["kind"]["kind"] != "session-command" or response["kind"]["payload"]["op"] != "degraded-mode": raise RuntimeError(response)
assert_liveness(request({"op": "get-liveness"}))
PY
kill "$displayd_pid"; wait "$displayd_pid" 2>/dev/null || true; displayd_pid=""
python3 - "$runtime_dir/compd.sock" "$compd_pid" <<'PY'
import json, os, socket, sys
path, pid = sys.argv[1:]
message = {"source": "sessiond", "destination": "compd", "kind": {"kind": "display-command", "payload": {"op": "get-liveness"}}}
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); sock.settimeout(8); sock.connect(path); sock.sendall(json.dumps(message).encode() + b"\n"); data = b""
while b"\n" not in data:
    chunk = sock.recv(65536)
    if not chunk: raise RuntimeError("compd closed connection")
    data += chunk
sock.close(); response = json.loads(data.split(b"\n", 1)[0])
if response["kind"]["kind"] != "display-event" or response["kind"]["payload"]["op"] != "rejected": raise RuntimeError(response)
os.kill(int(pid), 0)
PY
start_displayd
python3 - "$runtime_dir/compd.sock" "$compd_pid" <<'PY'
import json, os, socket, sys
path, pid = sys.argv[1:]
message = {"source": "sessiond", "destination": "compd", "kind": {"kind": "display-command", "payload": {"op": "get-liveness"}}}
for _ in range(20):
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); sock.settimeout(8); sock.connect(path); sock.sendall(json.dumps(message).encode() + b"\n"); data = b""
    while b"\n" not in data:
        chunk = sock.recv(65536)
        if not chunk: raise RuntimeError("compd closed connection")
        data += chunk
    sock.close(); response = json.loads(data.split(b"\n", 1)[0])
    if response["kind"]["kind"] != "display-event" or response["kind"]["payload"]["op"] != "liveness": raise RuntimeError(response)
    os.kill(int(pid), 0)
PY
echo "COMPD IPC ISOLATION SMOKE TEST PASSED"
