#!/bin/sh

set -e

ROLE_ATTR="nomos_role=executor"
if [ -n "${OTEL_RESOURCE_ATTRIBUTES:-}" ]; then
  export OTEL_RESOURCE_ATTRIBUTES="${ROLE_ATTR},${OTEL_RESOURCE_ATTRIBUTES}"
else
  export OTEL_RESOURCE_ATTRIBUTES="${ROLE_ATTR}"
fi

export CFG_FILE_PATH="/config.yaml" \
       CFG_SERVER_ADDR="${CFG_SERVER_ADDR:-http://cfgsync:4400}" \
       CFG_HOST_IP=$(hostname -i) \
       CFG_HOST_KIND="executor" \
       CFG_HOST_IDENTIFIER="executor-$(hostname -i)" \
       LOG_LEVEL="INFO" \
       POL_PROOF_DEV_MODE=true

/usr/bin/cfgsync-client && \
    exec /usr/bin/nomos-executor /config.yaml
