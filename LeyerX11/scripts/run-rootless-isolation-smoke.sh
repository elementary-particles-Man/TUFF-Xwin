#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/waybroker-x11bridge-isolation.XXXXXX")"
export WAYBROKER_RUNTIME_DIR="$runtime_dir" DISPLAY="" WAYLAND_DISPLAY=""
displayd_pid=""; stub_pid=""
cleanup() {
  for pid in "$stub_pid" "$displayd_pid"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
  rm -rf "$runtime_dir"
}
trap cleanup EXIT
target_dir="$(cargo metadata --format-version 1 --no-deps --quiet | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug"
scene="$repo_root/LeyerX11/examples/minimal-rootless-scene.json"
cargo build -p displayd -p x11bridge >/dev/null
TUFF_XWIN_DISPLAY_BACKEND=mock "$target_dir/displayd" --no-vulkan >"$runtime_dir/displayd.log" 2>&1 & displayd_pid=$!
for _ in $(seq 1 100); do [[ -S "$runtime_dir/displayd.sock" ]] && break; sleep .05; done
[[ -S "$runtime_dir/displayd.sock" ]] && kill -0 "$displayd_pid"
timeout 30s cargo run -p x11bridge -- --scene "$scene" --commit-demo >/tmp/x11bridge-valid.log
grep -q 'committed_surfaces=' /tmp/x11bridge-valid.log
bad_json="$runtime_dir/bad.json"; printf 'not-json\n' >"$bad_json"
if cargo run -p x11bridge -- --scene "$bad_json" --commit-demo >/dev/null 2>&1; then exit 1; fi
python3 - "$scene" "$runtime_dir/duplicate.json" <<'PY'
import json, sys
x=json.load(open(sys.argv[1])); x["windows"][1]["id"]=x["windows"][0]["id"]; json.dump(x,open(sys.argv[2],"w"))
PY
if cargo run -p x11bridge -- --scene "$runtime_dir/duplicate.json" --commit-demo >/dev/null 2>&1; then exit 1; fi
python3 - "$scene" "$runtime_dir/dangling.json" "$runtime_dir/overflow.json" <<'PY'
import json, sys
x=json.load(open(sys.argv[1])); x.setdefault("selection", {})["clipboard_owner"]="ghost"; json.dump(x,open(sys.argv[2],"w"))
y=json.load(open(sys.argv[1])); y["windows"][0]["x"]=2147483647; json.dump(y,open(sys.argv[3],"w"))
PY
for invalid in dangling overflow; do if cargo run -p x11bridge -- --scene "$runtime_dir/$invalid.json" --commit-demo >/dev/null 2>&1; then exit 1; fi; done
kill -0 "$displayd_pid"
python3 - "$runtime_dir/displayd.sock" <<'PY'
import json, socket, sys
p=sys.argv[1]; s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); s.settimeout(8); s.connect(p); m={"source":"sessiond","destination":"displayd","kind":{"kind":"display-command","payload":{"op":"get-liveness"}}}; s.sendall(json.dumps(m).encode()+bytes([10])); d=b""
while bytes([10]) not in d: d+=s.recv(65536)
if json.loads(d.split(bytes([10]),1)[0])["kind"]["payload"]["op"] != "liveness": raise SystemExit(1)
PY
kill "$displayd_pid"; wait "$displayd_pid" 2>/dev/null || true; displayd_pid=""
for _ in $(seq 1 100); do [[ ! -e "$runtime_dir/displayd.sock" ]] && break; sleep .05; done
rm -f "$runtime_dir/displayd.sock"
if cargo run -p x11bridge -- --scene "$scene" --commit-demo >/dev/null 2>&1; then exit 1; fi
python3 -c 'import socket,json,sys; s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); s.bind(sys.argv[1]); s.listen(1); c,_=s.accept(); c.recv(65536); r={"source":"displayd","destination":"sessiond","kind":{"kind":"display-event","payload":{"op":"rejected","reason":"stub"}}}; c.sendall(json.dumps(r).encode()+bytes([10])); c.close(); s.close()' "$runtime_dir/displayd.sock" & stub_pid=$!
if cargo run -p x11bridge -- --scene "$scene" --commit-demo >/dev/null 2>&1; then exit 1; fi
wait "$stub_pid"; stub_pid=""
echo "X11BRIDGE ROOTLESS ISOLATION SMOKE TEST PASSED"
