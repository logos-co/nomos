#!/usr/bin/env python3
"""Discrete-slot simulator of the model used in
"[1.0.0][Analysis] Block Times & Blend Network".

Honest network: `n` equal-stake leaders run a Praos lottery with active slot
coefficient `f` (per-node probability 1 - (1 - f)^(share)).  A winner extends
the longest chain it knows at that slot; its block reaches every other node
after an independent delay drawn from `delay_sampler` (seconds).  Each node
tracks its tip, and every tip switch that does not simply extend the previous
tip is a reorg whose depth is `old_tip_height - lca_height`.

Adversary (optional, as in the paper): a private-chain attacker holding
`adversary` of the stake.  For every honest canonical block B it forks from
B's parent and extends in secret with all of its later slot wins; the deepest
reorg it can force is the largest honest canonical growth above the fork
point at any moment where its private chain is strictly longer.  Abandoned
honest branches are not reused (the paper's "boost"), so this is slightly
conservative.

The module is imported by report.py and can also be run directly:

    simulate.py --f 0.25 --nodes 2 --slots 20000 --delay-mean 2.5
"""
import argparse
import heapq
import math

import numpy as np


class Block:
    __slots__ = ("id", "parent", "height", "slot", "producer")

    def __init__(self, id_, parent, height, slot, producer):
        self.id = id_
        self.parent = parent  # Block or None
        self.height = height
        self.slot = slot
        self.producer = producer


def lca_height(a: Block, b: Block) -> int:
    while a is not b:
        if a.height > b.height:
            a = a.parent
        elif b.height > a.height:
            b = b.parent
        else:
            a, b = a.parent, b.parent
    return a.height


def per_node_probability(f: float, share: float) -> float:
    return 1.0 - (1.0 - f) ** share


def simulate_honest(n_nodes, f, slots, delay_sampler, rng, slot_secs=1.0,
                    honest_share=1.0, adversary_wins=None):
    """Returns a dict with the block tree and per-node reorg observations.

    delay_sampler(size) -> array of delays in seconds.
    adversary_wins: optional bool array per slot (precomputed), only recorded.
    """
    p = per_node_probability(f, honest_share / n_nodes)
    genesis = Block(0, None, 0, -1, -1)
    blocks = [genesis]
    tips = [genesis] * n_nodes
    inbox = [[] for _ in range(n_nodes)]  # heaps of (arrival_time, seq, block)
    seq = 0
    reorg_depths = [[] for _ in range(n_nodes)]
    processed = [0] * n_nodes          # blocks applied per node (own + received)
    non_canonical = [0] * n_nodes      # applied blocks that did not become tip
    canonical_height_at_slot = np.zeros(slots + 1, dtype=np.int64)
    wins = rng.random((slots, n_nodes)) < p

    def apply(node, block):
        nonlocal tips
        processed[node] += 1
        old = tips[node]
        if block.height > old.height:
            if block.parent is not old:
                depth = old.height - lca_height(old, block)
                if depth > 0:
                    reorg_depths[node].append(depth)
            tips[node] = block
        else:
            non_canonical[node] += 1

    for t in range(slots):
        now = t * slot_secs
        for node in range(n_nodes):
            heap = inbox[node]
            while heap and heap[0][0] <= now:
                _, _, block = heapq.heappop(heap)
                apply(node, block)
        for node in range(n_nodes):
            if not wins[t, node]:
                continue
            parent = tips[node]
            block = Block(len(blocks), parent, parent.height + 1, t, node)
            blocks.append(block)
            apply(node, block)
            delays = delay_sampler(n_nodes)
            for other in range(n_nodes):
                if other == node:
                    continue
                seq += 1
                heapq.heappush(inbox[other], (now + float(delays[other]), seq, block))
        canonical_height_at_slot[t] = max(tip.height for tip in tips)
    canonical_height_at_slot[slots] = canonical_height_at_slot[slots - 1]

    # Final canonical chain: the longest tip; mark canonical blocks.
    best = max(tips, key=lambda b: (b.height, -b.slot))
    canonical = set()
    cursor = best
    while cursor is not None:
        canonical.add(cursor.id)
        cursor = cursor.parent

    return {
        "blocks": blocks,
        "canonical_ids": canonical,
        "reorg_depths": reorg_depths,
        "processed": processed,
        "non_canonical": non_canonical,
        "canonical_height_at_slot": canonical_height_at_slot,
        "tip": best,
        "p_per_node": p,
        "wins": wins,
    }


