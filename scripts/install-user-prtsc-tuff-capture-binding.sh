#!/bin/bash
set -euo pipefail

# Detect DE/WM
DESKTOP_ENV=""
if [[ "${XDG_CURRENT_DESKTOP:-}" =~ "KDE" ]]; then
    DESKTOP_ENV="kde"
elif [[ "${XDG_CURRENT_DESKTOP:-}" =~ "GNOME" ]]; then
    DESKTOP_ENV="gnome"
elif [[ "${XDG_CURRENT_DESKTOP:-}" =~ "XFCE" ]]; then
    DESKTOP_ENV="xfce"
fi

if [[ "$DESKTOP_ENV" != "kde" ]]; then
    echo "Error: Current desktop environment ('${XDG_CURRENT_DESKTOP:-}') is not supported or not KDE Plasma." >&2
    echo "Aborting shortcut configuration to avoid breaking existing bindings." >&2
    exit 2
fi

echo "Detected KDE Plasma environment."

# Config paths
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LAUNCHER_PATH="$REPO_ROOT/scripts/tuff-xwin-capture-once.sh"
DESKTOP_FILE="$HOME/.local/share/applications/org.flameshot.Flameshot.desktop"
BACKUP_FILE="$REPO_ROOT/target/xsm/tuff-xwin-prtsc-backup.txt"
ROLLBACK_SCRIPT="$REPO_ROOT/scripts/restore-user-prtsc-binding.sh"

LOCAL_BIN_DIR="$HOME/.local/bin"
LOCAL_FLAMESHOT="$LOCAL_BIN_DIR/flameshot"
ENV_CONF_DIR="$HOME/.config/environment.d"
ENV_CONF_FILE="$ENV_CONF_DIR/path.conf"

mkdir -p "$(dirname "$BACKUP_FILE")"

# 1. Read existing Flameshot config using kreadconfig6
echo "Reading current global shortcut configuration..."
CURRENT_FLAMESHOT_BINDING=$(kreadconfig6 --file kglobalshortcutsrc --group "org.flameshot.Flameshot.desktop" --key "Capture" || echo "")

if [[ -z "$CURRENT_FLAMESHOT_BINDING" ]]; then
    # Fallback to services group if not found in main group
    CURRENT_FLAMESHOT_BINDING=$(kreadconfig6 --file kglobalshortcutsrc --group "services" --group "org.flameshot.Flameshot.desktop" --key "Capture" || echo "")
fi

if [[ -z "$CURRENT_FLAMESHOT_BINDING" ]]; then
    CURRENT_FLAMESHOT_BINDING="Print"
fi

echo "Current Flameshot shortcut binding: '$CURRENT_FLAMESHOT_BINDING'"
echo "$CURRENT_FLAMESHOT_BINDING" > "$BACKUP_FILE"
if [[ ! -f "$BACKUP_FILE" ]]; then
    echo "Error: Failed to create hotkey backup file at $BACKUP_FILE" >&2
    exit 4
fi
echo "Saved backup of current binding to $BACKUP_FILE"

# Capture original systemd PATH
ORIGINAL_SYSTEMD_PATH=$(systemctl --user show-environment | grep "^PATH=" | cut -d'=' -f2- || echo "")

# Kill running Flameshot process
if killall flameshot 2>/dev/null; then
    echo "Stopped running Flameshot daemon."
fi

# 2. Generate rollback script
echo "Generating rollback script at $ROLLBACK_SCRIPT..."
cat <<EOF > "$ROLLBACK_SCRIPT"
#!/bin/bash
set -euo pipefail

# Safely check backup before doing any destructive rollback actions
if [[ ! -f "$BACKUP_FILE" ]]; then
    echo "Error: Hotkey backup file not found at $BACKUP_FILE" >&2
    echo "Aborting rollback to prevent destructive actions." >&2
    exit 1
fi

ORIGINAL_BINDING=\$(cat "$BACKUP_FILE")
if [[ -z "\$ORIGINAL_BINDING" ]]; then
    echo "Error: Hotkey backup file is empty." >&2
    exit 1
fi

echo "Rolling back PrintScreen hotkey binding to original configuration: '\$ORIGINAL_BINDING'..."

# Remove local override desktop entry, binary wrapper, and config files
rm -f "$DESKTOP_FILE"
rm -f "$LOCAL_FLAMESHOT"
rm -f "$ENV_CONF_FILE"

