import { banner } from "../components/banner.js";
import { ago, bytes, esc, fmtPct, icon } from "../formatting.js";

export function createStorageScreen({
  fleetCounts, fleetProblem, isState, metricSum, pipelinePill, pressureName,
  recordingDot, settingValue, storageFacts,
}) {
  function screenStorage() {
    const counts = fleetCounts();
    const storage = storageFacts();
    const retention = settingValue("retention.max_age");
    const segment = settingValue("camera.recording.segment_duration");
    const root = settingValue("storage.recording_root");
    const watermarks = ["storage.watermark.warning", "storage.watermark.critical", "storage.watermark.emergency"]
      .map((key) => ({ key, value: settingValue(key) }));
  
    const used = storage.used || 0;
    const free = storage.free || 0;
    const total = storage.total || 0;
    const usedFraction = total ? used / total : 0;
  
    return `${fleetProblem()}
      <div class="grid cols-4">
        <div class="card" style="grid-column:span 2">
          <h3>${icon("db")} Volume Usage</h3>
          <p class="note">Measured on the recording volume by the server, not estimated.</p>
          ${total ? `<div class="donut">
              <svg viewBox="0 0 42 42">
                <circle cx="21" cy="21" r="15.9" fill="none" stroke="#1c2937" stroke-width="5"></circle>
                <circle cx="21" cy="21" r="15.9" fill="none" stroke="${usedFraction > 0.8 ? "#ef4444" : "#3b82f6"}" stroke-width="5"
                  stroke-dasharray="${(usedFraction * 100).toFixed(2)} ${(100 - usedFraction * 100).toFixed(2)}"
                  stroke-dashoffset="25" stroke-linecap="butt"></circle>
                <text class="mid" x="21" y="20.5" text-anchor="middle">${esc(bytes(used))}</text>
                <text class="mid2" x="21" y="26" text-anchor="middle">of ${esc(bytes(total))} &middot; ${esc(fmtPct(storage.usedPct, 0))}</text>
              </svg>
              <div class="legend">
                <div><i class="sw" style="background:${usedFraction > 0.8 ? "#ef4444" : "#3b82f6"}"></i><span class="nm">Used</span><b>${esc(bytes(used))}</b></div>
                <div><i class="sw" style="background:#1c2937"></i><span class="nm">Free</span><b>${esc(bytes(free))}</b></div>
                <div style="border-top:1px solid var(--line);margin-top:6px;padding-top:10px">
                  <span class="nm">Pressure</span><b class="${storage.pressure ? "warn" : "ok"}">${esc(pressureName(storage.pressure))}</b>
                </div>
              </div>
            </div>
            <div class="tiny muted mt">
              This is the whole volume. A breakdown by recordings, events, snapshots and system is not available:
              snapshots and stored events do not exist, and there is no aggregate over the recording index.
            </div>`
            : `<div class="muted">Storage capacity is not being reported by <code>/metrics</code>.</div>`}
        </div>
  
        <div class="card">
          <h3>${icon("shield")} Watermarks</h3>
          <p class="note">Retention reacts to these with hysteresis.</p>
          <div class="kv">
            ${watermarks.map((w) => `<div class="k">${esc(w.key.split(".").pop())}</div><div class="v">${esc(w.value === null ? "—" : w.value)}</div>`).join("")}
            <div class="k">Hysteresis</div><div class="v">${esc(settingValue("storage.watermark.hysteresis") || "—")}</div>
            <div class="k">Current</div><div class="v ${storage.pressure ? "warn" : "ok"}">${esc(pressureName(storage.pressure))}</div>
          </div>
        </div>
  
        <div class="card">
          <h3>${icon("clock")} Retention</h3>
          <div class="kv">
            <div class="k">Maximum age</div><div class="v">${esc(retention || "—")}</div>
            <div class="k">Protected recent</div><div class="v">${esc(settingValue("retention.protected_recent") || "—")}</div>
            <div class="k">Interval</div><div class="v">${esc(settingValue("retention.interval") || "—")}</div>
            <div class="k">Segment duration</div><div class="v">${esc(segment || "—")}</div>
            <div class="k">Passes run</div><div class="v">${esc(metricSum("surveillance_retention_passes_total") ?? "—")}</div>
            <div class="k">Files deleted</div><div class="v">${esc(metricSum("surveillance_retention_deleted_total") ?? "—")}</div>
            <div class="k">Bytes reclaimed</div><div class="v">${esc(bytes(metricSum("surveillance_retention_bytes_reclaimed_total")))}</div>
            <div class="k">Failures</div><div class="v ${metricSum("surveillance_retention_failures_total") ? "bad" : ""}">${esc(metricSum("surveillance_retention_failures_total") ?? "—")}</div>
          </div>
          <div class="tiny muted mt">Root: <code>${esc(root || "—")}</code></div>
        </div>
      </div>
  
      ${storage.usedPct !== null && storage.usedPct > 70
        ? banner("warn", `Volume usage has reached ${fmtPct(storage.usedPct, 0)}.`,
          `Retention deletes from indexed metadata as pressure rises. Current pressure state is <b>${esc(pressureName(storage.pressure))}</b>.`)
        : ""}
  
      <div class="card mt">
        <h3>${icon("cam")} Camera Recording Status <span class="right">${counts.readable ? `${counts.total} camera(s)` : "fleet not readable"}</span></h3>
        <div class="wrap"><table>
          <thead><tr>
            <th>Camera</th><th>State</th><th>Recording</th><th class="num">Segments</th>
            <th class="num">Failed</th><th class="num">Recorder queue</th><th>Last media</th><th>Health</th>
          </tr></thead>
          <tbody>${counts.cameras.map((camera) => {
            const depth = camera.recorder_queue_depth;
            const capacity = camera.recorder_queue_capacity;
            const healthy = isState(camera.pipeline_state, "running") && isState(camera.connection_state, "connected") && !camera.last_actionable_failure;
            return `<tr>
              <td><a href="#/cameras/${encodeURIComponent(camera.camera_id)}">${esc(camera.camera_id)}</a></td>
              <td>${pipelinePill(camera)}</td>
              <td>${recordingDot(camera)}</td>
              <td class="num">${esc(camera.recorder_completed_segments)}</td>
              <td class="num ${camera.recorder_failed_segments ? "bad" : ""}">${esc(camera.recorder_failed_segments)}</td>
              <td class="num">${depth === null || depth === undefined ? "—" : `${esc(depth)} / ${esc(capacity)}`}</td>
              <td class="num muted">${esc(ago(camera.last_media_unix_ms))}</td>
              <td class="${healthy ? "ok" : "warn"}">${healthy ? "healthy" : esc(camera.last_actionable_failure || "degraded")}</td>
            </tr>`;
          }).join("")}</tbody>
        </table></div>
        <div class="tiny muted mt">
          Average bitrate and per-camera stored bytes are not shown: there is no aggregate query over the recording
          index, and deriving them from a sampled page of segments would be an estimate presented as a measurement.
        </div>
      </div>`;
  }

  return screenStorage;
}
