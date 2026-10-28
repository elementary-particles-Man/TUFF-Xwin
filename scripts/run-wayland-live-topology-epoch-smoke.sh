#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/waybroker-live-topology.XXXXXX")"
export WAYBROKER_RUNTIME_DIR="$runtime_dir" DISPLAY="" WAYLAND_DISPLAY=""
displayd_pid= compd_pid= waylandd_pid= client_pid=
wayland_socket="$runtime_dir/wayland-live-topology"

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

target_dir="$(cargo metadata --format-version 1 --no-deps --quiet | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug"
wait_for_socket() {
  for _ in $(seq 1 240); do [[ -S "$1" ]] && return 0; sleep 0.05; done
  echo "timeout waiting for socket $1" >&2; return 1
}
wait_for_file() {
  for _ in $(seq 1 240); do [[ -f "$1" ]] && return 0; sleep 0.05; done
  echo "timeout waiting for marker $1" >&2; return 1
}
wait_for_log() {
  local pattern=$1 path=$2
  for _ in $(seq 1 240); do
    [[ -f "$path" ]] && grep -Fq -- "$pattern" "$path" && return 0
    sleep 0.05
  done
  echo "timeout waiting for '$pattern' in $path" >&2
  for log in "$runtime_dir"/*.log; do echo "---- $log ----" >&2; tail -n 100 "$log" >&2 || true; done
  return 1
}
start_displayd() {
  TUFF_XWIN_DISPLAY_BACKEND=mock "$target_dir/displayd" --no-vulkan >"$runtime_dir/displayd.log" 2>&1 &
  displayd_pid=$!
  wait_for_socket "$runtime_dir/displayd.sock"
  kill -0 "$displayd_pid"
}

cargo build -p displayd -p compd -p waylandd >/dev/null
start_displayd
"$target_dir/compd" --serve-ipc --no-vulkan >"$runtime_dir/compd.log" 2>&1 &
compd_pid=$!
wait_for_socket "$runtime_dir/compd.sock"
"$target_dir/waylandd" --serve-ipc --production --no-vulkan \
  --bind-wayland-display "$wayland_socket" >"$runtime_dir/waylandd.log" 2>&1 &
waylandd_pid=$!
wait_for_socket "$wayland_socket"
wait_for_log 'event=reconnected epoch=' "$runtime_dir/waylandd.log"

python3 scripts/wayland-live-topology-test-client.py "$wayland_socket" \
  "$runtime_dir/client.ready" "$runtime_dir/client.stop" "$runtime_dir/client.stopped" \
  "$runtime_dir/client.resume" "$runtime_dir/client.done" >"$runtime_dir/client.log" 2>&1 &
client_pid=$!
wait_for_file "$runtime_dir/client.ready"
waylandd_before=$waylandd_pid
compd_before=$compd_pid

echo "==> stopping displayd only"
kill "$displayd_pid"
wait "$displayd_pid" 2>/dev/null || true
displayd_pid=
kill -0 "$waylandd_before"
kill -0 "$compd_before"
touch "$runtime_dir/client.stop"
wait_for_file "$runtime_dir/client.stopped"
kill -0 "$waylandd_before"
kill -0 "$compd_before"
wait_for_log 'event=subscription_lost' "$runtime_dir/waylandd.log"

start_displayd
displayd_after=$displayd_pid
wait_for_log 'event=reconnected epoch=' "$runtime_dir/waylandd.log"
[[ "$displayd_after" != "" ]]
touch "$runtime_dir/client.resume"
wait_for_file "$runtime_dir/client.done"
wait "$client_pid"
client_pid=
kill -0 "$waylandd_before"
kill -0 "$compd_before"
if grep -Fq -- 'production_client_error' "$runtime_dir/waylandd.log"; then
  echo "unexpected production client error" >&2; exit 1
fi
grep -Fq -- 'surface_commit' "$runtime_dir/client.log"
grep -Fq -- 'callback_done' "$runtime_dir/client.log" || true
echo "WAYLAND LIVE TOPOLOGY EPOCH SMOKE TEST PASSED"
