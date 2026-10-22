#!/usr/bin/env python3
"""Build the block-times report for one block-times-lab run.

Reads lab.json and events.jsonl (see collect.py) from the scenario directory,
measures what the paper's simulation assumed (block delay distribution, its
legs, fork and reorg frequencies), re-runs the paper's model with the
measured delays via simulate.py, and writes report/report.html with the
figures, laid out like "[1.0.0][Analysis] Block Times & Blend Network".

Usage: report.py SCENARIO_DIR [--sim-slots N] [--repeats R] [--seed S]
"""
import argparse
import base64
import html
import io
import json
import math
import re
import sys
from collections import defaultdict
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import simulate  # noqa: E402

EXPECTED_METRICS = [
    "consensus_block_delay_seconds",
    "consensus_proposal_publish_delay_seconds",
    "consensus_leader_proof_seconds",
    "consensus_reorgs_total",
    "consensus_reorg_depth",
    "consensus_fork_blocks_total",
    "consensus_forks_count",
    "blend_message_hold_seconds",
    "blend_proposals_broadcast_total",
]
EPSILON = 1e-6


def percentile(values, q):
    values = np.asarray(values, dtype=np.float64)
    return float(np.percentile(values, q)) if values.size else float("nan")


def fmt(x, digits=2):
    if x is None or (isinstance(x, float) and not math.isfinite(x)):
        return "n/a"
    return f"{x:.{digits}f}"


def lab_from_deployment(path: Path) -> dict:
    """The parameters report.py needs, read from a node deployment settings
    file (the one the node embeds, or the one passed with --deployment)."""
    import yaml  # only needed for deployments; the lab writes lab.json

    class Loader(yaml.SafeLoader):
        pass

    # The genesis block carries enum tags such as `!ZkSig`; they are not needed
    # here, so every tagged node is loaded as its plain value.
    Loader.add_multi_constructor("!", lambda loader, suffix, node: loader.construct_mapping(node) if isinstance(node, yaml.MappingNode) else loader.construct_sequence(node) if isinstance(node, yaml.SequenceNode) else loader.construct_scalar(node))
    settings = yaml.load(path.read_text(), Loader=Loader)
    eras = settings["eras"]
    era = eras[min(eras, key=int)] if isinstance(eras, dict) else eras
    f_num = int(era["cryptarchia"]["slot_activation_coeff"]["numerator"])
    f_den = int(era["cryptarchia"]["slot_activation_coeff"]["denominator"])
    return {
        "source": str(path),
        "slot_secs": float(era["time"]["slot_duration"]),
        "f": f"{f_num}/{f_den}",
        "f_value": f_num / f_den,
        "blend_layers": int(era["blend"]["common"]["num_blend_layers"]),
        "release_delay_rounds": int(era["blend"]["core"]["scheduler"]["delayer"]["maximum_release_delay_in_rounds"]),
        "security_param": int(era["cryptarchia"]["security_param"]),
    }


