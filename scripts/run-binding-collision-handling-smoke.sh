#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin Binding Collision Handling Smoke Test
# This script verifies that binding collisions and misconfigurations are
# deterministically detected and blocked from execution.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-collision-smoke}"
rm -rf "$WAYBROKER_RUNTIME_DIR"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

echo "==> Running Binding Collision Handling Smoke Test"
echo "==> Runtime directory: $WAYBROKER_RUNTIME_DIR"

target_dir="/home/flux/.cache/tuff-xwin-target/debug"

cleanup() {
  echo "==> Cleaning up..."
  pkill -u "$USER" -f "$target_dir/displayd" || true
  pkill -u "$USER" -f "$target_dir/watchdog" || true
  pkill -u "$USER" -f "$target_dir/lockd" || true
  pkill -u "$USER" -f "$target_dir/compd" || true
  pkill -u "$USER" -f "$target_dir/sessiond" || true
  sleep 0.5
  rm -f "$WAYBROKER_RUNTIME_DIR"/*.sock
}

trap cleanup EXIT

wait_for_socket() {
  local socket=$1
  local timeout=5
  local count=0
  echo "Waiting for socket $socket..."
  while [[ ! -S "$socket" ]]; do
    if [[ $count -ge $((timeout * 10)) ]]; then
      echo "Error: Timeout waiting for socket $socket" >&2
      return 1
    fi
    sleep 0.1
    count=$((count + 1))
  done
  echo "Socket $socket found."
  sleep 0.1
}

run_scenario() {
  local scenario=$1
  local profile_id=$2
  local trigger_scenario=$3
  local expect_resolution=$4
  local expect_exec_result=$5
  local fail_lockd=${6:-"false"}
  local fail_compd=${7:-"false"}

  echo
  echo "==> [Scenario: $scenario] (Profile: $profile_id)"

  # Start minimal stack
  "$target_dir/displayd" > "$WAYBROKER_RUNTIME_DIR/displayd-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/displayd.sock"

  "$target_dir/watchdog" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/watchdog-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/watchdog.sock"

  # Pre-select profile
  "$target_dir/sessiond" --select-profile "$profile_id" --write-selection > /dev/null

  # Start sessiond in manage-active mode
  "$target_dir/sessiond" --serve-ipc --manage-active --spawn-components --notify-watchdog > "$WAYBROKER_RUNTIME_DIR/sessiond-managed-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/sessiond.sock"

  # We need lockd and compd running for the scenario to reach recovery
  local lockd_args="--serve-ipc"
  if [[ "$fail_lockd" == "true" ]]; then lockd_args="$lockd_args --fail-resume"; fi
  "$target_dir/lockd" $lockd_args > "$WAYBROKER_RUNTIME_DIR/lockd-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/lockd.sock"

  local compd_args="--serve-ipc"
  if [[ "$fail_compd" == "true" ]]; then compd_args="$compd_args --fail-resume"; fi
  "$target_dir/compd" $compd_args > "$WAYBROKER_RUNTIME_DIR/compd-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/compd.sock"

  # Trigger resume failure
  echo "==> Triggering $trigger_scenario..."
  "$target_dir/sessiond" --resume-scenario "$trigger_scenario" > "$WAYBROKER_RUNTIME_DIR/resume-trigger-$scenario.log" 2>&1

  # Wait a bit for potential background execution or collision detection
  sleep 2

  # Check reports
  echo "==> Verifying collision report..."
  local collision_report="$WAYBROKER_RUNTIME_DIR/binding-collision-report.json"
  if [[ ! -f "$collision_report" ]]; then
    echo "FAILED: Collision report not found"
    exit 1
  fi

  echo "==> Verifying resolution artifact..."
  local service_role=$(echo "$trigger_scenario" | cut -d'-' -f1)
  local resolution_artifact="$WAYBROKER_RUNTIME_DIR/binding-resolution-$service_role.json"
  if [[ ! -f "$resolution_artifact" ]]; then
    echo "FAILED: Resolution artifact for $service_role not found"
    exit 1
  fi

  local actual_resolution=$(grep '"result":' "$resolution_artifact" | cut -d'"' -f4)
  if [[ "$actual_resolution" != "$expect_resolution" ]]; then
    echo "FAILED: Expected resolution result $expect_resolution, got $actual_resolution"
    cat "$resolution_artifact"
    exit 1
  fi

  echo "==> Verifying execution result..."
  local exec_artifact="$WAYBROKER_RUNTIME_DIR/watchdog-action-execution-$service_role.json"
  if [[ ! -f "$exec_artifact" ]]; then
    echo "FAILED: Execution artifact for $service_role not found"
    exit 1
  fi

  local actual_exec_result=$(grep '"result":' "$exec_artifact" | cut -d'"' -f4)
  if [[ "$actual_exec_result" != "$expect_exec_result" ]]; then
    echo "FAILED: Expected execution result $expect_exec_result, got $actual_exec_result"
    cat "$exec_artifact"
    exit 1
  fi

  echo "==> Scenario $scenario PASSED"
  cleanup
}

echo "==> Pre-building all packages..."
cargo build --workspace

# 1. compd-binding-collision
run_scenario "compd-binding-collision" "demo-x11-compd-binding-collision" "compd-trouble" "collision" "config-error" "false" "true"

# 2. lockd-binding-collision
run_scenario "lockd-binding-collision" "demo-x11-lockd-binding-collision" "lockd-trouble" "collision" "config-error" "true" "false"

# 3. lockd-missing-target
echo "==> [Scenario: lockd-missing-target]"
cat << 'EOF' > "$WAYBROKER_RUNTIME_DIR/active-profile.json"
{
  "id": "demo-missing-target",
  "display_name": "Demo Missing Target",
  "protocol": "layer-x11",
  "summary": "demo",
  "broker_services": ["displayd", "sessiond", "watchdog", "x11bridge", "lockd"],
  "session_components": [],
  "service_component_bindings": [
    {
      "service": "lockd",
      "component_id": "ghost-ui"
    }
  ],
  "service_recovery_execution_policies": [
    {
      "service": "lockd",
      "mode": "supervisor-restart"
    }
  ]
}
EOF

"$target_dir/displayd" > "$WAYBROKER_RUNTIME_DIR/displayd-missing.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/displayd.sock"
"$target_dir/watchdog" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/watchdog-missing.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/watchdog.sock"
"$target_dir/lockd" --serve-ipc --fail-resume > "$WAYBROKER_RUNTIME_DIR/lockd-missing.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/lockd.sock"
"$target_dir/compd" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/compd-missing.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/compd.sock"
"$target_dir/sessiond" --serve-ipc --manage-active > "$WAYBROKER_RUNTIME_DIR/sessiond-managed-missing.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/sessiond.sock"

"$target_dir/sessiond" --resume-scenario "lockd-trouble" > "$WAYBROKER_RUNTIME_DIR/resume-trigger-missing.log" 2>&1
sleep 2

res_file="$WAYBROKER_RUNTIME_DIR/binding-resolution-lockd.json"
if [[ ! -f "$res_file" ]]; then echo "FAILED: lockd resolution artifact not found"; exit 1; fi
actual_res=$(grep '"result":' "$res_file" | cut -d'"' -f4)
if [[ "$actual_res" != "missing-target" ]]; then echo "FAILED: Expected result missing-target, got $actual_res"; exit 1; fi

exec_file="$WAYBROKER_RUNTIME_DIR/watchdog-action-execution-lockd.json"
if [[ ! -f "$exec_file" ]]; then echo "FAILED: lockd execution artifact not found"; exit 1; fi
actual_exec=$(grep '"result":' "$exec_file" | cut -d'"' -f4)
if [[ "$actual_exec" != "config-error" ]]; then echo "FAILED: Expected execution config-error, got $actual_exec"; exit 1; fi

echo "==> Scenario lockd-missing-target PASSED"

echo
echo "==> BINDING COLLISION HANDLING SMOKE TEST PASSED"
