#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

runtime_dir="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker}"
crashy_launch_state="$runtime_dir/launch-state-demo-x11-crashy.json"
degraded_launch_state="$runtime_dir/launch-state-demo-x11-degraded.json"
sessiond_pid=""

cleanup_state() {
  local launch_state="$1"

  if [[ -f "$launch_state" ]]; then
    while read -r pid; do
      kill "$pid" 2>/dev/null || true
    done < <(rg -o '"pid":\s*[0-9]+' "$launch_state" | rg -o '[0-9]+')
  fi
}

cleanup() {
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
echo "==> cargo run -p sessiond -- --launch-active --spawn-components --supervise-seconds 4 --restart-limit 2"
cargo run -p sessiond -- --launch-active --spawn-components --supervise-seconds 4 --restart-limit 2

echo
echo "==> cargo run -p sessiond -- --serve-ipc --once --spawn-components"
cargo run -p sessiond -- --serve-ipc --once --spawn-components &
sessiond_pid=$!
sleep 1

echo
echo "==> cargo run -p watchdog -- --profile-id demo-x11-crashy --write-reports --notify-sessiond"
cargo run -p watchdog -- --profile-id demo-x11-crashy --write-reports --notify-sessiond
wait "$sessiond_pid"
sessiond_pid=""

echo
echo "==> cargo run -p watchdog -- --profile-id demo-x11-degraded --write-reports"
cargo run -p watchdog -- --profile-id demo-x11-degraded --write-reports
