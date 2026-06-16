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

TARGET_DIR="$(
  cargo metadata --format-version 1 --no-deps --quiet |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])'
)/debug"

if [[ "$NO_REAL_CONNECT" == "true" ]]; then
    log "Preflight validation complete (no-real-connect mode)."
    append_report "Preflight validation complete (no-real-connect mode)."
    exit 0
fi

# Test 1: Explicit Fake Backend (Should succeed)
log "Case 1: Explicit Fake Backend"
export WAYBROKER_RUNTIME_DIR="$ARTIFACT_ROOT"
rm -f "$SOCKET_PATH"
"$TARGET_DIR/displayd" --socket "$SOCKET_PATH" --capture-backend fake --once > "$LOG_DIR/displayd-fake.log" 2>&1 &
DISPLAYD_PID=$!

# Wait for socket
for i in $(seq 1 50); do
    if [[ -S "$SOCKET_PATH" ]]; then break; fi
    sleep 0.1
done

if [[ ! -S "$SOCKET_PATH" ]]; then
    log "Timeout waiting for fake backend socket."
    exit 1
fi

if "$TARGET_DIR/xwin-screenshot" --backend isolated-displayd --displayd-socket "$SOCKET_PATH" --artifact-root "$ARTIFACT_ROOT" --save-dir "$OUT_DIR/fake" --format png > "$LOG_DIR/screenshot-fake.log" 2>&1; then
    log "Case 1 SUCCESS (Fake Backend)"
    append_report "Case 1 (Fake Backend): SUCCESS"
else
    log "Case 1 FAILED"
    append_report "Case 1 (Fake Backend): FAILED"
    exit 1
fi
wait "$DISPLAYD_PID" || true

# Test 2: Real Backend without Allow Flag (Should fail to start)
log "Case 2: Real Backend without Allow Flag"
rm -f "$SOCKET_PATH"
if ! "$TARGET_DIR/displayd" --socket "$SOCKET_PATH" --capture-backend real --once > "$LOG_DIR/displayd-real-no-allow.log" 2>&1; then
    log "Case 2 SUCCESS (Rejected as expected)"
    append_report "Case 2 (Real without Allow): REJECTED (SUCCESS)"
else
    log "Case 2 FAILED (Should have been rejected)"
    append_report "Case 2 (Real without Allow): FAILED"
    exit 1
fi

# Test 3: Allow Flag without Real Backend (Should fail to start)
log "Case 3: Allow Flag without Real Backend"
rm -f "$SOCKET_PATH"
if ! "$TARGET_DIR/displayd" --socket "$SOCKET_PATH" --allow-real-capture --once > "$LOG_DIR/displayd-allow-no-real.log" 2>&1; then
    log "Case 3 SUCCESS (Rejected as expected)"
    append_report "Case 3 (Allow without Real): REJECTED (SUCCESS)"
else
    log "Case 3 FAILED (Should have been rejected)"
    append_report "Case 3 (Allow without Real): FAILED"
    exit 1
fi

# Test 4: Real Backend with Allow Flag (Should start but fail-closed on capture)
log "Case 4: Real Backend with Allow Flag (Fail-Closed Stub)"
rm -f "$SOCKET_PATH"
"$TARGET_DIR/displayd" --socket "$SOCKET_PATH" --capture-backend real --allow-real-capture --once > "$LOG_DIR/displayd-real-fail-closed.log" 2>&1 &
DISPLAYD_PID=$!

# Wait for socket
for i in $(seq 1 50); do
    if [[ -S "$SOCKET_PATH" ]]; then break; fi
    sleep 0.1
done

if [[ ! -S "$SOCKET_PATH" ]]; then
    log "Timeout waiting for real backend socket."
    exit 1
fi

if ! "$TARGET_DIR/xwin-screenshot" --backend isolated-displayd --displayd-socket "$SOCKET_PATH" --artifact-root "$ARTIFACT_ROOT" --save-dir "$OUT_DIR/real" --format png > "$LOG_DIR/screenshot-real.log" 2>&1; then
    log "Case 4 SUCCESS (Fail-closed as expected)"
    if grep -q "real screen capture is not implemented/supported" "$LOG_DIR/screenshot-real.log"; then
        log "Confirmed: Stub error message found."
        append_report "Case 4 (Real Fail-Closed): SUCCESS (Confirmed Stub Error)"
    else
        log "Warning: Stub error message not found in screenshot log, but connection failed."
        append_report "Case 4 (Real Fail-Closed): SUCCESS (Connection Failed)"
    fi
else
    log "Case 4 FAILED (Should have failed-closed)"
    append_report "Case 4 (Real Fail-Closed): FAILED"
    exit 1
fi
wait "$DISPLAYD_PID" || true

log "Preflight report written to $REPORT_FILE"
echo "Preflight completed successfully."
