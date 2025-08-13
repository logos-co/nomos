#!/bin/bash

RISC0_SKIP_BUILD=true RUSTFLAGS="-D warnings" cargo hack --feature-powerset --no-dev-deps check
