#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
prefix="${TUFF_XWIN_PREFIX:-$HOME/.local/share/tuff-xwin}"
bin_dir="$prefix/bin"
user_bin_dir="$HOME/.local/bin"
config_dir="$HOME/.config/tuff-xwin"
unit_dir="$HOME/.config/systemd/user"
socket_script="$repo_root/scripts/tuff-xwin-distro-socket.sh"

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1090
  . "$HOME/.cargo/env"
fi

ensure_rust() {
  if command -v cargo >/dev/null 2>&1 && command -v rustc >/dev/null 2>&1; then
    return 0
  fi

  echo "Rust toolchain is missing. Installing rustup into $HOME/.cargo" >&2
  local rustup_script
  rustup_script="$(mktemp)"
  if command -v wget >/dev/null 2>&1; then
    wget -qO "$rustup_script" https://sh.rustup.rs
  elif command -v curl >/dev/null 2>&1; then
    curl -fsSL https://sh.rustup.rs -o "$rustup_script"
  else
    echo "missing wget/curl; install one of them before running install-user.sh" >&2
    exit 1
  fi
  sh "$rustup_script" -y --profile minimal --default-toolchain 1.85.0
  rm -f "$rustup_script"

  # shellcheck disable=SC1090
  . "$HOME/.cargo/env"
}

ensure_rust

if ! command -v systemctl >/dev/null 2>&1; then
  echo "systemctl is required; TUFF-Xwin major Linux socket supports systemd-based distros only" >&2
  exit 1
fi

if [[ ! -x "$socket_script" ]]; then
  echo "missing $socket_script" >&2
  exit 1
fi

socket_env="$(mktemp)"
"$socket_script" --emit-env > "$socket_env"
set -a
# shellcheck disable=SC1090
. "$socket_env"
set +a
rm -f "$socket_env"

mkdir -p "$bin_dir" "$user_bin_dir" "$config_dir" "$unit_dir"

cd "$repo_root"
cargo build --workspace --release

release_dir="${CARGO_TARGET_DIR:-/home/flux/.cache/tuff-xwin-target}/release"
for bin in compd displayd lockd sessiond watchdog waylandd x11bridge; do
  install -m 0755 "$release_dir/$bin" "$bin_dir/$bin"
done

install -m 0755 "$repo_root/scripts/tuff-xwin-start.sh" "$user_bin_dir/tuff-xwin-start"
install -m 0755 "$repo_root/scripts/tuff-xwin-stop.sh" "$user_bin_dir/tuff-xwin-stop"
install -m 0755 "$repo_root/scripts/tuff-xwin-recover.sh" "$user_bin_dir/tuff-xwin-recover"
install -m 0755 "$repo_root/scripts/tuff-xwin-autostart.sh" "$user_bin_dir/tuff-xwin-autostart"
install -m 0755 "$socket_script" "$user_bin_dir/tuff-xwin-distro-socket"

cp "$repo_root"/contrib/systemd/user/tuff-xwin.target "$unit_dir"/
cp "$repo_root"/contrib/systemd/user/tuff-xwin-*.service "$unit_dir"/

{
  printf 'TUFF_XWIN_PREFIX=%q\n' "$prefix"
  printf 'TUFF_XWIN_REPO_ROOT=%q\n' "$repo_root"
  printf 'TUFF_XWIN_PROFILES_DIR=%q\n' "$repo_root/profiles"
  printf 'TUFF_XWIN_SOCKET_FLAVOR=%q\n' "$TUFF_XWIN_SOCKET_FLAVOR"
  printf 'TUFF_XWIN_DISTRO_FAMILY=%q\n' "$TUFF_XWIN_DISTRO_FAMILY"
  printf 'TUFF_XWIN_DISTRO_ID=%q\n' "$TUFF_XWIN_DISTRO_ID"
  printf 'TUFF_XWIN_PACKAGE_MANAGER=%q\n' "$TUFF_XWIN_PACKAGE_MANAGER"
  printf 'TUFF_XWIN_PROFILE=%q\n' "$TUFF_XWIN_PROFILE"
  printf 'TUFF_XWIN_SESSION_INSTANCE_ID=%q\n' "$TUFF_XWIN_SESSION_INSTANCE_ID"
  printf 'TUFF_XWIN_DISPLAYD_ARGS=%q\n' ""
  printf 'TUFF_XWIN_WAYLANDD_ARGS=%q\n' ""
  printf 'TUFF_XWIN_LOCKD_ARGS=%q\n' ""
  printf 'TUFF_XWIN_WATCHDOG_ARGS=%q\n' ""
  printf 'TUFF_XWIN_SESSIOND_ARGS=%q\n' ""
  printf 'TUFF_XWIN_HOST_SHELL=%q\n' "$TUFF_XWIN_HOST_SHELL"
  printf 'TUFF_XWIN_HOST_PANEL=%q\n' "$TUFF_XWIN_HOST_PANEL"
  printf 'TUFF_XWIN_HOST_SETTINGSD=%q\n' "$TUFF_XWIN_HOST_SETTINGSD"
} >"$config_dir/session.env"

systemctl --user daemon-reload

echo "Installed TUFF-Xwin into $prefix"
echo "Linux socket: family=$TUFF_XWIN_DISTRO_FAMILY distro=$TUFF_XWIN_DISTRO_ID package_manager=$TUFF_XWIN_PACKAGE_MANAGER"
echo "Session env: $config_dir/session.env"
echo "Start command: $HOME/.local/bin/tuff-xwin-start"
echo "Recover command: $HOME/.local/bin/tuff-xwin-recover"
