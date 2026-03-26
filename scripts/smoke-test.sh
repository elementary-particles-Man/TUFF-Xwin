#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export WAYBROKER_RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-smoke}"
mkdir -p "$WAYBROKER_RUNTIME_DIR"

echo "Running E2E smoke test in $WAYBROKER_RUNTIME_DIR"

cleanup() {
  echo "Cleaning up..."
  pkill -P $$ || true
  rm -rf "$WAYBROKER_RUNTIME_DIR"
}

trap cleanup EXIT

# 1. Start displayd
echo "==> Starting displayd"
cargo run -p displayd -- --once &
sleep 1

# 2. Verify waylandd can talk to displayd
echo "==> Running waylandd"
cargo run -p waylandd -- --require-displayd

# 3. Verify compd can commit scene
echo "==> Starting displayd again for compd"
cargo run -p displayd -- --once &
sleep 1
echo "==> Running compd --commit-demo"
cargo run -p compd -- --commit-demo

# 4. Verify lockd IPC
echo "==> Starting lockd"
cargo run -p lockd -- --serve-ipc --once &
sleep 1
echo "==> Sending lock command"
python3 -c 'import socket; s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect("'"$WAYBROKER_RUNTIME_DIR"'/lockd.sock"); s.sendall(b"{\"source\":\"sessiond\",\"destination\":\"lockd\",\"kind\":{\"kind\":\"lock-command\",\"payload\":{\"op\":\"set-lock-state\",\"state\":\"locked\"}}}\n")'

# 5. Verify sessiond profile loading
echo "==> Running sessiond --list-profiles"
cargo run -p sessiond -- --list-profiles

# 6. Verify watchdog health check logic
echo "==> Running sessiond to write launch state"
cargo run -p sessiond -- --select-profile demo-x11 --write-selection --launch-active
echo "==> Running watchdog inspection"
cargo run -p watchdog -- --profile-id demo-x11 --write-reports

# 7. Verify resume sequence orchestration
echo "==> Starting services for resume test"
cargo run -p displayd &
displayd_pid=$!
cargo run -p compd -- --serve-ipc --once &
compd_pid=$!
cargo run -p lockd -- --serve-ipc &
lockd_pid=$!
sleep 2

echo "==> Running sessiond --resume-demo"
cargo run -p sessiond -- --resume-demo

kill "$displayd_pid" "$lockd_pid" 2>/dev/null || true

echo "==> Smoke test PASSED"
