import { esc, icon } from "../formatting.js";

// `detail` is trusted markup. Callers must escape interpolated server data.
export function banner(kind, title, detail) {
  const glyph = kind === "bad" || kind === "warn" ? "alert" : "check";
  return `<div class="banner ${kind}">${icon(glyph)}<div><b>${esc(title)}</b>${detail || ""}</div></div>`;
}
