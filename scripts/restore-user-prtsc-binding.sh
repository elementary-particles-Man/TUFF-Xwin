#!/bin/bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BACKUP_FILE="$REPO_ROOT/target/xsm/tuff-xwin-prtsc-backup.txt"

# Safely check backup before doing any destructive rollback actions
if [[ ! -f "$BACKUP_FILE" ]]; then
    echo "Error: Hotkey backup file not found at $BACKUP_FILE" >&2
    echo "Aborting rollback to prevent destructive actions." >&2
    exit 1
fi

ORIGINAL_BINDING=$(cat "$BACKUP_FILE")
if [[ -z "$ORIGINAL_BINDING" ]]; then
    echo "Error: Hotkey backup file is empty." >&2
    exit 1
fi

echo "Rolling back PrintScreen hotkey binding to original configuration: '$ORIGINAL_BINDING'..."

# Remove local override desktop entry, binary wrapper, and config files
rm -f "$HOME/.local/share/applications/org.flameshot.Flameshot.desktop"
rm -f "$HOME/.local/bin/flameshot"
rm -f "$HOME/.config/environment.d/path.conf"

# Rebuild cache
kbuildsycoca6 --noincremental || true

# Reload KWin
qdbus6 org.kde.KWin /KWin org.kde.KWin.reconfigure || true

# Notify global settings
dbus-send --session --type=signal /KGlobalSettings org.kde.KGlobalSettings.notifyChange int32:3 int32:5 || true

# Restore shortcut to original in kglobalshortcutsrc
kwriteconfig6 --file kglobalshortcutsrc --group "org.flameshot.Flameshot.desktop" --key "Capture" "$ORIGINAL_BINDING"
kwriteconfig6 --file kglobalshortcutsrc --group "services" --group "org.flameshot.Flameshot.desktop" --key "Capture" "$ORIGINAL_BINDING"

# Revert systemd user PATH by dynamically removing $HOME/.local/bin from the environment
CURRENT_PATH=$(systemctl --user show-environment | grep "^PATH=" | cut -d'=' -f2- || echo "")
if [[ -n "$CURRENT_PATH" ]]; then
    NEW_PATH=$(echo "$CURRENT_PATH" | sed -E "s|($HOME/.local/bin:?)||g" | sed -E "s|(:?$HOME/.local/bin)||g" || echo "$CURRENT_PATH")
    systemctl --user set-environment PATH="$NEW_PATH" || true
fi

# Restart Flameshot daemon if it was running previously
if which flameshot >/dev/null 2>&1; then
    echo "Starting Flameshot daemon..."
    flameshot &
fi

echo "Rollback completed. Original shortcut binding restored."