def adversary_reorg_depths(result, f, adversary_share, slots, rng, horizon=3000):
    """Max reorg depth the private-chain adversary can force per honest
    canonical block, following the paper's construction."""
    p_adv = per_node_probability(f, adversary_share)
    adv_wins = rng.random(slots) < p_adv
    cum = np.concatenate([[0], np.cumsum(adv_wins)])  # cum[t] = wins in slots < t
    height_at = result["canonical_height_at_slot"]
    depths = []
    for block in result["blocks"]:
        if block.id == 0 or block.id not in result["canonical_ids"]:
            continue
        s = block.slot
        end = min(slots, s + horizon)
        if end <= s + 1:
            continue
        ts = np.arange(s + 1, end + 1)
        adv_len = cum[ts] - cum[s + 1]
        honest_len = height_at[np.minimum(ts, slots)] - (block.height - 1)
        mask = adv_len > honest_len
        depths.append(int(honest_len[mask].max()) if mask.any() else 0)
    return np.array(depths, dtype=np.int64)


def survival(depths, n_blocks, max_depth=None):
    """frequency (per block) of reorgs of depth >= d, like the paper's plots."""
    depths = np.asarray(depths, dtype=np.int64)
    if max_depth is None:
        max_depth = int(depths.max()) if depths.size else 1
    ds = np.arange(1, max(max_depth, 1) + 1)
    counts = np.array([(depths >= d).sum() for d in ds], dtype=np.float64)
    return ds, counts / max(n_blocks, 1)


def parametric_delay_sampler(rng, proof_secs, hops, release_delay_secs,
                             absorption_secs=0.0, gossip_mean=0.05):
    """The paper's network model shape: PoL proof time, then per hop a
    blending delay uniform in (0, max] plus the network absorption, then a
    short exponential gossip delay."""
    def sample(size):
        total = np.full(size, proof_secs, dtype=np.float64)
        for _ in range(hops):
            total += rng.uniform(0.0, release_delay_secs, size) + absorption_secs
        total += rng.exponential(gossip_mean, size)
        return total
    return sample


def empirical_delay_sampler(rng, samples):
    samples = np.asarray(samples, dtype=np.float64)
    samples = samples[np.isfinite(samples)]
    samples = np.clip(samples, 0.0, None)

    def sample(size):
        return rng.choice(samples, size=size, replace=True)
    return sample


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--f", type=float, default=0.25)
    parser.add_argument("--nodes", type=int, default=2)
    parser.add_argument("--slots", type=int, default=20000)
    parser.add_argument("--delay-mean", type=float, default=2.5, help="mean of a parametric delay (uniform 0..2*mean)")
    parser.add_argument("--adversary", type=float, default=0.0)
    parser.add_argument("--seed", type=int, default=0)
    args = parser.parse_args()
    rng = np.random.default_rng(args.seed)
    sampler = parametric_delay_sampler(rng, 0.0, 1, 2 * args.delay_mean)
    res = simulate_honest(args.nodes, args.f, args.slots, sampler, rng, honest_share=1 - args.adversary)
    n_blocks = len(res["blocks"]) - 1
    print(f"blocks={n_blocks} canonical={len(res['canonical_ids']) - 1} "
          f"fork_fraction={1 - (len(res['canonical_ids']) - 1) / max(n_blocks, 1):.3f}")
    for node in range(args.nodes):
        ds, freq = survival(res["reorg_depths"][node], res["processed"][node])
        print(f"node {node}: reorgs={len(res['reorg_depths'][node])} "
              f"freq(d>=1)={freq[0] if freq.size else 0:.4f} max_depth={ds[-1] if ds.size else 0}")
    if args.adversary > 0:
        depths = adversary_reorg_depths(res, args.f, args.adversary, args.slots, rng)
        ds, freq = survival(depths, len(depths))
        print("adversary depth survival:", list(zip(ds[:10].tolist(), np.round(freq[:10], 4).tolist())))


if __name__ == "__main__":
    main()
