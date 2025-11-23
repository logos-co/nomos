#!/bin/bash
#
# Rebuild the rapidsnark prover for the current architecture.
#
# Usage: ./scripts/build-rapidsnark.sh <circuits_dir>

set -euo pipefail

if [ $# -lt 1 ]; then
    echo "usage: $0 <circuits_dir>" >&2
    exit 1
fi

TARGET_ARCH="$(uname -m)"
CIRCUITS_DIR="$1"
RAPIDSNARK_REPO="${RAPIDSNARK_REPO:-https://github.com/iden3/rapidsnark.git}"
RAPIDSNARK_REF="${RAPIDSNARK_REF:-main}"

if [ ! -d "$CIRCUITS_DIR" ]; then
    echo "circuits directory '$CIRCUITS_DIR' does not exist" >&2
    exit 1
fi

case "$TARGET_ARCH" in
    arm64 | aarch64)
        ;;
    *)
        echo "rapidsnark rebuild skipped for architecture '$TARGET_ARCH'" >&2
        exit 0
        ;;
esac

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

echo "Building rapidsnark ($RAPIDSNARK_REF) for $TARGET_ARCH..." >&2
git clone --depth 1 --branch "$RAPIDSNARK_REF" "$RAPIDSNARK_REPO" "$workdir/rapidsnark" >&2
cd "$workdir/rapidsnark"
git submodule update --init --recursive >&2

if [ "${RAPIDSNARK_BUILD_GMP:-1}" = "1" ]; then
    GMP_TARGET="${RAPIDSNARK_GMP_TARGET:-aarch64}"
    ./build_gmp.sh "$GMP_TARGET" >&2
fi

MAKE_TARGET="${RAPIDSNARK_MAKE_TARGET:-host_arm64}"
PACKAGE_DIR="${RAPIDSNARK_PACKAGE_DIR:-package_arm64}"

make "$MAKE_TARGET" -j"$(nproc)" >&2

install -m 0755 "${PACKAGE_DIR}/bin/prover" "$CIRCUITS_DIR/prover"
echo "rapidsnark prover installed to $CIRCUITS_DIR/prover" >&2
