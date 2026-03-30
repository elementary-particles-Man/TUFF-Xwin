#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin Lockd Recovery Execution Optionalization Smoke Test
# This script verifies that lockd recovery execution is disabled by default,
# but executes successfully when explicitly opted-in via profile policy.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-lockd-recovery-optionalization}"
rm -rf "$WAYBROKER_RUNTIME_DIR"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

echo "==> Running Lockd Recovery Execution Optionalization Smoke Test"
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
  local profile_id=$2
  local expect_watchdog_request=$3
  local expect_execution_result=$4

  echo
  echo "==> [Scenario: $scenario] (Profile: $profile_id)"

  # Pre-select profile
  "$target_dir/sessiond" --select-profile "$profile_id" --write-selection > /dev/null

  # Start services
  "$target_dir/displayd" > "$WAYBROKER_RUNTIME_DIR/displayd-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/displayd.sock"

  "$target_dir/watchdog" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/watchdog-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/watchdog.sock"

  # Start sessiond in manage-active mode
  "$target_dir/sessiond" --serve-ipc --manage-active --spawn-components --notify-watchdog > "$WAYBROKER_RUNTIME_DIR/sessiond-managed-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/sessiond.sock"

  # Inject faulty lockd
  pkill -u "$USER" -f "$target_dir/lockd" || true
  sleep 0.5
  rm -f "$WAYBROKER_RUNTIME_DIR/lockd.sock"
  "$target_dir/lockd" --serve-ipc --fail-resume > "$WAYBROKER_RUNTIME_DIR/lockd-faulty-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/lockd.sock"

  # Make sure compd is running so we can talk to it
  pkill -u "$USER" -f "$target_dir/compd" || true
  sleep 0.5
  rm -f "$WAYBROKER_RUNTIME_DIR/compd.sock"
  "$target_dir/compd" --serve-ipc > "$WAYBROKER_RUNTIME_DIR/compd-$scenario.log" 2>&1 &
  wait_for_socket "$WAYBROKER_RUNTIME_DIR/compd.sock"

  # Trigger resume failure on lockd
  echo "==> Triggering lockd resume failure..."
  "$target_dir/sessiond" --resume-scenario "lockd-trouble" > "$WAYBROKER_RUNTIME_DIR/resume-trigger-$scenario.log" 2>&1

  # Wait a bit for potential background execution
  sleep 1.5

  echo "==> Verifying artifacts..."

  local lock_artifact="$WAYBROKER_RUNTIME_DIR/lock-ui-path-lockd-trouble.json"
  if [[ ! -f "$lock_artifact" ]]; then
    echo "FAILED: Lock artifact not found"
    exit 1
  fi

  local actual_final_state=$(grep '"final_state":' "$lock_artifact" | cut -d'"' -f4)
  if [[ "$actual_final_state" != "blank-only" ]]; then
    echo "FAILED: Expected final_state blank-only, got $actual_final_state"
    exit 1
  fi

  local actual_watchdog_request=$(grep '"watchdog_request_outcome":' "$lock_artifact" | cut -d'"' -f4)
  if [[ "$actual_watchdog_request" != "$expect_watchdog_request" ]]; then
    echo "FAILED: Expected watchdog_request_outcome $expect_watchdog_request, got $actual_watchdog_request"
    cat "$lock_artifact"
    exit 1
  fi

  local exec_artifact="$WAYBROKER_RUNTIME_DIR/watchdog-action-execution-lockd.json"
  if [[ "$expect_execution_result" == "none" ]]; then
    if [[ -f "$exec_artifact" ]]; then
      echo "FAILED: Execution artifact should not exist"
      exit 1
    fi
  else
    if [[ ! -f "$exec_artifact" ]]; then
      echo "FAILED: Execution artifact not found"
      exit 1
    fi
    local actual_exec_result=$(grep '"result":' "$exec_artifact" | cut -d'"' -f4)
    if [[ "$actual_exec_result" != "$expect_execution_result" ]]; then
      echo "FAILED: Expected execution result $expect_execution_result, got $actual_exec_result"
      cat "$exec_artifact"
      exit 1
    fi
  fi

  echo "==> Scenario $scenario PASSED"
  cleanup
}

echo "==> Pre-building all packages..."
cargo build --workspace

# 1. lockd-trouble-default-disabled (uses demo-x11)
run_scenario "lockd-trouble-default-disabled" "demo-x11" "skipped" "none"

# 2. lockd-trouble-optin-enabled (uses demo-x11-lockd-recovery-optin)
run_scenario "lockd-trouble-optin-enabled" "demo-x11-lockd-recovery-optin" "accepted" "succeeded"

# 3. lockd-trouble-optin-missing-binding
echo "==> [Scenario: lockd-trouble-optin-missing-binding]"
cat << 'EOF' > "$WAYBROKER_RUNTIME_DIR/active-profile.json"
{
  "id": "demo-optin-missing",
  "display_name": "Demo Optin Missing Binding",
  "protocol": "layer-x11",
  "summary": "demo",
  "broker_services": ["displayd", "sessiond", "watchdog", "x11bridge", "lockd"],
  "session_components": [],
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

sleep 1.5

exec_artifact="$WAYBROKER_RUNTIME_DIR/watchdog-action-execution-lockd.json"
if [[ ! -f "$exec_artifact" ]]; then
  echo "FAILED: Execution artifact not found for missing binding"
  exit 1
fi
actual_exec_result=$(grep '"result":' "$exec_artifact" | cut -d'"' -f4)
if [[ "$actual_exec_result" != "no-executor" ]]; then
  echo "FAILED: Expected execution result no-executor, got $actual_exec_result"
  exit 1
fi

echo "==> Scenario lockd-trouble-optin-missing-binding PASSED"

echo
echo "==> LOCKD RECOVERY EXECUTION OPTIONALIZATION SMOKE TEST PASSED"
