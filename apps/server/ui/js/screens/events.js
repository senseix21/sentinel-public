import { absent } from "../components/empty-state.js";
import { esc, icon } from "../formatting.js";

export function createEventsScreen({ metric, metricSamples, metricSum, kpi, sparkline, trend }) {
  return function screenEvents() {
    const byKind = metricSamples("surveillance_event_publications_by_kind_total");
    const byClass = metricSamples("surveillance_event_publications_by_class_total");
    const total = metricSum("surveillance_event_publications_total");
    const deliveries = metricSum("surveillance_event_deliveries_total");
    const dropped = metricSum("surveillance_event_dropped_newest_total");
    const replaced = metricSum("surveillance_event_replaced_oldest_total");
    const rejected = metricSum("surveillance_event_rejected_full_total");
    const subscribers = metric("surveillance_event_bus_subscribers");

    return `<div class="grid cols-4">
        ${kpi({ label: "Published", value: total === null ? "—" : String(total), foot: "since process start", icon: "bell", colour: "var(--purple)", spark: sparkline(trend.events, "#a855f7") })}
        ${kpi({ label: "Delivered", value: deliveries === null ? "—" : String(deliveries), foot: "to subscribers", icon: "net", colour: "var(--blue)" })}
        ${kpi({ label: "Overflowed", tone: dropped || replaced || rejected ? "warn" : "", value: String((dropped || 0) + (replaced || 0) + (rejected || 0)),
          foot: `${dropped || 0} dropped &middot; ${replaced || 0} replaced &middot; ${rejected || 0} rejected`, icon: "alert", colour: "var(--amber)" })}
        ${kpi({ label: "Subscribers", value: subscribers === null ? "—" : String(subscribers), foot: "attached to the bus now", icon: "user", colour: "var(--green)" })}
      </div>
      <div class="split mt">
        <div class="card">
          <h3>${icon("bell")} Publications by kind</h3>
          <p class="note">Live counters from the bounded in-process bus. These are totals, not a searchable history.</p>
          ${byKind.length ? `<div class="wrap"><table>
              <thead><tr><th>Kind</th><th class="num">Count</th></tr></thead>
              <tbody>${byKind.map((sample) => `<tr><td>${esc(sample.labels.kind || Object.values(sample.labels)[0] || "—")}</td><td class="num">${esc(sample.value)}</td></tr>`).join("")}</tbody>
            </table></div>` : `<div class="muted tiny">No event has been published yet.</div>`}
          ${byClass.length ? `<h3 class="mt">${icon("shield")} Overflow policy by class</h3>
            <div class="wrap"><table><thead><tr><th>Class</th><th class="num">Count</th></tr></thead>
              <tbody>${byClass.map((sample) => `<tr><td>${esc(sample.labels.class || Object.values(sample.labels)[0] || "—")}</td><td class="num">${esc(sample.value)}</td></tr>`).join("")}</tbody>
            </table></div>` : ""}
        </div>
        <div class="card">
          <h3>${icon("search")} Event history</h3>
          ${absent("There is no event history to search",
            "The reference design filters and paginates thousands of stored events, previews a clip per event, and exports the result. None of that exists.",
            ["ADR 002 defers event persistence and replay to explicit adapters; the bus is bounded and in-process.",
             "Detection produces a person class from <code>DeterministicFakeInference</code>. There is no vehicle, loitering or motion class, and no production model.",
             "Regions do not exist, so events cannot be attributed to one.",
             "Per-event clip extraction and bookmarks do not exist."])}
          <div class="tiny muted mt">Everything above this panel is real: those counters come from <code>/metrics</code> and describe the bus as it runs now.</div>
        </div>
      </div>`;
  };
}
