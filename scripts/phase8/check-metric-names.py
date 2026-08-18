#!/usr/bin/env python3
"""Verify every metric named by an alert or dashboard actually exists.

P8.7. This exists because the first draft of `docs/phase-8/alerts.yml` invented
seven metric names, and an alert on a series that does not exist never fires --
it is silent in exactly the way a working alert is silent. A dashboard panel on
a missing series is the same failure, quieter.

Grafana and Prometheus are not needed to catch that. This compares every
`surveillance_*` identifier in the alert rules and dashboard JSON against the
metric names a real server exports.

    # against a running server
    scripts/phase8/check-metric-names.py --url http://127.0.0.1:3000/metrics

    # or against a captured dump
    scripts/phase8/check-metric-names.py --file /tmp/metrics.txt

Exit code 0 if every referenced name exists, 1 otherwise.

What it does NOT check: whether an expression is correct, whether a panel
renders, or whether a threshold is sensible. Those need promtool and Grafana.
"""

from __future__ import annotations

import argparse
import glob
import json
import re
import sys
import urllib.request

NAME = re.compile(r"surveillance_[a-z0-9_]+")


def exported_names(text: str) -> set[str]:
    """Metric names from Prometheus text exposition output."""
    names = set()
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        names.add(line.split("{")[0].split(" ")[0])
    return names


def referenced_names(path: str) -> set[str]:
    """Metric names referenced by a rules or dashboard file, ignoring comments.

    Comment lines are skipped so that a file may *name* a non-existent metric in
    prose -- alerts.yml deliberately lists the seven invented names as a warning
    -- without failing its own check.
    """
    names: set[str] = set()
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            if line.lstrip().startswith("#"):
                continue
            names.update(NAME.findall(line))
    return names


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--url", help="a running server's /metrics endpoint")
    source.add_argument("--file", help="a captured /metrics dump")
    arguments = parser.parse_args()

    if arguments.url:
        with urllib.request.urlopen(arguments.url, timeout=10) as response:
            text = response.read().decode("utf-8")
    else:
        with open(arguments.file, encoding="utf-8") as handle:
            text = handle.read()

    live = exported_names(text)
    if not live:
        print("no metric names found in the exposition output", file=sys.stderr)
        return 1

    targets = sorted(glob.glob("docs/phase-8/dashboards/*.json")) + [
        "docs/phase-8/alerts.yml",
        "docs/phase-8/alerts_test.yml",
    ]

    failed = False
    for path in targets:
        if path.endswith(".json"):
            with open(path, encoding="utf-8") as handle:
                json.load(handle)  # malformed JSON fails here, loudly
        missing = sorted(n for n in referenced_names(path) if n not in live)
        if missing:
            failed = True
            print(f"{path}:")
            for name in missing:
                print(f"    MISSING: {name}")
        else:
            print(f"  ok: {path}")

    if failed:
        print(
            "\nA referenced metric that is not exported produces a panel that is "
            "always empty and an alert that never fires.",
            file=sys.stderr,
        )
        return 1
    print(f"\nall referenced metric names exist ({len(live)} exported)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
