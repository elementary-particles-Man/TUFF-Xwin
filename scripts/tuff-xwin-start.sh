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

profile="${1:-${TUFF_XWIN_PROFILE:-host-wayland}}"
session_id="${TUFF_XWIN_SESSION_INSTANCE_ID:-debian-host}"
repo_root="${TUFF_XWIN_REPO_ROOT:?}"
profiles_dir="${TUFF_XWIN_PROFILES_DIR:-$repo_root/profiles}"
bin_dir="${TUFF_XWIN_PREFIX:-$HOME/.local/share/tuff-xwin}/bin"

export PATH="$bin_dir:$HOME/.local/bin:$PATH"
export TUFF_XWIN_PROFILE="$profile"
export TUFF_XWIN_SESSION_INSTANCE_ID="$session_id"
export TUFF_XWIN_REPO_ROOT="$repo_root"
export TUFF_XWIN_PROFILES_DIR="$profiles_dir"

sessiond --repo-root "$repo_root" \
  --profiles-dir "$profiles_dir" \
  --select-profile "$profile" \
  --write-selection \
  --session-instance-id "$session_id"

systemctl --user import-environment \
  TUFF_XWIN_PROFILE TUFF_XWIN_SESSION_INSTANCE_ID TUFF_XWIN_REPO_ROOT TUFF_XWIN_PROFILES_DIR \
  TUFF_XWIN_DISPLAYD_ARGS TUFF_XWIN_WAYLANDD_ARGS TUFF_XWIN_LOCKD_ARGS TUFF_XWIN_WATCHDOG_ARGS TUFF_XWIN_SESSIOND_ARGS \
  TUFF_XWIN_HOST_SHELL TUFF_XWIN_HOST_PANEL TUFF_XWIN_HOST_SETTINGSD

systemctl --user start tuff-xwin.target
