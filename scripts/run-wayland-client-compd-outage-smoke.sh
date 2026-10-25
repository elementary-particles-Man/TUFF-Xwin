#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/waybroker-wayland-client-recovery.XXXXXX")"
export WAYBROKER_RUNTIME_DIR="$runtime_dir"
export DISPLAY=""
export WAYLAND_DISPLAY=""

displayd_pid=""
compd_pid=""
waylandd_pid=""
client_pid=""
cleanup() {
  for pid in "$client_pid" "$waylandd_pid" "$compd_pid" "$displayd_pid"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  rm -rf "$runtime_dir"
}
trap cleanup EXIT

target_dir="$(cargo metadata --format-version 1 --no-deps --quiet | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')/debug"
wayland_socket="$runtime_dir/wayland-client-recovery"

wait_for_socket() {
  local path=$1
  for _ in $(seq 1 200); do
    [[ -S "$path" ]] && return 0
    sleep 0.05
  done
  echo "timeout waiting for socket: $path" >&2
  return 1
}

start_displayd() {
  TUFF_XWIN_DISPLAY_BACKEND=mock "$target_dir/displayd" --no-vulkan >"$runtime_dir/displayd.log" 2>&1 &
  displayd_pid=$!
  wait_for_socket "$runtime_dir/displayd.sock"
  kill -0 "$displayd_pid"
}

start_compd() {
  "$target_dir/compd" --serve-ipc --no-vulkan >"$runtime_dir/compd.log" 2>&1 &
  compd_pid=$!
  wait_for_socket "$runtime_dir/compd.sock"
  kill -0 "$compd_pid"
}

start_waylandd() {
  "$target_dir/waylandd" --serve-ipc --production --no-vulkan \
    --bind-wayland-display "$wayland_socket" \
    >"$runtime_dir/waylandd.log" 2>&1 &
  waylandd_pid=$!
  wait_for_socket "$wayland_socket"
  kill -0 "$waylandd_pid"
}

run_readiness_until_success() {
  local log_path=$1
  for _ in $(seq 1 40); do
    if timeout 3 "$target_dir/waylandd" --check-readiness "$wayland_socket" >>"$log_path" 2>&1; then
      return 0
    fi
    kill -0 "$waylandd_pid"
    sleep 0.25
  done
  echo "timed out waiting for native Wayland readiness" >&2
  echo "---- displayd.log ----" >&2
  tail -n 40 "$runtime_dir/displayd.log" >&2 || true
  echo "---- compd.log ----" >&2
  tail -n 40 "$runtime_dir/compd.log" >&2 || true
  echo "---- waylandd.log ----" >&2
  tail -n 80 "$runtime_dir/waylandd.log" >&2 || true
  return 1
}

cargo build -p displayd -p compd -p waylandd >/dev/null
start_displayd
start_compd
start_waylandd

echo "==> validating normal native Wayland scene and presentation"
python3 - "$wayland_socket" <<'PY'
import socket
import struct
import sys
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.settimeout(3.0)
sock.connect(sys.argv[1])
sock.sendall(struct.pack("<II", 8, 1) + struct.pack("<I", 2))
sock.sendall(struct.pack("<II", 8, 0) + struct.pack("<I", 3))
sock.close()
PY
displayd_pid_before=$displayd_pid
waylandd_pid_before=$waylandd_pid

python3 - "$wayland_socket" "$runtime_dir/client.ready" "$runtime_dir/client.resume" <<'PY' &
import os
import socket
import struct
import sys
import time

path, ready_path, resume_path = sys.argv[1:]
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.settimeout(2.0)
sock.connect(path)
sock.sendall(struct.pack("<II", 8, 1) + struct.pack("<I", 2))
sock.sendall(struct.pack("<II", 8, 0) + struct.pack("<I", 3))
open(ready_path, "w", encoding="utf-8").close()
while not os.path.exists(resume_path):
    time.sleep(0.05)
sock.sendall(struct.pack("<II", 8, 0) + struct.pack("<I", 3))
sock.close()
PY
client_pid=$!
for _ in $(seq 1 100); do [[ -f "$runtime_dir/client.ready" ]] && break; sleep 0.05; done
[[ -f "$runtime_dir/client.ready" ]]

echo "==> stopping only compd while the native client remains connected"
kill "$compd_pid"
wait "$compd_pid" 2>/dev/null || true
compd_pid=""
kill -0 "$waylandd_pid_before"
kill -0 "$displayd_pid_before"

echo "==> a commit/presentation attempt fails without disconnecting the client"
if timeout 8 "$target_dir/waylandd" --check-readiness "$wayland_socket" >"$runtime_dir/outage-client.log" 2>&1; then
  echo "unexpected presentation success while compd is stopped" >&2
  exit 1
fi
kill -0 "$waylandd_pid_before"
kill -0 "$displayd_pid_before"
[[ -d "/proc/$client_pid" ]]

echo "==> restarting compd and waiting for deferred canonical replay"
start_compd
touch "$runtime_dir/client.resume"
python3 - "$wayland_socket" <<'PY'
import socket
import struct
import sys
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.settimeout(3.0)
sock.connect(sys.argv[1])
sock.sendall(struct.pack("<II", 8, 1) + struct.pack("<I", 2))
sock.sendall(struct.pack("<II", 8, 0) + struct.pack("<I", 3))
sock.close()
PY
kill -0 "$waylandd_pid_before"
kill -0 "$displayd_pid_before"

echo "WAYLAND CLIENT COMPD OUTAGE SMOKE TEST PASSED"
