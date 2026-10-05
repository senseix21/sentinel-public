// Node-runnable tests for the pure functions in detection-overlay.js.
// Run with: node apps/server/ui/js/detection-overlay.test.js
//
// Only the DOM-free parts are exercised here. The DetectionOverlay class
// uses `ResizeObserver`, `requestAnimationFrame`, and `document`; those
// are smoke-tested in the browser from the live view.

import assert from "node:assert/strict";
import { contentRect, DETECTION_MAX_AGE_MS, DETECTION_TOPIC } from "./detection-overlay.js";

let passed = 0;
let failed = 0;
function test(name, fn) {
  try {
    fn();
    console.log("  ok  " + name);
    passed += 1;
  } catch (error) {
    console.error("  FAIL " + name + ": " + error.message);
    failed += 1;
  }
}

// --- constants ---

test("exported DETECTION_TOPIC matches the server-side constant", () => {
  assert.strictEqual(DETECTION_TOPIC, "det/v1");
});

test("DETECTION_MAX_AGE_MS is 3000", () => {
  assert.strictEqual(DETECTION_MAX_AGE_MS, 3000);
});

// --- contentRect: letterbox/pillarbox math ---

test("16:9 content in 4:3 container pillarboxes correctly", () => {
  // element 400x300, video 1280x720
  //   scale = min(400/1280, 300/720) = 0.3125
  //   content 400 x 225, offset x=0, y=37.5
  const video = { clientWidth: 400, clientHeight: 300, videoWidth: 1280, videoHeight: 720 };
  const r = contentRect(video);
  assert.ok(r !== null);
  assert.ok(Math.abs(r.w - 400) < 0.5, "w=" + r.w);
  assert.ok(Math.abs(r.h - 225) < 0.5, "h=" + r.h);
  assert.ok(Math.abs(r.x) < 0.5, "x=" + r.x);
  assert.ok(Math.abs(r.y - 37.5) < 0.5, "y=" + r.y);
});

test("4:3 content in 16:9 container letterboxes correctly", () => {
  // element 640x360, video 640x480
  //   scale = min(640/640, 360/480) = 0.75
  //   content 480 x 360, offset x=80, y=0
  const video = { clientWidth: 640, clientHeight: 360, videoWidth: 640, videoHeight: 480 };
  const r = contentRect(video);
  assert.ok(Math.abs(r.w - 480) < 0.5, "w=" + r.w);
  assert.ok(Math.abs(r.h - 360) < 0.5, "h=" + r.h);
  assert.ok(Math.abs(r.x - 80) < 0.5, "x=" + r.x);
  assert.ok(Math.abs(r.y) < 0.5, "y=" + r.y);
});

test("exact aspect match has no bars", () => {
  const video = { clientWidth: 300, clientHeight: 300, videoWidth: 300, videoHeight: 300 };
  const r = contentRect(video);
  assert.ok(Math.abs(r.x) < 0.5 && Math.abs(r.y) < 0.5);
  assert.ok(Math.abs(r.w - 300) < 0.5 && Math.abs(r.h - 300) < 0.5);
});

test("returns null before the video knows its intrinsic size", () => {
  assert.strictEqual(
    contentRect({ clientWidth: 400, clientHeight: 300, videoWidth: 0, videoHeight: 0 }),
    null,
  );
});

test("returns null when the element has zero size", () => {
  assert.strictEqual(
    contentRect({ clientWidth: 0, clientHeight: 0, videoWidth: 640, videoHeight: 480 }),
    null,
  );
});

// --- coordinate transform: a few representative points ---

test("full-frame normalised box covers the whole content rect", () => {
  const video = { clientWidth: 400, clientHeight: 300, videoWidth: 1280, videoHeight: 720 };
  const r = contentRect(video);
  const det = { left: 0, top: 0, width: 1, height: 1 };
  const x = r.x + det.left * r.w;
  const y = r.y + det.top * r.h;
  const w = det.width * r.w;
  const h = det.height * r.h;
  assert.ok(Math.abs(x - r.x) < 0.5);
  assert.ok(Math.abs(y - r.y) < 0.5);
  assert.ok(Math.abs(w - r.w) < 0.5);
  assert.ok(Math.abs(h - r.h) < 0.5);
});

test("centred quarter-frame box maps to the content centre", () => {
  const video = { clientWidth: 400, clientHeight: 400, videoWidth: 400, videoHeight: 400 };
  const r = contentRect(video);
  const det = { left: 0.25, top: 0.25, width: 0.5, height: 0.5 };
  const x = r.x + det.left * r.w;
  const y = r.y + det.top * r.h;
  const w = det.width * r.w;
  const h = det.height * r.h;
  assert.ok(Math.abs(x - 100) < 0.5, "x=" + x);
  assert.ok(Math.abs(y - 100) < 0.5, "y=" + y);
  assert.ok(Math.abs(w - 200) < 0.5, "w=" + w);
  assert.ok(Math.abs(h - 200) < 0.5, "h=" + h);
});

console.log("");
console.log(passed + " passed, " + failed + " failed");
if (failed > 0) process.exit(1);
