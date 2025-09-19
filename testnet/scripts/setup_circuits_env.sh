#!/bin/bash

if [ "$#" -ne 4 ]; then
    echo "Error: Invalid arguments." >&2
    echo "Usage: source $0 <BINARIES_DIR> <NODE_SRC_DIR> <VERSION> <PLATFORM>" >&2
    exit 1
fi

BINARIES_DIR="$1"
NODE_SRC_DIR="$2"
VERSION="$3"
PLATFORM="$4"

PROVER_VERIFIER_BINS=("prover" "verifier")
WITNESS_GEN_BINS=("poc" "pol" "poq" "zksign")

echo "Setting Nomos environment variables for ${VERSION} on ${PLATFORM}." >&2

for bin_name in "${PROVER_VERIFIER_BINS[@]}"; do
    var_name="NOMOS_$(echo "$bin_name" | tr '[:lower:]' '[:upper:]')"
    bin_path="${BINARIES_DIR}/${bin_name}-${VERSION}-${PLATFORM}/${bin_name}/${bin_name}"
    export "$var_name"="$bin_path"
done

for bin_name in "${WITNESS_GEN_BINS[@]}"; do
    var_name="NOMOS_$(echo "$bin_name" | tr '[:lower:]' '[:upper:]')"
    bin_path="${BINARIES_DIR}/${bin_name}-${VERSION}-${PLATFORM}/witness-generator/${bin_name}"
    export "$var_name"="$bin_path"
done

export NOMOS_POL_PROVING_KEY_PATH="${NODE_SRC_DIR}/zk/proofs/pol/src/proving_key/pol.zkey"
export NOMOS_POC_PROVING_KEY_PATH="${NODE_SRC_DIR}/zk/proofs/poc/src/proving_key/proof_of_claim.zkey"
export NOMOS_POQ_PROVING_KEY_PATH="${NODE_SRC_DIR}/zk/proofs/poq/src/proving_key/poq.zkey"
export NOMOS_ZKSIGN_PROVING_KEY_PATH="${NODE_SRC_DIR}/zk/proofs/zksign/src/proving_key/zksign.zkey"

echo "Environment variables set successfully." >&2
