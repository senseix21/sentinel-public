import assert from "node:assert/strict";
import test from "node:test";
import { contiguousRuns, coverageBuckets, mediaErrorMessage, playbackPath, playbackTime, recordingSearchPath, recordingVideoPath, searchRange, startPlayback, stopPlayback, togglePlayback, wallTimeToUtc } from "../js/playback.js";

test("search uses the server start/end names", () => {
  const range = searchRange({ date: "2026-08-18", from: "01:02:03", to: "04:05:06" });
  assert.equal(recordingSearchPath("north", range), "/api/cameras/north/recordings?start=2026-08-18T01%3A02%3A03Z&end=2026-08-18T04%3A05%3A06Z");
});

test("playback ranges interpret Bangladesh and US Eastern wall time", () => {
  assert.equal(wallTimeToUtc("2026-08-18", "01:02:03", "Asia/Dhaka"), "2026-08-17T19:02:03Z");
  assert.equal(wallTimeToUtc("2026-08-18", "01:02:03", "America/New_York"), "2026-08-18T05:02:03Z");
  assert.equal(playbackTime(Date.parse("2026-08-18T05:02:03Z"), "America/New_York"), "2026-08-18 01:02:03");
});

test("playback preserves recorded timestamp precision", () => {
  const path = playbackPath("north", "2026-08-18T01:02:03.011609Z", "2026-08-18T01:03:03.123456Z");
  assert.match(path, /011609Z/);
  assert.match(path, /123456Z/);
});

test("recording video duration is optional and bounded", () => {
  assert.equal(recordingVideoPath("north", "rec-1"), "/api/cameras/north/recordings/rec-1/video");
  assert.equal(recordingVideoPath("north", "rec-1", 5), "/api/cameras/north/recordings/rec-1/video?duration_seconds=5");
  assert.throws(() => recordingVideoPath("north", "rec-1", 0), RangeError);
  assert.throws(() => recordingVideoPath("north", "rec-1", 61), RangeError);
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

test("a wall time that happens twice is refused rather than guessed", () => {
  // 2026-11-01 01:30 US Eastern is both 05:30Z and 06:30Z. Returning either
  // one silently searches an hour the operator did not ask for.
  assert.throws(
    () => wallTimeToUtc("2026-11-01", "01:30:00", "America/New_York"),
    /happens twice/,
  );
  // The hour either side is unambiguous and must still resolve.
  assert.equal(wallTimeToUtc("2026-11-01", "00:30:00", "America/New_York"), "2026-11-01T04:30:00Z");
  assert.equal(wallTimeToUtc("2026-11-01", "03:30:00", "America/New_York"), "2026-11-01T08:30:00Z");
});

test("a wall time that does not exist is still refused", () => {
  assert.throws(
    () => wallTimeToUtc("2026-03-08", "02:30:00", "America/New_York"),
    /does not exist/,
  );
});

test("media failures get operator language instead of silence", () => {
  assert.match(mediaErrorMessage({ error: { code: 4 } }), /would not stream/);
  assert.match(mediaErrorMessage({ error: { code: 2 } }), /connection dropped/);
  assert.match(mediaErrorMessage({ error: { code: 3 } }), /could not be decoded/);
  assert.match(mediaErrorMessage({}), /could not be played/);
  assert.match(mediaErrorMessage(null), /could not be played/);
});
