"use strict";

import { ApiError, api, apiText } from "./api.js";
import { banner } from "./components/banner.js";
import { createCameraTile } from "./components/camera-tile.js";
import { absent } from "./components/empty-state.js";
import { ago, bytes, esc, fmtPct, icon, pct, utc } from "./formatting.js";
import { createMetricsView, parseMetrics } from "./metrics.js";
import { createLiveKitOwner } from "./livekit.js";
import { contiguousRuns, coverageBuckets, createPlaybackState, playbackPath, recordingSearchPath, searchRange } from "./playback.js";
import { SCREENS, parseRoute, screenFor } from "./router.js";
import { createDashboardScreen } from "./screens/dashboard.js";
import { createEventsScreen } from "./screens/events.js";
import { createCamerasScreen } from "./screens/cameras.js";
import { screenHeatmap } from "./screens/heatmap.js";
import { createLiveScreen } from "./screens/live.js";
import { createPlaybackScreen } from "./screens/playback.js";
import { createStorageScreen } from "./screens/storage.js";
import { createSettingsScreen } from "./screens/settings.js";
import { createSystemScreen } from "./screens/system.js";
import { createAppState, createTrendStore } from "./state.js";
import { createThemeController } from "./theme.js";

const state = createAppState();
const { trends: trend, push: pushTrend } = createTrendStore();
const metrics = createMetricsView(() => state.metrics);
const metricSamples = metrics.samples;
const metric = metrics.first;
const metricSum = metrics.sum;
const metricWhere = metrics.where;
const theme = createThemeController();
const liveKit = createLiveKitOwner({ ApiError, api, banner, errorBanner });
const { live, liveVideos, startLive, startVisibleLive, stopLive, stopAllLive, reattachLive } = liveKit;
const cameraTile = createCameraTile({ cameraLabel, pipelinePill, recordingDot, livekitChip });
const screenEvents = createEventsScreen({ metric, metricSamples, metricSum, kpi, sparkline, trend });
const liveScreen = createLiveScreen({ cameraTile, fleetCounts, fleetProblem, live });

function route() {
  return parseRoute(location.hash);
}

function currentScreen() {
  return screenFor(location.hash);
}

// ---------------------------------------------------------------- utilities

function errorBanner(error) {
  if (!(error instanceof ApiError)) {
    return banner("bad", "The server could not be reached.",
      "The page is loaded but no request is completing. The server may be stopped.");
  }
  if (error.status === 404) {
    return banner("warn", "Not found.", "The server has no such resource.");
  }
  if (error.status === 401) {
    return banner("bad", "Authentication required.",
      "Open SENTINEL through its authenticated gateway. This direct server endpoint does not carry a browser session.");
  }
  if (error.status === 403) {
    return banner("warn", "Permission denied.",
      "Your identity is valid but does not have permission for this operation.");
  }
  if (error.status === 503) {
    return banner("warn", "That part of the system is not running.",
      `The server answered <code>${esc(error.code)}</code>. Other screens may still work.`);
  }
  return banner("bad", `Request failed (${esc(error.status)}).`,
    `The server answered <code>${esc(error.code)}</code>.${error.detail ? ` ${esc(error.detail)}` : ""}`);
}

// ---------------------------------------------------------- metrics reading

// `/metrics` is the only source for facts the JSON API does not model — disk
// capacity, retention passes, event-bus counters. It is Prometheus text, and it
// is deliberately unauthenticated, so reading it here adds no API surface and
// no new authorisation decision.
// -------------------------------------------------------------------- state

// Detection ships with `DeterministicFakeInference` and no production model.
// Until `/api/system` says otherwise, every detection count on this page is a
// fixture artefact and is labelled as one. The expression is deliberately not
// simplified: with `/api/system` unloaded, unknown must mean fixture.
function detectionIsFixture() {
  return !state.system || state.system.detection.results_are_fixture;
}

// --------------------------------------------------------------- navigation

function renderNav() {
  const active = currentScreen().id;
  document.getElementById("nav").innerHTML = SCREENS.map((s) =>
    `<a href="#${s.path}" class="${s.id === active ? "on" : ""}">${icon(s.icon)}<span>${esc(s.label)}</span></a>`
  ).join("");
  const screen = currentScreen();
  document.getElementById("page-title").textContent = screen.title;
  document.getElementById("page-sub").textContent = screen.sub;
  document.getElementById("page-icon").innerHTML = `<use href="#i-${screen.icon}"></use>`;
}

