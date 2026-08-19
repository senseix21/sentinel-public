import { absent } from "../components/empty-state.js";
import { icon } from "../formatting.js";

export function screenHeatmap() {
  return `<div class="card">
      <h3>${icon("heat")} Heatmap Analytics</h3>
      ${absent("Heatmap analytics are not implemented",
        "The reference design shows spatial intensity over a camera view, hotspot ranking, activity by hour and dwell time. None of these quantities is computed anywhere in this system, and the meaning of the heatmap is an open product decision rather than a missing implementation.",
        ["<code>crates/analytics</code> is an empty stub. There is no occupancy, dwell-time or path derivation.",
         "Whether a heatmap means density, occupancy or movement is explicitly undecided; the engineering policy forbids implementing one before that is settled.",
         "There is no region model to attribute activity to, and no durable event log to aggregate over time.",
         "Detection has no production model, so any intensity drawn today would be an artefact of a fixture."])}
      <div class="tiny muted mt">
        This screen is kept in the navigation so the gap is visible rather than quietly missing.
      </div>
    </div>`;
}
