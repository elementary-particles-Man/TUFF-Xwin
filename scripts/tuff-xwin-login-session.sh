#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
usage: tuff-xwin-login-session [--profile ID] [--select]

Display-manager oriented TUFF-Xwin session entrypoint.
EOF
}

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

profile_arg=""
select_profile=0

while (($# > 0)); do
  case "$1" in
    --profile)
      shift
      profile_arg="${1:-}"
      if [[ -z "$profile_arg" ]]; then
        echo "--profile requires an id" >&2
        exit 1
      fi
      ;;
    --select)
      select_profile=1
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

recover_cmd="$HOME/.local/bin/tuff-xwin-recover"
selector_cmd="$HOME/.local/bin/tuff-xwin-select-profile"

if [[ ! -x "$recover_cmd" ]]; then
  echo "missing $recover_cmd; run install-user.sh first" >&2
  exit 1
fi

profile="${profile_arg:-${TUFF_XWIN_PROFILE:-host-wayland}}"
if [[ $select_profile -eq 1 || ( -z "$profile_arg" && "${TUFF_XWIN_PROFILE_PROMPT:-0}" == "1" ) ]]; then
  if [[ ! -x "$selector_cmd" ]]; then
    echo "missing $selector_cmd; run install-user.sh first" >&2
    exit 1
  fi
  profile="$("$selector_cmd" --choose --default "$profile")"
fi

cleanup() {
  systemctl --user stop tuff-xwin.target >/dev/null 2>&1 || true
}

trap cleanup EXIT HUP INT TERM

"$recover_cmd" "$profile"

while systemctl --user is-active --quiet tuff-xwin-sessiond.service; do
  sleep 2
done
