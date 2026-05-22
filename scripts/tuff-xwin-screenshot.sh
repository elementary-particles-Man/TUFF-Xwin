#!/bin/bash
# TUFF-Xwin Screenshot Tool
# Sends CaptureOutput command to displayd

RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp}/waybroker"
SESSION_ID="${1:-default-single-session}"
OUTPUT_NAME="${2:-eDP-1}"

SOCKET_PATH="$RUNTIME_DIR/$SESSION_ID/displayd.sock"

if [ ! -S "$SOCKET_PATH" ]; then
    echo "Error: displayd socket not found at $SOCKET_PATH"
    exit 1
fi

PAYLOAD=$(cat <<EOF
{
  "source": "sessiond",
  "destination": "displayd",
  "kind": {
    "kind": "display-command",
    "payload": {
      "op": "capture-output",
      "output": "$OUTPUT_NAME"
    }
  }
}
EOF
)

echo "Sending screenshot request for $OUTPUT_NAME to $SOCKET_PATH..."
echo "$PAYLOAD" | socat - UNIX-CONNECT:"$SOCKET_PATH" | jq .
