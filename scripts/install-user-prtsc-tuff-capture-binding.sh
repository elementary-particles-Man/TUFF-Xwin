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
ROLLBACK_SCRIPT="$REPO_ROOT/scripts/restore-user-prtsc-binding.sh"

LOCAL_BIN_DIR="$HOME/.local/bin"
LOCAL_FLAMESHOT="$LOCAL_BIN_DIR/flameshot"
ENV_CONF_DIR="$HOME/.config/environment.d"
ENV_CONF_FILE="$ENV_CONF_DIR/tuff-xwin-path.conf"

# Generate backup directory with timestamp
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
BACKUP_DIR="$REPO_ROOT/target/xsm/tuff-xwin-prtsc-backup-$TIMESTAMP"
MANIFEST_FILE="$BACKUP_DIR/manifest.tsv"

if ! mkdir -p "$BACKUP_DIR"; then
    echo "Error: Failed to create backup directory at $BACKUP_DIR" >&2
    exit 4
fi

MANIFEST_CONTENT=""

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
echo "$CURRENT_FLAMESHOT_BINDING" > "$BACKUP_DIR/binding.txt"
if [[ ! -f "$BACKUP_DIR/binding.txt" ]]; then
    echo "Error: Failed to create hotkey backup file at $BACKUP_DIR/binding.txt" >&2
    exit 4
fi
MANIFEST_CONTENT+="binding	true	kglobalshortcutsrc	binding.txt"$'\n'

# Capture original systemd PATH
ORIGINAL_SYSTEMD_PATH=$(systemctl --user show-environment 2>/dev/null | grep "^PATH=" | cut -d'=' -f2- || echo "")

# Kill running Flameshot process
if killall flameshot 2>/dev/null; then
    echo "Stopped running Flameshot daemon."
fi

# 2. Backup existing user-local files
# ~/.local/bin/flameshot
if [[ -f "$LOCAL_FLAMESHOT" ]]; then
    cp "$LOCAL_FLAMESHOT" "$BACKUP_DIR/flameshot"
    MANIFEST_CONTENT+="flameshot	true	$LOCAL_FLAMESHOT	flameshot"$'\n'
    echo "Backed up existing local flameshot to $BACKUP_DIR/flameshot"
else
    MANIFEST_CONTENT+="flameshot	false	$LOCAL_FLAMESHOT	none"$'\n'
fi

# ~/.config/environment.d/tuff-xwin-path.conf
if [[ -f "$ENV_CONF_FILE" ]]; then
    cp "$ENV_CONF_FILE" "$BACKUP_DIR/tuff-xwin-path.conf"
    MANIFEST_CONTENT+="path_conf	true	$ENV_CONF_FILE	tuff-xwin-path.conf"$'\n'
    echo "Backed up existing tuff-xwin-path.conf to $BACKUP_DIR/tuff-xwin-path.conf"
else
    MANIFEST_CONTENT+="path_conf	false	$ENV_CONF_FILE	none"$'\n'
fi

# ~/.local/share/applications/org.flameshot.Flameshot.desktop
if [[ -f "$DESKTOP_FILE" ]]; then
    cp "$DESKTOP_FILE" "$BACKUP_DIR/org.flameshot.Flameshot.desktop"
    MANIFEST_CONTENT+="desktop	true	$DESKTOP_FILE	org.flameshot.Flameshot.desktop"$'\n'
    echo "Backed up existing desktop entry to $BACKUP_DIR/org.flameshot.Flameshot.desktop"
else
    MANIFEST_CONTENT+="desktop	false	$DESKTOP_FILE	none"$'\n'
fi

# Write manifest file
echo -n "$MANIFEST_CONTENT" > "$MANIFEST_FILE"
if [[ ! -f "$MANIFEST_FILE" ]]; then
    echo "Error: Failed to write backup manifest at $MANIFEST_FILE" >&2
    exit 4
fi
echo "Backup manifest created successfully at $MANIFEST_FILE"

# Create a symlink to the latest backup directory for restore script convenience
LATEST_LINK="$REPO_ROOT/target/xsm/tuff-xwin-prtsc-backup-latest"
rm -f "$LATEST_LINK"
ln -s "$BACKUP_DIR" "$LATEST_LINK"

# Safe verification before mutating anything
if [[ -f "$LOCAL_FLAMESHOT" ]]; then
    if [[ ! -f "$BACKUP_DIR/flameshot" ]]; then
        echo "Error: Backup verification failed for $LOCAL_FLAMESHOT" >&2
        exit 4
     fi
fi
if [[ -f "$ENV_CONF_FILE" ]]; then
    if [[ ! -f "$BACKUP_DIR/tuff-xwin-path.conf" ]]; then
        echo "Error: Backup verification failed for $ENV_CONF_FILE" >&2
        exit 4
     fi
fi
if [[ -f "$DESKTOP_FILE" ]]; then
    if [[ ! -f "$BACKUP_DIR/org.flameshot.Flameshot.desktop" ]]; then
        echo "Error: Backup verification failed for $DESKTOP_FILE" >&2
        exit 4
     fi
fi

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
systemctl --user set-environment PATH="$NEW_SYSTEMD_PATH" || true

# 4. Copy system Flameshot desktop entry and override Exec paths
echo "Creating user-local Flameshot override at $DESKTOP_FILE..."
mkdir -p "$(dirname "$DESKTOP_FILE")"
SYSTEM_DESKTOP="${TUFF_MOCK_SYSTEM_DESKTOP:-/usr/share/applications/org.flameshot.Flameshot.desktop}"
if [[ -f "$SYSTEM_DESKTOP" ]]; then
    cp "$SYSTEM_DESKTOP" "$DESKTOP_FILE"
    
    # Replace Exec paths with our capture launcher
    sed -i "s|^Exec=flameshot|Exec=bash $LAUNCHER_PATH --portal-real-capture|g" "$DESKTOP_FILE"
    sed -i "s|^Exec=flameshot gui --delay 500|Exec=bash $LAUNCHER_PATH --portal-real-capture|g" "$DESKTOP_FILE"
else
    # For testing and compatibility fallback, if /usr/share/... does not exist but a backup does, we use it
    if [[ -f "$BACKUP_DIR/org.flameshot.Flameshot.desktop" ]]; then
        cp "$BACKUP_DIR/org.flameshot.Flameshot.desktop" "$DESKTOP_FILE"
        sed -i "s|^Exec=flameshot|Exec=bash $LAUNCHER_PATH --portal-real-capture|g" "$DESKTOP_FILE" || true
    else
        echo "Error: system Flameshot desktop entry not found at $SYSTEM_DESKTOP" >&2
        exit 3
    fi
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

