#!/usr/bin/env bash

# Start poolore client

set -e

CLI=$HOME/miner/poolore/target/release/poolorec

MKP="$HOME/.config/solana/id.json"

THREADS=8

CMD="$CLI \
        --use-http \
        --url poolore.miraland.io \
        --keypair $MKP \
        mine --threads $THREADS"

echo "$CMD"
until bash -c "$CMD"; do
    echo "Starting client command failed. Restart..."
    sleep 2
done
