import assert from "node:assert/strict";
import test from "node:test";
import { oppositeTheme, preferredTheme } from "../js/theme.js";

test("system preference selects the initial theme", () => {
  assert.equal(preferredTheme(true), "light");
  assert.equal(preferredTheme(false), "dark");
});

test("theme toggle is symmetric", () => {
  assert.equal(oppositeTheme("light"), "dark");
  assert.equal(oppositeTheme("dark"), "light");
});
