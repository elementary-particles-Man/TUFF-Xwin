#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin Lockd Identity and UI Path Smoke Test
# This script verifies that Lockd binding and UI path are deterministically 
# recorded in trace artifacts, and missing bindings fallback deterministically.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-lockd-smoke}"
rm -rf "$WAYBROKER_RUNTIME_DIR"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

echo "==> Running Lockd Identity and UI Path Smoke Test"
echo "==> Runtime directory: $WAYBROKER_RUNTIME_DIR"

target_dir="/home/flux/.cache/tuff-xwin-target/debug"

cleanup() {
  echo "==> Cleaning up..."
  pkill -P $$ || true
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
  local expected_final_state=$2
  local expected_binding_source=$3
  local expected_bound_id=$4
  local expected_lock_outcome=$5
  local expected_auth_outcome=$6
  
  echo
  echo "==> [Scenario: $scenario] (Expecting: $expected_final_state, $expected_binding_source)"

  # We use demo-x11 which has lockd explicit binding
  "$target_dir/sessiond" --select-profile demo-x11 --write-selection > /dev/null

  local lockd_args="--serve-ipc"
  if [[ "$scenario" == "lockd-trouble" ]]; then
    lockd_args="--serve-ipc --fail-resume"
  fi

  "$target_dir/displayd" > "$WAYBROKER_RUNTIME_DIR/displayd-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/displayd.sock"

  "$target_dir/lockd" $lockd_args > "$WAYBROKER_RUNTIME_DIR/lockd-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/lockd.sock"

  "$target_dir/compd" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/compd-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/compd.sock"

  # Run sessiond scenario
  "$target_dir/sessiond" --resume-scenario "$scenario" > "$WAYBROKER_RUNTIME_DIR/sessiond-$scenario.log" 2>&1

  # Verify trace
  local trace_file="$WAYBROKER_RUNTIME_DIR/lock-ui-path-$scenario.json"
  if [[ ! -f "$trace_file" ]]; then
    echo "Error: Trace file not found: $trace_file"
    return 1
  fi

  local actual_state=$(grep '"final_state":' "$trace_file" | cut -d'"' -f4)
  if [[ "$actual_state" != "$expected_final_state" ]]; then
    echo "Error: Unexpected final state. Expected: $expected_final_state, Actual: $actual_state"
    cat "$trace_file"
    return 1
  fi

  local actual_binding=$(grep '"binding_source":' "$trace_file" | cut -d'"' -f4)
  if [[ "$actual_binding" != "$expected_binding_source" ]]; then
    echo "Error: Unexpected binding source. Expected: $expected_binding_source, Actual: $actual_binding"
    return 1
  fi

  if [[ "$expected_bound_id" != "none" ]]; then
    grep "\"bound_component_id\": \"$expected_bound_id\"" "$trace_file" > /dev/null || { echo "Error: Expected bound_component_id $expected_bound_id not found."; return 1; }
  else
    grep "\"bound_component_id\": null" "$trace_file" > /dev/null || { echo "Error: Expected bound_component_id null not found."; return 1; }
  fi

  local actual_lock=$(grep '"lock_state_outcome":' "$trace_file" | cut -d'"' -f4)
  if [[ "$actual_lock" != "$expected_lock_outcome" ]]; then
    echo "Error: Unexpected lock state outcome. Expected: $expected_lock_outcome, Actual: $actual_lock"
    return 1
  fi

  local actual_auth=$(grep '"auth_outcome":' "$trace_file" | cut -d'"' -f4)
  if [[ "$actual_auth" != "$expected_auth_outcome" ]]; then
    echo "Error: Unexpected auth outcome. Expected: $expected_auth_outcome, Actual: $actual_auth"
    return 1
  fi

  echo "==> Scenario $scenario PASSED"

  cleanup
}

# Pre-build
echo "==> Pre-building all packages..."
cargo build --workspace

# 1. Normal with binding
run_scenario "normal" "normal" "explicit" "demo-lockui" "success" "success"

# 2. lockd-trouble with binding
run_scenario "lockd-trouble" "blank-only" "explicit" "demo-lockui" "failed" "skipped"

echo
echo "==> Test missing binding behavior"
# Create a temporary profile with no lockd binding
cat << 'EOF' > "$WAYBROKER_RUNTIME_DIR/active-profile.json"
{
  "id": "demo-no-lockd",
  "display_name": "Demo No Lockd",
  "protocol": "layer-x11",
  "summary": "no lockd",
  "degraded_profile_id": null,
  "broker_services": ["displayd", "sessiond", "compd"],
  "session_components": [],
  "service_component_bindings": []
}
EOF

"$target_dir/displayd" > "$WAYBROKER_RUNTIME_DIR/displayd-missing.log" 2>&1 &
wait_for_socket "$WAYBROKER_RUNTIME_DIR/displayd.sock"

# We won't start lockd, because sessiond should fail early
"$target_dir/sessiond" --resume-scenario "normal" > "$WAYBROKER_RUNTIME_DIR/sessiond-missing.log" 2>&1

trace_file="$WAYBROKER_RUNTIME_DIR/lock-ui-path-normal.json"
if [[ ! -f "$trace_file" ]]; then
  echo "Error: Trace file not found: $trace_file"
  exit 1
fi

actual_state=$(grep '"final_state":' "$trace_file" | cut -d'"' -f4)
if [[ "$actual_state" != "blank-only" ]]; then
  echo "Error: Missing binding did not result in blank-only. Actual: $actual_state"
  exit 1
fi

actual_binding=$(grep '"binding_source":' "$trace_file" | cut -d'"' -f4)
if [[ "$actual_binding" != "missing" ]]; then
  echo "Error: Binding source should be missing. Actual: $actual_binding"
  exit 1
fi

echo "==> Missing binding scenario PASSED"

echo
echo "==> LOCKD IDENTITY AND UI PATH SMOKE TEST PASSED"
