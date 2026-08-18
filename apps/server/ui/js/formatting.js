export function esc(value) {
  return String(value === null || value === undefined ? "" : value)
    .replace(/&/g, "&amp;").replace(/</g, "&lt;")
    .replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

export function icon(name, cls) {
  return `<svg class="ic ${cls || ""}"><use href="#i-${name}"></use></svg>`;
}

export function utc(ms) {
  if (ms === null || ms === undefined) return "—";
  const date = new Date(Number(ms));
  if (Number.isNaN(date.getTime())) return "—";
  return date.toISOString().replace("T", " ").replace(/\.\d+Z$/, "Z");
}

export function ago(ms, now = Date.now()) {
  if (ms === null || ms === undefined) return "—";
  const seconds = Math.max(0, Math.round((now - Number(ms)) / 1000));
  if (seconds < 60) return `${seconds}s ago`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}

export function bytes(value) {
  if (value === null || value === undefined || !Number.isFinite(Number(value))) return "—";
  let amount = Number(value);
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let unit = 0;
  while (amount >= 1024 && unit < units.length - 1) { amount /= 1024; unit += 1; }
  return `${amount >= 100 || unit === 0 ? Math.round(amount) : amount.toFixed(1)} ${units[unit]}`;
}

export function pct(part, whole) {
  if (!whole || !Number.isFinite(part / whole)) return null;
  return (part / whole) * 100;
}

export function fmtPct(value, digits) {
  return value === null || value === undefined ? "—" : `${value.toFixed(digits === undefined ? 1 : digits)}%`;
}
