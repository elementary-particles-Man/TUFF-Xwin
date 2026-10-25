#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

runtime_dir="$(mktemp -d "${TMPDIR:-/tmp}/waybroker-rootless-demo.XXXXXX")"
export WAYBROKER_RUNTIME_DIR="$runtime_dir"
export DISPLAY=""
export WAYLAND_DISPLAY=""
displayd_socket="$runtime_dir/displayd.sock"
scene_path="$repo_root/LeyerX11/examples/minimal-rootless-scene.json"

cleanup() {
  if [[ -n "${displayd_pid:-}" ]] && kill -0 "$displayd_pid" 2>/dev/null; then
    kill "$displayd_pid" 2>/dev/null || true
    wait "$displayd_pid" 2>/dev/null || true
  fi
  rm -rf "$runtime_dir"
}

trap cleanup EXIT

echo "Running LeyerX11 rootless demo from $repo_root"
echo "Using runtime dir: $runtime_dir"

echo
echo "==> cargo run -p displayd -- --no-vulkan --once"
TUFF_XWIN_DISPLAY_BACKEND=mock cargo run -p displayd -- --no-vulkan --once &
displayd_pid=$!

for _ in $(seq 1 100); do
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
if ! timeout 30s cargo run -p x11bridge -- --scene "$scene_path" --commit-demo; then
  echo "rootless demo commit failed" >&2
  exit 1
fi

wait "$displayd_pid"
unset displayd_pid
