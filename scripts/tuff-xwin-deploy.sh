#!/usr/bin/env bash
set -euo pipefail

# TUFF-Xwin Unified Deploy & Takeover Script
# This script handles installation and the aggressive takeover of hardware resources
# from existing desktop environments (KDE, GNOME, etc.)

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/tuff-xwin"
env_file="$config_dir/session.env"
runtime_base="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
unit_dir="$runtime_base/systemd/user"
runtime_dir="${WAYBROKER_RUNTIME_DIR:-$runtime_base/waybroker}"

log() { printf '\e[32m[deploy]\e[0m %s\n' "$*"; }
warn() { printf '\e[33m[warn]\e[0m %s\n' "$*" >&2; }
die() { printf '\e[31m[error]\e[0m %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<EOF
usage: $0 <command> [options]

Commands:
  install    Build and install TUFF-Xwin to ~/.local/share/tuff-xwin
  takeover   Aggressively take control of DRM/KMS from existing DE
  status     Show current process and resource state

Options:
  --profile ID     Profile to select (default: host-wayland)
  --retries N      Number of retries for DRM takeover (default: 5)
EOF
}

# --- Detection ---

detect_de() {
  if [[ -n "${XDG_CURRENT_DESKTOP:-}" ]]; then
    echo "${XDG_CURRENT_DESKTOP,,}"
  elif pgrep -x kwin_wayland >/dev/null; then
    echo "kde"
  elif pgrep -x gnome-shell >/dev/null; then
    echo "gnome"
  else
    echo "unknown"
  fi
}

# --- Commands ---

cmd_install() {
  log "Starting installation..."
  bash "$repo_root/scripts/install-user.sh"
  log "Installation complete."
}

cmd_takeover() {
  local de
  de=$(detect_de)
  log "Detected environment: $de"

  local units_to_mask=()
  case "$de" in
    *kde*|*plasma*)
      units_to_mask=(
        plasma-kwin_wayland.service plasma-plasmashell.service 
        plasma-workspace.target plasma-core.target
      )
      ;;
    *gnome*)
      units_to_mask=(
        org.gnome.Shell.service org.gnome.Shell.target
        gnome-session-wayland@*.service
      )
      ;;
  esac

  if [[ ${#units_to_mask[@]} -gt 0 ]]; then
    log "Masking units: ${units_to_mask[*]}"
    systemctl --user mask --runtime "${units_to_mask[@]}" || true
    systemctl --user stop "${units_to_mask[@]}" || true
  fi

  log "Cleaning up Wayland locks..."
  rm -f "$runtime_base"/wayland-*.lock

  log "Starting TUFF-Xwin current-session stack..."
  bash "$repo_root/scripts/tuff-xwin-current-session.sh" start

  # Aggressive retry loop for displayd to get DRM master
  local max_retries=${RETRIES:-5}
  local count=0
  while [[ $count -lt $max_retries ]]; do
    if systemctl --user is-active tuff-xwin-current-session-displayd.service >/dev/null; then
      log "displayd is active and should have DRM master."
      break
    fi
    warn "displayd not active yet, retrying in 2s... ($((count+1))/$max_retries)"
    systemctl --user restart tuff-xwin-current-session-displayd.service || true
    sleep 2
    ((count++))
  done

  if [[ $count -eq $max_retries ]]; then
    die "Failed to take over DRM after $max_retries retries. Check journalctl -n 50 --user -u tuff-xwin-current-session-displayd.service"
  fi

  log "Takeover successful!"
}

cmd_status() {
  bash "$repo_root/scripts/tuff-xwin-current-session.sh" status
}

# --- Main ---

if [[ $# -lt 1 ]]; then
  usage
  exit 1
fi

COMMAND="$1"
shift

RETRIES=5
PROFILE="host-wayland"

while (($# > 0)); do
  case "$1" in
    --retries) shift; RETRIES="$1" ;;
    --profile) shift; PROFILE="$1" ;;
    *) die "Unknown option: $1" ;;
  esac
  shift
done

case "$COMMAND" in
  install) cmd_install ;;
  takeover) export TUFF_XWIN_PROFILE="$PROFILE"; cmd_takeover ;;
  status) cmd_status ;;
  *) usage; exit 1 ;;
esac
