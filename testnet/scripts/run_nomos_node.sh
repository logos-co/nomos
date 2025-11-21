#!/bin/sh

set -e

CFG_SERVER_ADDR="${CFG_SERVER_ADDR:-http://cfgsync:4400}"

export CFG_FILE_PATH="/config.yaml" \
       CFG_SERVER_ADDR="${CFG_SERVER_ADDR}" \
       CFG_HOST_IP=$(hostname -i) \
       CFG_HOST_IDENTIFIER="${CFG_HOST_IDENTIFIER:-validator-$(hostname -i)}" \
       NOMOS_NODE_DUMP_CONFIG="/config.dump.yaml" \
       LOG_BACKEND="${LOG_BACKEND:-stdout}" \
       LOG_LEVEL="${LOG_LEVEL:-INFO}" \
       POL_PROOF_DEV_MODE=true

/usr/bin/cfgsync-client && \
    exec /usr/bin/nomos-node /config.yaml
