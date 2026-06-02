#!/usr/bin/env bash
set -euo pipefail

config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/tuff-xwin"
env_file="$config_dir/session.env"

if [[ ! -f "$env_file" ]]; then
  echo "missing $env_file; run install-user.sh first" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
. "$env_file"
set +a

profile_arg="${1:-}"
session_id="${TUFF_XWIN_SESSION_INSTANCE_ID:-linux-host}"
repo_root="${TUFF_XWIN_REPO_ROOT:?}"
profiles_dir="${TUFF_XWIN_PROFILES_DIR:-$repo_root/profiles}"
bin_dir="${TUFF_XWIN_PREFIX:-$HOME/.local/share/tuff-xwin}/bin"
runtime_dir="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker}"
active_profile_artifact="$runtime_dir/session-${session_id}-active-profile.json"
recover_log="$runtime_dir/recover-${session_id}.log"
recover_stamp="$runtime_dir/recover-${session_id}.stamp"

log() {
  printf '[tuff-xwin-recover] %s\n' "$*" | tee -a "$recover_log"
}

extract_profile_id() {
  local path=$1
  sed -n 's/^[[:space:]]*"id":[[:space:]]*"\([^"]*\)".*/\1/p' "$path" | head -n 1
}

detect_profile() {
  if [[ -n "$profile_arg" ]]; then
    printf '%s\n' "$profile_arg"
    return 0
  fi

  if [[ -f "$active_profile_artifact" ]]; then
    local detected
    detected="$(extract_profile_id "$active_profile_artifact" || true)"
    if [[ -n "$detected" ]]; then
      printf '%s\n' "$detected"
      return 0
    fi
  fi

  printf '%s\n' "${TUFF_XWIN_PROFILE:-host-wayland}"
}

wait_active() {
  local unit=$1
  local timeout=${2:-80}
  local i
  for ((i=0; i<timeout; i++)); do
    if [[ "$(systemctl --user is-active "$unit" 2>/dev/null || true)" == "active" ]]; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

profile="$(detect_profile)"

mkdir -p "$runtime_dir"
: > "$recover_log"
log "begin session=$session_id profile=$profile runtime_dir=$runtime_dir"

export PATH="$bin_dir:$HOME/.local/bin:$PATH"
export TUFF_XWIN_PROFILE="$profile"
export TUFF_XWIN_SESSION_INSTANCE_ID="$session_id"
export TUFF_XWIN_REPO_ROOT="$repo_root"
export TUFF_XWIN_PROFILES_DIR="$profiles_dir"

systemctl --user daemon-reload
systemctl --user reset-failed tuff-xwin.target \
  tuff-xwin-displayd.service \
  tuff-xwin-waylandd.service \
  tuff-xwin-lockd.service \
  tuff-xwin-watchdog.service \
  tuff-xwin-sessiond.service >/dev/null 2>&1 || true
systemctl --user import-environment \
  TUFF_XWIN_PROFILE TUFF_XWIN_SESSION_INSTANCE_ID TUFF_XWIN_REPO_ROOT TUFF_XWIN_PROFILES_DIR \
  TUFF_XWIN_SOCKET_FLAVOR TUFF_XWIN_DISTRO_FAMILY TUFF_XWIN_DISTRO_ID TUFF_XWIN_PACKAGE_MANAGER \
  TUFF_XWIN_DISPLAYD_ARGS TUFF_XWIN_WAYLANDD_ARGS TUFF_XWIN_LOCKD_ARGS TUFF_XWIN_WATCHDOG_ARGS TUFF_XWIN_SESSIOND_ARGS \
  TUFF_XWIN_HOST_SHELL TUFF_XWIN_HOST_PANEL TUFF_XWIN_HOST_SETTINGSD

sessiond --repo-root "$repo_root" \
  --profiles-dir "$profiles_dir" \
  --select-profile "$profile" \
  --write-selection \
  --session-instance-id "$session_id"

log "stopping existing tuff-xwin target"
systemctl --user stop tuff-xwin.target >/dev/null 2>&1 || true
rm -f "$runtime_dir"/*.sock
log "starting tuff-xwin target"
systemctl --user start tuff-xwin.target

wait_active tuff-xwin-displayd.service
wait_active tuff-xwin-waylandd.service
wait_active tuff-xwin-lockd.service
wait_active tuff-xwin-watchdog.service
wait_active tuff-xwin-sessiond.service

{
  printf 'session=%s\n' "$session_id"
  printf 'profile=%s\n' "$profile"
  printf 'timestamp=%s\n' "$(date -Iseconds)"
} > "$recover_stamp"

log "recovered profile=$profile session=$session_id"
printf 'Recovered profile=%s session=%s\n' "$profile" "$session_id"
