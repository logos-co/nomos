#!/bin/bash
set -euo pipefail

REPO="logos-co/nomos-pocs"
BINARIES=("poc" "pol" "poq" "prover" "verifier" "zksign")
OUTPUT_DIR="bin/circuits"

if [ "$#" -ne 2 ]; then
    echo "Error: Invalid number of arguments." >&2
    echo "Usage: $0 <VERSION> <PLATFORM>" >&2
    echo "Example: $0 v0.3.0 linux-x86_64" >&2
    exit 1
fi

VERSION="$1"
PLATFORM="$2"

RELEASE_TAG="circom_circuits-${VERSION}"
PLATFORM_DIR="${OUTPUT_DIR}/${PLATFORM}"
mkdir -p "$PLATFORM_DIR"

echo "Downloading and extracting binaries."

for bin_name in "${BINARIES[@]}"; do
    FILENAME="${bin_name}-${VERSION}-${PLATFORM}.tar.gz"
    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${RELEASE_TAG}/${FILENAME}"
    echo "Attempting to download ${FILENAME}."

    if ! curl -sL "$DOWNLOAD_URL" | tar -xz -C "$PLATFORM_DIR"; then
        echo "Warning: Could not download ${FILENAME}. Skipping." >&2
    fi
done

chmod +x "$OUTPUT_DIR"/*

echo "Binaries are ready in the '$OUTPUT_DIR' directory:"
ls -l "$OUTPUT_DIR"
