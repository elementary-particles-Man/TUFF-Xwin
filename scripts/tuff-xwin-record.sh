#!/bin/bash
# TUFF-Xwin Screen Recording Tool
# Sends StartRecord / StopRecord command to displayd

RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp}/waybroker"
SESSION_ID="${1:-default-single-session}"
OP="${2:-start}"
OUTPUT_NAME="${3:-eDP-1}"

SOCKET_PATH="$RUNTIME_DIR/$SESSION_ID/displayd.sock"

if [ ! -S "$SOCKET_PATH" ]; then
    echo "Error: displayd socket not found at $SOCKET_PATH"
    exit 1
fi

if [ "$OP" == "start" ]; then
  PAYLOAD=$(cat <<EOF
{
  "source": "sessiond",
  "destination": "displayd",
  "kind": {
    "kind": "display-command",
    "payload": {
      "op": "start-record",
      "output": "$OUTPUT_NAME",
      "fps": 30
    }
  }
}
EOF
)
elif [ "$OP" == "stop" ]; then
  PAYLOAD=$(cat <<EOF
{
  "source": "sessiond",
  "destination": "displayd",
  "kind": {
    "kind": "display-command",
    "payload": {
      "op": "stop-record",
      "output": "$OUTPUT_NAME"
    }
  }
}
EOF
)
else
  echo "Usage: $0 [session_id] [start|stop] [output_name]"
  exit 1
fi

echo "Sending $OP request for $OUTPUT_NAME to $SOCKET_PATH..."
echo "$PAYLOAD" | socat - UNIX-CONNECT:"$SOCKET_PATH" | jq .
