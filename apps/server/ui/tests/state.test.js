import assert from "node:assert/strict";
import test from "node:test";
import { createAppState, createTrendStore } from "../js/state.js";

test("application state instances do not share domains", () => {
  const first = createAppState();
  const second = createAppState();
  first.cameras = [{ camera_id: "north" }];
  assert.equal(second.cameras, null);
});

test("trend stores reject non-finite values and enforce their bound", () => {
  const store = createTrendStore(2);
  store.push("cameras", 1);
  store.push("cameras", Number.NaN);
  store.push("cameras", 2);
  store.push("cameras", 3);
  assert.deepEqual(store.trends.cameras, [2, 3]);
  assert.deepEqual(store.trends.events, []);
});
