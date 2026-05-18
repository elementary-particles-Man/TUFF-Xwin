#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/tuff-xwin"
env_file="$config_dir/session.env"
runtime_base="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
unit_dir="$runtime_base/systemd/user"
runtime_dir="${WAYBROKER_RUNTIME_DIR:-$runtime_base/waybroker}"
target="tuff-xwin-current-session.target"
session_id="current-session"
profile="host-wayland"
runtime_mask=0

usage() {
  cat <<'EOF'
usage: tuff-xwin-current-session.sh [options] <command>

Commands:
  start      Create transient user units and start TUFF-Xwin in this session
  status     Show TUFF-Xwin and Plasma/KWin process state
  takeover   Stop Plasma/KWin after TUFF-Xwin is active
  unmask-plasma
             Remove runtime Plasma/KWin masks created for takeover
  stop       Stop the transient TUFF-Xwin current-session target
  cleanup    Stop target, remove transient units, and daemon-reload

Options:
  --profile ID              Profile to select before start (default: host-wayland)
  --session-instance-id ID  Session instance id (default: current-session)
  --runtime-mask-plasma     During takeover, runtime-mask Plasma/KWin units too
  -h, --help                Show this help
EOF
}

if [[ -f "$env_file" ]]; then
  set -a
  # shellcheck disable=SC1090
  . "$env_file"
  set +a
fi

profile="${TUFF_XWIN_PROFILE:-$profile}"
session_id="${TUFF_XWIN_SESSION_INSTANCE_ID:-$session_id}"
profiles_dir="${TUFF_XWIN_PROFILES_DIR:-$repo_root/profiles}"
prefix="${TUFF_XWIN_PREFIX:-$HOME/.local/share/tuff-xwin}"
release_dir="${CARGO_TARGET_DIR:-/home/flux/.cache/tuff-xwin-target}/release"
bin_path="$prefix/bin:$release_dir:/usr/local/bin:/usr/bin:/bin:$HOME/.local/bin"

log() {
  printf '[tuff-xwin-current-session] %s\n' "$*"
}

die() {
  printf '[tuff-xwin-current-session] %s\n' "$*" >&2
  exit 1
}

need_systemd_user() {
  command -v systemctl >/dev/null 2>&1 || die "systemctl is required"
  systemctl --user show-environment >/dev/null 2>&1 || die "systemd --user is not available"
}

write_unit() {
  local path=$1
  shift
  cat > "$path" <<EOF
$*
EOF
}

