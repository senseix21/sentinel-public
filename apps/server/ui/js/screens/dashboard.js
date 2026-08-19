import { ago, bytes, esc, fmtPct, icon, pct } from "../formatting.js";

export function createDashboardScreen({
  cameraTile, fleetCounts, fleetProblem, kpi, metricSum, pressureName, sparkline,
  state, storageFacts, trend,
}) {
  function screenDashboard() {
    const counts = fleetCounts();
    const storage = storageFacts();
    const published = metricSum("surveillance_event_publications_total");
  
    const kpis = `<div class="grid cols-4">
        ${kpi({
          label: "Cameras Online", tone: !counts.readable ? "warn" : counts.connected === counts.total && counts.total ? "ok" : "warn",
          value: counts.readable ? `${counts.connected} <small>/ ${counts.total}</small>` : "&mdash;",
          foot: !counts.readable
            ? "the fleet could not be read"
            : counts.total ? `${fmtPct(pct(counts.connected, counts.total), 0)} connected` : "no cameras configured",
          footTone: counts.readable && counts.connected === counts.total && counts.total ? "ok" : "warn",
          icon: "cam", colour: "var(--green)", spark: sparkline(trend.cameras, "#22c55e"),
        })}
        ${kpi({
          label: "Recording", tone: counts.readable && counts.connected ? "rec" : "",
          value: counts.readable ? `${counts.connected} <small>/ ${counts.total}</small>` : "&mdash;",
          foot: !counts.readable
            ? "the fleet could not be read"
            : counts.total && counts.connected === counts.total ? "All cameras recording" : "Some cameras are not recording",
          icon: "db", colour: "var(--red)",
        })}
        ${kpi({
          label: "Volume Used",
          value: storage.used === null ? "—" : `${bytes(storage.used)} <small>/ ${bytes(storage.total)}</small>`,
          foot: storage.usedPct === null
            ? "storage not reported"
            : `<div class="bar" style="margin-top:6px"><i style="width:${Math.min(100, storage.usedPct).toFixed(1)}%"></i></div>${fmtPct(storage.usedPct)} used &middot; ${esc(pressureName(storage.pressure))}`,
          icon: "db", colour: "var(--blue)",
        })}
        ${kpi({
          label: "Events Published", tone: "warn",
          value: published === null ? "—" : String(published),
          foot: "since this server started &middot; not a daily total",
          icon: "bell", colour: "var(--amber)", spark: sparkline(trend.events, "#f59e0b"),
        })}
      </div>`;
  
    const tiles = counts.cameras.slice(0, 10);
  
    return `${fleetProblem()}
      ${kpis}
      <div class="split mt">
        <div class="card">
          <h3>${icon("live")} Live Camera Overview
            <span class="right"><a href="#/live">View all cameras</a></span>
          </h3>
          <p class="note">Tiles show real pipeline and publication state. Video starts only when you press watch.</p>
          ${tiles.length
            ? `<div class="tiles" style="grid-template-columns:repeat(auto-fill,minmax(230px,1fr))">
                ${tiles.map((camera) => cameraTile(camera)).join("")}
              </div>`
            : `<div class="muted tiny">${counts.readable
                ? "No cameras are configured."
                : "The fleet could not be read, so there is nothing to show here."}</div>`}
        </div>
        <div class="card">
          <h3>${icon("bell")} Recent Activity</h3>
          <p class="note">Derived from current per-camera state, not from an event log.</p>
          ${recentActivity(counts.cameras)}
        </div>
      </div>
      <div class="split mt">
        <div class="card">
          <h3>${icon("net")} System Health</h3>
          <p class="note">Queue depths and disk are measured. Host resource usage is not instrumented.</p>
          ${healthPanel(counts.cameras, storage)}
        </div>
        <div class="card">
          <h3>${icon("shield")} System Status</h3>
          ${servicePanel(counts)}
        </div>
      </div>`;
  }
  
  function recentActivity(cameras) {
    const rows = [];
    for (const camera of cameras) {
      if (camera.last_media_unix_ms) {
        rows.push({ at: camera.last_media_unix_ms, camera: camera.camera_id, what: "Media received", tone: "ok" });
      }
      if (camera.last_connected_unix_ms) {
        rows.push({ at: camera.last_connected_unix_ms, camera: camera.camera_id, what: "Connected", tone: "" });
      }
      if (camera.last_actionable_failure) {
        rows.push({ at: camera.last_media_unix_ms || 0, camera: camera.camera_id, what: camera.last_actionable_failure, tone: "bad" });
      }
    }
    rows.sort((a, b) => b.at - a.at);
    if (!rows.length) {
      return `<div class="muted tiny">${state.fleetError === null
        ? "Nothing has been observed yet."
        : "This is derived from per-camera state, which could not be read."}</div>`;
    }
    return `<div class="wrap"><table>
        <thead><tr><th>When</th><th>Camera</th><th>Fact</th></tr></thead>
        <tbody>${rows.slice(0, 9).map((row) => `<tr>
          <td class="num muted">${esc(ago(row.at))}</td>
          <td>${esc(row.camera)}</td>
          <td class="${row.tone}">${esc(row.what)}</td>
        </tr>`).join("")}</tbody>
      </table></div>
      <div class="tiny muted mt">There is no durable event log, so this cannot look further back than the running process.</div>`;
  }
  
  function healthPanel(cameras, storage) {
    const worstQueue = (depth, capacity) => {
      let value = null;
      for (const camera of cameras) {
        if (camera[depth] === null || camera[depth] === undefined || !camera[capacity]) continue;
        const ratio = (camera[depth] / camera[capacity]) * 100;
        if (value === null || ratio > value) value = ratio;
      }
      return value;
    };
    const rows = [
      { label: "Disk usage", value: storage.usedPct, detail: storage.used === null ? "not reported" : `${bytes(storage.used)} / ${bytes(storage.total)}` },
      { label: "Recorder queue (worst)", value: worstQueue("recorder_queue_depth", "recorder_queue_capacity"), detail: "reject-when-full; a rejection stops ingest" },
      { label: "Live queue (worst)", value: worstQueue("live_queue_depth", "live_queue_capacity"), detail: "drop-oldest; favours freshness" },
      { label: "Detection queue (worst)", value: worstQueue("detection_queue_depth", "detection_queue_capacity"), detail: "drop-oldest; favours freshness" },
    ];
    return `<div class="grid cols-2">${rows.map((row) => `
        <div>
          <div class="lbl row" style="justify-content:space-between;font-size:12px">
            <span class="muted">${esc(row.label)}</span>
            <b class="num">${row.value === null ? "—" : fmtPct(row.value, 0)}</b>
          </div>
          <div class="bar" style="margin:6px 0"><i style="width:${row.value === null ? 0 : Math.min(100, row.value).toFixed(1)}%"></i></div>
          <div class="tiny muted">${esc(row.detail)}</div>
        </div>`).join("")}</div>
      <div class="tiny muted mt">CPU, memory, network throughput and process uptime have no metric in this build.</div>`;
  }
  
  function servicePanel(counts) {
    const surfaces = state.system && state.system.surfaces;
    const services = [
      // "no camera connected" and "idle" are claims about the fleet. When the
      // fleet request failed we know nothing about either, so say that instead.
      { name: "Camera ingest", icon: "cam", ok: counts.connected > 0,
        text: !counts.readable ? "not readable" : counts.connected > 0 ? `${counts.connected} connected` : "no camera connected" },
      { name: "Recording", icon: "db", ok: counts.connected > 0,
        text: !counts.readable ? "not readable" : counts.connected > 0 ? "recording" : "idle" },
      { name: "Recording index", icon: "db", ok: !!(surfaces && surfaces.recording_index), text: surfaces ? (surfaces.recording_index ? "available" : "not configured") : "unknown" },
      { name: "Settings surface", icon: "gear", ok: !!(surfaces && surfaces.settings), text: surfaces ? (surfaces.settings ? "available" : "not configured") : "unknown" },
      { name: "Identity store", icon: "key", ok: !!(surfaces && surfaces.identities), text: surfaces ? (surfaces.identities ? "available" : "not configured") : "unknown" },
      { name: "Access audit", icon: "shield", ok: !!(surfaces && surfaces.access_audit), text: surfaces ? (surfaces.access_audit ? "available" : "not configured") : "unknown" },
    ];
    return `<div class="grid cols-2">${services.map((service) => `
        <div class="row" style="gap:11px;padding:9px 0">
          <span style="color:${service.ok ? "var(--green)" : "var(--dimmer)"}">${icon(service.icon, "lg")}</span>
          <span style="min-width:0">
            <div style="font-size:12.5px">${esc(service.name)}</div>
            <div class="tiny ${service.ok ? "ok" : "muted"}">${esc(service.text)}</div>
          </span>
        </div>`).join("")}</div>`;
  }
  
  // ------------------------------------------------------------------ live view

  return screenDashboard;
}
