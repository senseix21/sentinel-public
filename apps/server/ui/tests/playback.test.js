import assert from "node:assert/strict";
import test from "node:test";
import { contiguousRuns, coverageBuckets, playbackPath, recordingSearchPath, searchRange } from "../js/playback.js";

test("search uses the server start/end names", () => {
  const range = searchRange({ date: "2026-08-18", from: "01:02:03", to: "04:05:06" });
  assert.equal(recordingSearchPath("north", range), "/api/cameras/north/recordings?start=2026-08-18T01%3A02%3A03Z&end=2026-08-18T04%3A05%3A06Z");
});

test("playback preserves recorded timestamp precision", () => {
  const path = playbackPath("north", "2026-08-18T01:02:03.011609Z", "2026-08-18T01:03:03.123456Z");
  assert.match(path, /011609Z/);
  assert.match(path, /123456Z/);
});

test("contiguous segments merge and gaps remain separate", () => {
  const recordings = [
    { started_at: "2026-08-18T00:01:00Z", ended_at: "2026-08-18T00:02:00Z", size_bytes: 2 },
    { started_at: "2026-08-18T00:00:00Z", ended_at: "2026-08-18T00:01:00Z", size_bytes: 1 },
    { started_at: "2026-08-18T00:03:00Z", ended_at: "2026-08-18T00:04:00Z", size_bytes: 4 },
  ];
  assert.deepEqual(contiguousRuns(recordings).map(({ count, bytes }) => ({ count, bytes })), [
    { count: 2, bytes: 3 }, { count: 1, bytes: 4 },
  ]);
});

test("coverage handles empty and zero-span input", () => {
  assert.equal(coverageBuckets([]), null);
  const coverage = coverageBuckets([{ started_at: "2026-08-18T00:00:00Z", ended_at: "2026-08-18T00:00:00Z" }], 4);
  assert.deepEqual(coverage.filled, [true, false, false, false]);
});
