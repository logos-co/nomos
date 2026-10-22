#!/usr/bin/env bash
# Build the node and the lab harness, run a local cluster for a while, then
# turn its logs into the report. Any extra arguments go to the harness:
#
#   tools/block-times-lab/run.sh --minutes 20 --f 1/4 --release-delay-rounds 3
#
# Env: OUT (runs dir, default tools/block-times-lab/runs), PYTHON (interpreter
# with numpy+matplotlib, default python3), SKIP_BUILD=1 to reuse binaries.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
out="${OUT:-$here/runs}"
python="${PYTHON:-python3}"
mkdir -p "$out"

if [[ "${SKIP_BUILD:-0}" != 1 ]]; then
    (cd "$repo" && cargo build -p logos-blockchain-node -p logos-blockchain-tests \
        --bin logos-blockchain-node --bin block-times-lab)
fi

export LOGOS_BLOCKCHAIN_NODE_BIN="${LOGOS_BLOCKCHAIN_NODE_BIN:-$repo/target/debug/logos-blockchain-node}"
export E2E_KEEP_LOGS=1

log="$out/harness-$(date +%Y%m%d-%H%M%S).log"
"$repo/target/debug/block-times-lab" --out "$out" "$@" 2>&1 | tee "$log"
scenario="$(grep -m1 '^scenario dir: ' "$log" | sed 's/^scenario dir: //')"
[[ -d "$scenario" ]] || { echo "scenario dir not found in $log" >&2; exit 1; }

"$python" -I "$here/collect.py" "$scenario"
"$python" -I "$here/report.py" "$scenario"