class Run:
    def __init__(self, scenario: Path, deployment: Path = None):
        self.scenario = scenario
        lab_path = scenario / "lab.json"
        if deployment is not None:
            self.lab = lab_from_deployment(deployment)
        elif lab_path.exists():
            self.lab = json.loads(lab_path.read_text())
        else:
            raise SystemExit(f"{lab_path} not found; pass --deployment <settings.yaml> for a deployment run")
        self.events = [json.loads(line) for line in (scenario / "events.jsonl").read_text().splitlines() if line.strip()]
        self.nodes = sorted({e["node"] for e in self.events}, key=lambda n: int(n.split("-")[1]) if n.split("-")[-1].isdigit() else 0)
        self.slot_secs = float(self.lab.get("slot_secs", 1))
        self.f = float(self.lab.get("f_value", 0.05))
        self.layers = int(self.lab.get("blend_layers", 1))
        self.release_rounds = int(self.lab.get("release_delay_rounds", 1))
        self.lab.setdefault("nodes", len(self.nodes))
        time_files = sorted(scenario.glob("time-info-node-*.json"))
        if time_files and "time" not in self.lab:
            self.lab["time"] = json.loads(time_files[0].read_text())
        self._index()

    def _index(self):
        by = defaultdict(list)
        for e in self.events:
            by[e["event"]].append(e)
        self.by = by
        # Leader-side view of each block.
        self.produced = {}
        for e in by["proposal_published"]:
            self.produced[e["block_id"]] = e
        # Per-node processing of each block.
        self.processed = defaultdict(dict)  # node -> block_id -> event
        for e in by["block_processed"]:
            if e.get("origin") == "Network":
                self.processed[e["node"]][e["block_id"]] = e
        self.received = defaultdict(dict)
        for e in by["block_received"]:
            self.received[e["node"]][e["block_id"]] = e
        self.blend_exit = defaultdict(list)  # block_id -> [(node, ts)]
        for e in by["blend_proposal_broadcast"]:
            self.blend_exit[e["block_id"]].append((e["node"], e["ts_ms"]))
        # Union block tree.
        self.tree = {}
        for node in self.nodes:
            for bid, e in self.processed[node].items():
                self.tree.setdefault(bid, {"slot": e["slot"], "parent": e["parent"], "height": e.get("height", 0)})

    # ---- measurements -------------------------------------------------
    def delays(self):
        return np.array([e["delay_ms"] / 1000.0 for node in self.nodes for e in self.received[node].values()])

    def publish_delays(self):
        return np.array([e["since_slot_start_ms"] / 1000.0 for e in self.by["proposal_published"]])

    def proof_times(self):
        return np.array([e["proof_ms"] / 1000.0 for e in self.by["proposal_published"]])

    def blend_legs(self):
        # Replication sends several copies of a proposal through Blend; the
        # first copy to come out is the one that matters for block delay.
        legs = []
        for bid, exits in self.blend_exit.items():
            pub = self.produced.get(bid)
            if not pub:
                continue
            legs.append((min(ts for _, ts in exits) - pub["ts_ms"]) / 1000.0)
        return np.array(legs)

    def gossip_legs(self):
        legs = []
        for node in self.nodes:
            for bid, e in self.received[node].items():
                exits = self.blend_exit.get(bid)
                if not exits:
                    continue
                first_exit = min(ts for _, ts in exits)
                legs.append((e["ts_ms"] - first_exit) / 1000.0)
        return np.array(legs)

    def hold_times(self):
        return np.array([e["hold_ms"] / 1000.0 for e in self.by["blend_hold_released"]])

    def reorgs(self, node):
        depths = [e["reorg_depth"] for e in self.processed[node].values() if e.get("reorg_depth", 0) > 0]
        return np.array(depths, dtype=np.int64), len(self.processed[node])

    def non_canonical_fraction(self, node):
        evs = list(self.processed[node].values())
        if not evs:
            return float("nan")
        return sum(1 for e in evs if not e.get("canonical", True)) / len(evs)

    def fork_points(self):
        children = defaultdict(set)
        for bid, b in self.tree.items():
            children[b["parent"]].add(bid)
        return sum(1 for kids in children.values() if len(kids) > 1)

    def slot_span(self):
        slots = [b["slot"] for b in self.tree.values()]
        if not slots:
            return 0, 0
        return min(slots), max(slots)

    def metric_totals(self, name, label=None):
        """Sum of a counter over the scraped nodes (0 when not scraped)."""
        total = 0
        pattern = re.compile(rf"^{name}(?:_total)?\{{([^}}]*)\}}\s+([0-9.e+]+)", re.M)
        for path in sorted(self.scenario.glob("metrics-*.txt")):
            for labels, value in pattern.findall(path.read_text()):
                if label is None or label in labels:
                    total += float(value)
        return int(total)

    def metrics_check(self):
        found = {}
        for path in sorted(self.scenario.glob("metrics-*.txt")):
            text = path.read_text()
            for name in EXPECTED_METRICS:
                if re.search(rf"^{name}(_bucket|_count|_sum|_total)?(\{{|\s)", text, re.M):
                    found.setdefault(name, set()).add(path.stem.replace("metrics-", ""))
        return found, bool(list(self.scenario.glob("metrics-*.txt")))


