#!/bin/bash
# txgen.sh — Automatic real transaction generator for Ferrous testnet
#
# Sends a random amount (1–10 FRR) to DEST_ADDR every INTERVAL seconds
# once the chain reaches coinbase maturity (>100 blocks).
#
# Usage:
#   On seed1: nohup bash /root/ferrous/txgen.sh myqpJpMZ14CrHTo7NMRpTrNAzkoSM1oa1W >> /root/ferrous/txgen_log.txt 2>&1 &
#   On seed4: nohup bash /root/ferrous/txgen.sh mjyDe3YRuTPRRbYCanhD4s8WMyDZRwn2p1 >> /root/ferrous/txgen_log.txt 2>&1 &

DEST_ADDR="${1}"
MIN_HEIGHT=101
INTERVAL=20
LOG="/root/ferrous/txgen_log.txt"

if [ -z "$DEST_ADDR" ]; then
    echo "Usage: txgen.sh <dest_address>" >&2
    exit 1
fi

rpc() {
    printf '{"jsonrpc":"2.0","method":"%s","params":%s,"id":1}' "$1" "$2" | \
        curl -s -X POST http://127.0.0.1:8332 \
             -H 'Content-Type: application/json' -d @-
}

echo "$(date -u +"%Y-%m-%dT%H:%M:%SZ")  txgen started, dest=$DEST_ADDR, waiting for height >$MIN_HEIGHT" >> "$LOG"

while true; do
    TS=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    HEIGHT=$(rpc getblockchaininfo '[]' \
        | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('blocks',0))" 2>/dev/null \
        || echo 0)

    if [ "$HEIGHT" -le "$MIN_HEIGHT" ]; then
        echo "$TS  height=$HEIGHT, waiting for maturity (need >$MIN_HEIGHT)" >> "$LOG"
        sleep "$INTERVAL"
        continue
    fi

    AMOUNT=$(python3 -c "import random; print(round(random.uniform(1, 10), 4))")
    RESULT=$(rpc sendtoaddress "[\"$DEST_ADDR\", $AMOUNT]")
    TXID=$(echo "$RESULT" \
        | python3 -c "import sys,json; r=json.load(sys.stdin); print(r.get('result',{}).get('txid','ERR'))" 2>/dev/null \
        || echo "ERR")

    echo "$TS  height=$HEIGHT  →$DEST_ADDR  ${AMOUNT} FRR  txid=$TXID" >> "$LOG"
    sleep "$INTERVAL"
done
