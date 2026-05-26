#!/bin/bash
# TUFF-Xwin Idle Inhibition Tool
# Sends InhibitIdle / ReleaseIdle command to sessiond

RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp}/waybroker"
SESSION_ID="${1:-default-single-session}"
OP="${2:-inhibit}"
REASON="${3:-testing idle inhibition}"

SOCKET_PATH="$RUNTIME_DIR/$SESSION_ID/sessiond.sock"

if [ ! -S "$SOCKET_PATH" ]; then
    echo "Error: sessiond socket not found at $SOCKET_PATH"
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
  echo "Usage: $0 [session_id] [inhibit|release] [reason]"
  exit 1
fi

echo "Sending $OP request with reason \"$REASON\" to $SOCKET_PATH..."
echo "$PAYLOAD" | socat - UNIX-CONNECT:"$SOCKET_PATH" | jq .
