#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
usage: tuff-xwin-select-profile [--list] [--choose] [--write-selection] [--default ID] [--include-demo]

List or choose a TUFF-Xwin desktop profile.

Defaults:
  - action: --list
  - filter: non-demo profiles only
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

repo_root="${TUFF_XWIN_REPO_ROOT:?}"
profiles_dir="${TUFF_XWIN_PROFILES_DIR:-$repo_root/profiles}"
bin_dir="${TUFF_XWIN_PREFIX:-$HOME/.local/share/tuff-xwin}/bin"
session_id="${TUFF_XWIN_SESSION_INSTANCE_ID:-linux-host}"

export PATH="$bin_dir:$HOME/.local/bin:$PATH"

action="list"
write_selection=0
include_demo=0
default_profile=""

while (($# > 0)); do
  case "$1" in
    --list)
      action="list"
      ;;
    --choose)
      action="choose"
      ;;
    --write-selection)
      write_selection=1
      ;;
    --default)
      shift
      default_profile="${1:-}"
      if [[ -z "$default_profile" ]]; then
        echo "--default requires a profile id" >&2
        exit 1
      fi
      ;;
    --include-demo)
      include_demo=1
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

profile_rows() {
  sessiond \
    --repo-root "$repo_root" \
    --profiles-dir "$profiles_dir" \
    --list-profiles \
    --session-instance-id "$session_id" |
    sed -n 's/^service=sessiond op=profile_entry id=\([^ ]*\) protocol=\([^ ]*\) name="\([^"]*\)" summary="\([^"]*\)"/\1\t\2\t\3\t\4/p'
}

declare -a PROFILE_IDS=()
declare -a PROFILE_PROTOCOLS=()
declare -a PROFILE_NAMES=()
declare -a PROFILE_SUMMARIES=()

while IFS=$'\t' read -r id protocol display_name summary; do
  [[ -n "$id" ]] || continue
  if [[ $include_demo -ne 1 && "$id" == demo-* ]]; then
    continue
  fi
  PROFILE_IDS+=("$id")
  PROFILE_PROTOCOLS+=("$protocol")
  PROFILE_NAMES+=("$display_name")
  PROFILE_SUMMARIES+=("$summary")
done < <(profile_rows)

if [[ ${#PROFILE_IDS[@]} -eq 0 ]]; then
  echo "no selectable profiles found in $profiles_dir" >&2
  exit 1
fi

profile_exists() {
  local needle=$1
  local id
  for id in "${PROFILE_IDS[@]}"; do
    if [[ "$id" == "$needle" ]]; then
      return 0
    fi
  done
  return 1
}

emit_list() {
  local i
  for ((i = 0; i < ${#PROFILE_IDS[@]}; i++)); do
    printf '%s\t%s\t%s\t%s\n' \
      "${PROFILE_IDS[$i]}" \
      "${PROFILE_PROTOCOLS[$i]}" \
      "${PROFILE_NAMES[$i]}" \
      "${PROFILE_SUMMARIES[$i]}"
  done
}

choose_with_whiptail() {
  local default_id=$1
  local -a menu=()
  local i
  for ((i = 0; i < ${#PROFILE_IDS[@]}; i++)); do
    menu+=(
      "${PROFILE_IDS[$i]}"
      "${PROFILE_NAMES[$i]} [${PROFILE_PROTOCOLS[$i]}]"
    )
  done

  local selected=""
  selected="$(
    whiptail \
      --title "TUFF-Xwin Profile Select" \
      --menu "起動する profile を選んでください" \
      20 100 10 \
      "${menu[@]}" \
      3>&1 1>&2 2>&3
  )" || return 1

  if [[ -z "$selected" && -n "$default_id" ]]; then
    selected="$default_id"
  fi

  printf '%s\n' "$selected"
}

choose_with_select() {
  local default_id=$1
  local i

  {
    echo "TUFF-Xwin profile を選択してください。"
    if [[ -n "$default_id" ]]; then
      echo "Enter だけでは進まないため、番号を入力してください。既定候補: $default_id"
    fi
    echo
    for ((i = 0; i < ${#PROFILE_IDS[@]}; i++)); do
      printf '%2d) %s [%s]\n' \
        "$((i + 1))" \
        "${PROFILE_NAMES[$i]}" \
        "${PROFILE_IDS[$i]}"
    done
  } >&2

  while true; do
    printf '番号> ' >&2
    IFS= read -r choice || return 1

    if [[ -z "$choice" && -n "$default_id" ]]; then
      if profile_exists "$default_id"; then
        printf '%s\n' "$default_id"
        return 0
      fi
    fi

    if [[ "$choice" =~ ^[0-9]+$ ]] && ((choice >= 1 && choice <= ${#PROFILE_IDS[@]})); then
      printf '%s\n' "${PROFILE_IDS[$((choice - 1))]}"
      return 0
    fi

    echo "無効な入力です。" >&2
  done
}

selected_profile=""

case "$action" in
  list)
    emit_list
    exit 0
    ;;
  choose)
    if [[ -n "$default_profile" ]] && ! profile_exists "$default_profile"; then
      echo "default profile not found: $default_profile" >&2
      exit 1
    fi

    if command -v whiptail >/dev/null 2>&1 && [[ -t 0 && -t 1 ]]; then
      selected_profile="$(choose_with_whiptail "$default_profile")"
    elif [[ -t 0 && -t 1 ]]; then
      selected_profile="$(choose_with_select "$default_profile")"
    elif [[ -n "$default_profile" ]]; then
      selected_profile="$default_profile"
    else
      echo "interactive selection requires a tty or whiptail; pass --default for non-interactive fallback" >&2
      exit 1
    fi
    ;;
esac

if [[ -z "$selected_profile" ]]; then
  echo "profile selection failed" >&2
  exit 1
fi

if [[ $write_selection -eq 1 ]]; then
  sessiond \
    --repo-root "$repo_root" \
    --profiles-dir "$profiles_dir" \
    --select-profile "$selected_profile" \
    --write-selection \
    --session-instance-id "$session_id" >/dev/null
fi

printf '%s\n' "$selected_profile"