write_units() {
  mkdir -p "$unit_dir" "$runtime_dir"

  write_unit "$unit_dir/$target" "[Unit]
Description=TUFF-Xwin current-session runtime target
Wants=tuff-xwin-current-session-displayd.service tuff-xwin-current-session-waylandd.service tuff-xwin-current-session-lockd.service tuff-xwin-current-session-watchdog.service tuff-xwin-current-session-sessiond.service
"

  write_unit "$unit_dir/tuff-xwin-current-session-displayd.service" "[Unit]
Description=TUFF-Xwin current-session display broker
PartOf=$target

[Service]
Type=simple
Environment=PATH=$bin_path
EnvironmentFile=-%h/.config/tuff-xwin/session.env
Environment=WAYBROKER_RUNTIME_DIR=$runtime_dir
Environment=TUFF_XWIN_SESSION_INSTANCE_ID=$session_id
ExecStart=/usr/bin/env displayd \$TUFF_XWIN_DISPLAYD_ARGS --session-instance-id \${TUFF_XWIN_SESSION_INSTANCE_ID}
Restart=on-failure
RestartSec=1
"

  write_unit "$unit_dir/tuff-xwin-current-session-waylandd.service" "[Unit]
Description=TUFF-Xwin current-session Wayland broker
PartOf=$target
After=tuff-xwin-current-session-displayd.service
Requires=tuff-xwin-current-session-displayd.service

[Service]
Type=simple
Environment=PATH=$bin_path
EnvironmentFile=-%h/.config/tuff-xwin/session.env
Environment=WAYBROKER_RUNTIME_DIR=$runtime_dir
Environment=TUFF_XWIN_SESSION_INSTANCE_ID=$session_id
ExecStart=/usr/bin/env waylandd --serve-ipc --require-displayd \$TUFF_XWIN_WAYLANDD_ARGS --session-instance-id \${TUFF_XWIN_SESSION_INSTANCE_ID}
Restart=on-failure
RestartSec=1
"

  write_unit "$unit_dir/tuff-xwin-current-session-lockd.service" "[Unit]
Description=TUFF-Xwin current-session lock broker
PartOf=$target

[Service]
Type=simple
Environment=PATH=$bin_path
EnvironmentFile=-%h/.config/tuff-xwin/session.env
Environment=WAYBROKER_RUNTIME_DIR=$runtime_dir
ExecStart=/usr/bin/env lockd --serve-ipc \$TUFF_XWIN_LOCKD_ARGS
Restart=on-failure
RestartSec=1
"

  write_unit "$unit_dir/tuff-xwin-current-session-watchdog.service" "[Unit]
Description=TUFF-Xwin current-session watchdog
PartOf=$target

[Service]
Type=simple
Environment=PATH=$bin_path
EnvironmentFile=-%h/.config/tuff-xwin/session.env
Environment=WAYBROKER_RUNTIME_DIR=$runtime_dir
ExecStart=/usr/bin/env watchdog --serve-ipc \$TUFF_XWIN_WATCHDOG_ARGS
Restart=on-failure
RestartSec=1
"

  write_unit "$unit_dir/tuff-xwin-current-session-sessiond.service" "[Unit]
Description=TUFF-Xwin current-session session supervisor
PartOf=$target
After=tuff-xwin-current-session-displayd.service tuff-xwin-current-session-waylandd.service tuff-xwin-current-session-lockd.service tuff-xwin-current-session-watchdog.service
Requires=tuff-xwin-current-session-displayd.service tuff-xwin-current-session-waylandd.service tuff-xwin-current-session-lockd.service tuff-xwin-current-session-watchdog.service

[Service]
Type=simple
Environment=PATH=$bin_path
EnvironmentFile=-%h/.config/tuff-xwin/session.env
Environment=WAYBROKER_RUNTIME_DIR=$runtime_dir
Environment=TUFF_XWIN_PROFILE=$profile
Environment=TUFF_XWIN_SESSION_INSTANCE_ID=$session_id
Environment=TUFF_XWIN_REPO_ROOT=$repo_root
Environment=TUFF_XWIN_PROFILES_DIR=$profiles_dir
ExecStartPre=/usr/bin/env sessiond --repo-root \${TUFF_XWIN_REPO_ROOT} --profiles-dir \${TUFF_XWIN_PROFILES_DIR} --select-profile \${TUFF_XWIN_PROFILE} --write-selection --session-instance-id \${TUFF_XWIN_SESSION_INSTANCE_ID}
ExecStart=/usr/bin/env sessiond --repo-root \${TUFF_XWIN_REPO_ROOT} --profiles-dir \${TUFF_XWIN_PROFILES_DIR} --serve-ipc --manage-active --spawn-components --notify-watchdog --session-instance-id \${TUFF_XWIN_SESSION_INSTANCE_ID} \$TUFF_XWIN_SESSIOND_ARGS
Restart=on-failure
RestartSec=1
"
}

import_environment() {
  export TUFF_XWIN_PROFILE="$profile"
  export TUFF_XWIN_SESSION_INSTANCE_ID="$session_id"
  export TUFF_XWIN_REPO_ROOT="$repo_root"
  export TUFF_XWIN_PROFILES_DIR="$profiles_dir"
  export WAYBROKER_RUNTIME_DIR="$runtime_dir"
  export TUFF_XWIN_DISPLAYD_ARGS="${TUFF_XWIN_DISPLAYD_ARGS:-}"
  export TUFF_XWIN_WAYLANDD_ARGS="${TUFF_XWIN_WAYLANDD_ARGS:-}"
  export TUFF_XWIN_LOCKD_ARGS="${TUFF_XWIN_LOCKD_ARGS:-}"
  export TUFF_XWIN_WATCHDOG_ARGS="${TUFF_XWIN_WATCHDOG_ARGS:-}"
  export TUFF_XWIN_SESSIOND_ARGS="${TUFF_XWIN_SESSIOND_ARGS:-}"
  export TUFF_XWIN_HOST_SHELL="${TUFF_XWIN_HOST_SHELL:-}"
  export TUFF_XWIN_HOST_PANEL="${TUFF_XWIN_HOST_PANEL:-}"
  export TUFF_XWIN_HOST_SETTINGSD="${TUFF_XWIN_HOST_SETTINGSD:-}"

  systemctl --user import-environment \
    TUFF_XWIN_PROFILE TUFF_XWIN_SESSION_INSTANCE_ID TUFF_XWIN_REPO_ROOT TUFF_XWIN_PROFILES_DIR \
    WAYBROKER_RUNTIME_DIR TUFF_XWIN_DISPLAYD_ARGS TUFF_XWIN_WAYLANDD_ARGS TUFF_XWIN_LOCKD_ARGS \
    TUFF_XWIN_WATCHDOG_ARGS TUFF_XWIN_SESSIOND_ARGS TUFF_XWIN_HOST_SHELL TUFF_XWIN_HOST_PANEL \
    TUFF_XWIN_HOST_SETTINGSD
}

