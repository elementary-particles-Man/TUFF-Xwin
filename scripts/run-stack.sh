#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

runtime_dir="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker}"
displayd_socket="$runtime_dir/displayd.sock"

packages=(
  compd
  lockd
  sessiond
  watchdog
)

echo "Running Waybroker stack stubs from $repo_root"
echo "Using runtime dir: $runtime_dir"

cleanup() {
  if [[ -n "${displayd_pid:-}" ]]; then
    kill "$displayd_pid" 2>/dev/null || true
    wait "$displayd_pid" 2>/dev/null || true
  fi
}

trap cleanup EXIT

rm -f "$displayd_socket"

echo
echo "==> cargo run -p displayd -- --once"
cargo run -p displayd -- --once &
displayd_pid=$!

for _ in $(seq 1 50); do
  if [[ -S "$displayd_socket" ]]; then
    break
  fi
  sleep 0.1
done

if [[ ! -S "$displayd_socket" ]]; then
  echo "displayd socket did not appear: $displayd_socket" >&2
  exit 1
fi

echo
echo "==> cargo run -p waylandd -- --require-displayd"
cargo run -p waylandd -- --require-displayd

echo
echo "==> Starting displayd for compd"
cargo run -p displayd -- --once &
displayd_pid=$!
sleep 1

echo
echo "==> cargo run -p compd -- --commit-demo --require-displayd"
cargo run -p compd -- --commit-demo --require-displayd

wait "$displayd_pid"
unset displayd_pid

echo
echo "==> cargo run -p lockd -- --serve-ipc --once"
cargo run -p lockd -- --serve-ipc --once &
sleep 1
python3 -c 'import socket; s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect("'"$runtime_dir"'/lockd.sock"); s.sendall(b"{\"source\":\"sessiond\",\"destination\":\"lockd\",\"kind\":{\"kind\":\"lock-command\",\"payload\":{\"op\":\"set-lock-state\",\"state\":\"locked\"}}}\n")'
wait $!

for pkg in sessiond watchdog; do
  echo
  echo "==> cargo run -p $pkg"
  cargo run -p "$pkg"
done
