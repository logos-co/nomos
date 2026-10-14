#!/bin/sh

set -e

OUT="${BLEND_PROVIDERS_FILE:-/node-data/providers.yaml}"

if ! command -v yq > /dev/null 2>&1; then
    apt-get update -qq > /dev/null 2>&1
    apt-get install -y --no-install-recommends yq > /dev/null 2>&1
fi

: > "$OUT"

for idx in $(ls /node-data | grep -xE '[0-9]+' | sort -n); do
    yq -r "\"# node ${idx}\", \
           \"- blend_zk: \" + .public_keys.BlendZk, \
           \"  sdp_funding: \" + .public_keys.SdpFunding, \
           \"\"" "/node-data/${idx}/keystore.yaml" >> "$OUT"
done

echo "wrote ${OUT}"
