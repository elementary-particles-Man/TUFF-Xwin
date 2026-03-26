#!/usr/bin/env bash
set -euo pipefail

component_id="mock-component"
hold_seconds="${WAYBROKER_MOCK_HOLD_SECONDS:-30}"

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
    *)
      echo "unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

echo "mock-desktop-component id=$component_id pid=$$ hold_seconds=$hold_seconds"
sleep "$hold_seconds"
