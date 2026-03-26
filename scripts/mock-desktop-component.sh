#!/usr/bin/env bash
set -euo pipefail

component_id="mock-component"
hold_seconds="${WAYBROKER_MOCK_HOLD_SECONDS:-30}"
exit_code="${WAYBROKER_MOCK_EXIT_CODE:-0}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --component)
      component_id="${2:?missing component id}"
      shift 2
      ;;
    --hold-seconds)
      hold_seconds="${2:?missing hold seconds}"
      shift 2
      ;;
    --exit-code)
      exit_code="${2:?missing exit code}"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

echo "mock-desktop-component id=$component_id pid=$$ hold_seconds=$hold_seconds exit_code=$exit_code"
sleep "$hold_seconds"
exit "$exit_code"
