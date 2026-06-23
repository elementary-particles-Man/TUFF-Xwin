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
    if [[ -d "$HOME/Pictures/Screenshots" ]]; then
        SAVE_DIR="$HOME/Pictures/Screenshots"
    elif [[ -d "$HOME/Screenshots" ]]; then
        SAVE_DIR="$HOME/Screenshots"
    else
        SAVE_DIR="$HOME/Pictures/Screenshots"
        mkdir -p "$SAVE_DIR"
    fi
else
    mkdir -p "$SAVE_DIR"
fi

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
EOF
    exit 1
fi

# Perform capture
if "$TARGET_DIR/xwin-screenshot" \
    --backend isolated-displayd \
    --displayd-socket "$SOCKET_PATH" \
    --artifact-root "$ARTIFACT_ROOT" \
    --save-dir "$SAVE_DIR" \
    --format png > "$LOG_DIR/screenshot.log" 2>&1; then
    
    # Locate PNG in SAVE_DIR
    PNG_ART=$(find "$SAVE_DIR" -name "*.png" -printf '%T@ %p\n' | sort -n | tail -1 | cut -f2- -d" " || true)
    if [[ -n "$PNG_ART" && -f "$PNG_ART" ]]; then
        echo "tuff-xwin-capture event=success png_path=$PNG_ART size=$(stat -c%s "$PNG_ART")"
        cat <<EOF > "$RUN_ROOT/report.md"
# TUFF-Xwin Capture Success Report
- status: SUCCESS
- run_root: $RUN_ROOT
- png_path: $PNG_ART
- png_size: $(stat -c%s "$PNG_ART") bytes
EOF
    else
        echo "Error: xwin-screenshot succeeded but no PNG file found in $SAVE_DIR" >&2
        exit 1
    fi
else
    RC=$?
    echo "Error: xwin-screenshot failed with exit code $RC" >&2
    # Write report.md
    cat <<EOF > "$RUN_ROOT/report.md"
# TUFF-Xwin Capture Failure Report
- status: FAILED (exit $RC)
- run_root: $RUN_ROOT
EOF
    exit "$RC"
fi

wait "$DISPLAYD_PID" || true
