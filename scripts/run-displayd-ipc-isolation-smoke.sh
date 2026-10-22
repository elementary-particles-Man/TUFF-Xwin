#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/waybroker-displayd-isolation.XXXXXX")"
export WAYBROKER_RUNTIME_DIR="$runtime_dir"
displayd_pid=""

cleanup() {
  if [[ -n "$displayd_pid" ]] && kill -0 "$displayd_pid" 2>/dev/null; then
    kill "$displayd_pid" 2>/dev/null || true
    wait "$displayd_pid" 2>/dev/null || true
  fi
  rm -rf "$runtime_dir"
}
trap cleanup EXIT

target_dir="$(cargo metadata --format-version 1 --no-deps --quiet | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')/debug"
cargo build -p displayd >/dev/null

TUFF_XWIN_DISPLAY_BACKEND=mock "$target_dir/displayd" --no-vulkan >"$runtime_dir/displayd.log" 2>&1 &
displayd_pid=$!
for _ in $(seq 1 100); do
  [[ -S "$runtime_dir/displayd.sock" ]] && break
  sleep 0.05
done
[[ -S "$runtime_dir/displayd.sock" ]]
kill -0 "$displayd_pid"

python3 - "$runtime_dir/displayd.sock" "$displayd_pid" <<'PY'
import json
import os
import socket
import sys
import time

path, pid = sys.argv[1:]
pid = int(pid)

def request(payload, destination="displayd"):
    message = {
        "source": "sessiond",
        "destination": destination,
        "kind": {"kind": "display-command", "payload": payload},
    }
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(5)
    sock.connect(path)
    sock.sendall(json.dumps(message).encode() + b"\n")
    data = b""
    while b"\n" not in data:
        chunk = sock.recv(65536)
        if not chunk:
            raise RuntimeError("displayd closed connection before response")
        data += chunk
    sock.close()
    return json.loads(data.split(b"\n", 1)[0])

def assert_op(response, expected):
    actual = response["kind"]["payload"]["op"]
    if actual != expected:
        raise RuntimeError(f"expected {expected}, got {actual}: {response}")

assert_op(request({"op": "get-liveness"}), "liveness")

sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.settimeout(2)
sock.connect(path)
sock.sendall(b"not-json\n")
sock.close()
time.sleep(0.05)
os.kill(pid, 0)

sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.settimeout(2)
sock.connect(path)
sock.close()
time.sleep(0.05)
os.kill(pid, 0)

assert_op(request({"op": "get-liveness"}, "sessiond"), "rejected")
assert_op(request({"op": "get-readiness"}), "readiness")
assert_op(request({"op": "get-output-topology"}), "output-topology")
for _ in range(20):
    assert_op(request({"op": "get-liveness"}), "liveness")
    os.kill(pid, 0)
PY

echo "DISPLAYD IPC ISOLATION SMOKE TEST PASSED"
