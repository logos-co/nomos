# Tests

## Tests Debugging Setup

This document provides instructions for setting up and using the testing environment, including how to start the Docker setup, run tests with a feature flag, and access the Grafana dashboard.

## Prerequisites

Ensure that the following are installed on your system:
- [Docker](https://docs.docker.com/get-docker/)
- [Docker Compose](https://docs.docker.com/compose/install/)

## Setup and Usage

### 1. Start `compose.debug.yml`

To start the services defined in `compose.debug.yml` using Docker Compose, run the following command:

```bash
docker-compose -f compose.debug.yml up -d
```

This command will:
    Use the configuration specified in compose.debug.yml.
    Start all services in detached mode (-d), allowing the terminal to be used for other commands.

To stop the services, you can run:
```
docker-compose -f compose.debug.yml down   # compose filename needs to be the same
```

### 2. Run Tests with Debug Feature Flag

To execute the test suite with the debug feature flag, use the following command:

```bash
POL_PROOF_DEV_MODE=true cargo test -p tests -F debug disseminate_and_retrieve
```

`-F debug`: Enables the debug feature flag for the integration tests, allowing for extra debug output or specific debug-only code paths to be enabled during the tests.
To modify the tracing configuration when using `-F debug` flag go to `tests/src/topology/configs/tracing.rs`. If debug flag is not used, logs will be written into each nodes temporary directory.

### 3. Access the Grafana Dashboard
> It's important that the test is performed after the docker compose is started

Once the Docker setup is running, you can access the Grafana dashboard to view metrics and logs:
    Open a browser and navigate to http://localhost:9091.

Use "Explore" tab to select data source: "Loki", "Tempo", "Prometheus". Prometheus source is unusable at the moment in local setup.

- Loki - to kickstart your query, select "host" as label filter, and "nomo-0" or other nodes as value, this will show all logs for selected host.
- Tempo - to kickstart your query, enter "{}" as TraceQL query to see all traces.


