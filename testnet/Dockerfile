# syntax=docker/dockerfile:1
# check=skip=SecretsUsedInArgOrEnv
# Ignore warnings about sensitive information as this is test data.

ARG VERSION=v0.3.0
ARG PLATFORM=linux-x86_64

# ===========================
# BUILD IMAGE
# ===========================

FROM rust:1.90.0-slim-bookworm AS builder

ARG VERSION
ARG PLATFORM

LABEL maintainer="augustinas@status.im" \
    source="https://github.com/logos-co/nomos-node" \
    description="Nomos testnet build image"

WORKDIR /nomos
COPY . .

# Install dependencies needed for building RocksDB.
RUN apt-get update && apt-get install -yq \
    git gcc g++ clang libssl-dev pkg-config ca-certificates curl

RUN chmod +x testnet/scripts/download_circuit_binaries.sh && \
    testnet/scripts/download_circuit_binaries.sh "$VERSION" "$PLATFORM"

RUN cargo build --release --all-features

# ===========================
# NODE IMAGE
# ===========================

FROM debian:bookworm-slim

ARG VERSION
ARG PLATFORM

LABEL maintainer="augustinas@status.im" \
    source="https://github.com/logos-co/nomos-node" \
    description="Nomos node image"

RUN apt-get update && apt-get install -yq \
    libstdc++6 \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /nomos/zk/proofs /opt/nomos/proofs
COPY --from=builder /nomos/bin/circuits /opt/nomos/circuits

COPY --from=builder /nomos/target/release/nomos-node /usr/bin/nomos-node
COPY --from=builder /nomos/target/release/nomos-executor /usr/bin/nomos-executor
COPY --from=builder /nomos/target/release/nomos-cli /usr/bin/nomos-cli
COPY --from=builder /nomos/target/release/cfgsync-server /usr/bin/cfgsync-server
COPY --from=builder /nomos/target/release/cfgsync-client /usr/bin/cfgsync-client

ENV NOMOS_POL_PROVING_KEY_PATH="/opt/nomos/proofs/pol/src/proving_key/pol.zkey" \
    NOMOS_POC_PROVING_KEY_PATH="/opt/nomos/proofs/poc/src/proving_key/proof_of_claim.zkey" \
    NOMOS_POQ_PROVING_KEY_PATH="/opt/nomos/proofs/poq/src/proving_key/poq.zkey" \
    NOMOS_ZKSIGN_PROVING_KEY_PATH="/opt/nomos/proofs/zksign/src/proving_key/zksign.zkey"

ENV NOMOS_PROVER="/opt/nomos/circuits/${PLATFORM}/prover-${VERSION}-${PLATFORM}/prover/prover" \
    NOMOS_VERIFIER="/opt/nomos/circuits/${PLATFORM}/verifier-${VERSION}-${PLATFORM}/verifier/verifier" \
    NOMOS_POC="/opt/nomos/circuits/${PLATFORM}/poc-${VERSION}-${PLATFORM}/witness-generator/poc" \
    NOMOS_POL="/opt/nomos/circuits/${PLATFORM}/pol-${VERSION}-${PLATFORM}/witness-generator/pol" \
    NOMOS_POQ="/opt/nomos/circuits/${PLATFORM}/poq-${VERSION}-${PLATFORM}/witness-generator/poq" \
    NOMOS_ZKSIGN="/opt/nomos/circuits/${PLATFORM}/zksign-${VERSION}-${PLATFORM}/witness-generator/zksign"

EXPOSE 3000 8080 9000 60000

ENTRYPOINT ["/usr/bin/nomos-node"]