// ------------------------------------------------------------- shared parts

function cameraLabel(camera) {
  // The model carries an identifier and nothing else: there is no display name
  // and no site. Inventing "Front Entrance" here would be inventing an
  // installation. The identifier is what the system actually knows.
  return esc(camera.camera_id);
}

function pipelinePill(camera) {
  const running = isState(camera.pipeline_state, "running");
  const connected = isState(camera.connection_state, "connected");
  if (running && connected) return `<span class="pill live"><i class="dot ok"></i>LIVE</span>`;
  if (running) return `<span class="pill warn"><i class="dot warn"></i>${esc(camera.connection_state || camera.pipeline_state)}</span>`;
  return `<span class="pill off"><i class="dot bad"></i>${esc(camera.pipeline_state)}</span>`;
}

function recordingDot(camera) {
  const on = isState(camera.pipeline_state, "running") && isState(camera.connection_state, "connected");
  return `<span class="row" style="gap:6px"><i class="dot ${on ? "rec" : ""}"></i><span>${on ? "REC" : "idle"}</span></span>`;
}

// The API renders pipeline and connection state in lower case and publication
// and detection state capitalised. Comparisons here fold the case rather than
// pick one spelling and quietly report the wrong colour when the other arrives.
function isState(value, name) {
  return String(value === null || value === undefined ? "" : value).toLowerCase() === name;
}

function livekitChip(camera) {
  if (!camera.live_publication_enabled) {
    return `<span class="row tiny shrink" title="LiveKit off" style="gap:6px"><i class="dot"></i>
      <span class="muted lk-word">LiveKit off</span></span>`;
  }
  const publishing = isState(camera.live_publication_state, "publishing")
    || isState(camera.live_publication_state, "connected");
  // At the densest grid this footer is ~216px wide and the three controls come
  // first, so the words here are what gives up room. The dot never shrinks and
  // the title carries the full state, so nothing is lost to a narrow tile.
  const label = `LiveKit ${camera.live_publication_state || ""}`.trim();
  return `<span class="row tiny shrink" title="${esc(label)}" style="gap:6px"><span class="lk-word">LiveKit</span>
      <i class="dot ${publishing ? "ok" : "warn"}"></i>
      <span class="muted lk-word">${esc(camera.live_publication_state || "")}</span>
    </span>`;
}

function sparkline(series, colour) {
  if (!series || series.length < 2) return "";
  const min = Math.min(...series);
  const max = Math.max(...series);
  const span = max - min || 1;
  const points = series.map((value, index) => {
    const x = (index / (series.length - 1)) * 100;
    const y = 26 - ((value - min) / span) * 22;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  }).join(" ");
  return `<svg class="spark" viewBox="0 0 100 30" preserveAspectRatio="none">
      <polyline points="${points}" fill="none" stroke="${colour}" stroke-width="1.6" vector-effect="non-scaling-stroke"/>
    </svg>`;
}

function kpi(options) {
  return `<div class="card kpi">
      <div class="k">
        <div class="head"><i class="dot ${options.tone || ""}"></i>${esc(options.label)}</div>
        <div class="big">${options.value}</div>
        <div class="foot ${options.footTone || "muted"}">${options.foot || "&nbsp;"}</div>
        ${options.spark || ""}
      </div>
      <div class="orb" style="color:${options.colour}">${icon(options.icon, "lg")}</div>
    </div>`;
}

function fleetCounts() {
  const cameras = Array.isArray(state.cameras) ? state.cameras : [];
  const running = cameras.filter((c) => isState(c.pipeline_state, "running"));
  const connected = running.filter((c) => isState(c.connection_state, "connected"));
  const publishing = cameras.filter((c) => c.live_publication_enabled
    && (isState(c.live_publication_state, "publishing") || isState(c.live_publication_state, "connected")));
  // `readable` separates "there are no cameras" from "we were not allowed to
  // ask, or the request failed". Reporting 0/0 for the second case states a
  // fleet fact this page does not have — the 401 answer carries no camera list.
  return {
    cameras, readable: state.fleetError === null,
    total: cameras.length, running: running.length,
    connected: connected.length, publishing: publishing.length,
  };
}

