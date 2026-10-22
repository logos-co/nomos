#!/usr/bin/env bash
# Gather what report.py needs from nodes started with the deployment's
# compose.blend.run.yml: each node's log files from the node-data volume, and
# its /metrics, /time/info and /cryptarchia/info from the API port published
# on the host's loopback.
#
#   tools/block-times-lab/deploy-collect.sh OUT_DIR [--count 10] [--api-base 18080]
#       [--container-prefix blend-node-] [--data-root /node-data]
#
# The result has one <idx>/ directory per node with its logos-blockchain.log.*
# files, plus metrics-node-<idx>.txt and time-info-node-<idx>.json, which is the
# layout collect.py and report.py read:
#
#   python3 tools/block-times-lab/collect.py OUT_DIR
#   python3 tools/block-times-lab/report.py OUT_DIR \
#       --deployment nodes/node/binary/src/config/deployment/settings.yaml
set -euo pipefail

out="${1:?usage: deploy-collect.sh OUT_DIR [--count N] [--api-base PORT] [--container-prefix P] [--data-root DIR]}"
shift
count=10
api_base=18080
prefix=blend-node-
data_root=/node-data
while [[ $# -gt 0 ]]; do
    case "$1" in
        --count) count="$2"; shift 2 ;;
        --api-base) api_base="$2"; shift 2 ;;
        --container-prefix) prefix="$2"; shift 2 ;;
        --data-root) data_root="$2"; shift 2 ;;
        *) echo "unknown flag $1" >&2; exit 1 ;;
    esac
done
mkdir -p "$out"

for ((idx = 0; idx < count; idx++)); do
    container="${prefix}${idx}"
    port=$((api_base + idx))
    node_out="$out/$idx"
    mkdir -p "$node_out"

    # Log files: copied out of the container's node-data directory. The
    # `.latest` symlink is dropped so collect.py does not read a file twice.
    if docker cp "$container:$data_root/$idx/." "$node_out/" 2>/dev/null; then
        find "$node_out" -mindepth 1 -maxdepth 1 \
            ! -name 'logos-blockchain.log*' -exec rm -rf {} +
        find "$node_out" -maxdepth 1 -type l -delete
        echo "$container: $(ls "$node_out" | wc -l) log files"
    else
        echo "$container: docker cp failed; is the container running?" >&2
    fi

    for endpoint in metrics time/info cryptarchia/info; do
        name="${endpoint//\//-}"
        case "$endpoint" in
            metrics) target="$out/metrics-node-$idx.txt" ;;
            *) target="$out/$name-node-$idx.json" ;;
        esac
        if ! curl -sSf --max-time 10 "http://127.0.0.1:$port/$endpoint" -o "$target"; then
            echo "$container: GET /$endpoint on port $port failed" >&2
            rm -f "$target"
        fi
    done
done

echo "collected into $out"
