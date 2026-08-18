import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const uiRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

function modules(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return modules(path);
    return entry.isFile() && entry.name.endsWith(".js") ? [path] : [];
  });
}

test("every shipped application module parses", () => {
  const files = modules(join(uiRoot, "js"));
  assert.ok(files.length > 0);
  for (const file of files) execFileSync(process.execPath, ["--check", file]);
});
