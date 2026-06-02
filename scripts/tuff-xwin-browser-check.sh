#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
session_env="${XDG_CONFIG_HOME:-$HOME/.config}/tuff-xwin/session.env"
profile="${TUFF_XWIN_PROFILE:-}"

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
    *)
      break
      ;;
  esac
done

check_status=0

report() {
  local level="$1"
  local message="$2"
  printf '%s: %s\n' "$level" "$message"
}

mark_fail() {
  report "FAIL" "$1"
  check_status=1
}

mark_warn() {
  report "WARN" "$1"
}

mark_pass() {
  report "PASS" "$1"
}

browser_cmd=""
for candidate in chromium chromium-browser google-chrome-stable google-chrome brave-browser vivaldi; do
  if command -v "$candidate" >/dev/null 2>&1; then
    browser_cmd="$candidate"
    break
  fi
done

if [[ -z "$browser_cmd" ]]; then
  mark_fail "browser command missing; package layer must install chromium/chromium-sandbox or an equivalent browser package"
else
  mark_pass "browser command detected: $browser_cmd"
  mark_pass "browser version: $("$browser_cmd" --version 2>&1 | tr '\n' ' ')"
fi

if [[ -n "${XDG_RUNTIME_DIR:-}" && -d "$XDG_RUNTIME_DIR" && -O "$XDG_RUNTIME_DIR" ]]; then
  mark_pass "XDG_RUNTIME_DIR exists and is owned by the current user: $XDG_RUNTIME_DIR"
else
  mark_fail "XDG_RUNTIME_DIR is missing or not owned by the current user; start the TUFF-Xwin session first"
fi

if [[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then
  mark_pass "DBUS_SESSION_BUS_ADDRESS is present"
elif command -v dbus-run-session >/dev/null 2>&1; then
  mark_warn "DBUS_SESSION_BUS_ADDRESS is missing, but dbus-run-session is available"
else
  mark_fail "DBUS_SESSION_BUS_ADDRESS is missing and dbus-run-session is not available; recommend dbus-run-session or systemd --user environment import"
fi

if [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
  mark_pass "WAYLAND_DISPLAY is present: $WAYLAND_DISPLAY"
else
  mark_warn "WAYLAND_DISPLAY is not set"
fi

if [[ -n "${DISPLAY:-}" ]]; then
  mark_pass "DISPLAY is present: $DISPLAY"
else
  mark_warn "DISPLAY is not set"
fi

if [[ -n "${WAYLAND_DISPLAY:-}" || -n "${DISPLAY:-}" ]]; then
  mark_pass "a display endpoint is available"
else
  mark_fail "no display endpoint is present; start a TUFF-Xwin profile first"
fi

if command -v xdg-desktop-portal >/dev/null 2>&1; then
  mark_pass "xdg-desktop-portal is available"
else
  mark_fail "xdg-desktop-portal is missing; install the portal stack from the distro layer"
fi

if [[ "${profile:-}" == host-kde-chromium* || "${XDG_CURRENT_DESKTOP:-}" == *KDE* || "${XDG_CURRENT_DESKTOP:-}" == *Plasma* ]]; then
  if command -v xdg-desktop-portal-kde >/dev/null 2>&1; then
    mark_pass "xdg-desktop-portal-kde is available for KDE profile usage"
  else
    mark_fail "xdg-desktop-portal-kde is missing for the KDE profile"
  fi
else
  mark_warn "KDE portal check skipped because the active profile is not KDE-oriented"
fi

if [[ -d /dev/shm && -w /dev/shm ]]; then
  mark_pass "/dev/shm exists and is writable"
else
  mark_fail "/dev/shm is missing or not writable"
fi

if command -v sysctl >/dev/null 2>&1; then
  userns_value="$(sysctl -n kernel.unprivileged_userns_clone 2>/dev/null || true)"
  if [[ "$userns_value" == "1" ]]; then
    mark_pass "kernel.unprivileged_userns_clone is enabled"
  elif [[ "$userns_value" == "0" ]]; then
    mark_warn "kernel.unprivileged_userns_clone is disabled; Chromium may need a working setuid sandbox helper"
  else
    mark_warn "kernel.unprivileged_userns_clone is not readable on this host"
  fi
else
  mark_warn "sysctl is unavailable; cannot inspect kernel.unprivileged_userns_clone"
fi

if [[ -n "$browser_cmd" ]]; then
  browser_bin="$(command -v "$browser_cmd")"
  browser_name="$(basename "$browser_bin")"
  browser_dir="$(dirname "$browser_bin")"

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

  if [[ "$browser_name" == chromium || "$browser_name" == chromium-browser || "$browser_name" == google-chrome-stable || "$browser_name" == google-chrome || "$browser_name" == brave-browser || "$browser_name" == vivaldi ]]; then
    if [[ -n "$sandbox_found" && -u "$sandbox_found" && -x "$sandbox_found" ]]; then
      mark_pass "sandbox helper looks usable: $sandbox_found"
    elif [[ -n "${userns_value:-}" && "$userns_value" == "1" ]]; then
      mark_warn "sandbox helper is not present or not setuid; user namespace support is expected to carry the sandbox"
    else
      mark_fail "sandbox looks broken: neither user namespaces nor a usable sandbox helper are available; install chromium-sandbox or enable kernel.unprivileged_userns_clone"
    fi
  fi

  if timeout 15s "$browser_cmd" --headless --disable-gpu --dump-dom about:blank >/dev/null 2>&1; then
    mark_pass "headless smoke succeeded"
  else
    mark_fail "headless smoke failed; check sandbox permissions, DBus, and browser package integrity"
  fi

  if [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
    if timeout 15s "$browser_cmd" --ozone-platform=wayland --headless --disable-gpu --dump-dom about:blank >/dev/null 2>&1; then
      mark_pass "Wayland smoke succeeded"
    else
      mark_fail "Wayland smoke failed; confirm Wayland session support and xdg-desktop-portal availability"
    fi
  else
    mark_warn "Wayland smoke skipped because WAYLAND_DISPLAY is not present"
  fi
fi

exit "$check_status"
