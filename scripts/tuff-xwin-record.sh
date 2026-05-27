#!/bin/bash
# TUFF-Xwin Screen Recording Tool
# Sends StartRecord / StopRecord command to displayd

RUNTIME_DIR="${WAYBROKER_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/tmp}/waybroker}"
OP="${1:-start}"
OUTPUT_NAME="${2:-eDP-1}"
SESSION_ID="${3:-}"

# Candidate paths
CANDIDATES=(
    "$RUNTIME_DIR/displayd.sock"
)
if [ -n "$SESSION_ID" ]; then
    CANDIDATES+=("$RUNTIME_DIR/$SESSION_ID/displayd.sock")
fi

SOCKET_PATH=""
for CANDIDATE in "${CANDIDATES[@]}"; do
    if [ -S "$CANDIDATE" ]; then
        SOCKET_PATH="$CANDIDATE"
        break
    fi
done

if [ -z "$SOCKET_PATH" ]; then
    echo "Error: displayd socket not found."
    echo "Searched candidates:"
    for CANDIDATE in "${CANDIDATES[@]}"; do
        echo "  - $CANDIDATE"
    done
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
  echo "Usage: $0 [start|stop] [output_name] [optional_session_id]"
  exit 1
fi

echo "Sending $OP request for $OUTPUT_NAME to $SOCKET_PATH..."
echo "$PAYLOAD" | socat - UNIX-CONNECT:"$SOCKET_PATH" | jq .
