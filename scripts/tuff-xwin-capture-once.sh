#!/bin/bash
set -euo pipefail

# Check for required flag
REAL_PORTAL_CAPTURE="false"
SAVE_DIR=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --portal-real-capture)
            REAL_PORTAL_CAPTURE="true"
            shift
            ;;
        --save-dir)
            SAVE_DIR="$2"
            shift 2
            ;;
        *)
            echo "Unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

if [[ "$REAL_PORTAL_CAPTURE" != "true" ]]; then
    echo "Error: This launcher refuses to run without the explicit '--portal-real-capture' flag." >&2
    exit 1
fi

# Find repo root
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Resolve target directory considering .cargo/config.toml or CARGO_TARGET_DIR environment variable
CARGO_TARGET_DIR_CONF=""
if [[ -f "$REPO_ROOT/.cargo/config.toml" ]]; then
    CARGO_TARGET_DIR_CONF=$(grep -E '^\s*target-dir\s*=' "$REPO_ROOT/.cargo/config.toml" | sed -E 's/.*target-dir\s*=\s*["'\'']([^"'\'']+)["'\'']/\1/' || true)
fi

TARGET_DIR_BASE="${CARGO_TARGET_DIR:-${CARGO_TARGET_DIR_CONF:-$REPO_ROOT/target}}"
PREFIX_BIN_DIR="$HOME/.local/share/tuff-xwin/bin"

if [[ -f "$PREFIX_BIN_DIR/displayd" && -f "$PREFIX_BIN_DIR/xwin-screenshot" ]]; then
    TARGET_DIR="$PREFIX_BIN_DIR"
elif [[ -f "$TARGET_DIR_BASE/release/displayd" && -f "$TARGET_DIR_BASE/release/xwin-screenshot" ]]; then
    TARGET_DIR="$TARGET_DIR_BASE/release"
else
    TARGET_DIR="$TARGET_DIR_BASE/debug"
    if [[ ! -f "$TARGET_DIR/displayd" || ! -f "$TARGET_DIR/xwin-screenshot" ]]; then
        echo "Warning: binaries not found in target. Rebuilding workspace..." >&2
        cargo build --workspace --features real-x11,real-portal
    fi
fi

# Determine default save directory
if [[ -z "$SAVE_DIR" ]]; then
    SAVE_DIR="$HOME/Pictures/TUFF-Xwin"
fi
mkdir -p "$SAVE_DIR"

# Create a temporary run directory
RUN_ROOT=$(mktemp -d -t tuff-capture-run-XXXXXX)
LOG_DIR="$RUN_ROOT/logs"
ARTIFACT_ROOT="$RUN_ROOT/artifacts"
SOCKET_PATH="$RUN_ROOT/tmp/displayd-capture.sock"

mkdir -p "$LOG_DIR" "$ARTIFACT_ROOT" "$(dirname "$SOCKET_PATH")"

# Match displayd output and client ingestion paths by setting WAYBROKER_RUNTIME_DIR
export WAYBROKER_RUNTIME_DIR="$ARTIFACT_ROOT"

# Store path config for debugging/validation
echo "tuff-xwin-capture event=start run_root=$RUN_ROOT save_dir=$SAVE_DIR"

# Launch displayd
rm -f "$SOCKET_PATH"
"$TARGET_DIR/displayd" \
    --socket "$SOCKET_PATH" \
    --capture-backend real \
    --allow-real-capture \
    --capture-method portal \
    --allow-portal-capture \
    --allow-portal-dialog \
    --once > "$LOG_DIR/displayd.log" 2>&1 &
DISPLAYD_PID=$!

# Wait for socket
SOCKET_TIMEOUT="false"
for i in $(seq 1 50); do
    if [[ -S "$SOCKET_PATH" ]]; then break; fi
    if ! kill -0 "$DISPLAYD_PID" 2>/dev/null; then
        echo "Error: displayd exited prematurely before socket bound." >&2
        SOCKET_TIMEOUT="true"
        break
    fi
    sleep 0.1
done

if [[ "$SOCKET_TIMEOUT" == "true" || ! -S "$SOCKET_PATH" ]]; then
    echo "Error: Timeout or launch failure waiting for displayd socket." >&2
    kill "$DISPLAYD_PID" 2>/dev/null || true
    wait "$DISPLAYD_PID" 2>/dev/null || true

    # Write report.md before exiting
    cat <<EOF > "$RUN_ROOT/report.md"
# TUFF-Xwin Capture Failure Report
- status: FAILED (socket timeout or launch failure)
- run_root: $RUN_ROOT
- notice: 'wlr-screencopy-unstable-v1 unsupported' errors are related to standard compositor fallback checks and are NOT failures in the TUFF-Xwin system.
- preview_notice: White/blank previews in the portal are expected security isolation behavior and not a failure.
EOF

    echo "=== TUFF-Xwin Capture Failure Details ===" >&2
    echo "RUN_ROOT: $RUN_ROOT" >&2
    echo "Report Path: $RUN_ROOT/report.md" >&2
    echo "--- displayd.log ---" >&2
    cat "$LOG_DIR/displayd.log" >&2
    echo "=========================================" >&2
    exit 1
fi

echo "================================================================="
echo "TUFF-Xwin Capture: Starting screen capture sequence..."
echo "Note (KDE/Plasma):"
echo "  - The monitor device tile (e.g. 'HP Inc. HP 27f 4k') represents the Fullscreen capture option."
echo "  - Please click that tile and then click 'Share' to capture the full screen."
echo "  - White/blank preview tiles in the portal are expected security isolation behavior and not a failure."
echo "================================================================="

# Perform capture
if "$TARGET_DIR/xwin-screenshot" \
    --backend isolated-displayd \
    --displayd-socket "$SOCKET_PATH" \
    --artifact-root "$ARTIFACT_ROOT" \
    --save-dir "$ARTIFACT_ROOT" \
    --format png > "$LOG_DIR/screenshot.log" 2>&1; then

    # Locate PNG in ARTIFACT_ROOT
    PNG_ART=$(find "$ARTIFACT_ROOT" -name "*.png" -printf '%T@ %p\n' | sort -n | tail -1 | cut -f2- -d" " || true)
    if [[ -n "$PNG_ART" && -f "$PNG_ART" ]]; then
        # Ask user for action
        ACTION=""
        if command -v kdialog >/dev/null 2>&1; then
            if kdialog --yesno "スクリーンショットを撮影しました。\nクリップボードにコピーしますか？\n(いいえ を選択するとフォルダへ保存します)" --title "TUFF-Xwin Capture" --yes-label "コピー" --no-label "保存"; then
                ACTION="copy"
            else
                ACTION="save"
            fi
        elif command -v zenity >/dev/null 2>&1; then
            if zenity --question --text="スクリーンショットを撮影しました。クリップボードにコピーしますか？" --title="TUFF-Xwin Capture" --ok-label="コピー" --cancel-label="保存" 2>/dev/null; then
                ACTION="copy"
            else
                ACTION="save"
            fi
        else
            # Fallback if no GUI dialog tool found
            ACTION="save"
        fi

        TIMESTAMP=$(date +%Y%m%d_%H%M%S_%3N)
        MODE="fullscreen"
        SOURCE="portal"
        NEW_NAME="tuff-xwin-${MODE}-${SOURCE}-${TIMESTAMP}.png"

        FINAL_PNG=""
        if [[ "$ACTION" == "copy" ]]; then
            mv "$PNG_ART" "$ARTIFACT_ROOT/$NEW_NAME"
            FINAL_PNG="$ARTIFACT_ROOT/$NEW_NAME"
            if command -v wl-copy >/dev/null 2>&1; then
                wl-copy < "$FINAL_PNG"
                echo "tuff-xwin-capture event=success_copy png_path=$FINAL_PNG"
            elif command -v xclip >/dev/null 2>&1; then
                xclip -selection clipboard -t image/png -i "$FINAL_PNG"
                echo "tuff-xwin-capture event=success_copy_xclip png_path=$FINAL_PNG"
            else
                # Fallback to save if no clipboard tools
                mv "$FINAL_PNG" "$SAVE_DIR/"
                FINAL_PNG="$SAVE_DIR/$NEW_NAME"
                echo "tuff-xwin-capture event=success_save_fallback png_path=$FINAL_PNG"
            fi
        else
            # Save to target directory
            mv "$PNG_ART" "$SAVE_DIR/$NEW_NAME"
            FINAL_PNG="$SAVE_DIR/$NEW_NAME"
            echo "tuff-xwin-capture event=success_save png_path=$FINAL_PNG"
        fi

        PNG_SIZE=$(stat -c%s "$FINAL_PNG" 2>/dev/null || echo "0")
        echo "CAPTURED_PNG_PATH: $FINAL_PNG SIZE: $PNG_SIZE bytes"

        if command -v notify-send >/dev/null 2>&1; then
            if [[ "$ACTION" == "copy" ]]; then
                notify-send "TUFF-Xwin Capture" "スクリーンショットをクリップボードにコピーしました (サイズ: $PNG_SIZE バイト)" -i "$FINAL_PNG" || true
            else
                notify-send "TUFF-Xwin Capture" "スクリーンショットを保存しました\n$FINAL_PNG\n(サイズ: $PNG_SIZE バイト)" -i "$FINAL_PNG" || true
            fi
        fi

        cat <<EOF > "$RUN_ROOT/report.md"
# TUFF-Xwin Capture Success Report
- status: SUCCESS
- run_root: $RUN_ROOT
- action: $ACTION
- png_path: $FINAL_PNG
- png_size: $PNG_SIZE bytes
- notice: 'wlr-screencopy-unstable-v1 unsupported' errors are related to standard compositor fallback checks and are NOT failures in the TUFF-Xwin system.
- preview_notice: White/blank previews in the portal are expected security isolation behavior and not a failure.
EOF
    else
        echo "Error: xwin-screenshot succeeded but no PNG file found in $ARTIFACT_ROOT" >&2
        exit 1
    fi
else
    RC=$?
    if grep -E -q -i "cancelled|response\(cancelled\)|the request was cancelled" "$LOG_DIR/screenshot.log" 2>/dev/null; then
        echo "tuff-xwin-capture event=portal_cancel"
        echo "Expected fail-closed: screen capture was cancelled by the user."

        cat <<EOF > "$RUN_ROOT/report.md"
# TUFF-Xwin Capture Cancel Report
- status: CANCELLED (expected fail-closed)
- run_root: $RUN_ROOT
- notice: 'wlr-screencopy-unstable-v1 unsupported' errors are related to standard compositor fallback checks and are NOT failures in the TUFF-Xwin system.
- preview_notice: White/blank previews in the portal are expected security isolation behavior and not a failure.
EOF
        exit 0
    else
        echo "Error: xwin-screenshot failed with exit code $RC" >&2

        cat <<EOF > "$RUN_ROOT/report.md"
# TUFF-Xwin Capture Failure Report
- status: FAILED (exit $RC)
- run_root: $RUN_ROOT
- notice: 'wlr-screencopy-unstable-v1 unsupported' errors are related to standard compositor fallback checks and are NOT failures in the TUFF-Xwin system.
- preview_notice: White/blank previews in the portal are expected security isolation behavior and not a failure.
EOF

        echo "=== TUFF-Xwin Capture Failure Details ===" >&2
        echo "RUN_ROOT: $RUN_ROOT" >&2
        echo "Report Path: $RUN_ROOT/report.md" >&2
        echo "--- displayd.log ---" >&2
        cat "$LOG_DIR/displayd.log" >&2
        echo "--- screenshot.log ---" >&2
        if [[ -f "$LOG_DIR/screenshot.log" ]]; then
            cat "$LOG_DIR/screenshot.log" >&2
        else
            echo "(screenshot.log not found)" >&2
        fi
        echo "=========================================" >&2
        exit "$RC"
    fi
fi

wait "$DISPLAYD_PID" || true
