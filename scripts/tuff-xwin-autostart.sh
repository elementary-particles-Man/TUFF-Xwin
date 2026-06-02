#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${DISPLAY:-}" || -n "${WAYLAND_DISPLAY:-}" ]]; then
  exit 0
fi

if [[ ! -t 0 ]]; then
  exit 0
fi

if [[ "${XDG_VTNR:-}" != "1" ]]; then
  exit 0
fi

exec "$HOME/.local/bin/tuff-xwin-recover" "${TUFF_XWIN_PROFILE:-host-wayland}"
