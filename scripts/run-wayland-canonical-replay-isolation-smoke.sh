#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/waybroker-canonical-replay.XXXXXX")"
export WAYBROKER_RUNTIME_DIR="$runtime_dir" DISPLAY="" WAYLAND_DISPLAY=""
displayd_pid="" compd_pid="" waylandd_pid="" client_pid=""
wayland_socket="$runtime_dir/wayland-canonical-replay"
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
  local path=$1
  for _ in $(seq 1 200); do
    [[ -S "$path" ]] && return 0
    sleep 0.05
  done
  echo "timeout waiting for socket: $path" >&2
  return 1
}
wait_for_log() {
  local pattern=$1 path=$2
  for _ in $(seq 1 240); do
    [[ -f "$path" ]] && grep -Fq -- "$pattern" "$path" && return 0
    sleep 0.05
  done
  echo "timeout waiting for '$pattern' in $path" >&2
  return 1
}
start_displayd() {
  TUFF_XWIN_DISPLAY_BACKEND=mock "$target_dir/displayd" --no-vulkan >"$runtime_dir/displayd.log" 2>&1 &
  displayd_pid=$!
  wait_for_socket "$runtime_dir/displayd.sock"
}
start_compd() {
  "$target_dir/compd" --serve-ipc --no-vulkan >"$runtime_dir/compd.log" 2>&1 &
  compd_pid=$!
  wait_for_socket "$runtime_dir/compd.sock"
}
start_waylandd() {
  "$target_dir/waylandd" --serve-ipc --production --no-vulkan \
    --bind-wayland-display "$wayland_socket" >"$runtime_dir/waylandd.log" 2>&1 &
  waylandd_pid=$!
  wait_for_socket "$wayland_socket"
  wait_for_log 'production_listener_bound' "$runtime_dir/waylandd.log"
}
dump_logs() {
  for name in displayd compd waylandd client; do
    [[ -f "$runtime_dir/$name.log" ]] && { echo "---- $name.log ----" >&2; tail -n 120 "$runtime_dir/$name.log" >&2; }
  done
}

cargo build -p displayd -p compd -p waylandd >/dev/null
python3 -m py_compile scripts/wayland-replay-test-client.py
start_displayd
start_compd
start_waylandd

python3 scripts/wayland-replay-test-client.py \
  "$wayland_socket" "$runtime_dir/ready" "$runtime_dir/outage" \
  "$runtime_dir/outage-ok" "$runtime_dir/resume" "$runtime_dir/done" \
  >"$runtime_dir/client.log" 2>&1 &
client_pid=$!
for _ in $(seq 1 120); do [[ -f "$runtime_dir/ready" ]] && break; sleep 0.05; done
[[ -f "$runtime_dir/ready" ]] || { dump_logs; exit 1; }
displayd_pid_before=$displayd_pid
waylandd_pid_before=$waylandd_pid

echo '==> stopping only compd with two native clients connected'
kill "$compd_pid"
wait "$compd_pid" 2>/dev/null || true
compd_pid=""
touch "$runtime_dir/outage"
sleep 1
for _ in $(seq 1 120); do [[ -f "$runtime_dir/outage-ok" ]] && break; sleep 0.05; done
if [[ ! -f "$runtime_dir/outage-ok" ]]; then
  # The native readiness clients above provide the presentation assertion;
  # retain the persistent client logs for diagnosis if its optional replay
  # phase cannot complete in a constrained runner.
  echo 'two-client replay phase did not report its outage barrier' >&2
  dump_logs
  exit 1
fi
kill -0 "$waylandd_pid_before"
kill -0 "$displayd_pid_before"

echo '==> restarting compd and waiting for shared presentation callbacks'
start_compd
touch "$runtime_dir/resume"
for _ in $(seq 1 240); do [[ -f "$runtime_dir/done" ]] && break; sleep 0.05; done
[[ -f "$runtime_dir/done" ]] || { dump_logs; exit 1; }
kill -0 "$waylandd_pid_before"
kill -0 "$displayd_pid_before"
wait "$client_pid"
client_pid=""
retry_failures="$(grep -Fc 'op=canonical_recommit event=retry' "$runtime_dir/waylandd.log" || true)"
callback_a="$(grep -Fc 'client=A callback_done' "$runtime_dir/client.log" || true)"
callback_b="$(grep -Fc 'client=B callback_done' "$runtime_dir/client.log" || true)"
[[ "$retry_failures" -ge 2 ]] || { dump_logs; echo "expected at least 2 recoverable retry failures, got $retry_failures" >&2; exit 1; }
[[ "$callback_a" -eq 1 && "$callback_b" -eq 1 ]] || { dump_logs; echo "expected one callback per client, got A=$callback_a B=$callback_b" >&2; exit 1; }
echo "canonical_replay retry_failures=$retry_failures callback_A=$callback_a callback_B=$callback_b"
echo 'WAYLAND CANONICAL REPLAY ISOLATION SMOKE TEST PASSED'
