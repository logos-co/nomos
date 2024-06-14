#!/bin/sh

set -e

CONSENSUS_CHAIN_START=$(date +%s)
CONSENSUS_COIN_SK=$BOOTSTRAP_NODE_KEY
CONSENSUS_COIN_NONCE=$BOOTSTRAP_NODE_KEY
CONSENSUS_COIN_VALUE=1
DA_VOTER=$BOOTSTRAP_NODE_KEY
NET_NODE_KEY=$BOOTSTRAP_NODE_KEY
OVERLAY_NODES=$(/etc/nomos/scripts/consensus_node_list.sh)

export CONSENSUS_COIN_SK \
       CONSENSUS_COIN_NONCE \
       CONSENSUS_COIN_VALUE \
       CONSENSUS_CHAIN_START \
       DA_VOTER \
       OVERLAY_NODES \
       NET_NODE_KEY

echo "I am a container ${HOSTNAME} node ${NET_NODE_KEY}"
echo "CONSENSUS_COIN_SK: ${CONSENSUS_COIN_SK}"
echo "CONSENSUS_COIN_NONCE: ${CONSENSUS_COIN_NONCE}"
echo "CONSENSUS_COIN_VALUE: ${CONSENSUS_COIN_VALUE}"
echo "DA_VOTER: ${DA_VOTER}"
echo "OVERLAY_NODES: ${OVERLAY_NODES}"

exec /usr/bin/nomos-node /etc/nomos/bootstrap_config.yaml --with-metrics --log-backend gelf --log-addr graylog:12201
