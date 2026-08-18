export const SCREENS = [
  { id: "dashboard", path: "/", icon: "dashboard", label: "Dashboard", title: "Surveillance Dashboard", sub: "System overview and real-time status" },
  { id: "live", path: "/live", icon: "live", label: "Live View", title: "Live View", sub: "Attached publishers, one subscription per tile" },
  { id: "playback", path: "/playback", icon: "clock", label: "Playback", title: "Historical Playback", sub: "Recorded segments from the indexed archive" },
  { id: "events", path: "/events", icon: "bell", label: "Events", title: "Event Center", sub: "Domain events published on the in-process bus" },
  { id: "heatmap", path: "/heatmap", icon: "heat", label: "Heatmap", title: "Heatmap Analytics", sub: "Spatial activity analysis" },
  { id: "storage", path: "/storage", icon: "db", label: "Storage", title: "Storage", sub: "Capacity, retention and per-camera recording state" },
  { id: "cameras", path: "/cameras", icon: "cam", label: "Cameras", title: "Cameras", sub: "Fleet management and per-camera health" },
  { id: "settings", path: "/settings", icon: "gear", label: "Settings", title: "System Settings", sub: "Persistent, layered, transactionally applied configuration" },
  { id: "system", path: "/system", icon: "shield", label: "System", title: "System", sub: "Capabilities, surfaces and instrumentation" },
];

export function parseRoute(hash) {
  const raw = (hash || "#/").slice(1) || "/";
  const parts = raw.split("/").filter(Boolean);
  return { name: parts[0] || "dashboard", args: parts.slice(1).map(decodeURIComponent) };
}

export function screenFor(hash) {
  const route = parseRoute(hash);
  return SCREENS.find((screen) => screen.id === route.name) || SCREENS[0];
}
