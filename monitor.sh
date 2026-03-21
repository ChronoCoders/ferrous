#!/bin/bash
# Ferrous node health monitor — runs every 5 minutes via nohup
# Logs block height, best hash, peer count, and difficulty to sync_log.txt

RPC="http://127.0.0.1:8332"

while true; do
    TS=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    INFO=$(printf '{"jsonrpc":"2.0","method":"getblockchaininfo","params":[],"id":1}' \
        | curl -s -X POST "$RPC" -H "Content-Type: application/json" -d @-)

    BLOCKS=$(echo "$INFO" | grep -o '"blocks":[0-9]*' | grep -o '[0-9]*')
    HASH=$(echo "$INFO"   | grep -o '"bestblockhash":"[^"]*"' | cut -d'"' -f4)

    PEERS=$(printf '{"jsonrpc":"2.0","method":"getconnectioncount","params":[],"id":1}' \
        | curl -s -X POST "$RPC" -H "Content-Type: application/json" -d @- \
        | grep -o '"result":[0-9]*' | grep -o '[0-9]*')

    MINE=$(printf '{"jsonrpc":"2.0","method":"getmininginfo","params":[],"id":1}' \
        | curl -s -X POST "$RPC" -H "Content-Type: application/json" -d @-)

    DIFF=$(echo "$MINE" | grep -o '"difficulty":[^,}]*' | cut -d: -f2)

    echo "$TS  height=$BLOCKS  peers=${PEERS:-0}  diff=$DIFF  hash=$HASH"

    sleep 300
done
