import assert from "node:assert/strict";
import test from "node:test";
import { createMetricsView, parseMetrics } from "../js/metrics.js";

test("parses values and labels while ignoring comments and malformed samples", () => {
  const table = parseMetrics(`# HELP ignored help\nplain 2\nlabelled{camera="north",state="up"} 3\nbad\nnot_number nope\n`);
  const metrics = createMetricsView(() => table);
  assert.equal(metrics.first("plain"), 2);
  assert.equal(metrics.where("labelled", "camera", "north"), 3);
  assert.equal(metrics.first("bad"), null);
});

test("sums a metric family and reports absent families", () => {
  const table = parseMetrics("events{kind=\"a\"} 2\nevents{kind=\"b\"} 5\n");
  const metrics = createMetricsView(() => table);
  assert.equal(metrics.sum("events"), 7);
  assert.equal(metrics.sum("missing"), null);
});
