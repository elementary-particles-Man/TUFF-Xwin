#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

runtime_dir="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker-scene-recovery}"
displayd_socket="$runtime_dir/displayd.sock"
waylandd_socket="$runtime_dir/waylandd.sock"
snapshot_path="$runtime_dir/displayd-last-scene.json"
wayland_registry_snapshot="$runtime_dir/waylandd-surface-registry.json"
target_dir="/home/flux/.cache/tuff-xwin-target/debug"
scene_path="$repo_root/examples/minimal-scene/scene.json"
registry_path="$repo_root/examples/minimal-scene/surface-registry.json"

export WAYBROKER_RUNTIME_DIR="$runtime_dir"

echo "Running Waybroker scene recovery demo from $repo_root"
echo "Using runtime dir: $runtime_dir"

cleanup() {
  if [[ -n "${displayd_pid:-}" ]]; then
    kill "$displayd_pid" 2>/dev/null || true
    wait "$displayd_pid" 2>/dev/null || true
  fi
  if [[ -n "${waylandd_pid:-}" ]]; then
    kill "$waylandd_pid" 2>/dev/null || true
    wait "$waylandd_pid" 2>/dev/null || true
  fi
}

wait_for_socket() {
  local socket_path=$1

  for _ in $(seq 1 50); do
    if [[ -S "$socket_path" ]]; then
      return 0
    fi
    sleep 0.1
  done

  return 1
}

start_displayd() {
  "$target_dir/displayd" > "$runtime_dir/displayd.log" 2>&1 &
  displayd_pid=$!

  if ! wait_for_socket "$displayd_socket"; then
    echo "displayd socket did not appear: $displayd_socket" >&2
    cat "$runtime_dir/displayd.log" >&2
    exit 1
  fi
}

stop_displayd() {
  if [[ -n "${displayd_pid:-}" ]]; then
    kill "$displayd_pid" 2>/dev/null || true
    wait "$displayd_pid" 2>/dev/null || true
    unset displayd_pid
  fi
}

start_waylandd() {
  "$target_dir/waylandd" --serve-ipc --registry "$registry_path" > "$runtime_dir/waylandd.log" 2>&1 &
  waylandd_pid=$!

  if ! wait_for_socket "$waylandd_socket"; then
    echo "waylandd socket did not appear: $waylandd_socket" >&2
    cat "$runtime_dir/waylandd.log" >&2
    exit 1
  fi
}

trap cleanup EXIT

rm -rf "$runtime_dir"
mkdir -p "$runtime_dir"

echo
echo "==> cargo build -p displayd -p waylandd -p compd"
cargo build -p displayd -p waylandd -p compd

echo
echo "==> Starting displayd"
start_displayd

echo
echo "==> Starting waylandd surface registry broker"
start_waylandd

echo
echo "==> cargo run -p compd -- --scene $scene_path --commit-demo --require-displayd"
cargo run -p compd -- --scene "$scene_path" --commit-demo --require-displayd

if [[ ! -f "$snapshot_path" ]]; then
  echo "scene snapshot was not written: $snapshot_path" >&2
  exit 1
fi

echo
echo "==> Restarting displayd to verify snapshot reload"
stop_displayd
rm -f "$displayd_socket"
start_displayd

echo
echo "==> cargo run -p compd -- --restore-from-displayd --reconcile-waylandd --handoff-selection --print-scene --require-displayd --require-waylandd"
cargo run -p compd -- --restore-from-displayd --reconcile-waylandd --handoff-selection --print-scene --require-displayd --require-waylandd

if [[ ! -f "$wayland_registry_snapshot" ]]; then
  echo "waylandd runtime registry was not written: $wayland_registry_snapshot" >&2
  exit 1
fi

rg -q '"clipboard_owner": "terminal-1"' "$wayland_registry_snapshot"
rg -q '"primary_selection_owner": "terminal-1"' "$wayland_registry_snapshot"

echo
echo "==> waylandd runtime registry after handoff"
cat "$wayland_registry_snapshot"
