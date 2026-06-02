#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
session_env="${XDG_CONFIG_HOME:-$HOME/.config}/tuff-xwin/session.env"
profile="${TUFF_XWIN_PROFILE:-}"
allow_root="${TUFF_XWIN_BROWSER_ALLOW_ROOT:-0}"
allow_no_sandbox="${TUFF_XWIN_BROWSER_ALLOW_NO_SANDBOX:-0}"

if [[ -f "$session_env" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$session_env"
  set +a
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      profile="${2:-}"
      shift 2
      ;;
    --profile=*)
      profile="${1#*=}"
      shift
      ;;
    --allow-root)
      allow_root=1
      shift
      ;;
    --allow-no-sandbox)
      allow_no_sandbox=1
      shift
      ;;
    *)
      break
      ;;
  esac
done

browser_cmd=""
for candidate in chromium chromium-browser google-chrome-stable google-chrome brave-browser vivaldi; do
  if command -v "$candidate" >/dev/null 2>&1; then
    browser_cmd="$candidate"
    break
  fi
done

if [[ -z "$browser_cmd" ]]; then
  printf '%s\n' 'FAIL: browser command missing; package layer must install chromium/chromium-sandbox or an equivalent browser package' >&2
  exit 1
fi

if [[ "$(id -u)" -eq 0 && "$allow_root" != "1" ]]; then
  printf '%s\n' 'FAIL: refusing to run Chromium family browsers as root; set TUFF_XWIN_BROWSER_ALLOW_ROOT=1 or pass --allow-root only for diagnostics' >&2
  exit 1
fi

if [[ "${1:-}" == "--version" ]]; then
  exec "$browser_cmd" --version
fi

if [[ -n "${XDG_RUNTIME_DIR:-}" ]]; then
  if [[ ! -d "$XDG_RUNTIME_DIR" || ! -O "$XDG_RUNTIME_DIR" ]]; then
    printf '%s\n' "FAIL: XDG_RUNTIME_DIR is missing or not owned by the current user: ${XDG_RUNTIME_DIR}" >&2
    exit 1
  fi
else
  printf '%s\n' 'FAIL: XDG_RUNTIME_DIR is not set; start the TUFF-Xwin session first' >&2
  exit 1
fi

if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then
  if command -v dbus-run-session >/dev/null 2>&1; then
    printf '%s\n' 'WARN: DBUS_SESSION_BUS_ADDRESS is missing; consider dbus-run-session or systemd --user environment import' >&2
  else
    printf '%s\n' 'WARN: DBUS_SESSION_BUS_ADDRESS is missing and dbus-run-session is unavailable' >&2
  fi
fi

if [[ -z "${WAYLAND_DISPLAY:-}" && -z "${DISPLAY:-}" ]]; then
  printf '%s\n' 'FAIL: neither WAYLAND_DISPLAY nor DISPLAY is present; start a TUFF-Xwin profile first' >&2
  exit 1
fi

browser_bin="$(command -v "$browser_cmd")"
browser_dir="$(dirname "$browser_bin")"
browser_name="$(basename "$browser_bin")"

sandbox_candidates=(
  "$browser_dir/chromium-sandbox"
  "$browser_dir/chrome-sandbox"
  /usr/lib/chromium/chromium-sandbox
  /usr/lib/chromium/chrome-sandbox
  /opt/google/chrome/chrome-sandbox
)

sandbox_found=""
for candidate in "${sandbox_candidates[@]}"; do
  if [[ -e "$candidate" ]]; then
    sandbox_found="$candidate"
    break
  fi
done

userns_value=""
if command -v sysctl >/dev/null 2>&1; then
  userns_value="$(sysctl -n kernel.unprivileged_userns_clone 2>/dev/null || true)"
fi

if [[ "$browser_name" == chromium || "$browser_name" == chromium-browser || "$browser_name" == google-chrome-stable || "$browser_name" == google-chrome || "$browser_name" == brave-browser || "$browser_name" == vivaldi ]]; then
  if [[ -n "$sandbox_found" && -u "$sandbox_found" && -x "$sandbox_found" ]]; then
    printf '%s\n' "PASS: sandbox helper looks usable: $sandbox_found"
  elif [[ "$allow_no_sandbox" == "1" ]]; then
    printf '%s\n' 'WARN: sandbox helper looks broken; launching with --no-sandbox because an explicit diagnostic override was requested'
  elif [[ "$userns_value" == "1" ]]; then
    printf '%s\n' 'PASS: user namespace support is enabled and will be used for the sandbox'
  else
    printf '%s\n' 'FAIL: sandbox looks broken; neither user namespaces nor a usable sandbox helper are available. Install chromium-sandbox, fix helper permissions, or enable kernel.unprivileged_userns_clone.' >&2
    exit 1
  fi
fi

if [[ "$browser_cmd" == chromium || "$browser_cmd" == chromium-browser || "$browser_cmd" == google-chrome-stable || "$browser_cmd" == google-chrome || "$browser_cmd" == brave-browser || "$browser_cmd" == vivaldi ]]; then
  if [[ -n "${WAYLAND_DISPLAY:-}" && -z "${DISPLAY:-}" ]]; then
    ozone_args=(--ozone-platform=wayland --enable-features=UseOzonePlatform)
  elif [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
    ozone_args=(--ozone-platform=wayland --enable-features=UseOzonePlatform)
  else
    ozone_args=(--ozone-platform=x11)
  fi
else
  ozone_args=()
fi

if [[ "$allow_no_sandbox" == "1" ]]; then
  ozone_args+=(--no-sandbox)
fi

if [[ "$browser_cmd" == chromium || "$browser_cmd" == chromium-browser || "$browser_cmd" == google-chrome-stable || "$browser_cmd" == google-chrome || "$browser_cmd" == brave-browser || "$browser_cmd" == vivaldi ]]; then
  if [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
    printf '%s\n' "PASS: preferring Wayland for $browser_cmd"
  elif [[ -n "${DISPLAY:-}" ]]; then
    printf '%s\n' "PASS: using XWayland/X11 fallback for $browser_cmd"
  fi
fi

exec "$browser_cmd" "${ozone_args[@]}" "$@"