current_session_units=(
  "$target"
  tuff-xwin-current-session-displayd.service
  tuff-xwin-current-session-waylandd.service
  tuff-xwin-current-session-lockd.service
  tuff-xwin-current-session-watchdog.service
  tuff-xwin-current-session-sessiond.service
)

plasma_units=(
  plasma-plasmashell.service
  plasma-kwin_wayland.service
  plasma-kwin_x11.service
  app-org.kde.xwaylandvideobridge@autostart.service
  plasma-workspace-wayland.target
  plasma-workspace-x11.target
  plasma-workspace.target
  plasma-core.target
)

assert_tuff_active() {
  local failed=0
  local unit
  for unit in "${current_session_units[@]}"; do
    if [[ "$(systemctl --user is-active "$unit" 2>/dev/null || true)" != "active" ]]; then
      printf '%s is not active\n' "$unit" >&2
      failed=1
    fi
  done
  [[ $failed -eq 0 ]] || die "TUFF-Xwin current-session is not fully active; aborting takeover"
}

cmd_start() {
  need_systemd_user
  write_units
  import_environment
  systemctl --user daemon-reload
  systemctl --user stop "$target" >/dev/null 2>&1 || true
  rm -f "$runtime_dir"/*.sock
  log "starting $target profile=$profile session=$session_id runtime_dir=$runtime_dir"
  systemctl --user start "$target"
}

cmd_status() {
  need_systemd_user
  systemctl --user --no-pager --plain list-units \
    'tuff-xwin-current-session*' 'plasma-kwin*' 'plasma-plasmashell*' 'plasma-workspace*' || true
  printf '\n'
  ps -eo pid,ppid,stat,etimes,cmd | grep -E '(^|[ /])(displayd|waylandd|compd|lockd|sessiond|watchdog|kwin_wayland|kwin_x11|plasmashell|Xwayland|xwaylandvideobridge)([ /]|$)' || true
}

cmd_takeover() {
  need_systemd_user
  assert_tuff_active

  if [[ $runtime_mask -eq 1 ]]; then
    log "runtime-masking Plasma/KWin units for this user-manager lifetime"
    systemctl --user mask --runtime "${plasma_units[@]}" >/dev/null 2>&1 || true
  fi

  log "stopping Plasma/KWin units"
  systemctl --user stop "${plasma_units[@]}" >/dev/null 2>&1 || true

  log "terminating remaining Plasma/KWin processes"
  pkill -TERM -x plasmashell || true
  pkill -TERM -x kwin_wayland || true
  pkill -TERM -x kwin_wayland_wrapper || true
  pkill -TERM -x kwin_x11 || true
  pkill -TERM -x Xwayland || true
  pkill -TERM -x xwaylandvideobridge || true
  sleep 2
  pkill -KILL -x plasmashell || true
  pkill -KILL -x kwin_wayland || true
  pkill -KILL -x kwin_wayland_wrapper || true
  pkill -KILL -x kwin_x11 || true
  pkill -KILL -x Xwayland || true
  pkill -KILL -x xwaylandvideobridge || true

  cmd_status
}

cmd_unmask_plasma() {
  need_systemd_user
  local unit
  local path
  for unit in "${plasma_units[@]}"; do
    path="$unit_dir/$unit"
    if [[ -L "$path" && "$(readlink "$path")" == "/dev/null" ]]; then
      rm -f "$path"
      log "removed runtime mask $unit"
    fi
  done
  systemctl --user daemon-reload
}

cmd_stop() {
  need_systemd_user
  systemctl --user stop "$target" >/dev/null 2>&1 || true
}

cmd_cleanup() {
  cmd_stop
  rm -f "$unit_dir"/tuff-xwin-current-session*
  systemctl --user daemon-reload
}

command=""
while (($# > 0)); do
  case "$1" in
    --profile)
      shift
      profile="${1:-}"
      [[ -n "$profile" ]] || die "--profile requires an id"
      ;;
    --session-instance-id)
      shift
      session_id="${1:-}"
      [[ -n "$session_id" ]] || die "--session-instance-id requires an id"
      ;;
    --runtime-mask-plasma)
      runtime_mask=1
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    start|status|takeover|unmask-plasma|stop|cleanup)
      command="$1"
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
  shift
done

case "$command" in
  start) cmd_start ;;
  status) cmd_status ;;
  takeover) cmd_takeover ;;
  unmask-plasma) cmd_unmask_plasma ;;
  stop) cmd_stop ;;
  cleanup) cmd_cleanup ;;
  "")
    usage >&2
    exit 1
    ;;
esac
