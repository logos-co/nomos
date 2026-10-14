#!/bin/sh

set -e

NODE_COUNT="${BLEND_NODE_COUNT}"
NET_BASE="${BLEND_NET_PORT_BASE:-3000}"
BLEND_BASE="${BLEND_BLEND_PORT_BASE:-3400}"
API_BASE="${BLEND_API_PORT_BASE:-18080}"
INITIAL_PEERS="${BLEND_INITIAL_PEERS:-}"

BIN=/usr/bin/logos-blockchain-node

idx=0
while [ "$idx" -lt "$NODE_COUNT" ]; do
    node_dir="/node-data/${idx}"
    config="${node_dir}/user_config.yaml"
    keystore="${node_dir}/keystore.yaml"

    mkdir -p "$node_dir"

    find "$node_dir" -mindepth 1 -maxdepth 1 \
        ! -name 'user_config.yaml' \
        ! -name 'keystore.yaml' \
        -exec rm -rf {} +

    if [ -f "$config" ]; then
        echo "node ${idx}: keeping existing user_config.yaml"
        idx=$((idx + 1))
        continue
    fi

    if [ -f "$keystore" ]; then
        echo "node ${idx}: keystore.yaml present but user_config.yaml missing." >&2
        echo "  Restore the config, or delete ${keystore} to mint a new identity." >&2
        exit 1
    fi

    net_port=$((NET_BASE + idx))
    blend_port=$((BLEND_BASE + idx))
    api_port=$((API_BASE + idx))

    echo "node ${idx}: generating user_config.yaml (net ${net_port}, blend ${blend_port}, api ${api_port})"

    set -- \
        --output "$config" \
        --keystore "$keystore" \
        --net-port "$net_port" \
        --blend-addr "/ip4/0.0.0.0/udp/${blend_port}/quic-v1" \
        --http-host "0.0.0.0:${api_port}" \
        --state-path "${node_dir}/state"

    if [ -n "$INITIAL_PEERS" ]; then
        set -- "$@" --net-initial-peers $INITIAL_PEERS
    else
        echo "node ${idx}: warning, BLEND_INITIAL_PEERS is empty" >&2
    fi

    "$BIN" init-config "$@"
    idx=$((idx + 1))
done

/etc/logos-blockchain/scripts/blend/extract_provider_keys.sh
