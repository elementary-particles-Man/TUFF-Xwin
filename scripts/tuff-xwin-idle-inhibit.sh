#!/bin/bash
# TUFF-Xwin Idle Inhibition Tool
# Sends InhibitIdle / ReleaseIdle command to sessiond

RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker}"
OP="${1:-inhibit}"
REASON="${2:-testing idle inhibition}"
SESSION_ID="${3:-}"

# Candidate paths
CANDIDATES=(
    "$RUNTIME_DIR/sessiond.sock"
)
if [ -n "$SESSION_ID" ]; then
    CANDIDATES+=("$RUNTIME_DIR/$SESSION_ID/sessiond.sock")
fi

SOCKET_PATH=""
for CANDIDATE in "${CANDIDATES[@]}"; do
    if [ -S "$CANDIDATE" ]; then
        SOCKET_PATH="$CANDIDATE"
        break
    fi
done

if [ -z "$SOCKET_PATH" ]; then
    echo "Error: sessiond socket not found."
    echo "Searched candidates:"
    for CANDIDATE in "${CANDIDATES[@]}"; do
        echo "  - $CANDIDATE"
    done
    exit 1
fi

if [ "$OP" == "inhibit" ]; then
  PAYLOAD=$(cat <<EOF
{
  "source": "waylandd",
  "destination": "sessiond",
  "kind": {
    "kind": "session-command",
    "payload": {
      "op": "inhibit-idle",
      "reason": "$REASON"
    }
  }
}
EOF
)
elif [ "$OP" == "release" ]; then
  PAYLOAD=$(cat <<EOF
{
  "source": "waylandd",
  "destination": "sessiond",
  "kind": {
    "kind": "session-command",
    "payload": {
      "op": "release-idle",
      "reason": "$REASON"
    }
  }
}
EOF
)
else
  echo "Usage: $0 [inhibit|release] [reason] [optional_session_id]"
  exit 1
fi

echo "Sending $OP request with reason \"$REASON\" to $SOCKET_PATH..."
echo "$PAYLOAD" | socat - UNIX-CONNECT:"$SOCKET_PATH" | jq .
