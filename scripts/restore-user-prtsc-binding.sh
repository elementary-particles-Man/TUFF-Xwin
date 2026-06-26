#!/bin/bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BACKUP_FILE="$REPO_ROOT/target/xsm/tuff-xwin-prtsc-backup.txt"

# Find backup directory. We first look for target/xsm/tuff-xwin-prtsc-backup-latest symlink,
# if not found, we look for the newest backup directory.
BACKUP_DIR=""
LATEST_LINK="$REPO_ROOT/target/xsm/tuff-xwin-prtsc-backup-latest"

if [[ -L "$LATEST_LINK" ]]; then
    BACKUP_DIR=$(readlink -f "$LATEST_LINK" || echo "")
fi

if [[ -z "$BACKUP_DIR" || ! -d "$BACKUP_DIR" ]]; then
    # Fallback to searching for directories
    # Use standard shell glob to find all backup directories, sort them, and pick the latest one
    # Note: we use target/xsm/tuff-xwin-prtsc-backup-* pattern
    # We use find to locate directories safely
    if [[ -d "$REPO_ROOT/target/xsm" ]]; then
        BACKUP_DIRS=($(find "$REPO_ROOT/target/xsm" -maxdepth 1 -type d -name "tuff-xwin-prtsc-backup-*" | sort || true))
        if [[ ${#BACKUP_DIRS[@]} -gt 0 ]]; then
            BACKUP_DIR="${BACKUP_DIRS[-1]}"
        fi
    fi
fi

if [[ -z "$BACKUP_DIR" || ! -d "$BACKUP_DIR" ]]; then
    echo "Error: No backup directory found." >&2
    exit 1
fi

MANIFEST_FILE="$BACKUP_DIR/manifest.tsv"

if [[ ! -f "$MANIFEST_FILE" ]]; then
    echo "Error: Backup manifest file not found at $MANIFEST_FILE" >&2
    exit 1
fi

echo "Using backup manifest from $MANIFEST_FILE"

# Parse manifest and restore or delete files
# Schema: file_id <tab> existed <tab> dest_path <tab> backup_file
while IFS=$'\t' read -r file_id existed dest_path backup_file || [[ -n "$file_id" ]]; do
    # Skip empty lines
    if [[ -z "$file_id" ]]; then
        continue
    fi
    
    echo "Processing $file_id (existed=$existed, dest_path=$dest_path)"
    
    if [[ "$file_id" == "binding" ]]; then
        # Handle hotkey binding restoration
        BINDING_BAK_FILE="$BACKUP_DIR/$backup_file"
        if [[ "$existed" == "true" && -f "$BINDING_BAK_FILE" ]]; then
            ORIGINAL_BINDING=$(cat "$BINDING_BAK_FILE" || echo "")
            if [[ -n "$ORIGINAL_BINDING" ]]; then
                echo "Restoring hotkey binding to: '$ORIGINAL_BINDING'"
                kwriteconfig6 --file kglobalshortcutsrc --group "org.flameshot.Flameshot.desktop" --key "Capture" "$ORIGINAL_BINDING" || true
                kwriteconfig6 --file kglobalshortcutsrc --group "services" --group "org.flameshot.Flameshot.desktop" --key "Capture" "$ORIGINAL_BINDING" || true
            fi
        fi
    else
        # Handle file restoration or deletion
        if [[ "$existed" == "true" ]]; then
            BAK_FILE="$BACKUP_DIR/$backup_file"
            if [[ -f "$BAK_FILE" ]]; then
                echo "Restoring original file to $dest_path..."
                mkdir -p "$(dirname "$dest_path")"
                cp "$BAK_FILE" "$dest_path"
            else
                echo "Warning: Backup file $backup_file not found. Cannot restore $dest_path." >&2
            fi
        else
            if [[ -f "$dest_path" ]]; then
                echo "Removing TUFF-generated file at $dest_path..."
                rm -f "$dest_path"
            else
                echo "File $dest_path does not exist, nothing to delete."
            fi
        fi
    fi
done < "$MANIFEST_FILE"

# Rebuild cache
kbuildsycoca6 --noincremental || true

# Reload KWin
qdbus6 org.kde.KWin /KWin org.kde.KWin.reconfigure || true

# Notify global settings
dbus-send --session --type=signal /KGlobalSettings org.kde.KGlobalSettings.notifyChange int32:3 int32:5 || true

# Revert systemd user PATH by dynamically removing $HOME/.local/bin from environment
CURRENT_PATH=$(systemctl --user show-environment 2>/dev/null | grep "^PATH=" | cut -d'=' -f2- || echo "")
if [[ -n "$CURRENT_PATH" ]]; then
    NEW_PATH=$(echo "$CURRENT_PATH" | sed -E "s|($HOME/.local/bin:?)||g" | sed -E "s|(:?$HOME/.local/bin)||g" || echo "$CURRENT_PATH")
    systemctl --user set-environment PATH="$NEW_PATH" || true
fi

# Clean up latest symlink
rm -f "$LATEST_LINK"

# Restart Flameshot daemon if it was running previously
if which flameshot >/dev/null 2>&1; then
    echo "Starting Flameshot daemon..."
    flameshot >/dev/null 2>&1 &
fi

echo "Rollback completed. Original shortcut binding and user-local environment restored."

