#!/usr/bin/env python3
"""Turn the node log files of a block-times-lab run into one events.jsonl.

Reads every `node-*/__logs-*` file under the scenario directory, keeps the
instrumentation events added for this analysis, and writes one JSON object
per line with the node name, the log timestamp (ms since the Unix epoch) and
every `key=value` field of the line.

Events kept:
  proposal_published        leader: block handed to Blend (chain-leader)
  blend_hold_released       a Blend hop released a fully unwrapped proposal
  blend_proposal_broadcast  proposal left Blend for gossip (blend dispatcher)
  block_received            proposal arrived from gossip (chain-network)
  block_processed           block applied: tip, canonical, reorg depth (chain-service)

Usage: collect.py SCENARIO_DIR [-o events.jsonl]
"""
import argparse
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

EVENTS = {
    "proposal_published",
    "blend_hold_released",
    "blend_proposal_broadcast",
    "block_received",
    "block_processed",
}
ANSI = re.compile(r"\x1b\[[0-9;]*m")
LINE = re.compile(r"^(?P<ts>\S+)\s+(?P<level>[A-Z]+)\s+(?P<target>\S+?):\s+(?P<rest>.*)$")
FIELD = re.compile(r"(\w+)=(HeaderId\([0-9a-f]+\)|\S+)")
HEADER_ID = re.compile(r"^HeaderId\(([0-9a-f]+)\)$")


def parse_ts(text: str) -> int:
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    return int(datetime.fromisoformat(text).astimezone(timezone.utc).timestamp() * 1000)


def parse_value(raw: str):
    m = HEADER_ID.match(raw)
    if m:
        return m.group(1)
    if raw in ("true", "false"):
        return raw == "true"
    try:
        return int(raw)
    except ValueError:
        pass
    try:
        return float(raw)
    except ValueError:
        return raw


def parse_line(line: str):
    line = ANSI.sub("", line).rstrip()
    m = LINE.match(line)
    if not m:
        return None
    rest = m.group("rest")
    # The message is everything before the first `key=`; spans, when present,
    # are rendered before the target and never reach `rest`.
    first_field = FIELD.search(rest)
    message = (rest[: first_field.start()] if first_field else rest).strip()
    if message not in EVENTS:
        return None
    try:
        ts_ms = parse_ts(m.group("ts"))
    except ValueError:
        return None
    event = {"event": message, "ts_ms": ts_ms, "target": m.group("target")}
    for key, raw in FIELD.findall(rest):
        event[key] = parse_value(raw)
    return event


def node_log_files(node_dir: Path):
    files = sorted(p for p in node_dir.glob("__logs-*") if p.is_file())
    if files:
        return files
    return sorted(p for p in node_dir.rglob("*.log*") if p.is_file())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("scenario_dir", type=Path)
    parser.add_argument("-o", "--out", type=Path, default=None)
    args = parser.parse_args()
    scenario = args.scenario_dir
    out = args.out or scenario / "events.jsonl"

    node_dirs = sorted(p for p in scenario.glob("node-*") if p.is_dir())
    if not node_dirs:
        print(f"no node-* directories under {scenario}", file=sys.stderr)
        return 1

    counts = {}
    with out.open("w") as sink:
        for node_dir in node_dirs:
            node = node_dir.name
            files = node_log_files(node_dir)
            if not files:
                print(f"{node}: no log files", file=sys.stderr)
            for path in files:
                with path.open(errors="replace") as source:
                    for line in source:
                        event = parse_line(line)
                        if event is None:
                            continue
                        event["node"] = node
                        sink.write(json.dumps(event) + "\n")
                        counts[(node, event["event"])] = counts.get((node, event["event"]), 0) + 1

    for (node, name), n in sorted(counts.items()):
        print(f"{node:10} {name:26} {n}")
    print(f"wrote {out}")
    return 0 if counts else 2


if __name__ == "__main__":
    sys.exit(main())
