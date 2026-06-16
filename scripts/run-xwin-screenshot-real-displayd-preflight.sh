#!/bin/bash
set -euo pipefail

# TUFF-Xwin Screenshot Real Displayd Explicit Socket Preflight Script
# This script confirms that xwin-screenshot can connect to a real displayd binary
# using an explicit socket path, without auto-discovery or system-wide resources.

IFS=$'\n\t'

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

RUN_ID="$(date +%Y%m%dT%H%M%S)-$$"
RUN_ROOT="$REPO_ROOT/target/xsm/preflight-$RUN_ID"
LOG_DIR="$RUN_ROOT/logs"
TMP_ROOT="$RUN_ROOT/tmp"
SOCKET_PATH="$TMP_ROOT/real-displayd.sock"
ARTIFACT_ROOT="$TMP_ROOT/artifacts"
OUT_DIR="$TMP_ROOT/out"
REPORT_FILE="$RUN_ROOT/XWIN_SCREENSHOT_PREFLIGHT_REPORT.md"

NO_REAL_CONNECT=false

usage() {
    cat <<EOF
Usage: $0 [--no-real-connect]

--no-real-connect: Validate script syntax and build binaries without actually starting displayd.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-real-connect) NO_REAL_CONNECT=true; shift ;;
        --help|-h) usage; exit 0 ;;
        *) echo "Unknown argument: $1"; usage; exit 1 ;;
    esac
done

mkdir -p "$LOG_DIR" "$TMP_ROOT" "$ARTIFACT_ROOT" "$OUT_DIR"

cat >"$REPORT_FILE" <<EOF
# TUFF-Xwin Screenshot Real Displayd Preflight Report

- run_id: $RUN_ID
- run_root: $RUN_ROOT
- socket_path: $SOCKET_PATH
- artifact_root: $ARTIFACT_ROOT

EOF

log() {
    echo "==> $*"
}

append_report() {
    echo "- $*" >>"$REPORT_FILE"
}

cleanup() {
    if [[ -n "${DISPLAYD_PID:-}" ]]; then
        kill "$DISPLAYD_PID" >/dev/null 2>&1 || true
    fi
}

trap cleanup EXIT INT TERM

log "Building binaries..."
cargo build -p displayd -p xwin-screenshot

if [[ "$NO_REAL_CONNECT" == "true" ]]; then
    log "Preflight validation complete (no-real-connect mode)."
    append_report "Preflight validation complete (no-real-connect mode)."
    exit 0
fi

log "Starting real displayd with explicit socket: $SOCKET_PATH"
"$REPO_ROOT/target/debug/displayd" --socket "$SOCKET_PATH" --once > "$LOG_DIR/displayd.log" 2>&1 &
DISPLAYD_PID=$!

# Wait for socket
log "Waiting for displayd socket..."
for i in $(seq 1 50); do
    if [[ -S "$SOCKET_PATH" ]]; then
        break
    fi
    if ! kill -0 "$DISPLAYD_PID" >/dev/null 2>&1; then
        log "displayd failed to start. Log:"
        cat "$LOG_DIR/displayd.log"
        append_report "displayd failed to start"
        exit 1
    fi
    sleep 0.1
done

if [[ ! -S "$SOCKET_PATH" ]]; then
    log "Timeout waiting for displayd socket."
    append_report "Timeout waiting for displayd socket"
    exit 1
fi

log "Connecting xwin-screenshot to real displayd..."
if "$REPO_ROOT/target/debug/xwin-screenshot" \
    --backend isolated-displayd \
    --displayd-socket "$SOCKET_PATH" \
    --artifact-root "$ARTIFACT_ROOT" \
    --save-dir "$OUT_DIR" \
    --format png > "$LOG_DIR/screenshot.log" 2>&1; then
    log "Preflight connection SUCCESS"
    append_report "Connection: SUCCESS"
else
    RC=$?
    log "Preflight connection FAILED (exit $RC). Log:"
    cat "$LOG_DIR/screenshot.log"
    append_report "Connection: FAILED (exit $RC)"
    exit "$RC"
fi

wait "$DISPLAYD_PID" || true

log "Preflight report written to $REPORT_FILE"
echo "Preflight completed successfully."