# Rebuild cache
kbuildsycoca6 --noincremental || true

# Reload KWin
qdbus6 org.kde.KWin /KWin org.kde.KWin.reconfigure || true

# Notify global settings
dbus-send --session --type=signal /KGlobalSettings org.kde.KGlobalSettings.notifyChange int32:3 int32:5 || true

# Restore shortcut to original in kglobalshortcutsrc
kwriteconfig6 --file kglobalshortcutsrc --group "org.flameshot.Flameshot.desktop" --key "Capture" "\$ORIGINAL_BINDING"
kwriteconfig6 --file kglobalshortcutsrc --group "services" --group "org.flameshot.Flameshot.desktop" --key "Capture" "\$ORIGINAL_BINDING"

# Revert systemd user PATH
if [[ -n "$ORIGINAL_SYSTEMD_PATH" ]]; then
    systemctl --user set-environment PATH="$ORIGINAL_SYSTEMD_PATH" || true
fi

# Restart Flameshot daemon if it was running previously
if which flameshot >/dev/null 2>&1; then
    echo "Starting Flameshot daemon..."
    flameshot &
fi

echo "Rollback completed. Original shortcut binding restored."
EOF
chmod +x "$ROLLBACK_SCRIPT"

# 3. Create local bin wrapper and systemd environment.d config
echo "Creating user-local Flameshot binary wrapper at $LOCAL_FLAMESHOT..."
mkdir -p "$LOCAL_BIN_DIR"
cat <<EOF > "$LOCAL_FLAMESHOT"
#!/bin/bash
# TUFF-Xwin Flameshot wrapper override
echo "flameshot wrapper triggered at \$(date)" >> /tmp/tuff-flameshot-wrapper.log
exec bash $LAUNCHER_PATH --portal-real-capture "\$@"
EOF
chmod +x "$LOCAL_FLAMESHOT"

echo "Creating systemd environment config at $ENV_CONF_FILE..."
mkdir -p "$ENV_CONF_DIR"
cat <<EOF > "$ENV_CONF_FILE"
PATH=\$HOME/.local/bin:\$PATH
EOF

# Update systemd user environment PATH immediately
NEW_SYSTEMD_PATH="$LOCAL_BIN_DIR"
if [[ -n "$ORIGINAL_SYSTEMD_PATH" ]]; then
    NEW_SYSTEMD_PATH="$LOCAL_BIN_DIR:$ORIGINAL_SYSTEMD_PATH"
fi
echo "Updating systemd user PATH environment..."
systemctl --user set-environment PATH="$NEW_SYSTEMD_PATH"

# 4. Copy system Flameshot desktop entry and override Exec paths
echo "Creating user-local Flameshot override at $DESKTOP_FILE..."
mkdir -p "$(dirname "$DESKTOP_FILE")"
if [[ -f "/usr/share/applications/org.flameshot.Flameshot.desktop" ]]; then
    cp "/usr/share/applications/org.flameshot.Flameshot.desktop" "$DESKTOP_FILE"
    
    # Replace Exec paths with our capture launcher
    sed -i "s|^Exec=flameshot|Exec=bash $LAUNCHER_PATH --portal-real-capture|g" "$DESKTOP_FILE"
    sed -i "s|^Exec=flameshot gui --delay 500|Exec=bash $LAUNCHER_PATH --portal-real-capture|g" "$DESKTOP_FILE"
else
    echo "Error: system Flameshot desktop entry not found at /usr/share/applications/org.flameshot.Flameshot.desktop" >&2
    exit 3
fi

# Clean up any leftover custom desktop files to avoid conflict
rm -f "$HOME/.local/share/applications/org.tuff.xwin.capture.desktop"

# 5. Reload configuration and apply immediately via D-Bus
echo "Rebuilding system services cache..."
kbuildsycoca6 --noincremental || true

echo "Triggering KWin reconfigure to apply shortcuts..."
qdbus6 org.kde.KWin /KWin org.kde.KWin.reconfigure || true

echo "Notifying KGlobalSettings of shortcut changes..."
dbus-send --session --type=signal /KGlobalSettings org.kde.KGlobalSettings.notifyChange int32:3 int32:5 || true

echo "Binding installation completed successfully."
echo "PrintScreen has been bound to TUFF-Xwin Capture."
echo "To revert, run: $ROLLBACK_SCRIPT"
