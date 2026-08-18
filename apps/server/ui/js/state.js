export function createAppState() {
  return {
    cameras: null,
    system: null,
    settings: null,
    settingsError: null,
    metrics: null,
    fleetError: null,
    systemError: null,
    loadedAt: null,
  };
}

export function createTrendStore(limit = 60) {
  const trends = { cameras: [], events: [], storage: [] };
  return {
    trends,
    push(key, value) {
      if (value === null || value === undefined || !Number.isFinite(value)) return;
      const series = trends[key];
      series.push(value);
      if (series.length > limit) series.shift();
    },
  };
}
