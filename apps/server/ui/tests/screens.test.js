import assert from "node:assert/strict";
import test from "node:test";
import { createPlaybackScreen } from "../js/screens/playback.js";
import { createStorageScreen } from "../js/screens/storage.js";
import { createSystemScreen } from "../js/screens/system.js";

test("playback segment rows can start their exact recording", () => {
  const recordings = [
    { recording_id: "rec-1", started_at: "2026-08-18T00:00:00.100Z", ended_at: "2026-08-18T00:01:00.200Z", size_bytes: 1 },
    { recording_id: "rec-2", started_at: "2026-08-18T00:01:00.200Z", ended_at: "2026-08-18T00:02:00.300Z", size_bytes: 1 },
  ];
  const render = createPlaybackScreen({
    state: { system: null },
    fleetCounts: () => ({ cameras: [{ camera_id: "north" }] }),
    fleetProblem: () => "",
    errorBanner: () => "error",
    pb: {
      camera: "north", date: "2026-08-18", from: "00:00:00", to: "01:00:00",
      result: { recordings }, error: null, busy: false,
    },
  });

  const html = render();
  assert.match(html, /data-recording="rec-1"/);
  assert.match(html, /data-recording="rec-2"/);
  assert.match(html, /Full recording/);
});

test("storage renders from its explicit metrics and settings dependencies", () => {
  const render = createStorageScreen({
    fleetCounts: () => ({
      cameras: [{
        camera_id: "north", pipeline_state: "running", connection_state: "connected",
        last_actionable_failure: null, recorder_queue_depth: 0, recorder_queue_capacity: 8,
        recorder_completed_segments: 1, recorder_failed_segments: 0, last_media_unix_ms: 1,
      }],
      total: 1,
      readable: true,
    }),
    fleetProblem: () => "",
    isState: (value, expected) => value === expected,
    metricSum: () => 0,
    pipelinePill: () => "running",
    pressureName: () => "nominal",
    recordingDot: () => "recording",
    settingValue: () => null,
    storageFacts: () => ({ used: null, free: null, total: null, usedPct: null, pressure: null }),
  });
  assert.match(render(), /Storage capacity is not being reported/);
});

test("system renders an unavailable capability report without throwing", () => {
  const render = createSystemScreen({
    state: { system: null, systemError: null },
    metric: () => null,
    metricSum: () => null,
    detectionIsFixture: () => true,
    errorBanner: () => "error",
  });
  assert.match(render(), /Loading capabilities/);
});
