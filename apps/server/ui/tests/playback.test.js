import assert from "node:assert/strict";
import test from "node:test";
import { contiguousRuns, coverageBuckets, playbackPath, recordingSearchPath, searchRange, startPlayback, stopPlayback, togglePlayback } from "../js/playback.js";

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

test("playback can pause, resume and stop without pretending to seek", async () => {
  const calls = [];
  const video = {
    paused: false,
    ended: false,
    pause() { calls.push("pause"); this.paused = true; },
    play() { calls.push("play"); this.paused = false; return Promise.resolve(); },
    removeAttribute(name) { calls.push(`remove:${name}`); },
    load() { calls.push("load"); },
  };

  await togglePlayback(video);
  await togglePlayback(video);
  stopPlayback(video);

  assert.deepEqual(calls, ["pause", "play", "pause", "remove:src", "load"]);
});

test("the play action loads the selected run before starting it", async () => {
  const calls = [];
  const video = {
    pause() { calls.push("pause"); },
    setAttribute(name, value) { calls.push(`${name}:${value}`); },
    load() { calls.push("load"); },
    play() { calls.push("play"); return Promise.resolve(); },
  };

  await startPlayback(video, "/recordings/run-one");

  assert.deepEqual(calls, ["pause", "src:/recordings/run-one", "load", "play"]);
});
