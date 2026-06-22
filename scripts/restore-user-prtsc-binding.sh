#!/bin/bash
set -euo pipefail

echo "Rolling back PrintScreen hotkey binding to original configuration..."

# Remove local override desktop entry, binary wrapper, and config files
rm -f "/home/flux/.local/share/applications/org.flameshot.Flameshot.desktop"
rm -f "/home/flux/.local/bin/flameshot"
rm -f "/home/flux/.config/environment.d/path.conf"

# Rebuild cache
kbuildsycoca6 --noincremental || true

# Reload KWin
qdbus6 org.kde.KWin /KWin org.kde.KWin.reconfigure || true

# Notify global settings
dbus-send --session --type=signal /KGlobalSettings org.kde.KGlobalSettings.notifyChange int32:3 int32:5 || true

# Restore shortcut to original in kglobalshortcutsrc
kwriteconfig6 --file kglobalshortcutsrc --group "org.flameshot.Flameshot.desktop" --key "Capture" "none"
kwriteconfig6 --file kglobalshortcutsrc --group "services" --group "org.flameshot.Flameshot.desktop" --key "Capture" "none"

# Revert systemd user PATH
if [[ -n "/home/flux/.local/bin:/home/flux/.local/bin:/home/flux/.bun/bin:/home/flux/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/local/games:/usr/games" ]]; then
    systemctl --user set-environment PATH="/home/flux/.local/bin:/home/flux/.local/bin:/home/flux/.bun/bin:/home/flux/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/local/games:/usr/games" || true
fi

# Restart Flameshot daemon if it was running previously
if which flameshot >/dev/null 2>&1; then
    echo "Starting Flameshot daemon..."
    flameshot &
fi

echo "Rollback completed. Original shortcut binding restored."
