#!/usr/bin/env python3
"""Turn one load-fixture run into a report that separates measurement from claim.

Everything printed here is a delta over the steady-state window, or a state read
at its end. Nothing is extrapolated to a camera count that was not run.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

# Counters worth a delta. Anything absent from a run is reported as absent
# rather than as zero, because "the metric does not exist" and "nothing
# happened" are different findings.
COUNTERS = [
    "surveillance_livekit_send_failures_total",
    "surveillance_livekit_stale_drops_total",
    "surveillance_livekit_media_rejections_total",
    "surveillance_livekit_reconnects_total",
    "surveillance_livekit_connection_attempts_total",
    "surveillance_livekit_connection_successes_total",
    "surveillance_livekit_access_units_total",
    "surveillance_livekit_bytes_total",
    "surveillance_livekit_keyframes_total",
    "surveillance_livekit_published_frames_total",
    "surveillance_livekit_adapter_drops_total",
    "surveillance_livekit_adapter_restarts_total",
    "surveillance_camera_live_evictions_total",
    "surveillance_camera_reconnects_total",
    "surveillance_camera_terminal_failures_total",
    "surveillance_recorder_segments_completed_total",
    "surveillance_recorder_segments_failed_total",
]

# Per-consumer queue behaviour is the whole point of the run: the bounded queues
# are where 10 cameras plus 10 viewers would show up first. `dropped_oldest` on
# the live consumer is the freshness policy doing its job; `rejected_full` on the
# recorder is recording falling behind, which is the one the priority ordering
# says must never be silent.
QUEUE_SUFFIXES = [
    "depth",
    "capacity",
    "accepted_total",
    "received_total",
    "rejected_full_total",
    "dropped_oldest_total",
    "dropped_on_close_total",
]

GAUGES = [
    "surveillance_camera_connected",
    "surveillance_livekit_state",
    # Summed across cameras, so this is subscribers fleet-wide. The transcoding
    # publisher only spends CPU while this is above zero, which makes it the
    # denominator for any CPU-per-viewer claim (ADR 012).
    "surveillance_livekit_viewers",
]


def parse(path: Path) -> dict[str, float]:
    """Sum every labelled series of a metric into one number."""
    totals: dict[str, float] = {}
    if not path.exists():
        return totals
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("#") or not line.strip():
            continue
        match = re.match(r"^([a-z_0-9]+)(\{[^}]*\})?\s+([0-9.eE+-]+)$", line.strip())
        if not match:
            continue
        name, _labels, value = match.groups()
        try:
            totals[name] = totals.get(name, 0.0) + float(value)
        except ValueError:
            continue
    return totals


def by_consumer(path: Path, metric: str) -> dict[str, float]:
    """Sum one queue metric per consumer label (recorder / live / detector)."""
    totals: dict[str, float] = {}
    if not path.exists():
        return totals
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.startswith(metric + "{"):
            continue
        consumer = re.search(r'consumer="([^"]+)"', line)
        value = line.rsplit(None, 1)[-1]
        if not consumer:
            continue
        try:
            totals[consumer.group(1)] = totals.get(consumer.group(1), 0.0) + float(value)
        except ValueError:
            continue
    return totals


def series_count(path: Path, name: str, predicate) -> int:
    if not path.exists():
        return 0
    count = 0
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.startswith(name + "{"):
            continue
        value = line.rsplit(None, 1)[-1]
        try:
            if predicate(float(value)):
                count += 1
        except ValueError:
            pass
    return count


def main() -> int:
    work = Path(sys.argv[1])
    cameras, viewers, window = int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])

    start = parse(work / "metrics-start.txt")
    end = parse(work / "metrics-end.txt")
    cpu = json.loads((work / "cpu-result.json").read_text(encoding="utf-8"))

    print("=" * 72)
    print(f"LiveKit load run — {cameras} cameras, {viewers} viewers, {window}s window")
    print("=" * 72)
    print()
    print(f"wall time measured: {cpu['wall_seconds']}s")
    print()

    print("-- counter deltas over the window " + "-" * 37)
    for name in COUNTERS:
        if name not in start and name not in end:
            print(f"  {name:<52} (metric not exported)")
            continue
        delta = end.get(name, 0.0) - start.get(name, 0.0)
        print(f"  {name:<52} {delta:>12,.0f}")
    print()

    print("-- bounded queues, by consumer " + "-" * 41)
    start_path, end_path_ = work / "metrics-start.txt", work / "metrics-end.txt"
    for suffix in QUEUE_SUFFIXES:
        metric = f"surveillance_media_queue_{suffix}"
        before, after = by_consumer(start_path, metric), by_consumer(end_path_, metric)
        consumers = sorted(set(before) | set(after))
        if not consumers:
            print(f"  {suffix:<24} (no series)")
            continue
        if suffix in ("depth", "capacity"):   # gauges: report the end state
            rendered = "  ".join(f"{c}={after.get(c, 0):,.0f}" for c in consumers)
            print(f"  {suffix:<24} {rendered}   (at end)")
        else:                                  # counters: report the delta
            rendered = "  ".join(
                f"{c}={after.get(c, 0.0) - before.get(c, 0.0):,.0f}" for c in consumers
            )
            print(f"  {suffix:<24} {rendered}   (delta)")
    print()

    print("-- state at the end of the window " + "-" * 38)
    end_path = work / "metrics-end.txt"
    print(f"  cameras connected                                    "
          f"{series_count(end_path, 'surveillance_camera_connected', lambda v: v == 1):>12}")
    print(f"  livekit publishers in state=3 (published)            "
          f"{series_count(end_path, 'surveillance_livekit_state', lambda v: v == 3):>12}")
    for name in GAUGES:
        if name in end:
            print(f"  {name:<52} {end[name]:>12,.0f}")
    print()

    print("-- CPU, average cores over the window " + "-" * 34)
    print("   (100 = one core saturated for the entire window)")
    for pid, entry in cpu["per_pid"].items():
        print(f"  pid {pid:<8} {entry['cpu_percent_of_one_core']:>8}%   "
              f"({entry['cpu_seconds']}s CPU, {entry['children_at_end']} children)")
    print()

    recorded = work / "recorded-kb.txt"
    if recorded.exists():
        kb = int(recorded.read_text().strip() or 0)
        print(f"-- recorded to disk: {kb / 1024:.1f} MiB in {window}s "
              f"({kb / 1024 / max(window, 1) * 8:.1f} Mib/s aggregate)")
        print()

    print("-- NOT measured " + "-" * 56)
    print("  Viewer-perceived latency. There is no end-to-end live latency")
    print("  histogram in this system; the only latency buckets are the")
    print("  detector's. A p50/p90/p99 for live viewing cannot be produced from")
    print("  these metrics and none is reported here.")
    print()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
