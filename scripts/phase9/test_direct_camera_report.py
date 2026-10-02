from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("direct-camera-report.py")
SPEC = importlib.util.spec_from_file_location("direct_camera_report", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
REPORT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REPORT)


def metrics(
    access_units: int,
    completed: int,
    *,
    connected: int = 1,
    recorder_failures: int = 0,
    recorder_rejected: int = 0,
) -> str:
    camera = 'camera_id="front-door"'
    other = 'camera_id="other"'
    return f"""
surveillance_camera_connected{{{camera}}} {connected}
surveillance_camera_pipeline_state{{{camera}}} 2
surveillance_camera_connection_attempts_total{{{camera}}} 1
surveillance_camera_connection_epochs_total{{{camera}}} 1
surveillance_camera_reconnects_total{{{camera}}} 0
surveillance_camera_terminal_failures_total{{{camera}}} 0
surveillance_camera_access_units_total{{{camera}}} {access_units}
surveillance_camera_packet_loss_events_total{{{camera}}} 0
surveillance_camera_lost_packets_total{{{camera}}} 0
surveillance_camera_teardown_failures_total{{{camera}}} 0
surveillance_recorder_segments_completed_total{{{camera}}} {completed}
surveillance_recorder_segments_failed_total{{{camera}}} {recorder_failures}
surveillance_media_queue_rejected_full_total{{{camera},consumer="recorder"}} {recorder_rejected}
surveillance_camera_access_units_total{{{other}}} 999999
"""


class DirectCameraReportTests(unittest.TestCase):
    def report(self, start: str, end: str) -> dict:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before = root / "before.prom"
            after = root / "after.prom"
            before.write_text(start, encoding="utf-8")
            after.write_text(end, encoding="utf-8")
            return REPORT.build_report(before, after, "front-door")

    def test_healthy_window_passes_and_ignores_other_cameras(self) -> None:
        report = self.report(metrics(10, 1), metrics(910, 4))
        self.assertEqual([], report["failures"])
        self.assertEqual(
            900,
            report["counter_deltas"]["surveillance_camera_access_units_total"],
        )
        self.assertEqual(
            3,
            report["counter_deltas"][
                "surveillance_recorder_segments_completed_total"
            ],
        )

    def test_recording_failure_and_rejection_fail_the_gate(self) -> None:
        report = self.report(
            metrics(10, 1),
            metrics(910, 4, recorder_failures=1, recorder_rejected=2),
        )
        rendered = "\n".join(report["failures"])
        self.assertIn("segments_failed_total increased", rendered)
        self.assertIn("recorder queue rejected 2", rendered)

    def test_disconnected_or_empty_window_fails_the_gate(self) -> None:
        report = self.report(metrics(10, 1), metrics(10, 1, connected=0))
        rendered = "\n".join(report["failures"])
        self.assertIn("not connected", rendered)
        self.assertIn("no access units", rendered)
        self.assertIn("fewer than two segments", rendered)


if __name__ == "__main__":
    unittest.main()