# ---- figures ---------------------------------------------------------------
def fig_to_b64(fig, out_dir=None, name=None):
    buf = io.BytesIO()
    fig.savefig(buf, format="png", dpi=110, bbox_inches="tight")
    if out_dir is not None and name is not None:
        out_dir.mkdir(parents=True, exist_ok=True)
        fig.savefig(out_dir / f"{name}.png", format="png", dpi=110, bbox_inches="tight")
    buf.seek(0)
    return base64.b64encode(buf.read()).decode("ascii")


def build_report(run: Run, sim_slots: int, repeats: int, seed: int, out_dir: Path):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    rng = np.random.default_rng(seed)
    n = len(run.nodes)
    figures = {}
    notes = []

    # 1. Block delay distribution -------------------------------------------
    delays = run.delays()
    proof = run.proof_times()
    proof_secs = float(np.median(proof)) if proof.size else 0.5
    model = simulate.parametric_delay_sampler(rng, proof_secs, run.layers, run.release_rounds * run.slot_secs)
    model_samples = model(200_000)
    p = {q: percentile(delays, q) for q in (50, 99, 99.9)}
    pm = {q: percentile(model_samples, q) for q in (50, 99, 99.9)}
    mean_excess = float(delays.mean() - model_samples.mean()) if delays.size else float("nan")
    round_secs = run.slot_secs  # at 0.3.2 a Blend round is one slot
    if delays.size:
        frac = np.mod(delays, round_secs) / round_secs
        # How tightly delays sit on a round grid: circular concentration of the
        # fractional part (1 = perfectly quantized, 0 = uniform).
        quantization = float(np.abs(np.exp(2j * np.pi * frac).mean()))
    else:
        quantization = float("nan")

    fig, ax = plt.subplots(figsize=(7.5, 4.2))
    if delays.size:
        bins = np.linspace(0, max(delays.max(), model_samples.max()) * 1.05, 60)
        ax.hist(delays, bins=bins, density=True, alpha=0.75, color="tab:orange", label=f"measured (n={delays.size})")
        ax.hist(model_samples, bins=bins, density=True, histtype="step", color="tab:blue", label="paper-style model, lab params")
        for q, c in ((50, "darkred"), (99, "red"), (99.9, "salmon")):
            ax.axvline(p[q], color=c, lw=1, label=f"measured {q}p={p[q]:.1f}s")
            ax.axvline(pm[q], color="navy", lw=1, ls=":", label=f"model {q}p={pm[q]:.1f}s")
    ax.set_xlabel("block delay (s): slot start -> proposal received at a node")
    ax.set_ylabel("density")
    ax.legend(fontsize=7)
    figures["delay"] = fig_to_b64(fig, out_dir, "delay")
    plt.close(fig)

    # 2. Legs of the delay --------------------------------------------------
    legs = {
        "leader: proof": proof,
        "leader: slot start -> Blend": run.publish_delays(),
        "Blend: hop hold time": run.hold_times(),
        "Blend: leader -> gossip": run.blend_legs(),
        "gossip: Blend exit -> received": run.gossip_legs(),
        "total: slot start -> received": delays,
    }
    fig, ax = plt.subplots(figsize=(7.5, 3.8))
    data = [v if v.size else np.array([np.nan]) for v in legs.values()]
    try:
        ax.boxplot(data, orientation="horizontal", showfliers=True)
    except TypeError:  # matplotlib < 3.10
        ax.boxplot(data, vert=False, showfliers=True)
    ax.set_yticks(range(1, len(legs) + 1), [f"{k} (n={v.size})" for k, v in legs.items()], fontsize=8)
    ax.set_xlabel("seconds")
    ax.grid(axis="x", alpha=0.3)
    figures["legs"] = fig_to_b64(fig, out_dir, "legs")
    plt.close(fig)

    # 3. Honest reorg depth sensitivity: measured vs simulated ---------------
    first_slot, last_slot = run.slot_span()
    run_slots = max(last_slot - first_slot, 1)
    empirical = simulate.empirical_delay_sampler(rng, delays) if delays.size >= 20 else model
    if delays.size < 20:
        notes.append(f"Only {delays.size} measured delays; simulations use the parametric model instead of the empirical distribution.")

    sim_long = simulate.simulate_honest(n, run.f, sim_slots, empirical, rng, run.slot_secs)
    sim_model = simulate.simulate_honest(n, run.f, sim_slots, model, rng, run.slot_secs)
    sim_nodelay = simulate.simulate_honest(n, run.f, sim_slots, lambda size: np.full(size, 0.01), rng, run.slot_secs)

    # Sampling spread at the run's own length.
    reps = []
    for _ in range(repeats):
        r = simulate.simulate_honest(n, run.f, run_slots, empirical, rng, run.slot_secs)
        d = np.concatenate([np.asarray(x) for x in r["reorg_depths"]]) if any(len(x) for x in r["reorg_depths"]) else np.array([])
        n_blocks = sum(r["processed"])
        n_total = len(r["blocks"]) - 1
        reps.append({
            "reorg1": (d >= 1).sum() / max(n_blocks, 1),
            "reorg2": (d >= 2).sum() / max(n_blocks, 1),
            "fork": 1 - (len(r["canonical_ids"]) - 1) / max(n_total, 1),
            "blocks_per_slot": n_total / run_slots,
        })

    def band(key):
        vals = np.array([r[key] for r in reps])
        return float(np.percentile(vals, 5)), float(np.median(vals)), float(np.percentile(vals, 95))

    fig, ax = plt.subplots(figsize=(7.5, 4.2))
    max_d = 1
    measured_curves = {}
    for node in run.nodes:
        depths, n_blocks = run.reorgs(node)
        ds, freq = simulate.survival(depths, n_blocks, max_depth=max(int(depths.max()) if depths.size else 1, 1))
        measured_curves[node] = (ds, freq, n_blocks)
        max_d = max(max_d, int(ds[-1]))
        ax.plot(ds, np.where(freq > 0, freq, np.nan), "o-", label=f"measured {node} (blocks={n_blocks}, reorgs={depths.size})")
    for label, res, style in (("simulated, measured delays", sim_long, "-"), ("simulated, paper-style model", sim_model, "--"), ("simulated, no delay", sim_nodelay, ":")):
        d = np.concatenate([np.asarray(x) for x in res["reorg_depths"]]) if any(len(x) for x in res["reorg_depths"]) else np.array([])
        ds, freq = simulate.survival(d, sum(res["processed"]), max_depth=max(int(d.max()) if d.size else 1, 1))
        max_d = max(max_d, int(ds[-1]))
        ax.plot(ds, np.where(freq > 0, freq, np.nan), style, color="black", alpha=0.8, label=f"{label} ({sim_slots} slots)")
    ax.set_yscale("log")
    ax.set_xlabel("reorg depth")
    ax.set_ylabel("frequency (per block), depth >= d")
    ax.set_title("reorg depth sensitivity, honest nodes only")
    ax.set_xticks(range(1, max_d + 1))
    ax.grid(alpha=0.3, which="both")
    ax.legend(fontsize=7)
    figures["reorg"] = fig_to_b64(fig, out_dir, "reorg")
    plt.close(fig)

    # 4. Adversary (simulation only) ----------------------------------------
    fig, axes = plt.subplots(1, 2, figsize=(11, 4))
    adversary_summary = []
    for share, color in ((0.10, "tab:blue"), (0.30, "tab:red")):
        res_blend = simulate.simulate_honest(n, run.f, sim_slots, empirical, rng, run.slot_secs, honest_share=1 - share)
        res_base = simulate.simulate_honest(n, run.f, sim_slots, lambda size: np.full(size, 0.01), rng, run.slot_secs, honest_share=1 - share)
        d_blend = simulate.adversary_reorg_depths(res_blend, run.f, share, sim_slots, rng)
        d_base = simulate.adversary_reorg_depths(res_base, run.f, share, sim_slots, rng)
        md = max(int(d_blend.max()) if d_blend.size else 1, int(d_base.max()) if d_base.size else 1, 1)
        ds, fb = simulate.survival(d_blend, len(d_blend), md)
        _, f0 = simulate.survival(d_base, len(d_base), md)
        axes[0].plot(ds, np.where(fb > 0, fb, np.nan), color=color, label=f"{int(share*100)}% adversary, measured delays")
        axes[0].plot(ds, np.where(f0 > 0, f0, np.nan), color=color, ls="--", label=f"{int(share*100)}% adversary, no delay baseline")
        with np.errstate(divide="ignore", invalid="ignore"):
            ratio = np.where(f0 > 0, fb / f0, np.nan)
        axes[1].plot(ds, ratio, color=color, label=f"{int(share*100)}% adversary")
        adversary_summary.append((share, float(fb[0]) if fb.size else 0.0, float(f0[0]) if f0.size else 0.0))
    axes[0].set_yscale("log")
    axes[0].set_xlabel("reorg depth")
    axes[0].set_ylabel("frequency (per honest block), depth >= d")
    axes[0].set_title("private-chain adversary (simulation)")
    axes[0].legend(fontsize=7)
    axes[0].grid(alpha=0.3, which="both")
    axes[1].set_xlabel("reorg depth")
    axes[1].set_ylabel("reorgs with measured delays / no-delay baseline")
    axes[1].set_title("cost of the measured network delay")
    axes[1].grid(alpha=0.3)
    axes[1].legend(fontsize=7)
    figures["adversary"] = fig_to_b64(fig, out_dir, "adversary")
    plt.close(fig)

    # 5. Praos condition ----------------------------------------------------
    delta_slots = int(math.ceil(p[99.9] / run.slot_secs)) if delays.size else 0
    fig, ax = plt.subplots(figsize=(7.5, 3.8))
    deltas = np.arange(1, max(delta_slots + 5, 15))
    for f_value, label in ((run.f, f"lab f={run.f:.3g}"), (1 / 30, "Nomos f=1/30"), (1 / 20, "Cardano f=1/20")):
        alpha = (1 + EPSILON) / (2 * (1 - f_value) ** (deltas + 1))
        ax.plot(deltas, np.minimum(alpha, 1.0), label=label)
    if delays.size:
        ax.axvline(delta_slots, color="gray", ls=":", label=f"measured 99.9p delay = {delta_slots} slot(s)")
    ax.set_xlabel("max network delay Δ (slots)")
    ax.set_ylabel("minimum honest stake α")
    ax.set_ylim(0.5, 1.0)
    ax.grid(alpha=0.3)
    ax.legend(fontsize=7)
    figures["praos"] = fig_to_b64(fig, out_dir, "praos")
    plt.close(fig)
    alpha_lab = (1 + EPSILON) / (2 * (1 - run.f) ** (delta_slots + 1)) if delays.size else float("nan")
    delta_nomos = int(math.ceil(p[99.9])) if delays.size else 0  # 1 s slots
    alpha_nomos = (1 + EPSILON) / (2 * (1 - 1 / 30) ** (delta_nomos + 1)) if delays.size else float("nan")

    # ---- checks ------------------------------------------------------------
    bypassed = run.metric_totals("blend_payloads_bypassed_total", 'payload_type="block_proposal"')
    created = run.metric_totals("consensus_proposals_created_total")
    bypass_share = bypassed / created if created else float("nan")
    n_blocks_total = len(run.tree)
    blocks_per_slot = n_blocks_total / run_slots
    measured_reorg1 = np.array([c[1][0] if c[1].size else 0.0 for c in measured_curves.values()])
    measured_fork = np.array([run.non_canonical_fraction(node) for node in run.nodes])
    b_r1, b_r2, b_fork, b_bps = band("reorg1"), band("reorg2"), band("fork"), band("blocks_per_slot")

    def verdict(value, lo, hi):
        if not math.isfinite(value):
            return "no data"
        return "OK" if lo <= value <= hi else "CHECK"

    checks = [
        ("blocks per slot", f"{n * simulate.per_node_probability(run.f, 1 / n):.4f} (= n·φ(1/n), f={run.f:.3g})", fmt(blocks_per_slot, 4),
         f"sim 5-95%: {b_bps[0]:.4f} - {b_bps[2]:.4f}", verdict(blocks_per_slot, b_bps[0] * 0.9, b_bps[2] * 1.1)),
        ("proposals bypassing Blend", "0 (not modelled)", f"{bypassed} of {created}", "none expected", "OK" if bypassed == 0 else "CHECK"),
        ("median block delay (s)", fmt(pm[50]), fmt(p[50]), "model vs measured, same lab params", verdict(p[50], pm[50] * 0.5, pm[50] * 1.5)),
        ("99p block delay (s)", fmt(pm[99]), fmt(p[99]), "model vs measured", verdict(p[99], pm[99] * 0.5, pm[99] * 1.5)),
        ("non-canonical block fraction", f"{b_fork[1]:.4f}", " / ".join(fmt(v, 4) for v in measured_fork),
         f"sim 5-95% at run length: {b_fork[0]:.4f} - {b_fork[2]:.4f}",
         verdict(float(np.nanmean(measured_fork)) if measured_fork.size else float('nan'), b_fork[0], b_fork[2])),
        ("reorg frequency, depth >= 1", f"{b_r1[1]:.4f}", " / ".join(fmt(v, 4) for v in measured_reorg1),
         f"sim 5-95% at run length: {b_r1[0]:.4f} - {b_r1[2]:.4f}",
         verdict(float(measured_reorg1.mean()) if measured_reorg1.size else float('nan'), b_r1[0], b_r1[2])),
    ]
    found, had_metrics = run.metrics_check()
    metric_rows = [(name, ", ".join(sorted(found.get(name, []))) or "missing") for name in EXPECTED_METRICS]

    # ---- html --------------------------------------------------------------
    def table(headers, rows):
        head = "".join(f"<th>{html.escape(str(h))}</th>" for h in headers)
        body = "".join("<tr>" + "".join(f"<td>{html.escape(str(c))}</td>" for c in r) + "</tr>" for r in rows)
        return f"<table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table>"

    def img(key, caption):
        return f'<figure><img src="data:image/png;base64,{figures[key]}" alt="{html.escape(caption)}"><figcaption>{html.escape(caption)}</figcaption></figure>'

    params = [
        ("nodes (equal stake, all Blend core)", n),
        ("slot duration (s)", run.slot_secs),
        ("active slot coefficient f", run.lab.get("f")),
        ("expected block time (s)", fmt(run.slot_secs / run.f, 1)),
        ("Blend layers (hops)", run.layers),
        ("Blend max release delay (rounds; 1 round = 1 slot)", run.release_rounds),
        ("security parameter k", run.lab.get("security_param")),
        ("run length", f"{fmt(run_slots * run.slot_secs / 60, 1)} min of blocks seen, slots {first_slot}-{last_slot}"),
        ("parameters from", run.lab.get("source", "lab.json")),
        ("blocks seen (union of nodes)", n_blocks_total),
        ("fork points in the block tree", run.fork_points()),
        ("simulation length", f"{sim_slots} slots, {repeats} repeats at run length for the bands"),
    ]
    doc = f"""<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Block Times Lab</title>
<style>:root{{--bg:#fff;--fg:#222;--muted:#555;--line:#ccc;--code:#f4f4f4}}
@media (prefers-color-scheme: dark){{:root:not([data-theme="light"]){{--bg:#161616;--fg:#e8e8e8;--muted:#aaa;--line:#444;--code:#262626}}}}
:root[data-theme="dark"]{{--bg:#161616;--fg:#e8e8e8;--muted:#aaa;--line:#444;--code:#262626}}
body{{font-family:system-ui,sans-serif;max-width:960px;margin:2em auto;padding:0 16px;color:var(--fg);background:var(--bg)}}
table{{border-collapse:collapse;margin:1em 0;font-size:14px;display:block;overflow-x:auto}}th,td{{border:1px solid var(--line);padding:4px 8px;text-align:left}}
figure{{margin:1em 0}}img{{max-width:100%;background:#fff}}figcaption{{font-size:13px;color:var(--muted)}}code{{background:var(--code);padding:1px 4px}}
.ok{{color:#07703a;font-weight:600}}.check{{color:#b45309;font-weight:600}}</style></head><body>
<h1>[lab] Block Times &amp; Blend Network: measured on a local {n}-node cluster</h1>
<p>Companion to <em>[1.0.0][Analysis] Block Times &amp; Blend Network</em>. The paper chose block time and Blend
parameters from a simulation whose inputs were a modelled network delay and a modelled honest block tree.
This report measures those inputs on real nodes built from the <code>block-times-lab</code> branch (tag 0.3.2 plus
instrumentation), then feeds the measured delays back into the paper's model to check that the implementation behaves
as the research assumed. Scenario: <code>{html.escape(str(run.scenario))}</code>.</p>

<h2>Parameters</h2>{table(["parameter", "value"], params)}

<h2>Network model: block delay distribution</h2>
<p>The paper's model: PoL proof time, then for each Blend hop a blending delay uniform up to the configured maximum,
then gossip. Measured here: <code>block_received</code> on every node gives the time from slot start to arrival of the
proposal via gossip (leader proof + Blend + gossip). The model curve uses the lab parameters and the measured median
proof time ({fmt(proof_secs)} s).</p>
{img("delay", f"Block delay: measured 50p={fmt(p[50])}s 99p={fmt(p[99])}s 99.9p={fmt(p[99.9])}s; model 50p={fmt(pm[50])}s 99p={fmt(pm[99])}s 99.9p={fmt(pm[99.9])}s")}
<p>Mean measured delay minus the paper-style model: <b>{fmt(mean_excess)} s</b> (= {fmt(mean_excess / round_secs)} rounds).
Round-grid concentration of the measured delays: <b>{fmt(quantization)}</b> (1 = every delay sits a fixed offset after a
round boundary, 0 = spread evenly). The implementation releases Blend messages only at release rounds, whose spacing is
drawn from 1..max rounds, so each release costs between one and max whole rounds; the paper's model drew the blending
delay uniformly from 0..max seconds, which is about half a round less per release.</p>
<p>Blend delivery fallback: <b>{bypassed}</b> of {created} proposals ({fmt(100 * bypass_share, 1)}%) missed Blend's delivery
deadline and were broadcast in the clear by the failure detector (<code>blend_payloads_bypassed_total</code>). The paper's model
has no such path; when this share is large, the measured delay is the fallback's, not the mixnet's.</p>
<p>Where the time goes, per leg (each sample is one block at one node):</p>
{img("legs", "Legs of the block delay. Leader: proof generation and slot-start-to-Blend; Blend: per-hop hold time (blend_hold_released) and leader-to-gossip (proposal_published -> blend_proposal_broadcast); gossip: Blend exit to block_received.")}

<h2>Choosing a block time: reorg depth sensitivity</h2>
<p>The paper's first step simulates the honest block tree and counts reorgs. Here the measured curve comes from
<code>block_processed</code> (reorg depth = canonical blocks displaced when the tip switched) on each node, and the
simulated curves rerun the paper's honest-network model for the same f and node count, once with delays resampled from
the measurement and once with the parametric model. A real adversary cannot be measured on a testnet; its curves are
simulation only (next section).</p>
{img("reorg", "Honest-only reorg depth sensitivity: measured per node vs the paper's model driven by the measured delays.")}

<h2>Powerful adversaries (simulation driven by measured delays)</h2>
<p>The paper's private-chain adversary, replayed with the measured delay distribution. The dashed baseline uses the same
adversary with (almost) no network delay, so the right-hand ratio is the price of the measured delay, analogous to the
paper's Nomos/Cardano ratio plots.</p>
{img("adversary", "Adversary reorg depth sensitivity and the ratio to the no-delay baseline.")}
{table(["adversary stake", "P(depth >= 1), measured delays", "P(depth >= 1), no delay", "ratio"],
       [(f"{int(s*100)}%", fmt(a, 4), fmt(b, 4), fmt(a / b if b else float('nan'), 2)) for s, a, b in adversary_summary])}

<h2>Annex: Praos common-prefix condition</h2>
<p>α(1 − f)^(Δ+1) ≥ (1+ε)/2 with Δ the maximum delay in slots. Using the measured 99.9p block delay as Δ:
lab f needs α ≥ {fmt(alpha_lab, 3)} (Δ = {delta_slots} slot(s) of {run.slot_secs}s); at Nomos' f = 1/30 with 1 s slots the same
delay ({fmt(p[99.9])} s → Δ = {delta_nomos}) needs α ≥ {fmt(alpha_nomos, 3)}. The paper's figures were 0.83 at Δ = 14 s and 0.75 at Δ = 11 s.</p>
{img("praos", "Minimum honest stake against Δ; the dotted line marks the measured 99.9p delay.")}

<h2>Does the implementation match the research?</h2>
{table(["quantity", "expected (model / theory)", "measured", "tolerance", "verdict"], checks)}
<p>Verdicts: <span class="ok">OK</span> = measured value falls inside the band the model produces at this run length
(or within ±50% for the delay percentiles); <span class="check">CHECK</span> = outside, look at the figure.</p>

<h2>Metrics coverage</h2>
<p>Scraped from each node's <code>/metrics</code> at the end of the run{"" if had_metrics else " (no metrics files found in the scenario directory)"}.</p>
{table(["metric", "present on"], metric_rows)}

<h2>Notes</h2><ul>{"".join(f"<li>{html.escape(x)}</li>" for x in notes) or "<li>none</li>"}
<li>Only honest behaviour is measurable; the adversary section is the paper's model fed with measured delays.</li>
<li>The simulator does not let the adversary reuse abandoned honest branches (the paper's "boost"), so its curves are slightly optimistic.</li>
<li>Reorg frequency is per processed block per node; with few nodes most reorgs are depth-1 slot battles.</li>
<li>Blend's data replication sends several copies of each proposal; the Blend leg uses the first copy to leave Blend, and <code>blend_proposal_broadcast</code> counts every copy.</li>
</ul>
</body></html>"""
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "report.html").write_text(doc)
    summary = {
        "delay_p50": p[50], "delay_p99": p[99], "delay_p999": p[99.9],
        "model_p50": pm[50], "model_p99": pm[99], "model_p999": pm[99.9],
        "mean_excess_s": mean_excess, "round_quantization": quantization,
        "blend_bypassed": bypassed, "proposals_created": created,
        "blocks_per_slot": blocks_per_slot, "f": run.f,
        "reorg_freq_depth1_measured": measured_reorg1.tolist(),
        "reorg_freq_depth1_sim_band": b_r1,
        "fork_fraction_measured": measured_fork.tolist(),
        "fork_fraction_sim_band": b_fork,
        "alpha_min_lab": alpha_lab, "alpha_min_nomos_f": alpha_nomos,
        "checks": checks, "metrics": metric_rows,
    }
    (out_dir / "summary.json").write_text(json.dumps(summary, indent=2, default=float))
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("scenario_dir", type=Path)
    parser.add_argument("--sim-slots", type=int, default=60000)
    parser.add_argument("--repeats", type=int, default=40)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--deployment", type=Path, default=None,
                        help="node deployment settings.yaml to read the chain parameters from (deployment runs have no lab.json)")
    args = parser.parse_args()
    run = Run(args.scenario_dir, args.deployment)
    summary = build_report(run, args.sim_slots, args.repeats, args.seed, args.scenario_dir / "report")
    for row in summary["checks"]:
        print(" | ".join(str(c) for c in row))
    for name, where in summary["metrics"]:
        print(f"metric {name}: {where}")
    print(f"report: {args.scenario_dir / 'report' / 'report.html'}")


if __name__ == "__main__":
    main()