function storageFacts() {
  // A zero-byte volume is not a measurement, it is the absence of one: the
  // watermark observer publishes these only once it has stat'd the recording
  // root. Reporting "0 B / 0 B" would read as an empty disk.
  const totalRaw = metric("surveillance_storage_total_bytes");
  const total = totalRaw ? totalRaw : null;
  const free = total === null ? null : metric("surveillance_storage_free_bytes");
  const used = total !== null && free !== null ? total - free : null;
  return { total, free, used, usedPct: pct(used, total), pressure: metric("surveillance_storage_pressure") };
}

const PRESSURE = ["nominal", "warning", "critical", "emergency"];

function pressureName(value) {
  return value === null || value === undefined ? "unknown" : (PRESSURE[value] || `level ${value}`);
}

// --------------------------------------------------------- sidebar gauges

function renderGauges() {
  const counts = fleetCounts();
  const storage = storageFacts();
  const cameras = counts.cameras;

  const worst = (depth, capacity) => {
    let best = null;
    for (const camera of cameras) {
      const d = camera[depth];
      const c = camera[capacity];
      if (d === null || d === undefined || !c) continue;
      const ratio = (d / c) * 100;
      if (best === null || ratio > best) best = ratio;
    }
    return best;
  };

  const rows = [
    { label: "Volume used", value: storage.usedPct, colour: storage.usedPct !== null && storage.usedPct > 80 ? "var(--red)" : "var(--blue)" },
    { label: "Recorder backlog", value: worst("recorder_queue_depth", "recorder_queue_capacity"), colour: "var(--purple)" },
    { label: "Live queue", value: worst("live_queue_depth", "live_queue_capacity"), colour: "var(--green)" },
    { label: "Detection queue", value: worst("detection_queue_depth", "detection_queue_capacity"), colour: "var(--amber)" },
  ];

  document.getElementById("gauges").innerHTML = `
    <h4>${icon("net")} System Health</h4>
    ${rows.map((row) => `
      <div class="gauge ${row.value === null ? "na" : ""}">
        <div class="lbl"><span>${esc(row.label)}</span><b>${row.value === null ? "not reported" : fmtPct(row.value, 0)}</b></div>
        <div class="bar"><i style="width:${row.value === null ? 0 : Math.min(100, row.value).toFixed(1)}%;background:${row.colour}"></i></div>
      </div>`).join("")}
    <div class="tiny" style="color:var(--dimmer);margin-top:12px;border-top:1px solid var(--line);padding-top:10px">
      Host CPU, memory, network and uptime are not instrumented by this build.
    </div>`;
}

// ------------------------------------------------------------------ screens

function fleetProblem() {
  if (!state.fleetError) return "";
  return errorBanner(state.fleetError);
}

const screenDashboard = createDashboardScreen({
  cameraTile, fleetCounts, fleetProblem, kpi, metricSum, pressureName, sparkline,
  state, storageFacts, trend,
});

// ------------------------------------------------------------------ playback

const pb = createPlaybackState();
const screenPlayback = createPlaybackScreen({ state, fleetCounts, fleetProblem, errorBanner, pb });

// What the operator has typed into the add-camera form.
//
// It lives out here because the fleet screen re-renders on every poll, and a
// form whose only copy of its contents is the DOM loses them wholesale every
// five seconds — a camera identifier and an RTSP URL cannot be typed faster
// than that. `pb` is the same idea for the playback filters; this is that
// pattern applied to the one form that did not have it.
const add = { id: "", rtsp: "", width: "1280", height: "720", live: "default" };

// ------------------------------------------------------------------- storage

// ------------------------------------------------------------------- cameras

const camerasScreen = createCamerasScreen({
  add, api, cameraTile, detectionIsFixture, errorBanner, fleetCounts, fleetProblem,
  livekitChip, pipelinePill, route, render,
});
const screenCameras = camerasScreen.render;
const { detail } = camerasScreen;

// ------------------------------------------------------------------ settings

const settingsScreen = createSettingsScreen({ state, errorBanner });
const { edits, settingValue, saveBar } = settingsScreen;
const screenSettings = settingsScreen.render;
const screenStorage = createStorageScreen({
  fleetCounts, fleetProblem, isState, metricSum, pipelinePill, pressureName,
  recordingDot, settingValue, storageFacts,
});

// -------------------------------------------------------------------- system

const screenSystem = createSystemScreen({ state, metric, metricSum, detectionIsFixture, errorBanner });

// -------------------------------------------------------------------- render

const SCREEN_RENDERERS = {
  dashboard: screenDashboard,
  live: liveScreen.render,
  playback: screenPlayback,
  events: screenEvents,
  heatmap: screenHeatmap,
  storage: screenStorage,
  cameras: screenCameras,
  settings: screenSettings,
  system: screenSystem,
};

