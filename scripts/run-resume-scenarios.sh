#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin Resume Scenario Runner
# This script executes 4 resume scenarios: normal, displayd-trouble, compd-trouble, lockd-trouble.
# It verifies that each scenario results in the expected final_state and generates a trace artifact.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-resume-scenarios}"
rm -rf "$WAYBROKER_RUNTIME_DIR"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

echo "==> Running Waybroker Resume Scenarios"
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
  while [[ ! -S "$socket" ]]; do
    if [[ $count -ge $((timeout * 10)) ]]; then
      echo "Error: Timeout waiting for socket $socket" >&2
      return 1
    fi
    sleep 0.1
    count=$((count + 1))
  done
  sleep 0.1
}

run_scenario() {
  local scenario=$1
  local expected_state=$2
  local extra_args=$3

  echo
  echo "==> [Scenario: $scenario] (Expecting: $expected_state)"
  
  # Start services with fault injection if needed
  local displayd_args=""
  local compd_args="--serve-ipc"
  local lockd_args="--serve-ipc"

  if [[ "$scenario" == "displayd-trouble" ]]; then
    displayd_args="$displayd_args --fail-resume"
  elif [[ "$scenario" == "compd-trouble" ]]; then
    compd_args="$compd_args --fail-resume"
  elif [[ "$scenario" == "lockd-trouble" ]]; then
    lockd_args="$lockd_args --fail-resume"
  fi

  "$target_dir/displayd" $displayd_args > "$WAYBROKER_RUNTIME_DIR/displayd-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/displayd.sock"

  "$target_dir/compd" $compd_args > "$WAYBROKER_RUNTIME_DIR/compd-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/compd.sock"

  "$target_dir/lockd" $lockd_args > "$WAYBROKER_RUNTIME_DIR/lockd-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/lockd.sock"

  # Run sessiond scenario
  "$target_dir/sessiond" --resume-scenario "$scenario" > "$WAYBROKER_RUNTIME_DIR/sessiond-$scenario.log" 2>&1

  # Cleanup for next scenario
  cleanup

  # Verify trace
  local trace_file="$WAYBROKER_RUNTIME_DIR/resume-trace-$scenario.json"
  if [[ ! -f "$trace_file" ]]; then
    echo "Error: Trace file not found: $trace_file"
    return 1
  fi

  local actual_state=$(grep '"final_state":' "$trace_file" | cut -d'"' -f4)
  if [[ "$actual_state" != "$expected_state" ]]; then
    echo "Error: Unexpected final state. Expected: $expected_state, Actual: $actual_state"
    cat "$trace_file"
    return 1
  fi

  echo "==> Scenario $scenario PASSED"
}

# 1. Normal
run_scenario "normal" "normal" ""

# 2. displayd-trouble
run_scenario "displayd-trouble" "hold" ""

# 3. compd-trouble
run_scenario "compd-trouble" "restart-request" ""

# 4. lockd-trouble
run_scenario "lockd-trouble" "blank-only" ""

echo
echo "==> ALL RESUME SCENARIOS PASSED"
