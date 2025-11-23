# syntax=docker/dockerfile:1

ARG VERSION=v0.3.1
ARG NOMOS_CIRCUITS_PLATFORM=linux-x86_64

# ===========================
# BUILD IMAGE
# ===========================

FROM rust:1.91.0-slim-bookworm AS builder

ARG VERSION
ARG NOMOS_CIRCUITS_PLATFORM
ARG TARGETARCH

LABEL maintainer="logos devs" \
    source="https://github.com/logos-co/nomos-node" \
    description="Nomos testing framework build image"

WORKDIR /nomos
COPY . .

RUN apt-get update && apt-get install -yq \
    git gcc g++ clang libssl-dev pkg-config ca-certificates curl wget \
    build-essential cmake libgmp-dev libsodium-dev nasm m4 && \
    rm -rf /var/lib/apt/lists/*

ENV NOMOS_CIRCUITS_PLATFORM=${NOMOS_CIRCUITS_PLATFORM}

RUN chmod +x scripts/setup-nomos-circuits.sh && \
    scripts/setup-nomos-circuits.sh "$VERSION" "/opt/circuits"

RUN if [ "${TARGETARCH:-amd64}" = "arm64" ]; then \
        chmod +x scripts/build-rapidsnark.sh && \
        scripts/build-rapidsnark.sh "/opt/circuits"; \
    fi

ENV NOMOS_CIRCUITS=/opt/circuits

# Use debug builds to keep the linker memory footprint low; we only need
# binaries for integration testing, not optimized releases.
RUN cargo build --all-features

# ===========================
# NODE IMAGE
# ===========================

FROM debian:bookworm-slim

ARG VERSION

LABEL maintainer="logos devs" \
    source="https://github.com/logos-co/nomos-node" \
    description="Nomos testing framework runtime image"

RUN apt-get update && apt-get install -yq \
    libstdc++6 \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /opt/circuits /opt/circuits

COPY --from=builder /nomos/target/debug/nomos-node /usr/bin/nomos-node
COPY --from=builder /nomos/target/debug/nomos-executor /usr/bin/nomos-executor
COPY --from=builder /nomos/target/debug/nomos-cli /usr/bin/nomos-cli
COPY --from=builder /nomos/target/debug/cfgsync-server /usr/bin/cfgsync-server
COPY --from=builder /nomos/target/debug/cfgsync-client /usr/bin/cfgsync-client

ENV NOMOS_CIRCUITS=/opt/circuits

EXPOSE 3000 8080 9000 60000

ENTRYPOINT ["/usr/bin/nomos-node"]
