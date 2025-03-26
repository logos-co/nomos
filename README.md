# Nomos

Nomos is the blockchain layer of the Logos technology stack, providing a privacy-preserving and censorship-resistant
framework for decentralized network states.

This monorepo serves as a unified codebase for the Nomos ecosystem, housing all core components, services, and tools
necessary for running and interacting with the Nomos blockchain. Key features include:

- Consensus mechanisms for secure and scalable network agreement
- Ledger management for state persistence and validation
- Networking layers leveraging libp2p for peer-to-peer communication
- CLI tools and clients for seamless interaction with the blockchain
- Testnet configurations for development and experimentation

## Table of Contents

- [Nomos](#nomos)
  - [Table of Contents](#table-of-contents)
  - [Requirements](#requirements)
  - [Design Goals](#design-goals)
    - [Service Architecture](#service-architecture)
    - [Static Dispatching](#static-dispatching)
  - [Project Structure](#project-structure)
  - [Development Workflow](#development-workflow)
    - [Docker](#docker)
      - [Building the Image](#building-the-image)
      - [Running a Nomos Node](#running-a-nomos-node)
  - [Running Tests](#running-tests)
  - [Generating Documentation](#generating-documentation)
  - [Contributing](#contributing)
  - [License](#license)
  - [Community](#community)

## Requirements

- **Rust**
    - We aim to maintain compatibility with the latest stable version of Rust.
    - [Installation Guide](https://www.rust-lang.org/tools/install)

- **Risc0**
    - Required for zero-knowledge proof functionality.
    - [Installation Guide](https://dev.risczero.com/api/zkvm/install)

## Design Goals

### Service Architecture

Nomos services follow a consistent design pattern: a front layer handles the `Overwatch` service, while a back layer
implements the actual service logic.

This modular approach allows for easy replacement of components in a declarative manner.

For example:

```rust ignore
#[derive_services]
struct MockPoolNode {
    logging: Logger,
    network: NetworkService<Waku>,
    mockpool: MempoolService<WakuAdapter<Tx>, MockPool<TxId, Tx>>,
    http: HttpService<AxumBackend>,
    bridges: HttpBridgeService,
}
```

### Static Dispatching

Nomos favours static dispatching over dynamic, influenced by Overwatch.
This means you'll encounter Generics sprinkled throughout the codebase.
While it might occasionally feel a bit over the top, it brings some solid advantages, such as:

- Compile-time type checking
- Highly modular and adaptable applications

## Project Structure

```
nomos/
├── book/               # Documentation in Markdown format
├── ci/                 # Non-GitHub scripts, such as Jenkins' nightly integration and fuzzy testing
├── clients/            # General-purpose clients
├── consensus/          # Engine and protocols for agreement and validation
├── ledger/             # Ledger management and state transition logic
├── nodes/              # Node implementations
├── nomos-blend/        # Blend Network, our privacy routing protocol
├── nomos-bundler/      # Crate packaging and bundling
├── nomos-cli/          # Command-line interface for interacting with the Nomos blockchain
├── nomos-core/         # Collection of essential structures
├── nomos-da/           # Data availability layer
├── nomos-libp2p/       # Libp2p integration
├── nomos-services/     # Building blocks for the Node
├── nomos-tracing/      # Tracing, logging, and metrics
├── nomos-utils/        # Shared utility functions and helpers
├── testnet/            # Testnet configurations, monitoring, and deployment scripts
└── tests/              # Integration and E2E test suites
```

## Development Workflow

### Docker

#### Building the Image

To build the Nomos Docker image, run:

```bash
docker build -t nomos .
```

#### Running a Nomos Node

To run a docker container with the Nomos node you need to mount both `config.yml` and `global_params_path` specified in
the configuration.

```bash
docker run -v "/path/to/config.yml" -v "/path/to/global_params:global/params/path" nomos /etc/nomos/config.yml
```

To use an example configuration located at `nodes/nomos-node/config.yaml`, first run the test that generates the random
kzgrs file and then run the docker container with the appropriate config and global params:

```bash
cargo test --package kzgrs-backend write_random_kzgrs_params_to_file -- --ignored

docker run -v "$(pwd)/nodes/nomos-node/config.yaml:/etc/nomos/config.yml" -v "$(pwd)/nomos-da/kzgrs-backend/kzgrs_test_params:/app/tests/kzgrs/kzgrs_test_params" nomos /etc/nomos/config.yml

```

## Running Tests

To run the test suite, use:

```bash
cargo test
```

## Generating Documentation

To generate the project documentation locally, run:

```bash
cargo doc
```

## Contributing

We welcome contributions! Please read our [Contributing Guidelines](CONTRIBUTING.md) for details on how to get started.

## License

This project is primarily distributed under the terms defined by either the MIT license or the
Apache License (Version 2.0), at your option.

See [LICENSE-APACHE2.0](LICENSE-APACHE2.0) and [LICENSE-MIT](LICENSE-MIT) for details.

## Community

Join the Nomos community on [Discord](https://discord.gg/8Q7Q7vz) and follow us
on [Twitter](https://twitter.com/nomos_tech).

For more information, visit [nomos.tech](https://nomos.tech/?utm_source=chatgpt.com).
