#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
prefix="${TUFF_XWIN_PREFIX:-$HOME/.local/share/tuff-xwin}"
bin_dir="$prefix/bin"
user_bin_dir="$HOME/.local/bin"
config_dir="$HOME/.config/tuff-xwin"
unit_dir="$HOME/.config/systemd/user"

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
  wget -qO "$rustup_script" https://sh.rustup.rs
  sh "$rustup_script" -y --profile minimal --default-toolchain 1.85.0
  rm -f "$rustup_script"

  # shellcheck disable=SC1090
  . "$HOME/.cargo/env"
}

ensure_rust

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

cp "$repo_root"/contrib/systemd/user/tuff-xwin.target "$unit_dir"/
cp "$repo_root"/contrib/systemd/user/tuff-xwin-*.service "$unit_dir"/

cat >"$config_dir/session.env" <<EOF
TUFF_XWIN_REPO_ROOT=$repo_root
TUFF_XWIN_PROFILES_DIR=$repo_root/profiles
TUFF_XWIN_PROFILE=host-wayland
TUFF_XWIN_SESSION_INSTANCE_ID=debian-host
TUFF_XWIN_DISPLAYD_ARGS=
TUFF_XWIN_WAYLANDD_ARGS=
TUFF_XWIN_LOCKD_ARGS=
TUFF_XWIN_WATCHDOG_ARGS=
TUFF_XWIN_SESSIOND_ARGS=
TUFF_XWIN_HOST_SHELL="gnome-shell --nested --wayland"
TUFF_XWIN_HOST_PANEL=
TUFF_XWIN_HOST_SETTINGSD=
EOF

systemctl --user daemon-reload

echo "Installed TUFF-Xwin into $prefix"
echo "Session env: $config_dir/session.env"
echo "Start command: $HOME/.local/bin/tuff-xwin-start"
echo "Recover command: $HOME/.local/bin/tuff-xwin-recover"
