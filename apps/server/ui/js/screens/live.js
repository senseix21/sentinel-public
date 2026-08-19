import { banner } from "../components/banner.js";
import { absent } from "../components/empty-state.js";
import { esc, icon } from "../formatting.js";

export function createLiveScreen({ cameraTile, fleetCounts, fleetProblem, live }) {
  let gridColumns = 5;

  function setGridColumns(value) {
    gridColumns = [2, 3, 4, 5].includes(value) ? value : 5;
  }

  function render() {
    const counts = fleetCounts();
    const publishing = counts.cameras.filter((camera) => camera.live_publication_enabled);
    const ratio = (part, whole) => (counts.readable ? `${part} / ${whole}` : "&mdash;");

    return `${fleetProblem()}
      <div class="row" style="justify-content:space-between;margin-bottom:14px">
        <div class="seg">
          ${[2, 3, 4, 5].map((columns) => `<button data-act="grid" data-cols="${columns}" class="${gridColumns === columns ? "on" : ""}">${columns}&times;</button>`).join("")}
        </div>
        <div class="row">
          <span class="muted tiny">${counts.readable ? `${counts.connected} / ${counts.total} cameras connected` : "fleet not readable"} &middot; ${live.rooms.size} live subscription(s)</span>
          <button class="btn primary" data-act="watch-all">${icon("play")} Watch all</button>
          <button class="btn" data-act="stop-all">${icon("stop")} Stop all</button>
        </div>
      </div>
      ${publishing.length < counts.total
        ? banner("info", `${counts.total - publishing.length} of ${counts.total} cameras do not publish to LiveKit.`,
          "Only a camera configured with LiveKit credentials at start-up publishes. A camera added through the API carries no LiveKit configuration, so it records but cannot be watched here.")
        : ""}
      ${counts.total
        ? `<div class="tiles" data-density="${gridColumns}" style="grid-template-columns:repeat(${gridColumns},minmax(0,1fr))">
            ${counts.cameras.map((camera, index) => cameraTile(camera, { primary: index === 0 })).join("")}
          </div>`
        : `<div class="card"><div class="muted">${counts.readable ? "No cameras are configured." : "The fleet could not be read, so there are no tiles to show."}</div></div>`}
      <div class="strip mt">
        <div><span style="color:var(--green)">${icon("net", "lg")}</span><span><div class="v">${!counts.readable ? "Unknown" : counts.connected === counts.total && counts.total ? "All Systems Live" : "Degraded"}</div><div class="l">${counts.readable ? `${counts.connected} of ${counts.total} streaming` : "the fleet could not be read"}</div></span></div>
        <div>${icon("cam", "lg")}<span><div class="v">${ratio(counts.connected, counts.total)}</div><div class="l">Cameras online</div></span></div>
        <div><span style="color:var(--red)">${icon("db", "lg")}</span><span><div class="v">${ratio(counts.connected, counts.total)}</div><div class="l">Recording</div></span></div>
        <div><span style="color:var(--blue)">${icon("live", "lg")}</span><span><div class="v">${ratio(counts.publishing, publishing.length)}</div><div class="l">Publishing</div></span></div>
        <div><span style="color:var(--cyan)">${icon("net", "lg")}</span><span><div class="v">${live.rooms.size}</div><div class="l">This browser subscribed</div></span></div>
      </div>
      <div class="card mt">
        ${absent("Region-of-interest overlays are not implemented",
          "The reference design draws detection zones over each tile. No region model exists: there is no ROI geometry, no coordinate system decision, and no rules crate.",
          ["<code>crates/rules</code> and <code>crates/analytics</code> are empty stubs.",
           "The ROI coordinate system and validation rules are an open product decision.",
           "Detection currently reports a person class from a fixture, with no spatial semantics attached."])}
      </div>`;
  }

  return { render, setGridColumns };
}
