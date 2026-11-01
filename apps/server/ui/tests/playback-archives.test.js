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
