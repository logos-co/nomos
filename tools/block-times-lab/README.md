# block-times-lab

Measures on real nodes what `[1.0.0][Analysis] Block Times & Blend Network`
assumed in simulation, and feeds the measurements back into the paper's model.

What the branch adds to the node (tag 0.3.2 + instrumentation):

| where | event (INFO log) | metric |
| --- | --- | --- |
| chain-leader | `proposal_published` block_id slot parent since_slot_start_ms proof_ms | `consensus_proposal_publish_delay_seconds`, `consensus_leader_proof_seconds` |
| blend core | `blend_hold_released` hold_ms (block proposals; others at DEBUG) | `blend_message_hold_seconds{kind}` |
| blend dispatcher | `blend_proposal_broadcast` block_id slot | `blend_proposals_broadcast_total` |
| chain-network | `block_received` block_id slot parent delay_ms | `consensus_block_delay_seconds` |
| chain-service | `block_processed` block_id slot parent height tip tip_slot canonical reorg_depth newly_canonical origin | `consensus_reorgs_total`, `consensus_reorg_depth`, `consensus_fork_blocks_total` |
| node API | `GET /metrics` serves the OpenMetrics document when `tracing.metrics.enable_open_metrics` is on, or `OPEN_METRICS=true` is in the environment (`--open-metrics`) | |

`delay_ms` is wall-clock time since the start of the block's slot, so it is the
end-to-end block delay (proof + Blend + gossip) the paper's network model
produces, as seen by one node.

## Running

```
pip install numpy matplotlib   # the analysis scripts need only these two
tools/block-times-lab/run.sh --nodes 2 --minutes 20 --slot-secs 1 --f 1/4 \
    --blend-layers 1 --release-delay-rounds 3 --security-param 10
```

`run.sh` builds the node and the `block-times-lab` harness (a bin in the
`logos-blockchain-tests` crate that uses the repo's local-cluster test
framework to generate a genesis for N equal-stake nodes and start them), runs
the cluster for `--minutes`, scrapes `/metrics` from every node, stops them,
and then:

- `collect.py SCENARIO_DIR` turns the node log files into `events.jsonl`;
- `report.py SCENARIO_DIR` writes `report/report.html` and `report/summary.json`.

Runs land under `tools/block-times-lab/runs/<scenario>/` (override with `OUT`).

## What the report compares

- Block delay distribution (50p/99p/99.9p) measured vs the paper-style model
  built from the lab parameters; per-leg breakdown (proof, Blend hold, gossip).
- Honest reorg depth sensitivity measured on each node vs the paper's honest
  simulation driven by the measured delays (`simulate.py`), with a 5-95% band
  from repeated simulations of the same run length.
- Block production rate vs f, non-canonical block fraction vs simulation.
- The paper's private-chain adversary (10%, 30%) replayed with measured
  delays, and the ratio to a no-delay baseline (its Nomos/Cardano ratio).
- The Praos common-prefix condition with the measured 99.9p delay as Δ.
- Which of the new metrics appeared in each node's `/metrics` scrape.

Only honest behaviour is measurable on a network; the adversary curves are
simulation. The simulator does not reuse abandoned honest branches for the
adversary (the paper's "boost"), so it is slightly optimistic there.

## Deploying on a real network

The `testnet-blend` branch's `deployment/compose.blend.setup.yml` and
`compose.blend.run.yml` start N controlled nodes that join the network at
`BLEND_INITIAL_PEERS`. They work unchanged with an image built from this
branch; the instrumentation lives in the service crates the node module is
built from, and the embedded deployment settings are 0.3.2's, so the nodes
join the 0.3.2 network.

What the controlled nodes record:

- `proposal_published` on a node only for blocks it proposed itself;
- `blend_hold_released` / `blend_proposal_broadcast` only on the core node
  that unwraps the last layer, so with a large Blend network these sample a
  share of the blocks;
- `block_received` and `block_processed` on every controlled node for every
  block, which is the complete end-to-end block delay and the block tree.

Steps:

1. Build the node image from this branch and point `NODE_IMAGE_LABEL` at it.
   The stock `TOOLS_IMAGE_LABEL=0.3.2` is fine: `init-config` there writes a
   config this node reads, and nothing in the config needs to change.
2. Add `OPEN_METRICS: "true"` to the `x-env` block of `compose.blend.run.yml`
   (or export it into the node containers some other way). It turns on the
   `/metrics` route; the API ports are published on the host's loopback
   only, so nothing else sees it. Without it only the log events exist, which
   is enough for the report but not for Prometheus.
3. Leave `LOG_BACKEND=file` as the compose sets it. The events are INFO
   lines under the node's own targets, so `LOG_LEVEL=INFO` is enough;
   `DEBUG` also works, just larger. The rolling appender keeps
   `LOG_APPENDER_MAX_FILES` hourly files, so collect at least that often or
   raise it.
4. After the measurement window, on the host running the compose:

   ```
   tools/block-times-lab/deploy-collect.sh OUT_DIR --count 10 --api-base 18080
   python3 tools/block-times-lab/collect.py OUT_DIR
   python3 tools/block-times-lab/report.py OUT_DIR \
       --deployment nodes/node/binary/src/config/deployment/settings.yaml
   ```

   `deploy-collect.sh` copies each node's `logos-blockchain.log.*` files out
   of the containers and fetches `/metrics`, `/time/info` and
   `/cryptarchia/info`. `report.py` reads f, slot duration, Blend layers and
   the release delay from the settings file the node embeds (pass the file
   the deployed nodes actually run with). PyYAML is needed for that path:
   `pip install numpy matplotlib pyyaml`.

Clocks: `delay_ms` is each node's own wall clock against the chain's genesis
time, so the controlled hosts need NTP; the node's time service already uses
it. Everything is in UTC.
