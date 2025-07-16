# BUILD IMAGE ---------------------------------------------------------

FROM rust:1.88.0-slim-bookworm AS builder

LABEL maintainer="augustinas@status.im" \
    source="https://github.com/logos-co/nomos-node" \
    description="Nomos testnet image"

WORKDIR /nomos
COPY . .

# Install dependencies needed for building RocksDB and etcd.
RUN apt-get update && apt-get install -yq \
    git gcc g++ clang libssl-dev pkg-config ca-certificates

RUN cargo install cargo-binstall --locked
RUN cargo install rzup --version 0.4.1 --locked
RUN rzup install cargo-risczero 2.3.0
RUN rzup install rust 1.88.0

RUN cargo build --release

RUN cp /nomos/target/release/nomos-node /usr/bin/nomos-node && \
    cp /nomos/target/release/nomos-executor /usr/bin/nomos-executor && \
    cp /nomos/target/release/nomos-cli /usr/bin/nomos-cli && \
    cp /nomos/target/release/cfgsync-server /usr/bin/cfgsync-server && \
    cp /nomos/target/release/cfgsync-client /usr/bin/cfgsync-client

# nomos default ports
EXPOSE 3000 8080 9000 60000

ENTRYPOINT ["/usr/bin/nomos-node"]
