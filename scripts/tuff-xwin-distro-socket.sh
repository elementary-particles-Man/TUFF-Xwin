#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
usage: tuff-xwin-distro-socket.sh [--emit-env]

Detect the supported major Linux distro family for TUFF-Xwin's host socket
and emit shell-compatible environment assignments.

Supported families:
  - Debian / Ubuntu
  - Fedora
  - RHEL / Rocky / Alma / CentOS Stream
EOF
}

contains_word() {
  local haystack="${1:-}"
  local needle="${2:-}"
  [[ " ${haystack} " == *" ${needle} "* ]]
}

load_os_release() {
  if [[ ! -r /etc/os-release ]]; then
    echo "missing /etc/os-release; cannot detect distro family" >&2
    exit 1
  fi

  # shellcheck disable=SC1091
  . /etc/os-release
}

detect_family() {
  local id="${ID:-unknown}"
  local like="${ID_LIKE:-}"

  case "$id" in
    debian|ubuntu|linuxmint|pop|elementary|kali|zorin)
      printf 'debian\n'
      return 0
      ;;
    fedora)
      printf 'fedora\n'
      return 0
      ;;
    rhel|centos|centos-stream|rocky|almalinux|ol)
      printf 'rhel\n'
      return 0
      ;;
  esac

  if contains_word "$like" debian || contains_word "$like" ubuntu; then
    printf 'debian\n'
    return 0
  fi

  if contains_word "$like" fedora; then
    printf 'fedora\n'
    return 0
  fi

  if contains_word "$like" rhel || contains_word "$like" centos; then
    printf 'rhel\n'
    return 0
  fi

  echo "unsupported distro family: ID=${id} ID_LIKE=${like:-none}" >&2
  echo "TUFF-Xwin major Linux socket supports Debian/Ubuntu, Fedora, and RHEL families only." >&2
  exit 1
}

detect_package_manager() {
  if command -v apt-get >/dev/null 2>&1; then
    printf 'apt-get\n'
    return 0
  fi
  if command -v dnf >/dev/null 2>&1; then
    printf 'dnf\n'
    return 0
  fi
  if command -v yum >/dev/null 2>&1; then
    printf 'yum\n'
    return 0
  fi
  printf 'unknown\n'
}

sanitize_id() {
  printf '%s' "$1" | tr -c 'A-Za-z0-9._-' '-'
}

detect_host_shell() {
  if command -v gnome-shell >/dev/null 2>&1; then
    printf 'gnome-shell --nested --wayland\n'
    return 0
  fi
  if command -v startplasma-wayland >/dev/null 2>&1; then
    printf 'startplasma-wayland\n'
    return 0
  fi
  if command -v weston >/dev/null 2>&1; then
    printf 'weston\n'
    return 0
  fi
  printf '\n'
}

detect_host_panel() {
  if command -v plasmashell >/dev/null 2>&1; then
    printf 'plasmashell\n'
    return 0
  fi
  if command -v xfce4-panel >/dev/null 2>&1; then
    printf 'xfce4-panel\n'
    return 0
  fi
  printf '\n'
}

detect_host_settingsd() {
  if command -v gnome-settings-daemon >/dev/null 2>&1; then
    printf 'gnome-settings-daemon\n'
    return 0
  fi
  if command -v kded6 >/dev/null 2>&1; then
    printf 'kded6\n'
    return 0
  fi
  if command -v xfsettingsd >/dev/null 2>&1; then
    printf 'xfsettingsd\n'
    return 0
  fi
  printf '\n'
}

emit_env() {
  local family="$1"
  local distro_id="$2"
  local package_manager="$3"
  local shell_cmd="$4"
  local panel_cmd="$5"
  local settingsd_cmd="$6"
  local session_id
  session_id="linux-$(sanitize_id "$distro_id")-host"

  printf 'TUFF_XWIN_SOCKET_FLAVOR=%q\n' "linux-systemd-user"
  printf 'TUFF_XWIN_DISTRO_FAMILY=%q\n' "$family"
  printf 'TUFF_XWIN_DISTRO_ID=%q\n' "$distro_id"
  printf 'TUFF_XWIN_PACKAGE_MANAGER=%q\n' "$package_manager"
  printf 'TUFF_XWIN_SESSION_INSTANCE_ID=%q\n' "$session_id"
  printf 'TUFF_XWIN_PROFILE=%q\n' "host-wayland"
  printf 'TUFF_XWIN_HOST_SHELL=%q\n' "$shell_cmd"
  printf 'TUFF_XWIN_HOST_PANEL=%q\n' "$panel_cmd"
  printf 'TUFF_XWIN_HOST_SETTINGSD=%q\n' "$settingsd_cmd"
}

main() {
  local mode="emit-env"

  while (($# > 0)); do
    case "$1" in
      --emit-env)
        mode="emit-env"
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

  load_os_release
  local family package_manager shell_cmd panel_cmd settingsd_cmd
  family="$(detect_family)"
  package_manager="$(detect_package_manager)"
  shell_cmd="$(detect_host_shell)"
  panel_cmd="$(detect_host_panel)"
  settingsd_cmd="$(detect_host_settingsd)"

  case "$mode" in
    emit-env)
      emit_env "$family" "${ID:-unknown}" "$package_manager" "$shell_cmd" "$panel_cmd" "$settingsd_cmd"
      ;;
  esac
}

main "$@"
