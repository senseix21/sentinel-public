import assert from "node:assert/strict";
import test from "node:test";
import { parseRoute, screenFor } from "../js/router.js";

test("parses root and camera arguments", () => {
  assert.deepEqual(parseRoute("#/"), { name: "dashboard", args: [] });
  assert.deepEqual(parseRoute("#/cameras/north%20door"), { name: "cameras", args: ["north door"] });
});

test("unknown screens fall back to dashboard", () => {
  assert.equal(screenFor("#/does-not-exist").id, "dashboard");
});
