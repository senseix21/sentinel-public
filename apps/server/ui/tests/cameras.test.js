import assert from "node:assert/strict";
import test from "node:test";
import { createCamerasScreen } from "../js/screens/cameras.js";

test("a missing camera does not reveal whether authorization hid it", async () => {
  const screen = createCamerasScreen({
    add: {},
    api: async () => { throw { status: 404 }; },
    cameraTile: () => "",
    detectionIsFixture: () => true,
    errorBanner: () => "unexpected error",
    fleetCounts: () => ({ cameras: [], total: 0, readable: true }),
    fleetProblem: () => "",
    livekitChip: () => "",
    pipelinePill: () => "",
    route: () => ({ name: "cameras", args: ["hidden-camera"] }),
    render: async () => {},
  });

  screen.render(["hidden-camera"]);
  await new Promise((resolve) => setTimeout(resolve, 0));
  const html = screen.render(["hidden-camera"]);

  assert.match(html, /No such camera/);
  for (const leak of ["permission", "access", "allowed", "scope", "authoris", "authoriz"]) {
    assert.equal(html.toLowerCase().includes(leak), false, `leaked ${leak}`);
  }
});

test("empty fleet offers durable camera onboarding with separate credential inputs", () => {
  const screen = createCamerasScreen({
    add: { id: "", rtsp: "", width: "1280", height: "720", detection: "off" },
    cameraTile: () => "", fleetCounts: () => ({ cameras: [], total: 0, connected: 0, readable: true }),
    fleetProblem: () => "", route: () => ({ name: "cameras", args: [] }),
  });
  const html = screen.render([]);
  assert.match(html, /No cameras are configured/);
  assert.match(html, /type="password" data-add="password"/);
  assert.match(html, /running cameras resume/);
  assert.doesNotMatch(html, /This form takes no RTSP/);
});