async function render() {
  renderNav();
  const { name, args } = route();
  const renderer = SCREEN_RENDERERS[name] || screenDashboard;
  let html;
  try {
    html = renderer(args);
  } catch (error) {
    html = banner("bad", "This screen failed to render.", esc(String(error && error.message)));
  }
  // Lift the playing `<video>` elements out before the swap and put the same
  // nodes back after it.
  //
  // Replacing `#main` destroys them, and `reattachLive` then re-attaches each
  // track to a brand-new element. A new element has a cold decoder: it reports
  // `readyState 0` and `videoWidth 0` until the pipeline warms again, so every
  // tile went black for part of every poll — five seconds apart, on a wall of
  // live video. Carrying the node across keeps one decode pipeline per tile for
  // the lifetime of the subscription, which is what a subscription already is.
  const main = document.getElementById("main");
  const carried = new Map();
  for (const cameraId of live.rooms.keys()) {
    const video = liveVideos(cameraId)[0];
    if (video && video.dataset.liveAttached === "yes") carried.set(cameraId, video);
  }
  main.innerHTML = html;
  for (const [cameraId, video] of carried) {
    const fresh = liveVideos(cameraId)[0];
    // The tile may be gone from this render — a removed camera, or a screen
    // that does not show it. The session is unaffected; the node is not needed.
    if (!fresh || !fresh.parentNode) continue;
    // Presentation follows the new render, decode state follows the old node.
    // Not `hidden`, though: a tile renders hidden and is revealed on attach,
    // and `reattachLive` will skip this node precisely because it is already
    // attached — so copying the fresh element's `hidden` would black out the
    // one tile that is genuinely playing. Same for the idle overlay, which the
    // new markup initially renders visible.
    video.className = fresh.className;
    video.hidden = false;
    fresh.parentNode.replaceChild(video, fresh);
    liveIdle(cameraId, false);
  }
  renderGauges();
  reattachLive();
  if (currentScreen().id === "live") await startVisibleLive();
}

// ---------------------------------------------------------------- data loads

async function loadSystem() {
  try {
    state.system = await api("/api/system");
    state.systemError = null;
  } catch (error) {
    state.system = null;
    // Remember *why* there is no capability report. Without this the System
    // screen sits on "Loading capabilities…" forever, which claims a request is
    // still in flight when it has already come back refused.
    state.systemError = error;
  }
  // No endpoint reports who the caller is. The reference design puts a named
  // account with a role in this corner; naming one here would be inventing an
  // identity the server never asserted. Requests are authorised at a chokepoint
  // from an identity established at the trust boundary in front of the process,
  // and that is exactly as much as this page knows.
  document.getElementById("who-name").textContent = "Operator";
  document.getElementById("who-role").textContent = state.system
    ? "identity established at the trust boundary"
    : "server not reporting";
}

async function loadSettings() {
  try {
    state.settings = await api("/api/settings");
    state.settingsError = null;
  } catch (error) {
    state.settings = null;
    state.settingsError = error;
  }
}

async function poll() {
  const [fleet, metrics] = await Promise.allSettled([api("/api/cameras"), apiText("/metrics")]);
  if (fleet.status === "fulfilled") {
    state.cameras = fleet.value;
    state.fleetError = null;
  } else {
    state.fleetError = fleet.reason;
  }
  if (metrics.status === "fulfilled") state.metrics = parseMetrics(metrics.value);
  state.loadedAt = Date.now();

  const counts = fleetCounts();
  // Do not push a sample the fleet request never gave us. A failed read is a
  // gap in the series, not an observation of zero connected cameras.
  if (counts.readable) pushTrend("cameras", counts.connected);
  pushTrend("events", metricSum("surveillance_event_publications_total"));
  pushTrend("storage", storageFacts().usedPct);

  document.getElementById("foot-right").textContent = `updated ${new Date().toISOString().slice(11, 19)}Z`;
}

// A re-render replaces the whole view, so a screen that polls must not fight
// with a screen the operator is interacting with. Only the screens whose whole
// content is fleet state redraw on a tick.
const LIVE_SCREENS = ["dashboard", "live", "storage", "cameras", "events", "system"];

async function tick() {
  await poll();
  const { name, args } = route();
  if (LIVE_SCREENS.includes(name) && !(name === "cameras" && args.length)) await render();
  else renderGauges();
}

