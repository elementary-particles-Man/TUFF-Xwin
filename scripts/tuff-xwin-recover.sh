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
session_id="${TUFF_XWIN_SESSION_INSTANCE_ID:-debian-host}"
repo_root="${TUFF_XWIN_REPO_ROOT:?}"
profiles_dir="${TUFF_XWIN_PROFILES_DIR:-$repo_root/profiles}"
bin_dir="${TUFF_XWIN_PREFIX:-$HOME/.local/share/tuff-xwin}/bin"
runtime_dir="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker}"
active_profile_artifact="$runtime_dir/session-${session_id}-active-profile.json"

detect_profile() {
  if [[ -n "$profile_arg" ]]; then
    printf '%s\n' "$profile_arg"
    return 0
  fi

  if [[ -f "$active_profile_artifact" ]]; then
    python3 - "$active_profile_artifact" <<'PY'
import json, sys
with open(sys.argv[1], 'r', encoding='utf-8') as fh:
    obj = json.load(fh)
print(obj.get('id', 'host-wayland'))
PY
    return 0
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

export PATH="$bin_dir:$HOME/.local/bin:$PATH"
export TUFF_XWIN_PROFILE="$profile"
export TUFF_XWIN_SESSION_INSTANCE_ID="$session_id"
export TUFF_XWIN_REPO_ROOT="$repo_root"
export TUFF_XWIN_PROFILES_DIR="$profiles_dir"

systemctl --user daemon-reload
systemctl --user import-environment \
  TUFF_XWIN_PROFILE TUFF_XWIN_SESSION_INSTANCE_ID TUFF_XWIN_REPO_ROOT TUFF_XWIN_PROFILES_DIR \
  TUFF_XWIN_DISPLAYD_ARGS TUFF_XWIN_WAYLANDD_ARGS TUFF_XWIN_LOCKD_ARGS TUFF_XWIN_WATCHDOG_ARGS TUFF_XWIN_SESSIOND_ARGS \
  TUFF_XWIN_HOST_SHELL TUFF_XWIN_HOST_PANEL TUFF_XWIN_HOST_SETTINGSD

sessiond --repo-root "$repo_root" \
  --profiles-dir "$profiles_dir" \
  --select-profile "$profile" \
  --write-selection \
  --session-instance-id "$session_id"

systemctl --user stop tuff-xwin.target >/dev/null 2>&1 || true
rm -f "$runtime_dir"/*.sock
systemctl --user start tuff-xwin.target

wait_active tuff-xwin-displayd.service
wait_active tuff-xwin-waylandd.service
wait_active tuff-xwin-lockd.service
wait_active tuff-xwin-watchdog.service
wait_active tuff-xwin-sessiond.service

printf 'Recovered profile=%s session=%s\n' "$profile" "$session_id"
