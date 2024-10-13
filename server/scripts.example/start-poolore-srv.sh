#!/usr/bin/env bash

# Start ore mining public pool server

set -e

SRV=$HOME/miner/poolore/target/release/poolores

# MKP="$HOME/.config/solana/id.json"

# default dynamic fee url. Uncomment next line if you plan to enable dynamic-fee mode
# DYNAMIC_FEE_URL="YOUR_RPC_URL_HERE"

BUFFER_TIME=5
RISK_TIME=0
PRIORITY_FEE=1000
PRIORITY_FEE_CAP=100000

EXP_MIN_DIFF=8
MESSAGING_DIFF=25
XTR_FEE_DIFF=25
XTR_FEE_PCT=900

CMD="$SRV \
        --buffer-time $BUFFER_TIME \
        --risk-time $RISK_TIME \
        --priority-fee $PRIORITY_FEE \
        --priority-fee-cap $PRIORITY_FEE_CAP \
        --expected-min-difficulty $EXP_MIN_DIFF \
        --messaging-diff $MESSAGING_DIFF \
        --extra-fee-difficulty $XTR_FEE_DIFF \
        --extra-fee-percent $XTR_FEE_PCT"

echo "$CMD"
until bash -c "$CMD"; do
    echo "Starting server command failed. Restart..."
    sleep 2
done
