#!/usr/bin/env python3
"""Summarize and gate one direct-camera Prometheus measurement window."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

SAMPLE = re.compile(
    r"^([a-zA-Z_:][a-zA-Z0-9_:]*)(?:\{([^}]*)\})?\s+([0-9.eE+-]+)$"
)
LABEL = re.compile(r'(\w+)="((?:\\.|[^"\\])*)"')

COUNTERS = (
    "surveillance_camera_connection_attempts_total",
    "surveillance_camera_connection_epochs_total",
    "surveillance_camera_reconnects_total",
    "surveillance_camera_terminal_failures_total",
    "surveillance_camera_access_units_total",
    "surveillance_camera_packet_loss_events_total",
    "surveillance_camera_lost_packets_total",
    "surveillance_camera_teardown_failures_total",
    "surveillance_recorder_segments_completed_total",
    "surveillance_recorder_segments_failed_total",
)


def parse(path: Path, camera: str) -> dict[tuple[str, str | None], float]:
    """Read camera-labelled metrics, separating queue consumers."""
    values: dict[tuple[str, str | None], float] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        if not raw or raw.startswith("#"):
            continue
        match = SAMPLE.match(raw.strip())
        if not match:
            continue
        name, rendered_labels, rendered_value = match.groups()
        labels = dict(LABEL.findall(rendered_labels or ""))
        if labels.get("camera_id", labels.get("camera")) != camera:
            continue
        try:
            value = float(rendered_value)
        except ValueError:
            continue
        values[(name, labels.get("consumer"))] = value
    return values


def delta(
    start: dict[tuple[str, str | None], float],
    end: dict[tuple[str, str | None], float],
    name: str,
    consumer: str | None = None,
) -> float | None:
    key = (name, consumer)
    if key not in start and key not in end:
        return None
    return end.get(key, 0.0) - start.get(key, 0.0)


def require_zero(summary: dict, failures: list[str], name: str) -> None:
    value = summary["counter_deltas"].get(name)
    if value is None:
        failures.append(f"required metric is absent: {name}")
    elif value != 0:
        failures.append(f"{name} increased by {value:g}")


def build_report(start_path: Path, end_path: Path, camera: str) -> dict:
    start = parse(start_path, camera)
    end = parse(end_path, camera)
    counter_deltas = {name: delta(start, end, name) for name in COUNTERS}
    queue_deltas: dict[str, dict[str, float | None]] = {}
    for consumer in ("recorder", "live", "detection"):
        queue_deltas[consumer] = {
            suffix: delta(
                start,
                end,
                f"surveillance_media_queue_{suffix}",
                consumer,
            )
            for suffix in (
                "accepted_total",
                "received_total",
                "rejected_full_total",
                "dropped_oldest_total",
                "dropped_on_close_total",
            )
        }

    final_connected = end.get(("surveillance_camera_connected", None))
    final_pipeline_state = end.get(("surveillance_camera_pipeline_state", None))
    report = {
        "camera_id": camera,
        "final_connected": final_connected,
        "final_pipeline_state": final_pipeline_state,
        "counter_deltas": counter_deltas,
        "queue_deltas": queue_deltas,
        "failures": [],
    }
    failures: list[str] = report["failures"]
    if final_connected != 1:
        failures.append("camera was not connected at the end of the measurement window")
    access_units = counter_deltas["surveillance_camera_access_units_total"]
    if access_units is None:
        failures.append("required metric is absent: surveillance_camera_access_units_total")
    elif access_units <= 0:
        failures.append("no access units arrived during the measurement window")
    completed = counter_deltas["surveillance_recorder_segments_completed_total"]
    if completed is None:
        failures.append(
            "required metric is absent: surveillance_recorder_segments_completed_total"
        )
    elif completed < 2:
        failures.append("fewer than two segments completed during the measurement window")

    for name in (
        "surveillance_camera_terminal_failures_total",
        "surveillance_camera_teardown_failures_total",
        "surveillance_recorder_segments_failed_total",
    ):
        require_zero(report, failures, name)
    recorder_rejected = queue_deltas["recorder"]["rejected_full_total"]
    if recorder_rejected is None:
        failures.append(
            "required metric is absent: recorder queue rejected_full_total"
        )
    elif recorder_rejected != 0:
        failures.append(f"recorder queue rejected {recorder_rejected:g} access units")
    return report


def main() -> int:
    if len(sys.argv) != 5:
        print(
            "usage: direct-camera-report.py START.prom END.prom CAMERA_ID OUTPUT.json",
            file=sys.stderr,
        )
        return 2
    start, end, camera, output = (
        Path(sys.argv[1]),
        Path(sys.argv[2]),
        sys.argv[3],
        Path(sys.argv[4]),
    )
    report = build_report(start, end, camera)
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if report["failures"]:
        for failure in report["failures"]:
            print(f"direct-camera metric gate: {failure}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
