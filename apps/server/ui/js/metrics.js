export function parseMetrics(text) {
  const table = new Map();
  for (const line of text.split("\n")) {
    if (!line || line.charCodeAt(0) === 35) continue;
    const cut = line.lastIndexOf(" ");
    if (cut < 0) continue;
    const value = Number(line.slice(cut + 1));
    if (!Number.isFinite(value)) continue;
    const key = line.slice(0, cut);
    const brace = key.indexOf("{");
    const name = brace < 0 ? key : key.slice(0, brace);
    const labels = {};
    if (brace >= 0) {
      const inner = key.slice(brace + 1, key.lastIndexOf("}"));
      for (const pair of inner.split(",")) {
        const equals = pair.indexOf("=");
        if (equals < 0) continue;
        labels[pair.slice(0, equals).trim()] = pair.slice(equals + 1).trim().replace(/^"|"$/g, "");
      }
    }
    if (!table.has(name)) table.set(name, []);
    table.get(name).push({ labels, value });
  }
  return table;
}

export function createMetricsView(tableProvider) {
  const samples = (name) => (tableProvider() && tableProvider().get(name)) || [];
  return {
    samples,
    first(name) {
      const values = samples(name);
      return values.length ? values[0].value : null;
    },
    sum(name) {
      const values = samples(name);
      return values.length ? values.reduce((total, sample) => total + sample.value, 0) : null;
    },
    where(name, label, value) {
      const found = samples(name).find((sample) => sample.labels[label] === value);
      return found ? found.value : null;
    },
  };
}
