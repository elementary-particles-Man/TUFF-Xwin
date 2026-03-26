#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

runtime_dir="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker}"
crashy_launch_state="$runtime_dir/launch-state-demo-x11-crashy.json"
degraded_launch_state="$runtime_dir/launch-state-demo-x11-degraded.json"
degraded_report="$runtime_dir/watchdog-report-demo-x11-degraded.json"
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

trap cleanup EXIT

echo "Running degraded mode demo from $repo_root"

echo
echo "==> cargo run -p sessiond -- --select-profile demo-x11-crashy --write-selection"
cargo run -p sessiond -- --select-profile demo-x11-crashy --write-selection

echo
echo "==> cargo run -p watchdog -- --serve-ipc --write-reports"
cargo run -p watchdog -- --serve-ipc --write-reports &
watchdog_pid=$!
sleep 1

echo
echo "==> cargo run -p sessiond -- --serve-ipc --spawn-components --manage-active --notify-watchdog"
cargo run -p sessiond -- --serve-ipc --spawn-components --manage-active --notify-watchdog &
sessiond_pid=$!
sleep 1

echo
echo "==> waiting for crash-loop state in $crashy_launch_state"
crash_loop_ready=0
for _ in $(seq 1 30); do
  if [[ -f "$crashy_launch_state" ]] && rg -q '"restart_count": 3' "$crashy_launch_state" && rg -q '"state": "failed"' "$crashy_launch_state"; then
    crash_loop_ready=1
    break
  fi
  sleep 1
done

if [[ "$crash_loop_ready" -ne 1 ]]; then
  echo "crash-loop state was not reached in time" >&2
  exit 1
fi

echo
echo "==> waiting for degraded watchdog report in $degraded_report"
degraded_report_ready=0
for _ in $(seq 1 30); do
  if [[ -f "$degraded_report" ]] && rg -q '"healthy_components": 3' "$degraded_report" && rg -q '"unhealthy_components": 0' "$degraded_report"; then
    degraded_report_ready=1
    break
  fi
  sleep 1
done

if [[ "$degraded_report_ready" -ne 1 ]]; then
  echo "degraded watchdog report was not reached in time" >&2
  exit 1
fi
