#!/bin/bash

cargo depgraph --all-deps --dedup-transitive-deps --workspace-only --all-features > ./dependencies_graph.dot
