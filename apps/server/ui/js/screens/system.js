import { banner } from "../components/banner.js";
import { esc, icon } from "../formatting.js";

export function createSystemScreen({ state, metric, metricSum, detectionIsFixture, errorBanner }) {
  function screenSystem() {
    const system = state.system;
    if (!system) {
      return state.systemError
        ? `${errorBanner(state.systemError)}<div class="card"><div class="muted">The capability report could not be read, so this screen has nothing to describe.</div></div>`
        : `<div class="card"><div class="muted">Loading capabilities…</div></div>`;
    }
    const surfaces = system.surfaces || {};
    const config = {
      generation: metric("surveillance_configuration_generation"),
      origin: metric("surveillance_configuration_origin"),
      stale: metric("surveillance_configuration_freshness"),
      store: metric("surveillance_configuration_store_available"),
      mutating: metric("surveillance_configuration_mutating_surface_enabled"),
    };
  
    return `<div class="grid cols-2">
        <div class="card">
          <h3>${icon("shield")} Detection</h3>
          ${detectionIsFixture()
            ? banner("warn", "Detection runs a fixture, not a model.",
              "The pipeline is wired end to end and its behaviour under load is real. Its output is not, and no accuracy claim is supported by this build.")
            : banner("info", "Detection reports a production model.", "")}
          <div class="kv mt">
            <div class="k">Model</div><div class="v">${esc(system.detection.model_name || "—")}</div>
            <div class="k">Version</div><div class="v">${esc(system.detection.model_version || "—")}</div>
            <div class="k">Production model</div><div class="v">${system.detection.is_production_model ? "yes" : "no"}</div>
            <div class="k">Results are fixture</div><div class="v">${system.detection.results_are_fixture ? "yes" : "no"}</div>
          </div>
        </div>
        <div class="card">
          <h3>${icon("clock")} Playback</h3>
          <div class="kv mt">
            <div class="k">Accuracy</div><div class="v">${esc(system.playback.accuracy)}</div>
            <div class="k">Seekable</div><div class="v">${system.playback.seekable ? "yes" : "no"}</div>
            <div class="k">Transport</div><div class="v">${esc(system.playback.transport || "—")}</div>
          </div>
          <div class="tiny muted mt">A player cannot offer a scrubber or a duration over a stream that answers neither.</div>
        </div>
        <div class="card">
          <h3>${icon("net")} Surfaces</h3>
          <div class="kv mt">
            ${Object.keys(surfaces).map((key) => `
              <div class="k">${esc(key.replace(/_/g, " "))}</div>
              <div class="v ${surfaces[key] ? "ok" : "muted"}">${surfaces[key] ? "available" : "not configured"}</div>`).join("")}
          </div>
        </div>
        <div class="card">
          <h3>${icon("gear")} Configuration plane</h3>
          <div class="kv mt">
            <div class="k">Generation</div><div class="v">${esc(config.generation ?? "—")}</div>
            <div class="k">Store reachable</div><div class="v ${config.store ? "ok" : "warn"}">${config.store === null ? "—" : (config.store ? "yes" : "no")}</div>
            <div class="k">Snapshot</div><div class="v ${config.stale ? "warn" : "ok"}">${config.stale === null ? "—" : (config.stale ? "stale" : "fresh")}</div>
            <div class="k">Mutating surface</div><div class="v">${config.mutating === null ? "—" : (config.mutating ? "enabled" : "disabled")}</div>
            <div class="k">Audit writes</div><div class="v">${esc(metricSum("surveillance_configuration_audit_write_total") ?? "—")}</div>
            <div class="k">Audit write failures</div><div class="v ${metricSum("surveillance_configuration_audit_write_failed_total") ? "bad" : ""}">${esc(metricSum("surveillance_configuration_audit_write_failed_total") ?? "—")}</div>
          </div>
        </div>
        <div class="card">
          <h3>${icon("key")} Authentication</h3>
          <div class="kv mt">
            <div class="k">Successful authentications</div><div class="v">${esc(metricSum("surveillance_auth_success_total") ?? "—")}</div>
            <div class="k">Failures</div><div class="v ${metricSum("surveillance_auth_failure_total") ? "warn" : ""}">${esc(metricSum("surveillance_auth_failure_total") ?? "—")}</div>
            <div class="k">Identity store unavailable</div><div class="v">${esc(metricSum("surveillance_auth_unavailable_total") ?? "—")}</div>
            <div class="k">Proxy-asserted origins</div><div class="v">${esc(metricSum("surveillance_auth_proxy_asserted_total") ?? "—")}</div>
            <div class="k">Authorisation decisions</div><div class="v">${esc(metricSum("surveillance_authz_decision_total") ?? "—")}</div>
          </div>
          <div class="tiny muted mt">
            This page sends no credential. Identity is established at the trust boundary in front of the server.
          </div>
        </div>
        <div class="card">
          <h3>${icon("db")} Persistence</h3>
          <div class="kv mt">
            <div class="k">Pool size</div><div class="v">${esc(metric("surveillance_database_pool_size") ?? "—")}</div>
            <div class="k">Pool idle</div><div class="v">${esc(metric("surveillance_database_pool_idle") ?? "—")}</div>
            <div class="k">Metadata persisted</div><div class="v">${esc(metricSum("surveillance_metadata_persisted_total") ?? "—")}</div>
            <div class="k">Metadata rejected (full)</div><div class="v ${metricSum("surveillance_metadata_rejected_full_total") ? "warn" : ""}">${esc(metricSum("surveillance_metadata_rejected_full_total") ?? "—")}</div>
            <div class="k">Reconciliation imports</div><div class="v">${esc(metricSum("surveillance_reconciliation_imported_total") ?? "—")}</div>
            <div class="k">Playbacks started</div><div class="v">${esc(metricSum("surveillance_playback_started_total") ?? "—")}</div>
          </div>
        </div>
      </div>
      <div class="card mt">
        <h3>${icon("net")} Raw instrumentation</h3>
        <p class="note">Everything on this screen is read from the Prometheus endpoint this server already exposes.</p>
        <div class="row"><a class="btn" href="/metrics" target="_blank" rel="noreferrer">${icon("expand")} Open /metrics</a>
        <a class="btn" href="/ready" target="_blank" rel="noreferrer">${icon("check")} Open /ready</a></div>
      </div>`;
  }

  return screenSystem;
}
