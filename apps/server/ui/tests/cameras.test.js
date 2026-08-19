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