// -------------------------------------------------------------------- events

document.addEventListener("click", async (event) => {
  const target = event.target.closest("[data-act]");
  if (!target) return;
  const act = target.dataset.act;
  const cameraId = target.dataset.camera;

  if (act === "collapse") {
    document.body.classList.toggle("narrow");
    return;
  }

  if (act === "refresh") {
    await tick();
    return;
  }

  if (act === "theme") {
    theme.toggle();
    return;
  }

  if (act === "logout") {
    const response = await fetch("/logout", { method: "POST" });
    if (response.redirected) window.location.assign(response.url);
    else window.location.assign("/login");
    return;
  }

  if (act === "grid") {
    liveScreen.setGridColumns(Number(target.dataset.cols));
    await render();
    return;
  }

  if (act === "full") {
    const video = liveVideos(cameraId)[0];
    const stage = video && video.closest(".stage");
    if (stage && stage.requestFullscreen) stage.requestFullscreen();
    return;
  }

  // Manual controls share the same owner as automatic live-screen startup.
  if (act === "watch-live" || act === "watch-all") {
    const targets = act === "watch-all"
      ? [...document.querySelectorAll("[data-live-tile][data-live-watchable]")]
      : liveVideos(cameraId);
    for (const video of targets) {
      await startLive(video.dataset.liveTile, video);
    }
    return;
  }

  if (act === "stop-live") { await stopLive(cameraId); return; }
  if (act === "stop-all") { await stopAllLive(); return; }

  if (act === "play") {
    const video = document.getElementById("v");
    if (!video) return;
    const url = playbackPath(pb.camera, target.dataset.start, target.dataset.end);
    // `setAttribute` rather than `.src`, so that the only assignment to a `src`
    // property in this file remains the one that loads the vendored SDK from
    // this server. That single site is what the no-external-request test pins.
    video.setAttribute("src", url);
    video.play().catch(() => { /* the element reports its own failure */ });
    return;
  }

  if (act === "pb-load") { await loadPlayback(); return; }
  if (act === "pb-clear") {
    pb.result = null;
    pb.error = null;
    await render();
    return;
  }

  if (act === "camera-start" || act === "camera-stop") {
    const verb = act === "camera-start" ? "start" : "stop";
    await cameraOperation(`/api/cameras/${encodeURIComponent(cameraId)}/${verb}`, "POST", `Camera ${verb} requested.`);
    return;
  }

  if (act === "camera-remove") {
    await cameraOperation(`/api/cameras/${encodeURIComponent(cameraId)}`, "DELETE", "Camera removed.");
    return;
  }

  if (act === "camera-add") {
    const out = document.getElementById("add-camera-out");
    if (!out) return;
    // Read from `add`, not from the DOM. The fleet screen re-renders on every
    // poll, so the inputs the operator typed into may already have been
    // replaced by the time this handler runs.
    const id = add.id.trim();
    const rtsp = add.rtsp.trim();
    const width = Number.parseInt(add.width, 10);
    const height = Number.parseInt(add.height, 10);
    // Checked here so the operator gets the missing field named, rather than
    // the server's single `invalid_camera_configuration` for all four. The
    // server still validates every one of them; this does not replace that.
    if (!id) { out.innerHTML = banner("warn", "Enter a camera identifier.", ""); return; }
    if (!rtsp) { out.innerHTML = banner("warn", "Enter an RTSP endpoint.", "A camera with no source cannot be started."); return; }
    if (!Number.isInteger(width) || width < 1 || !Number.isInteger(height) || height < 1) {
      out.innerHTML = banner("warn", "Enter the stream width and height.", "Both must be positive whole numbers.");
      return;
    }
    // No credential fields, deliberately. ADR 009 Decision 6 keeps this page
    // free of anything worth stealing from it, and an RTSP password sits in the
    // DOM while it is being typed. A camera that needs one is added through the
    // API, which takes both.
    const request = { camera_id: id, rtsp_endpoint: rtsp, width, height };
    // Absent means "follow the deployment". Sending `false` in that case is what
    // the page used to do implicitly, and it is why every camera added here was
    // created unable to publish.
    // Not named `live`: that identifier is the module-scope session map this
    // page hangs every LiveKit room off, and shadowing it here would be a trap
    // for the next person who reaches for `live.rooms` inside this block.
    const publication = add.live;
    if (publication === "on") request.live_publication_enabled = true;
    if (publication === "off") request.live_publication_enabled = false;
    try {
      await api("/api/cameras", { method: "POST", body: JSON.stringify(request) });
      // Cleared only after the server accepted it. A rejected add leaves what
      // was typed on screen to be corrected, rather than making the operator
      // retype a URL to fix one character of it.
      add.id = "";
      add.rtsp = "";
      const message = banner("info", `Camera ${esc(id)} added, stopped.`, publication === "off"
        ? "It will not publish to LiveKit. Press Start on its row to connect it."
        : "Press Start on its row to connect it.");
      await tick();
      const settled = document.getElementById("add-camera-out");
      if (settled) settled.innerHTML = message;
      else out.innerHTML = message;
    } catch (error) {
      out.innerHTML = errorBanner(error);
    }
    return;
  }

  if (act === "setting-revert") {
    edits.delete(target.dataset.key);
    await render();
    return;
  }

  if (act === "settings-discard") {
    edits.clear();
    await render();
    return;
  }

  if (act === "settings-reload") {
    await loadSettings();
    await render();
    return;
  }

  if (act === "settings-save") { await saveSettings(); return; }
});

