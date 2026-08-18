#!/usr/bin/env python3
"""Measure CPU consumed by a process tree over a window.

`ps -o %cpu` on macOS reports a decayed average over the whole process lifetime,
so it would fold warm-up and connection storms into a steady-state number. This
records cumulative CPU-seconds at two instants instead; the caller divides the
delta by the wall time it actually waited.

Children are resolved at both instants, because the server spawns one `lk`
publisher per camera and a publisher that restarts mid-window would otherwise
vanish from the accounting.
"""

from __future__ import annotations

import json
import subprocess
import sys
import time


def cputime_seconds(pid: str) -> float | None:
    """Cumulative user+system CPU seconds for one pid, or None if it is gone."""
    try:
        out = subprocess.run(
            ["ps", "-o", "cputime=", "-p", pid],
            capture_output=True, text=True, check=False,
        ).stdout.strip()
    except OSError:
        return None
    if not out:
        return None
    # ps prints [[dd-]hh:]mm:ss
    days, _, rest = out.partition("-")
    if not rest:
        days, rest = "0", days
    parts = [float(p) for p in rest.split(":")]
    while len(parts) < 3:
        parts.insert(0, 0.0)
    hours, minutes, seconds = parts
    return float(days) * 86400 + hours * 3600 + minutes * 60 + seconds


def children_of(pid: str) -> list[str]:
    out = subprocess.run(
        ["pgrep", "-P", pid], capture_output=True, text=True, check=False
    ).stdout.split()
    return [c for c in out if c.strip()]


def snapshot(pids: list[str]) -> dict:
    """CPU seconds for each pid and, separately, for each pid's children."""
    result = {"at": time.time(), "self": {}, "children": {}}
    for pid in pids:
        value = cputime_seconds(pid)
        if value is not None:
            result["self"][pid] = value
        kids = {}
        for child in children_of(pid):
            child_value = cputime_seconds(child)
            if child_value is not None:
                kids[child] = child_value
        if kids:
            result["children"][pid] = kids
    return result


def total(entry: dict, pid: str) -> float:
    """A pid's own CPU plus everything it spawned."""
    return entry["self"].get(pid, 0.0) + sum(entry["children"].get(pid, {}).values())


def main() -> int:
    mode, path, *pids = sys.argv[1:]
    if mode == "start":
        with open(path, "w", encoding="utf-8") as handle:
            json.dump(snapshot(pids), handle)
        return 0

    with open(path, encoding="utf-8") as handle:
        before = json.load(handle)
    after = snapshot(pids)
    wall = after["at"] - before["at"]

    report = {"wall_seconds": round(wall, 2), "per_pid": {}}
    for pid in pids:
        delta = total(after, pid) - total(before, pid)
        report["per_pid"][pid] = {
            "cpu_seconds": round(delta, 2),
            # 100 means one core saturated for the whole window.
            "cpu_percent_of_one_core": round(delta / wall * 100, 1) if wall > 0 else None,
            "children_at_end": len(after["children"].get(pid, {})),
        }
    json.dump(report, sys.stdout, indent=2)
    print()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
