#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/waybroker-waylandd-isolation.XXXXXX")"
export WAYBROKER_RUNTIME_DIR="$runtime_dir"
waylandd_pid=""
cleanup() {
  if [[ -n "$waylandd_pid" ]] && kill -0 "$waylandd_pid" 2>/dev/null; then kill "$waylandd_pid" 2>/dev/null || true; wait "$waylandd_pid" 2>/dev/null || true; fi
  rm -rf "$runtime_dir"
}
trap cleanup EXIT
target_dir="$(cargo metadata --format-version 1 --no-deps --quiet | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')/debug"
registry_source="$repo_root/examples/minimal-scene/surface-registry.json"
cargo build -p waylandd >/dev/null
"$target_dir/waylandd" --serve-ipc --no-vulkan --registry "$registry_source" >"$runtime_dir/waylandd.log" 2>&1 &
waylandd_pid=$!
for _ in $(seq 1 100); do [[ -S "$runtime_dir/waylandd.sock" ]] && break; sleep 0.05; done
[[ -S "$runtime_dir/waylandd.sock" ]] && kill -0 "$waylandd_pid"
python3 - "$runtime_dir/waylandd.sock" "$waylandd_pid" "$runtime_dir/session-default-single-session-surface-registry.json" <<'PY'
import json, os, socket, sys, time
path, pid, artifact = sys.argv[1:]
pid = int(pid)
nl = bytes([10])
def request(payload, destination="waylandd"):
    msg = {"source":"compd", "destination":destination, "kind":{"kind":"wayland-command", "payload":payload}}
    sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); sock.settimeout(8); sock.connect(path); sock.sendall(json.dumps(msg).encode()+nl); data=b""
    while nl not in data:
        chunk=sock.recv(65536)
        if not chunk: raise RuntimeError("closed")
        data += chunk
    sock.close(); return json.loads(data.split(nl,1)[0])
def registry():
    response=request({"op":"get-surface-registry"})
    if response["kind"]["payload"]["op"] != "surface-registry": raise RuntimeError(response)
    return response["kind"]["payload"]["snapshot"]
initial=registry()
if initial["generation"] == 0 or not initial["surfaces"]: raise RuntimeError(initial)
sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); sock.settimeout(2); sock.connect(path); sock.sendall(b"not-json"+nl); sock.close(); time.sleep(.05); os.kill(pid,0)
sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); sock.settimeout(2); sock.connect(path); sock.close(); time.sleep(.05); os.kill(pid,0)
sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); sock.settimeout(2); sock.connect(path); time.sleep(6)
if sock.recv(1): raise RuntimeError("idle response")
sock.close(); os.kill(pid,0)
rejected=request({"op":"get-surface-registry"},"displayd")
if rejected["kind"]["payload"]["op"] != "rejected": raise RuntimeError(rejected)
owner=next(surface["id"] for surface in initial["surfaces"] if surface["mapped"] and surface["buffer_attached"])
handoff={"op":"apply-selection-handoff","handoff":{"focus":{"type":"none"},"selection":{"clipboard_owner":owner,"clipboard_payload_id":"handoff-payload","clipboard_source_serial":99,"clipboard_offer":None,"primary_selection_owner":None,"primary_selection_payload_id":None,"primary_selection_source_serial":None,"primary_offer":None}}}
applied=request(handoff)
if applied["kind"]["payload"]["op"] != "selection-handoff-applied": raise RuntimeError(applied)
updated=registry()
if updated["generation"] <= initial["generation"] or updated["selection"]["clipboard_owner"] != owner: raise RuntimeError(updated)
if not os.path.exists(artifact) or json.load(open(artifact))["generation"] != updated["generation"]: raise RuntimeError("artifact")
for _ in range(20):
    if registry()["generation"] != updated["generation"]: raise RuntimeError("generation")
    os.kill(pid,0)
PY
kill "$waylandd_pid"; wait "$waylandd_pid" 2>/dev/null || true; waylandd_pid=""
"$target_dir/waylandd" --serve-ipc --once --no-vulkan --registry "$registry_source" >"$runtime_dir/waylandd-once.log" 2>&1 &
waylandd_pid=$!
for _ in $(seq 1 100); do [[ -S "$runtime_dir/waylandd.sock" ]] && break; sleep 0.05; done
python3 - "$runtime_dir/waylandd.sock" "$waylandd_pid" <<'PY'
import json, os, socket, sys, time
path, pid=sys.argv[1:]
pid=int(pid); nl=bytes([10])
sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); sock.settimeout(2); sock.connect(path); sock.sendall(b"not-json"+nl); sock.close(); time.sleep(.1); os.kill(pid,0)
msg={"source":"compd","destination":"waylandd","kind":{"kind":"wayland-command","payload":{"op":"get-surface-registry"}}}
sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); sock.settimeout(8); sock.connect(path); sock.sendall(json.dumps(msg).encode()+nl); data=b""
while nl not in data:
    chunk=sock.recv(65536)
    if not chunk: raise RuntimeError("once closed")
    data += chunk
sock.close(); time.sleep(.2)
if os.path.exists(f"/proc/{pid}"): raise RuntimeError("once did not terminate")
PY
wait "$waylandd_pid"; waylandd_pid=""
echo "WAYLANDD IPC ISOLATION SMOKE TEST PASSED"
