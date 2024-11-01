#!/usr/bin/env bash

# Start hashpoo client

set -e

CLI=$HOME/miner/hashpoo/target/release/hpc

MKP="$HOME/.config/solana/id.json"

THREADS=8

CMD="$CLI \
        --url ore.hashspace.me \
        --keypair $MKP \
        mine --threads $THREADS"

echo "$CMD"
until bash -c "$CMD"; do
    echo "Starting Hashpoo client command failed. Restart..."
    sleep 2
done
