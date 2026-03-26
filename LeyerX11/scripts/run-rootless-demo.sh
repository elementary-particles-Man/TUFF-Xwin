#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

runtime_dir="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker}"
displayd_socket="$runtime_dir/displayd.sock"
scene_path="$repo_root/LeyerX11/examples/minimal-rootless-scene.json"

cleanup() {
  if [[ -n "${displayd_pid:-}" ]]; then
    kill "$displayd_pid" 2>/dev/null || true
    wait "$displayd_pid" 2>/dev/null || true
  fi
}

trap cleanup EXIT

rm -f "$displayd_socket"

echo "Running LeyerX11 rootless demo from $repo_root"
echo "Using runtime dir: $runtime_dir"

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
echo "==> cargo run -p x11bridge -- --scene $scene_path --commit-demo"
cargo run -p x11bridge -- --scene "$scene_path" --commit-demo

wait "$displayd_pid"
unset displayd_pid
