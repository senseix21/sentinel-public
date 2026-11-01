import { banner } from "../components/banner.js";
import { absent } from "../components/empty-state.js";
import { bytes, esc, icon } from "../formatting.js";
import { contiguousRuns, coverageBuckets, playbackTime, PLAYBACK_TIMEZONES } from "../playback.js";

export function createPlaybackScreen({ state, fleetCounts, fleetProblem, errorBanner, pb }) {
  function screenPlayback() {
    const counts = fleetCounts();
    const cameras = [...new Set([...counts.cameras.map((camera) => camera.camera_id), ...(state.archiveCameras || [])])].sort();
    if (!pb.camera && cameras.length) pb.camera = cameras[0];
    if (!pb.date) pb.date = new Date().toISOString().slice(0, 10);
  
    return `${fleetProblem()}
      ${state.archiveError ? errorBanner(state.archiveError) : ""}
      <div class="card">
        <div class="filters">
          <div class="field">
            <label class="f">Camera</label>
            <select class="f" data-bind="camera">
              ${cameras.map((id) => `<option value="${esc(id)}" ${id === pb.camera ? "selected" : ""}>${esc(id)}</option>`).join("")}
            </select>
          </div>
          <div class="field">
            <label class="f">Timezone</label>
            <select class="f" data-bind="timezone">
              ${PLAYBACK_TIMEZONES.map((zone) => `<option value="${esc(zone.value)}" ${zone.value === (pb.timezone || "UTC") ? "selected" : ""}>${esc(zone.label)}</option>`).join("")}
            </select>
          </div>
          <div class="field"><label class="f">Date</label><input class="f" type="date" data-bind="date" value="${esc(pb.date)}"></div>
          <div class="field"><label class="f">From</label><input class="f" type="time" step="1" data-bind="from" value="${esc(pb.from)}"></div>
          <div class="field"><label class="f">To</label><input class="f" type="time" step="1" data-bind="to" value="${esc(pb.to)}"></div>
          <div class="row">
            <button class="btn" data-act="pb-clear">Clear</button>
            <button class="btn primary" data-act="pb-load" ${pb.busy ? "disabled" : ""}>${icon("refresh")} ${pb.busy ? "Loading…" : "Load"}</button>
          </div>
        </div>
      </div>
      ${pb.error ? errorBanner(pb.error) : ""}
      <div class="split-wide mt">
        <div class="stack">
          ${pb.result ? playerPanel(pb.result) : `<div class="card"><div class="muted">Choose a camera and a time range, then press Load.</div></div>`}
          ${pb.result ? segmentPanel(pb.result) : ""}
        </div>
        <div class="stack">
          ${playbackDetails()}
          ${`<div class="card"><h3>${icon("bell")} Event markers</h3>${absent(
            "No event markers on the timeline",
            "The reference design marks motion, person, vehicle and alarm events along the scrubber. There is no durable event log to read them from.",
            ["ADR 002 defers event persistence and replay to explicit adapters.",
             "Only a person class exists, produced by a fixture inference.",
             "Bookmarks and per-event clip extraction do not exist."])}</div>`}
        </div>
      </div>`;
  }
  
  function playbackDetails() {
    const capability = state.system && state.system.playback;
    return `<div class="card">
        <h3>${icon("clock")} Playback Details</h3>
        <div class="kv mt">
          <div class="k">Camera</div><div class="v">${esc(pb.camera || "—")}</div>
          <div class="k">Date</div><div class="v">${esc(pb.date || "—")}</div>
          <div class="k">From</div><div class="v">${esc(pb.from)}</div>
          <div class="k">To</div><div class="v">${esc(pb.to)}</div>
          <div class="k">Timezone</div><div class="v">${esc(PLAYBACK_TIMEZONES.find((zone) => zone.value === (pb.timezone || "UTC"))?.label || pb.timezone)}</div>
          <div class="k">Segments matched</div><div class="v">${pb.result ? pb.result.recordings.length : "—"}</div>
        </div>
        ${capability ? `<div class="tiny muted mt" style="border-top:1px solid var(--line);padding-top:11px">
          Accuracy <b>${esc(capability.accuracy)}</b>${capability.seekable ? "" : ", not seekable"}. The server answers
          <code>Accept-Ranges: none</code> and streams chunked, so seeking, duration and speed control are unavailable.
          Playback can still be paused, resumed or stopped.
        </div>` : ""}
      </div>`;
  }
  
  function playerPanel(result) {
    const runs = contiguousRuns(result.recordings);
    if (!runs.length) {
      return `<div class="card"><h3>${icon("live")} Player</h3>
        <div class="muted mt">No footage was recorded in that range.</div>
      </div>`;
    }
    if (runs.length > 1) {
      return `<div class="card"><h3>${icon("live")} Player</h3>
        ${banner("warn", "This range has gaps in the recorded footage.",
          `It contains ${runs.length} separate runs. They are not offered as one stream, because joined video would play continuously without being continuous. Play a run.`)}
        <div class="wrap"><table>
          <thead><tr><th>Run</th><th>From</th><th>To</th><th>Segments</th><th>Size</th><th></th></tr></thead>
          <tbody>${runs.map((run, index) => `<tr>
            <td>${index + 1}</td>
            <td class="num">${esc(playbackTime(Date.parse(run.started_at), pb.timezone))}</td>
            <td class="num">${esc(playbackTime(Date.parse(run.ended_at), pb.timezone))}</td>
            <td class="num">${run.count}</td>
            <td class="num">${esc(bytes(run.bytes))}</td>
            <td><button class="btn" data-act="play" data-start="${esc(run.started_at)}" data-end="${esc(run.ended_at)}">${icon("play")} Play</button></td>
          </tr>`).join("")}</tbody>
        </table></div>
        <video id="v" preload="none" playsinline controls class="mt" style="width:100%;border-radius:8px;background:#05080d"></video>
        <div class="row mt">
          <button class="btn" data-act="pb-toggle">Pause / Resume</button>
          <button class="btn" data-act="pb-stop">Stop</button>
        </div>
      </div>`;
    }
    const run = runs[0];
    const url = `/api/cameras/${encodeURIComponent(pb.camera)}/recordings/playback`;
    return `<div class="card"><h3>${icon("live")} Player
        <span class="right"><span class="pill arch">ARCHIVE</span></span>
      </h3>
      <p class="note">${esc(run.count)} contiguous segment(s), ${esc(bytes(run.bytes))}, from a single stream at <code>${esc(url)}</code>.</p>
      <video id="v" preload="none" playsinline controls style="width:100%;border-radius:8px;background:#05080d"></video>
      <div class="row mt">
        <button class="btn primary" data-act="play" data-start="${esc(run.started_at)}" data-end="${esc(run.ended_at)}">${icon("play")} Play this run</button>
        <button class="btn" data-act="pb-toggle">Pause / Resume</button>
        <button class="btn" data-act="pb-stop">Stop</button>
        <span class="muted tiny">${esc(playbackTime(Date.parse(run.started_at), pb.timezone))} &rarr; ${esc(playbackTime(Date.parse(run.ended_at), pb.timezone))}</span>
      </div>
    </div>`;
  }
  
  function segmentPanel(result) {
    const recordings = result.recordings;
    return `<div class="card">
        <h3>${icon("db")} Segments <span class="right">${recordings.length} matched</span></h3>
        <div class="filters mt">
          <div class="field">
            <label class="f">Recording video length</label>
            <select class="f" data-bind="clipDuration">
              <option value="full" ${pb.clipDuration === "full" ? "selected" : ""}>Full recording</option>
              <option value="5" ${pb.clipDuration === "5" ? "selected" : ""}>First 5 seconds</option>
              <option value="10" ${pb.clipDuration === "10" ? "selected" : ""}>First 10 seconds</option>
              <option value="custom" ${pb.clipDuration === "custom" ? "selected" : ""}>Custom</option>
            </select>
          </div>
          <div class="field">
            <label class="f">Custom seconds (1–60)</label>
            <input class="f" type="number" min="1" max="60" step="1" data-bind="customClipDuration" value="${esc(pb.customClipDuration || "15")}">
          </div>
          <div class="tiny muted">Duration is optional. A duration returns footage from the recording start; Full returns the complete recording.</div>
        </div>
        ${coverage(recordings)}
        <div class="wrap mt"><table>
          <thead><tr><th>Started</th><th>Ended</th><th>Codec</th><th>Resolution</th><th>Size</th><th>Status</th><th></th></tr></thead>
          <tbody>${recordings.map((recording) => `<tr>
            <td class="num">${esc(playbackTime(Date.parse(recording.started_at), pb.timezone))}</td>
            <td class="num">${esc(playbackTime(Date.parse(recording.ended_at), pb.timezone))}</td>
            <td>${esc(recording.codec || "—")}</td>
            <td class="num">${recording.width && recording.height ? `${esc(recording.width)}&times;${esc(recording.height)}` : "—"}</td>
            <td class="num">${esc(bytes(recording.size_bytes))}</td>
            <td>${esc(recording.status || "—")}</td>
            <td><button class="btn" data-act="play-recording" data-recording="${esc(recording.recording_id)}">${icon("play")} Play recording</button></td>
          </tr>`).join("")}</tbody>
        </table></div>
      </div>`;
  }
  
  function coverage(recordings) {
    const result = coverageBuckets(recordings);
    if (!result) return "";
    return `<div class="cover">${result.filled.map((has) => `<i class="${has ? "has" : ""}"></i>`).join("")}</div>
      <div class="ticks"><span>${esc(playbackTime(result.first, pb.timezone))}</span><span>${esc(playbackTime(result.last, pb.timezone))}</span></div>`;
  }
  
  // -------------------------------------------------------------------- events

  return screenPlayback;
}
