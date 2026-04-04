#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-compd-broker-recovery}"
rm -rf "$WAYBROKER_RUNTIME_DIR"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

target_dir="/home/flux/.cache/tuff-xwin-target/debug"
profile_id="demo-wayland-compd-recovery"
displayd_pid=""
waylandd_pid=""
watchdog_pid=""
lockd_pid=""
sessiond_pid=""

cleanup_launch_states() {
  local launch_state
  for launch_state in "$WAYBROKER_RUNTIME_DIR"/launch-state-*.json; do
    [[ -f "$launch_state" ]] || continue
    while read -r pid; do
      kill "$pid" 2>/dev/null || true
    done < <(rg -o '"pid":\s*[0-9]+' "$launch_state" | rg -o '[0-9]+')
  done
}

cleanup() {
  cleanup_launch_states

  for pid in "$sessiond_pid" "$lockd_pid" "$watchdog_pid" "$waylandd_pid" "$displayd_pid"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done

  rm -f "$WAYBROKER_RUNTIME_DIR"/*.sock
}

trap cleanup EXIT

wait_for_socket() {
  local socket=$1
  local timeout=${2:-5}
  local count=0

  while [[ ! -S "$socket" ]]; do
    if [[ $count -ge $((timeout * 10)) ]]; then
      echo "FAILED: timeout waiting for socket $socket" >&2
      return 1
    fi
    sleep 0.1
    count=$((count + 1))
  done

  sleep 0.1
}

wait_for_file() {
  local path=$1
  local timeout=${2:-10}
  local count=0

  while [[ ! -f "$path" ]]; do
    if [[ $count -ge $((timeout * 10)) ]]; then
      echo "FAILED: timeout waiting for file $path" >&2
      return 1
    fi
    sleep 0.1
    count=$((count + 1))
  done
}

wait_for_log() {
  local pattern=$1
  local log_file=$2
  local timeout=${3:-10}
  local count=0

  while ! [[ -f "$log_file" ]] || ! rg -q "$pattern" "$log_file"; do
    if [[ $count -ge $((timeout * 10)) ]]; then
      echo "FAILED: timeout waiting for pattern $pattern in $log_file" >&2
      return 1
    fi
    sleep 0.1
    count=$((count + 1))
  done
}

echo "==> Running Compd Broker Recovery Smoke Test"
echo "==> Runtime directory: $WAYBROKER_RUNTIME_DIR"

echo "==> Pre-building all packages..."
cargo build --workspace

echo "==> Starting displayd"
"$target_dir/displayd" > "$WAYBROKER_RUNTIME_DIR/displayd.log" 2>&1 &
displayd_pid=$!
wait_for_socket "$WAYBROKER_RUNTIME_DIR/displayd.sock"

echo "==> Starting waylandd registry broker"
"$target_dir/waylandd" --serve-ipc --registry "$repo_root/examples/minimal-scene/surface-registry.json" \
  > "$WAYBROKER_RUNTIME_DIR/waylandd.log" 2>&1 &
waylandd_pid=$!
wait_for_socket "$WAYBROKER_RUNTIME_DIR/waylandd.sock"

echo "==> Starting watchdog"
"$target_dir/watchdog" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/watchdog.log" 2>&1 &
watchdog_pid=$!
wait_for_socket "$WAYBROKER_RUNTIME_DIR/watchdog.sock"

echo "==> Starting lockd"
"$target_dir/lockd" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/lockd.log" 2>&1 &
lockd_pid=$!
wait_for_socket "$WAYBROKER_RUNTIME_DIR/lockd.sock"

echo "==> Priming displayd committed scene"
"$target_dir/compd" \
  --scene "$repo_root/examples/minimal-scene/scene.json" \
  --commit-demo \
  --require-displayd \
  > "$WAYBROKER_RUNTIME_DIR/compd-prime.log" 2>&1

snapshot_file="$WAYBROKER_RUNTIME_DIR/displayd-last-scene.json"
wayland_registry_file="$WAYBROKER_RUNTIME_DIR/waylandd-surface-registry.json"
wait_for_file "$snapshot_file"
rg -q '"id": "panel-1"' "$snapshot_file"
wait_for_file "$wayland_registry_file"
rg -q '"clipboard_owner": "panel-1"' "$wayland_registry_file"

echo "==> Selecting profile $profile_id"
"$target_dir/sessiond" --select-profile "$profile_id" --write-selection > /dev/null

echo "==> Starting managed sessiond"
managed_log="$WAYBROKER_RUNTIME_DIR/sessiond-managed.log"
launch_state_file="$WAYBROKER_RUNTIME_DIR/launch-state-$profile_id.json"
"$target_dir/sessiond" --serve-ipc --manage-active --spawn-components --notify-watchdog \
  > "$managed_log" 2>&1 &
sessiond_pid=$!
wait_for_socket "$WAYBROKER_RUNTIME_DIR/sessiond.sock"
wait_for_socket "$WAYBROKER_RUNTIME_DIR/compd.sock"
wait_for_file "$launch_state_file"
rg -q '"id": "demo-shell"' "$launch_state_file"
rg -q '"id": "demo-panel"' "$launch_state_file"

echo "==> Triggering resume scenario: compd-trouble"
"$target_dir/sessiond" --resume-scenario "compd-trouble" \
  > "$WAYBROKER_RUNTIME_DIR/resume-trigger.log" 2>&1

execution_artifact="$WAYBROKER_RUNTIME_DIR/watchdog-action-execution-compd.json"
wait_for_file "$execution_artifact"

rg -q '"result": "succeeded"' "$execution_artifact"
rg -q '"--restore-from-displayd"' "$execution_artifact"
rg -q '"--reconcile-waylandd"' "$execution_artifact"
rg -q '"--handoff-selection"' "$execution_artifact"

wait_for_log 'service=compd op=scene_recover event=success' "$managed_log"
wait_for_log 'service=compd op=scene_reconcile dropped_ids=panel-1' "$managed_log"
wait_for_log 'service=compd op=selection_handoff event=success' "$managed_log"
wait_for_log 'service=compd op=startup_rebuild event=scene_committed' "$managed_log"

wait_for_file "$snapshot_file"
rg -q '"id": "terminal-1"' "$snapshot_file"

if rg -q '"id": "panel-1"' "$snapshot_file"; then
  echo "FAILED: panel-1 should have been dropped from displayd snapshot" >&2
  cat "$snapshot_file" >&2
  exit 1
fi

wait_for_file "$wayland_registry_file"
rg -q '"clipboard_owner": "terminal-1"' "$wayland_registry_file"
rg -q '"primary_selection_owner": "terminal-1"' "$wayland_registry_file"

echo "==> Recovery artifact"
cat "$execution_artifact"

echo
echo "==> Rebuilt displayd snapshot"
cat "$snapshot_file"

echo
echo "==> Updated waylandd registry"
cat "$wayland_registry_file"

echo
echo "==> COMPD BROKER RECOVERY SMOKE TEST PASSED"
