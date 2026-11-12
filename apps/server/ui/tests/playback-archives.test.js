import assert from "node:assert/strict";
import test from "node:test";
import { createPlaybackScreen } from "../js/screens/playback.js";
import { createPlaybackState } from "../js/playback.js";

test("playback selects archived cameras with an empty runtime fleet", () => {
  const pb = createPlaybackState();
  const screen = createPlaybackScreen({
    state: { archiveCameras: ["removed-camera"], system: null }, pb,
    fleetCounts: () => ({ cameras: [] }), fleetProblem: () => "", errorBanner: () => "error",
  });
  const html = screen();
  assert.equal(pb.camera, "removed-camera");
  assert.match(html, /option value="removed-camera" selected/);
});

test("archive and active identities are merged once and escaped", () => {
  const screen = createPlaybackScreen({
    state: { archiveCameras: ["north", "<south>"], system: null }, pb: createPlaybackState(),
    fleetCounts: () => ({ cameras: [{ camera_id: "north" }] }), fleetProblem: () => "", errorBanner: () => "error",
  });
  const html = screen();
  assert.equal((html.match(/option value="north"/g) || []).length, 1);
  assert.match(html, /value="&lt;south&gt;"/);
});

test("folder selection uses its camera coverage and recorded date without a live camera", () => {
  const pb = createPlaybackState(); pb.source = "archive-1";
  const screen = createPlaybackScreen({
    state: { archiveCameras: ["primary-camera"], recordingSources: [{ id: "archive-1", label: ".local/laptop-recordings", state: "ready", cameras: [{camera_id: "camera-01", first_recorded_at: "2026-10-29T17:46:23Z", last_recorded_at: "2026-10-29T17:48:35Z", recordings: 5}] }] },
    pb, fleetCounts: () => ({ cameras: [] }), fleetProblem: () => "", errorBanner: () => "error",
  });
  const html = screen();
  assert.equal(pb.camera, "camera-01"); assert.equal(pb.date, "2026-10-29");
  assert.match(html, /Choose existing folder/); assert.match(html, /5 recordings indexed/);
  assert.doesNotMatch(html, /option value="primary-camera"/);
});