document.addEventListener("input", (event) => {
  const node = event.target;
  if (node.dataset && node.dataset.bind) {
    pb[node.dataset.bind] = node.value;
    return;
  }
  if (node.dataset && node.dataset.add) {
    add[node.dataset.add] = node.value;
    return;
  }
  if (node.dataset && node.dataset.setting) {
    const key = node.dataset.setting;
    const original = state.settings && state.settings.settings.find((entry) => entry.key === key);
    if (original && original.value === node.value) edits.delete(key);
    else edits.set(key, node.value);
    const bar = document.querySelector(".savebar");
    if (bar) bar.outerHTML = saveBar();
    const row = node.closest(".srow");
    if (row) row.classList.toggle("dirty", edits.has(key));
  }
});

async function cameraOperation(path, method, message) {
  const out = document.getElementById("lifecycle-out");
  try {
    await api(path, { method });
    if (out) out.innerHTML = banner("info", message, "");
    detail.id = null;
    await tick();
    if (route().name === "cameras" && route().args.length) await render();
  } catch (error) {
    if (out) out.innerHTML = errorBanner(error);
    else await render();
  }
}

async function loadPlayback() {
  if (!pb.camera) { pb.error = new ApiError(0, "no_camera_selected"); await render(); return; }
  pb.busy = true;
  pb.error = null;
  await render();
  try {
    // `start`/`end`, not `from`/`to`. The route reads those two names and
    // answers `missing_recording_range` to anything else, so the wrong pair
    // does not degrade — every search fails, whatever the range.
    pb.result = await api(recordingSearchPath(pb.camera, searchRange(pb)));
  } catch (error) {
    pb.result = null;
    pb.error = error;
  }
  pb.busy = false;
  await render();
}

async function saveSettings() {
  const out = document.getElementById("settings-out");
  if (!edits.size) return;
  const changes = {};
  for (const [key, value] of edits) changes[key] = value;
  try {
    const report = await api("/api/settings", { method: "PUT", body: JSON.stringify({ changes }) });
    edits.clear();
    await loadSettings();
    await render();
    const applied = report && report.applied ? report.applied : Object.keys(changes).length;
    const later = report && report.deferred ? report.deferred : null;
    document.getElementById("settings-out").innerHTML = banner("info", "Configuration applied.",
      `${esc(applied)} change(s) committed in one transaction with their audit rows.${later ? ` Deferred until restart: ${esc(later)}.` : ""}`);
  } catch (error) {
    if (out) out.innerHTML = errorBanner(error);
  }
}

// A navigation replaces the whole view, so every element a live track is
// attached to stops existing. Every room is disconnected first: without this
// the peer connections and subscriptions survive the navigation and the server
// keeps counting viewers who left.
window.addEventListener("hashchange", async () => {
  await stopAllLive();
  await render();
});

function startClock() {
  const paint = () => {
    const now = new Date();
    document.getElementById("clock-time").textContent = now.toISOString().slice(11, 19);
    document.getElementById("clock-date").textContent = now.toISOString().slice(0, 10);
  };
  paint();
  setInterval(paint, 1000);
}

(async function boot() {
  theme.paint();
  startClock();
  renderNav();
  await Promise.all([loadSystem(), loadSettings()]);
  await poll();
  await render();
  setInterval(tick, 5000);
})();
