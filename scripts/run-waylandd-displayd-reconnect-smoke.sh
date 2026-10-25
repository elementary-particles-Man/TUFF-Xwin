#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/waybroker-displayd-reconnect.XXXXXX")"
export WAYBROKER_RUNTIME_DIR="$runtime_dir"
export DISPLAY=""
export WAYLAND_DISPLAY=""

displayd_pid=""
compd_pid=""
waylandd_pid=""
client_pid=""
wayland_socket="$runtime_dir/wayland-displayd-reconnect"

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

wait_for_socket() {
  local path=$1
  for _ in $(seq 1 200); do
    [[ -S "$path" ]] && return 0
    sleep 0.05
  done
  echo "timeout waiting for socket: $path" >&2
  return 1
}

wait_for_log() {
  local pattern=$1
  local path=$2
  for _ in $(seq 1 240); do
    [[ -f "$path" ]] && grep -Fq -- "$pattern" "$path" && return 0
    sleep 0.05
  done
  echo "timeout waiting for '$pattern' in $path" >&2
  for log in "$runtime_dir"/*.log; do
    echo "---- $log ----" >&2
    tail -n 80 "$log" >&2 || true
  done
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
    --bind-wayland-display "$wayland_socket" >"$runtime_dir/waylandd.log" 2>&1 &
  waylandd_pid=$!
  wait_for_socket "$wayland_socket"
  kill -0 "$waylandd_pid"
}

cargo build -p displayd -p compd -p waylandd >/dev/null
start_displayd
start_compd
start_waylandd
wait_for_log 'production_listener_bound' "$runtime_dir/waylandd.log"
wait_for_log 'displayd_outputs=' "$runtime_dir/waylandd.log"

displayd_pid_before=$displayd_pid
compd_pid_before=$compd_pid
waylandd_pid_before=$waylandd_pid

python3 - "$wayland_socket" "$runtime_dir/client.ready" "$runtime_dir/client.stop" <<'PY' &
import os
import socket
import struct
import sys
import time

path, ready_path, stop_path = sys.argv[1:]
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.settimeout(3.0)
sock.connect(path)
sock.sendall(struct.pack("<II", 8, 1) + struct.pack("<I", 2))
sock.sendall(struct.pack("<II", 8, 0) + struct.pack("<I", 3))
open(ready_path, "w", encoding="utf-8").close()
while not os.path.exists(stop_path):
    time.sleep(0.05)
sock.close()
PY
client_pid=$!
for _ in $(seq 1 100); do [[ -f "$runtime_dir/client.ready" ]] && break; sleep 0.05; done
[[ -f "$runtime_dir/client.ready" ]]

echo "==> stopping displayd only"
kill "$displayd_pid"
wait "$displayd_pid" 2>/dev/null || true
displayd_pid=""
kill -0 "$waylandd_pid_before"
kill -0 "$compd_pid_before"

echo "==> waiting for topology subscription reconnect"
wait_for_log 'subscription_lost' "$runtime_dir/waylandd.log"
wait_for_log 'reconnect_failed' "$runtime_dir/waylandd.log"

start_displayd
displayd_pid_after=$displayd_pid
[[ "$displayd_pid_after" != "$displayd_pid_before" ]]
wait_for_log 'event=reconnected epoch=' "$runtime_dir/waylandd.log"
kill -0 "$waylandd_pid_before"
kill -0 "$compd_pid_before"

touch "$runtime_dir/client.stop"
wait "$client_pid"
client_pid=""
kill -0 "$waylandd_pid_before"
kill -0 "$compd_pid_before"

echo "WAYLANDD DISPLAYD TOPOLOGY RECONNECT SMOKE TEST PASSED"
