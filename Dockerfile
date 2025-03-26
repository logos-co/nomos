# ===========================
# BUILD IMAGE
# ===========================

FROM rust:1.85.0-slim-bookworm AS builder

LABEL maintainer="augustinas@status.im" \
      source="https://github.com/logos-co/nomos-node" \
      description="Nomos node image"

WORKDIR /nomos
COPY . .

# Install dependencies needed for building RocksDB, etc.
RUN apt-get update && apt-get install -yq \
    git gcc g++ clang libssl-dev pkg-config ca-certificates

RUN cargo install cargo-binstall --locked

# Versions of cargo-risczero > 1.2.0 use rzup install instead
RUN cargo binstall cargo-risczero@2.0.0 --no-confirm
RUN cargo risczero install --version r0.1.85.0

RUN cargo build --release -p nomos-node

RUN cp /nomos/target/release/nomos-node /usr/bin/nomos-node

# Expose default ports
EXPOSE 3000 8080 9000 60000

ENTRYPOINT ["/usr/bin/nomos-node"]
