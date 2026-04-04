#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

runtime_dir="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker}"
crashy_launch_state="$runtime_dir/launch-state-demo-x11-crashy.json"
degraded_launch_state="$runtime_dir/launch-state-demo-x11-degraded.json"
degraded_report="$runtime_dir/watchdog-report-demo-x11-degraded.json"
sessiond_log="$runtime_dir/sessiond-watchdog-resync.log"
watchdog_log="$runtime_dir/watchdog-watchdog-resync.log"
session_instance_id=""
sessiond_pid=""
watchdog_pid=""

cleanup_state() {
  local launch_state="$1"

  if [[ -f "$launch_state" ]]; then
    while read -r pid; do
      kill "$pid" 2>/dev/null || true
    done < <(rg -o '"pid":\s*[0-9]+' "$launch_state" | rg -o '[0-9]+')
  fi
}

cleanup() {
  if [[ -n "$watchdog_pid" ]] && kill -0 "$watchdog_pid" 2>/dev/null; then
    kill "$watchdog_pid" 2>/dev/null || true
    wait "$watchdog_pid" 2>/dev/null || true
  fi

  if [[ -n "$sessiond_pid" ]] && kill -0 "$sessiond_pid" 2>/dev/null; then
    kill "$sessiond_pid" 2>/dev/null || true
    wait "$sessiond_pid" 2>/dev/null || true
  fi

  cleanup_state "$crashy_launch_state"
  cleanup_state "$degraded_launch_state"
}

start_watchdog() {
  echo
  echo "==> cargo run -p watchdog -- --serve-ipc --write-reports"
  cargo run -p watchdog -- --serve-ipc --write-reports >>"$watchdog_log" 2>&1 &
  watchdog_pid=$!
  sleep 1
}

trap cleanup EXIT

mkdir -p "$runtime_dir"
: >"$sessiond_log"
: >"$watchdog_log"

echo "Running watchdog resync demo from $repo_root"
echo "sessiond log: $sessiond_log"
echo "watchdog log: $watchdog_log"

echo
echo "==> cargo run -p sessiond -- --select-profile demo-x11-crashy --write-selection"
cargo run -p sessiond -- --select-profile demo-x11-crashy --write-selection

start_watchdog

echo
echo "==> cargo run -p sessiond -- --serve-ipc --spawn-components --manage-active --notify-watchdog"
cargo run -p sessiond -- --serve-ipc --spawn-components --manage-active --notify-watchdog \
  >>"$sessiond_log" 2>&1 &
sessiond_pid=$!
sleep 1

echo
echo "==> waiting for first crash-loop update in $crashy_launch_state"
first_update_ready=0
for _ in $(seq 1 30); do
  if [[ -f "$crashy_launch_state" ]] && rg -q '"restart_count": 1' "$crashy_launch_state"; then
    first_update_ready=1
    break
  fi
  sleep 1
done

if [[ "$first_update_ready" -ne 1 ]]; then
  echo "first crash-loop update was not reached in time" >&2
  exit 1
fi

session_instance_id="$(
  rg -o '"session_instance_id":\s*"[^"]+"' "$crashy_launch_state" \
    | head -n1 \
    | sed -E 's/.*"([^"]+)"$/\1/'
)"

if [[ -z "$session_instance_id" ]]; then
  echo "session instance id was not written to $crashy_launch_state" >&2
  exit 1
fi

echo
echo "==> restarting watchdog to force sessiond resync"
kill "$watchdog_pid" 2>/dev/null || true
wait "$watchdog_pid" 2>/dev/null || true
watchdog_pid=""
sleep 1
start_watchdog

echo
echo "==> waiting for sessiond resync log in $sessiond_log"
resync_ready=0
for _ in $(seq 1 30); do
  if rg -q "watchdog_resync_required profile=demo-x11-crashy session_instance=$session_instance_id" "$sessiond_log"; then
    resync_ready=1
    break
  fi
  sleep 1
done

if [[ "$resync_ready" -ne 1 ]]; then
  echo "sessiond did not perform watchdog resync in time" >&2
  tail -n 40 "$sessiond_log" >&2 || true
  exit 1
fi

echo
echo "==> waiting for degraded watchdog report in $degraded_report"
degraded_report_ready=0
for _ in $(seq 1 30); do
  if [[ -f "$degraded_report" ]] && rg -q '"healthy_components": 4' "$degraded_report" && rg -q '"unhealthy_components": 0' "$degraded_report"; then
    degraded_report_ready=1
    break
  fi
  sleep 1
done

if [[ "$degraded_report_ready" -ne 1 ]]; then
  echo "degraded watchdog report was not reached in time" >&2
  tail -n 40 "$sessiond_log" >&2 || true
  tail -n 40 "$watchdog_log" >&2 || true
  exit 1
fi

if ! [[ -f "$degraded_launch_state" ]] || ! rg -q "\"session_instance_id\": \"$session_instance_id\"" "$degraded_launch_state"; then
  echo "degraded launch state did not preserve session instance id" >&2
  cat "$degraded_launch_state" >&2 || true
  exit 1
fi

echo
echo "==> resync evidence"
rg 'watchdog_resync_required|profile_transition|auto_launched_profile|session_instance' "$sessiond_log"

echo
echo "==> final watchdog state"
cat "$degraded_report"

echo
echo "==> final degraded launch state"
cat "$degraded_launch_state"
