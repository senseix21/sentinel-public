"use strict";

import { ApiError, api, apiText } from "./api.js";
import { banner } from "./components/banner.js";
import { absent } from "./components/empty-state.js";
import { ago, bytes, esc, fmtPct, icon, pct, utc } from "./formatting.js";
import { createMetricsView, parseMetrics } from "./metrics.js";
import { contiguousRuns, coverageBuckets, createPlaybackState, playbackPath, recordingSearchPath, searchRange } from "./playback.js";
import { SCREENS, parseRoute, screenFor } from "./router.js";
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

// ------------------------------------------------------------- live viewing

// The SDK is half a megabyte and live viewing is opt-in, so it is fetched on the
// first press rather than on every page load. It is same-origin:
// `/ui/livekit-client.js` is a compile-time constant inside the server binary,
// not a CDN, so this page still issues no external request.
let sdkPromise = null;
function loadLiveKit() {
  if (sdkPromise) return sdkPromise;
  sdkPromise = new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = "/ui/livekit-client.js";
    script.addEventListener("load", () => {
      if (window.LivekitClient) resolve(window.LivekitClient);
      else reject(new ApiError(0, "livekit_sdk_absent"));
    });
    script.addEventListener("error", () => {
      // Allow a later press to try again rather than caching the failure.
      sdkPromise = null;
      reject(new ApiError(0, "livekit_sdk_unreachable"));
    });
    document.head.appendChild(script);
  });
  return sdkPromise;
}

// One session per camera being watched, all owned here. A session holds a peer
// connection and a media track; dropping the reference without disconnecting
// leaks both and leaves the server counting a subscriber who stopped watching.
// The grid makes this plural, which makes the ownership rule more important,
// not less: nothing may create a room outside this map.
//
// The subscribed track is kept beside its room because the fleet screens
// re-render on a five-second poll, which destroys the element the track was
// attached to. Without the track in hand, a refresh would silently blank every
// tile while the subscriptions stayed open.
const live = { rooms: new Map() };

function liveNote(cameraId, html) {
  for (const node of document.querySelectorAll("[data-live-note]")) {
    if (node.dataset.liveNote === cameraId) node.innerHTML = html;
  }
}

function liveIdle(cameraId, visible) {
  for (const node of document.querySelectorAll("[data-live-idle]")) {
    if (node.dataset.liveIdle === cameraId) node.hidden = !visible;
  }
}

function liveVideos(cameraId) {
  return [...document.querySelectorAll("[data-live-tile]")]
    .filter((node) => node.dataset.liveTile === cameraId);
}

async function stopLive(cameraId) {
  const session = live.rooms.get(cameraId);
  live.rooms.delete(cameraId);
  if (session && session.track) {
    try { session.track.detach(); } catch (error) { /* already detached */ }
  }
  for (const video of liveVideos(cameraId)) {
    video.hidden = true;
    video.removeAttribute("src");
  }
  liveIdle(cameraId, true);
  liveNote(cameraId, "");
  if (!session) return;
  try { await session.room.disconnect(); } catch (error) { /* already gone */ }
}

async function stopAllLive() {
  for (const cameraId of [...live.rooms.keys()]) await stopLive(cameraId);
}

// The fleet screens re-render every five seconds, which replaces the elements
// the tracks were attached to. The subscriptions are untouched by that — they
// belong to the rooms, not to the DOM — so the video is put back rather than
// re-negotiated. Nothing here joins a room; a session that does not already
// exist is not created by a refresh.
function reattachLive() {
  for (const [cameraId, session] of live.rooms) {
    if (!session.track) continue;
    const video = liveVideos(cameraId)[0];
    if (!video || video.dataset.liveAttached === "yes") continue;
    session.track.detach();
    session.track.attach(video);
    video.dataset.liveAttached = "yes";
    video.hidden = false;
    liveIdle(cameraId, false);
    video.play().catch(() => { /* reported by the note the first time */ });
  }
}

async function startLive(cameraId, video) {
  await stopLive(cameraId);

  let sdk;
  try {
    sdk = await loadLiveKit();
  } catch (error) {
    liveNote(cameraId, banner("bad", "The live player could not be loaded.",
      "This page serves the LiveKit browser SDK from the server binary itself. " +
      "That request failed, so no player exists to connect with."));
    return;
  }

  let access;
  try {
    access = await api(`/api/cameras/${encodeURIComponent(cameraId)}/viewer-token`, { method: "POST" });
  } catch (error) {
    liveNote(cameraId, errorBanner(error));
    return;
  }

  const room = new sdk.Room();
  const session = { room, track: null };
  live.rooms.set(cameraId, session);

  // Attaching on subscribe rather than on connect: a room can be joined while
  // nothing is publishing into it, and a `<video>` shown before a track exists
  // is a black rectangle an operator cannot distinguish from a dead camera.
  room.on(sdk.RoomEvent.TrackSubscribed, (track) => {
    if (track.kind !== sdk.Track.Kind.Video) return;
    session.track = track;
    track.attach(video);
    video.dataset.liveAttached = "yes";
    video.hidden = false;
    liveIdle(cameraId, false);
    // `muted` on the element is what makes this permitted without a second
    // gesture; a rejection is still possible, and silence would look like a
    // failure of the camera rather than of the browser.
    video.play().then(() => liveNote(cameraId, ""), () => {
      liveNote(cameraId, banner("info", "Press play to start the stream.",
        "This browser declined to start playback automatically."));
    });
  });

  room.on(sdk.RoomEvent.TrackUnsubscribed, (track) => {
    track.detach();
    if (session.track === track) session.track = null;
    video.hidden = true;
    liveIdle(cameraId, true);
    liveNote(cameraId, banner("warn", "The live stream stopped.",
      "The publisher went away. Recording is independent of this and is " +
      "unaffected; the camera state above is reported separately."));
  });

  room.on(sdk.RoomEvent.Disconnected, () => {
    if (live.rooms.get(cameraId) !== session) return;  // a newer session replaced this one
    live.rooms.delete(cameraId);
    video.hidden = true;
    liveIdle(cameraId, true);
    liveNote(cameraId, banner("warn", "Disconnected from the live stream.",
      "Press watch to reconnect. Recording is unaffected."));
  });

  liveNote(cameraId, banner("info", "Connecting…",
    `Room ${esc(access.room)} on ${esc(access.endpoint)}.`));

  try {
    await room.connect(access.endpoint, access.token);
  } catch (error) {
    await stopLive(cameraId);
    liveNote(cameraId, banner("bad", "Could not connect to the live server.",
      "The token was issued, but this browser could not reach the LiveKit " +
      "server at " + esc(access.endpoint) + ". Recording is unaffected."));
    return;
  }
  if (live.rooms.get(cameraId) !== session) return;  // superseded while connecting

  // Nothing has been subscribed yet at this point. Saying so is the difference
  // between a page that is waiting and a page that is broken.
  if (video.hidden) {
    liveNote(cameraId, banner("info", "Connected — waiting for video.",
      `Development-only viewer session, valid for ${esc(access.expires_in_seconds)}s. The token is not displayed.`));
  }
}

// ------------------------------------------------------------ tile rendering

function cameraTile(camera, options) {
  const id = esc(camera.camera_id);
  const canWatch = camera.live_publication_enabled;
  // The element carries no `src` and no `autoplay`: a track is attached to it
  // by `startLive`, and only after a press. `hidden` keeps a black rectangle
  // off the screen until there is something in it to see.
  // `data-live-watchable` is what "Watch all" filters on. Without it that button
  // joins a room for a camera whose own tile offers no watch button, because
  // publication is disabled — the page would contradict itself in one press.
  const watchable = canWatch ? " data-live-watchable" : "";
  const element = options && options.primary
    ? `<video id="live" data-live-tile="${id}"${watchable} muted playsinline hidden></video>`
    : `<video data-live-tile="${id}"${watchable} muted playsinline hidden></video>`;
  return `<div class="tile">
      <div class="tile-head">
        <b>${cameraLabel(camera)}</b>
        <span class="sp"></span>
        ${pipelinePill(camera)}
      </div>
      <div class="stage">
        ${element}
        <div class="idle" data-live-idle="${id}" >
          ${icon(canWatch ? "live" : "cam", "lg")}
          <div>${canWatch
            ? "Not subscribed. Press watch to join the room."
            : "This camera does not publish to LiveKit."}</div>
        </div>
      </div>
      <div class="tile-foot">
        ${recordingDot(camera)}
        <span class="sp"></span>
        ${livekitChip(camera)}
        ${canWatch ? `<button class="btn icon" data-act="watch-live" data-camera="${id}" title="Watch live">${icon("play")}</button>
        <button class="btn icon" data-act="stop-live" data-camera="${id}" title="Stop">${icon("stop")}</button>` : ""}
        <button class="btn icon" data-act="full" data-camera="${id}" title="Fullscreen">${icon("expand")}</button>
      </div>
      <div class="tile-note" data-live-note="${id}"></div>
    </div>`;
}

// ------------------------------------------------------------------ screens

function fleetProblem() {
  if (!state.fleetError) return "";
  return errorBanner(state.fleetError);
}

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

let gridColumns = 5;

function screenLive() {
  const counts = fleetCounts();
  const publishing = counts.cameras.filter((c) => c.live_publication_enabled);
  // An unreadable fleet has no numerator and no denominator. "0 / 0" would read
  // as a measured fleet of zero cameras; the em dash reads as what it is.
  const ratio = (part, whole) => (counts.readable ? `${part} / ${whole}` : "&mdash;");

  return `${fleetProblem()}
    <div class="row" style="justify-content:space-between;margin-bottom:14px">
      <div class="seg">
        ${[2, 3, 4, 5].map((n) => `<button data-act="grid" data-cols="${n}" class="${gridColumns === n ? "on" : ""}">${n}&times;</button>`).join("")}
      </div>
      <div class="row">
        <span class="muted tiny">${counts.readable
          ? `${counts.connected} / ${counts.total} cameras connected`
          : "fleet not readable"} &middot; ${live.rooms.size} live subscription(s)</span>
        <button class="btn primary" data-act="watch-all">${icon("play")} Watch all</button>
        <button class="btn" data-act="stop-all">${icon("stop")} Stop all</button>
      </div>
    </div>
    ${publishing.length < counts.total
      ? banner("info", `${counts.total - publishing.length} of ${counts.total} cameras do not publish to LiveKit.`,
        "Only a camera configured with LiveKit credentials at start-up publishes. " +
        "A camera added through the API carries no LiveKit configuration, so it records but cannot be watched here.")
      : ""}
    ${counts.total
      ? `<div class="tiles" data-density="${gridColumns}" style="grid-template-columns:repeat(${gridColumns},minmax(0,1fr))">
          ${counts.cameras.map((camera, index) => cameraTile(camera, { primary: index === 0 })).join("")}
        </div>`
      : `<div class="card"><div class="muted">${counts.readable
          ? "No cameras are configured."
          : "The fleet could not be read, so there are no tiles to show."}</div></div>`}
    <div class="strip mt">
      <div>
        <span style="color:var(--green)">${icon("net", "lg")}</span>
        <span><div class="v">${!counts.readable ? "Unknown" : counts.connected === counts.total && counts.total ? "All Systems Live" : "Degraded"}</div>
        <div class="l">${counts.readable ? `${counts.connected} of ${counts.total} streaming` : "the fleet could not be read"}</div></span>
      </div>
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

// ------------------------------------------------------------------ playback

const pb = createPlaybackState();

// What the operator has typed into the add-camera form.
//
// It lives out here because the fleet screen re-renders on every poll, and a
// form whose only copy of its contents is the DOM loses them wholesale every
// five seconds — a camera identifier and an RTSP URL cannot be typed faster
// than that. `pb` is the same idea for the playback filters; this is that
// pattern applied to the one form that did not have it.
const add = { id: "", rtsp: "", width: "1280", height: "720", live: "default" };

function screenPlayback() {
  const counts = fleetCounts();
  if (!pb.camera && counts.cameras.length) pb.camera = counts.cameras[0].camera_id;
  if (!pb.date) pb.date = new Date().toISOString().slice(0, 10);

  return `${fleetProblem()}
    <div class="card">
      <div class="filters">
        <div class="field">
          <label class="f">Camera</label>
          <select class="f" data-bind="camera">
            ${counts.cameras.map((camera) => `<option value="${esc(camera.camera_id)}" ${camera.camera_id === pb.camera ? "selected" : ""}>${esc(camera.camera_id)}</option>`).join("")}
          </select>
        </div>
        <div class="field"><label class="f">Date (UTC)</label><input class="f" type="date" data-bind="date" value="${esc(pb.date)}"></div>
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
        <div class="k">Timezone</div><div class="v">UTC</div>
        <div class="k">Segments matched</div><div class="v">${pb.result ? pb.result.recordings.length : "—"}</div>
      </div>
      ${capability ? `<div class="tiny muted mt" style="border-top:1px solid var(--line);padding-top:11px">
        Accuracy <b>${esc(capability.accuracy)}</b>${capability.seekable ? "" : ", not seekable"}. The server answers
        <code>Accept-Ranges: none</code> and streams chunked, so there is no scrubber, no duration and no speed control here —
        none of them could be honoured.
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
          <td class="num">${esc(utc(Date.parse(run.started_at)))}</td>
          <td class="num">${esc(utc(Date.parse(run.ended_at)))}</td>
          <td class="num">${run.count}</td>
          <td class="num">${esc(bytes(run.bytes))}</td>
          <td><button class="btn" data-act="play" data-start="${esc(run.started_at)}" data-end="${esc(run.ended_at)}">${icon("play")} Play</button></td>
        </tr>`).join("")}</tbody>
      </table></div>
      <video id="v" preload="none" playsinline class="mt" style="width:100%;border-radius:8px;background:#05080d"></video>
    </div>`;
  }
  const run = runs[0];
  const url = `/api/cameras/${encodeURIComponent(pb.camera)}/recordings/playback`;
  return `<div class="card"><h3>${icon("live")} Player
      <span class="right"><span class="pill arch">ARCHIVE</span></span>
    </h3>
    <p class="note">${esc(run.count)} contiguous segment(s), ${esc(bytes(run.bytes))}, from a single stream at <code>${esc(url)}</code>.</p>
    <video id="v" preload="none" playsinline style="width:100%;border-radius:8px;background:#05080d"></video>
    <div class="row mt">
      <button class="btn primary" data-act="play" data-start="${esc(run.started_at)}" data-end="${esc(run.ended_at)}">${icon("play")} Play this run</button>
      <span class="muted tiny">${esc(utc(Date.parse(run.started_at)))} &rarr; ${esc(utc(Date.parse(run.ended_at)))}</span>
    </div>
  </div>`;
}

function segmentPanel(result) {
  const recordings = result.recordings;
  return `<div class="card">
      <h3>${icon("db")} Segments <span class="right">${recordings.length} matched</span></h3>
      ${coverage(recordings)}
      <div class="wrap mt"><table>
        <thead><tr><th>Started</th><th>Ended</th><th>Codec</th><th>Resolution</th><th>Size</th><th>Status</th></tr></thead>
        <tbody>${recordings.map((recording) => `<tr>
          <td class="num">${esc(utc(Date.parse(recording.started_at)))}</td>
          <td class="num">${esc(utc(Date.parse(recording.ended_at)))}</td>
          <td>${esc(recording.codec || "—")}</td>
          <td class="num">${recording.width && recording.height ? `${esc(recording.width)}&times;${esc(recording.height)}` : "—"}</td>
          <td class="num">${esc(bytes(recording.size_bytes))}</td>
          <td>${esc(recording.status || "—")}</td>
        </tr>`).join("")}</tbody>
      </table></div>
    </div>`;
}

function coverage(recordings) {
  const result = coverageBuckets(recordings);
  if (!result) return "";
  return `<div class="cover">${result.filled.map((has) => `<i class="${has ? "has" : ""}"></i>`).join("")}</div>
    <div class="ticks"><span>${esc(utc(result.first))}</span><span>${esc(utc(result.last))}</span></div>`;
}

// -------------------------------------------------------------------- events

function screenEvents() {
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
            <tbody>${byKind.map((sample) => `<tr>
              <td>${esc(sample.labels.kind || Object.values(sample.labels)[0] || "—")}</td>
              <td class="num">${esc(sample.value)}</td>
            </tr>`).join("")}</tbody>
          </table></div>` : `<div class="muted tiny">No event has been published yet.</div>`}
        ${byClass.length ? `<h3 class="mt">${icon("shield")} Overflow policy by class</h3>
          <div class="wrap"><table>
            <thead><tr><th>Class</th><th class="num">Count</th></tr></thead>
            <tbody>${byClass.map((sample) => `<tr>
              <td>${esc(sample.labels.class || Object.values(sample.labels)[0] || "—")}</td>
              <td class="num">${esc(sample.value)}</td>
            </tr>`).join("")}</tbody>
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
        <div class="tiny muted mt">
          Everything above this panel is real: those counters come from <code>/metrics</code> and describe the bus as it runs now.
        </div>
      </div>
    </div>`;
}

// ------------------------------------------------------------------- heatmap

function screenHeatmap() {
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

// ------------------------------------------------------------------- storage

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

// ------------------------------------------------------------------- cameras

function screenCameras(args) {
  if (args.length) return cameraDetail(args[0]);
  const counts = fleetCounts();
  return `${fleetProblem()}
    <div class="card">
      <h3>${icon("cam")} Fleet <span class="right">${counts.readable ? `${counts.connected} of ${counts.total} connected` : "fleet not readable"}</span></h3>
      <div class="wrap"><table>
        <thead><tr>
          <th>Camera</th><th>Pipeline</th><th>Connection</th><th class="num">Attempts</th>
          <th class="num">Reconnects</th><th class="num">Segments</th><th>LiveKit</th><th>Last media</th><th></th>
        </tr></thead>
        <tbody>${counts.cameras.map((camera) => `<tr>
          <td><a href="#/cameras/${encodeURIComponent(camera.camera_id)}">${esc(camera.camera_id)}</a></td>
          <td>${pipelinePill(camera)}</td>
          <td>${esc(camera.connection_state || "—")}</td>
          <td class="num">${esc(camera.connection_attempts)}</td>
          <td class="num ${camera.reconnects ? "warn" : ""}">${esc(camera.reconnects)}</td>
          <td class="num">${esc(camera.recorder_completed_segments)}</td>
          <td>${livekitChip(camera)}</td>
          <td class="num muted">${esc(ago(camera.last_media_unix_ms))}</td>
          <td class="row" style="gap:6px">
            <button class="btn icon" data-act="camera-start" data-camera="${esc(camera.camera_id)}" title="Start">${icon("play")}</button>
            <button class="btn icon" data-act="camera-stop" data-camera="${esc(camera.camera_id)}" title="Stop">${icon("stop")}</button>
          </td>
        </tr>`).join("")}</tbody>
      </table></div>
      ${counts.total ? "" : `<div class="muted mt">${counts.readable
        ? "No cameras are configured."
        : "The fleet could not be read, so this table is empty for a reason unrelated to how many cameras exist."}</div>`}
    </div>

    <div class="card mt">
      <h3>${icon("plus")} Add a camera</h3>
      <p class="note">Registers a camera with the supervisor. It is added stopped; press Start on its row when you want it to connect.</p>
      <div class="filters">
        <div class="field"><label class="f">Camera identifier</label><input class="f" id="add-camera-id" data-add="id" value="${esc(add.id)}" placeholder="camera-02" autocomplete="off"></div>
        <div class="field grow"><label class="f">RTSP endpoint</label><input class="f" id="add-camera-rtsp" data-add="rtsp" value="${esc(add.rtsp)}" placeholder="rtsp://192.168.1.20:554/stream1" autocomplete="off"></div>
        <div class="field"><label class="f">Width</label><input class="f" id="add-camera-width" data-add="width" type="number" min="1" value="${esc(add.width)}"></div>
        <div class="field"><label class="f">Height</label><input class="f" id="add-camera-height" data-add="height" type="number" min="1" value="${esc(add.height)}"></div>
      </div>
      <div class="filters">
        <div class="field"><label class="f">Live publication</label>
          <select class="f" id="add-camera-live" data-add="live">
            <option value="default" ${add.live === "default" ? "selected" : ""}>Same as this deployment</option>
            <option value="on" ${add.live === "on" ? "selected" : ""}>Publish to LiveKit</option>
            <option value="off" ${add.live === "off" ? "selected" : ""}>Do not publish</option>
          </select></div>
        <div class="row"><button class="btn primary" data-act="camera-add">${icon("plus")} Add camera</button></div>
      </div>
      <p class="note">This form takes no RTSP username or password: the dashboard holds no credential of any kind, so there is nothing on it for a script injection to read. A camera that needs one is added through <code>POST /api/cameras</code>, which accepts both.</p>
      <p class="note">Live publication carries no endpoint or credential from this page. The server holds one LiveKit deployment and stamps it onto the camera; leaving this on <em>Same as this deployment</em> publishes when the server publishes. Choosing <em>Publish to LiveKit</em> on a server with none configured is refused rather than quietly ignored.</p>
      <div id="add-camera-out"></div>
    </div>`;
}

// A camera the server will not describe is reported as absent, in the same
// words for every reason. The server answers `404` identically for one that was
// never configured and one this caller may not see, so that a viewer cannot
// enumerate the estate by reading error messages. Explaining the second case
// here would rebuild exactly the oracle the server removed.
function missingCamera(cameraId) {
  return `<div class="card">
      ${banner("warn", "No such camera.", `The server does not describe <code>${esc(cameraId)}</code>.`)}
      <div class="row mt"><a class="btn" href="#/cameras">${icon("left")} Back to the fleet</a></div>
    </div>`;
}

const detail = { id: null, camera: null, error: null };

function cameraDetail(cameraId) {
  if (detail.id !== cameraId) {
    detail.id = cameraId;
    detail.camera = null;
    detail.error = null;
    loadCamera(cameraId);
    return `<div class="card"><div class="muted">Loading ${esc(cameraId)}…</div></div>`;
  }
  if (detail.error) {
    if (detail.error.status === 404) return missingCamera(cameraId);
    return `<div class="card">${errorBanner(detail.error)}</div>`;
  }
  if (!detail.camera) return `<div class="card"><div class="muted">Loading ${esc(cameraId)}…</div></div>`;

  const camera = detail.camera;
  const fixture = detectionIsFixture();

  return `<div class="row" style="margin-bottom:14px">
      <a class="btn" href="#/cameras">${icon("left")} Fleet</a>
      <h2 style="margin:0;font-size:17px">${esc(camera.camera_id)}</h2>
      ${pipelinePill(camera)}
    </div>
    <div class="split-wide">
      <div class="stack">
        <div class="card">
          <h3>${icon("live")} Live</h3>
          <div class="tiles" style="grid-template-columns:minmax(0,1fr)">
            ${cameraTile(camera, { primary: true })}
          </div>
        </div>
        <div class="card">
          <h3>${icon("db")} Recording</h3>
          <div class="kv mt">
            <div class="k">Completed segments</div><div class="v">${esc(camera.recorder_completed_segments)}</div>
            <div class="k">Failed segments</div><div class="v ${camera.recorder_failed_segments ? "bad" : ""}">${esc(camera.recorder_failed_segments)}</div>
            <div class="k">Recorder queue</div><div class="v">${camera.recorder_queue_depth === null ? "—" : `${esc(camera.recorder_queue_depth)} / ${esc(camera.recorder_queue_capacity)}`}</div>
            <div class="k">Rejected (queue full)</div><div class="v ${camera.recorder_rejected ? "bad" : ""}">${esc(camera.recorder_rejected)}</div>
          </div>
          <div class="tiny muted mt">The recorder queue rejects when full, which stops ingest rather than losing footage.</div>
          <div class="row mt"><a class="btn" href="#/playback">${icon("clock")} Search recordings</a></div>
        </div>
      </div>
      <div class="stack">
        <div class="card">
          <h3>${icon("net")} Connection</h3>
          <div class="kv mt">
            <div class="k">State</div><div class="v">${esc(camera.connection_state || "—")}</div>
            <div class="k">Attempts</div><div class="v">${esc(camera.connection_attempts)}</div>
            <div class="k">Connected epochs</div><div class="v">${esc(camera.connection_epochs)}</div>
            <div class="k">Reconnects</div><div class="v ${camera.reconnects ? "warn" : ""}">${esc(camera.reconnects)}</div>
            <div class="k">Terminal failures</div><div class="v ${camera.terminal_failures ? "bad" : ""}">${esc(camera.terminal_failures)}</div>
            <div class="k">Last connected</div><div class="v">${esc(utc(camera.last_connected_unix_ms))}</div>
            <div class="k">Last media</div><div class="v">${esc(utc(camera.last_media_unix_ms))}</div>
          </div>
          ${camera.last_actionable_failure ? `<div class="tiny bad mt">${esc(camera.last_actionable_failure)}</div>` : ""}
        </div>
        <div class="card">
          <h3>${icon("live")} LiveKit</h3>
          <div class="kv mt">
            <div class="k">Enabled</div><div class="v">${camera.live_publication_enabled ? "yes" : "no"}</div>
            <div class="k">State</div><div class="v">${esc(camera.live_publication_state || "—")}</div>
            <div class="k">Access units</div><div class="v">${esc(camera.live_published_access_units)} / ${esc(camera.live_offered)} offered</div>
            <div class="k">Bytes</div><div class="v">${esc(bytes(camera.live_published_bytes))}</div>
            <div class="k">Live queue</div><div class="v">${camera.live_queue_depth === null ? "—" : `${esc(camera.live_queue_depth)} / ${esc(camera.live_queue_capacity)}`}</div>
            <div class="k">Dropped (oldest)</div><div class="v ${camera.live_dropped ? "warn" : ""}">${esc(camera.live_dropped)}</div>
            <div class="k">Declined by transport</div><div class="v ${camera.live_publication_adapter_drops ? "warn" : ""}">${esc(camera.live_publication_adapter_drops)}</div>
          </div>
          ${camera.live_publication_failure ? `<div class="tiny warn mt">${esc(camera.live_publication_failure)}</div>` : ""}
        </div>
        <div class="card">
          <h3>${icon("shield")} Detection</h3>
          ${fixture
            ? banner("warn", "Detection output here is fixture output.",
              "There is no production model in this build. The counts below describe a wired pipeline, not detections of people, and support no accuracy claim.")
            : ""}
          <div class="kv mt">
            <div class="k">Enabled</div><div class="v">${camera.detection_enabled ? "yes" : "no"}</div>
            <div class="k">State</div><div class="v">${esc(camera.detection_state || "—")}</div>
            <div class="k">Inferred</div><div class="v">${esc(camera.detection_inferred)}</div>
            <div class="k">Published</div><div class="v">${esc(camera.detections_published)}</div>
            <div class="k">Dropped</div><div class="v">${esc(camera.detection_dropped)}</div>
          </div>
        </div>
        <div class="card">
          <h3>${icon("gear")} Lifecycle</h3>
          <div class="row mt">
            <button class="btn" data-act="camera-start" data-camera="${esc(camera.camera_id)}">${icon("play")} Start</button>
            <button class="btn" data-act="camera-stop" data-camera="${esc(camera.camera_id)}">${icon("stop")} Stop</button>
            <button class="btn danger" data-act="camera-remove" data-camera="${esc(camera.camera_id)}">${icon("x")} Remove</button>
          </div>
          <div id="lifecycle-out"></div>
        </div>
      </div>
    </div>`;
}

async function loadCamera(cameraId) {
  try {
    detail.camera = await api(`/api/cameras/${encodeURIComponent(cameraId)}`);
    detail.error = null;
  } catch (error) {
    detail.camera = null;
    detail.error = error;
  }
  if (route().name === "cameras" && route().args[0] === cameraId) await render();
}

// ------------------------------------------------------------------ settings

const edits = new Map();

function settingValue(key) {
  if (!state.settings) return null;
  const found = state.settings.settings.find((entry) => entry.key === key);
  return found ? found.value : null;
}

function screenSettings() {
  if (!state.settings) {
    // A configuration surface that is not there must say so. Leaving a spinner
    // on screen would read as slow rather than as absent, and this endpoint
    // genuinely answers `503` when no settings store is configured.
    return `<div class="card">
        <h3>${icon("gear")} Configuration</h3>
        ${state.settingsError
          ? errorBanner(state.settingsError)
          : `<div class="muted">Loading configuration…</div>`}
        ${state.settingsError && state.settingsError.status === 503
          ? `<div class="tiny muted mt">Persistent configuration needs a database. Without one the server runs from its bootstrap layer, which this page cannot edit.</div>`
          : ""}
        <div class="row mt"><button class="btn" data-act="settings-reload">${icon("refresh")} Try again</button></div>
      </div>`;
  }
  const groups = new Map();
  for (const entry of state.settings.settings) {
    const group = entry.key.split(".")[0];
    if (!groups.has(group)) groups.set(group, []);
    groups.get(group).push(entry);
  }

  const stale = state.settings.freshness === "stale";
  return `${stale ? banner("warn", "The configuration snapshot is stale.",
      "The server is serving its last known good configuration because the store is unreachable. Changes cannot be applied while this is true.") : ""}
    <div class="row" style="justify-content:space-between;margin-bottom:14px">
      <div class="muted tiny">
        Generation <b>${esc(state.settings.generation)}</b> &middot; cache <b>${esc(state.settings.cache)}</b> &middot;
        ${esc(state.settings.settings.length)} settings
      </div>
      <div class="muted tiny">Changes are applied in one transaction, validated as a group, and audited in the same transaction.</div>
    </div>
    ${banner("info", "Secrets are not readable or writable from this page.",
      "Credentials render as <code>***</code> and are managed in the secret store or by environment override. " +
      "This page holds no credential of any kind, which is why there is no reveal control and no login here.")}
    <div class="grid cols-2">
      ${[...groups.entries()].map(([group, entries]) => settingsGroup(group, entries)).join("")}
    </div>
    ${saveBar()}
    <div id="settings-out"></div>
    <div class="card mt">
      ${absent("There is no message-broker configuration",
        "The reference design configures NATS, Redis Streams or Kafka as an event backend, with an endpoint, credentials and per-type topic names. This system has no external broker and is not going to grow one from a settings page.",
        ["The event bus is bounded and in-process by design; ADR 002 records that persistence and replay belong to explicit adapters.",
         "The engineering policy forbids introducing Kafka or NATS without a measured constraint and an architecture decision record.",
         "An external broker changes delivery, ordering, persistence and failure semantics — it is not a backend toggle."])}
    </div>`;
}

function settingsGroup(group, entries) {
  return `<div class="card">
      <h3>${icon("gear")} ${esc(group)}</h3>
      ${entries.map((entry) => settingRow(entry)).join("")}
    </div>`;
}

function settingRow(entry) {
  const editable = entry.class === "operational" && !entry.secret;
  const dirty = edits.has(entry.key);
  const shown = dirty ? edits.get(entry.key) : entry.value;
  const tail = entry.key.split(".").slice(1).join(".") || entry.key;
  return `<div class="srow ${dirty ? "dirty" : ""}">
      <div class="key">
        ${esc(tail)}
        <small>${esc(entry.origin)} &middot; ${esc(entry.boundary)}${entry.secret ? " &middot; secret" : ""}${editable ? "" : " &middot; read-only"}</small>
      </div>
      <div>
        <input class="f" data-setting="${esc(entry.key)}" value="${esc(shown)}" ${editable ? "" : "disabled"}>
      </div>
      <div>${dirty ? `<button class="btn icon" data-act="setting-revert" data-key="${esc(entry.key)}" title="Revert">${icon("refresh")}</button>` : ""}</div>
    </div>`;
}

function saveBar() {
  const count = edits.size;
  return `<div class="savebar">
      <span class="${count ? "warn" : "muted"}">${count
        ? `${icon("alert")} ${count} unsaved change${count === 1 ? "" : "s"}`
        : `${icon("check")} No unsaved changes`}</span>
      <span class="sp"></span>
      <button class="btn" data-act="settings-discard" ${count ? "" : "disabled"}>Discard changes</button>
      <button class="btn primary" data-act="settings-save" ${count ? "" : "disabled"}>${icon("check")} Save settings</button>
    </div>`;
}

// -------------------------------------------------------------------- system

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

// -------------------------------------------------------------------- render

const SCREEN_RENDERERS = {
  dashboard: screenDashboard,
  live: screenLive,
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
    // new markup renders in its "press watch" state.
    video.className = fresh.className;
    video.hidden = false;
    fresh.parentNode.replaceChild(video, fresh);
    liveIdle(cameraId, false);
  }
  renderGauges();
  reattachLive();
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
    gridColumns = Number(target.dataset.cols) || 5;
    await render();
    return;
  }

  if (act === "full") {
    const video = liveVideos(cameraId)[0];
    const stage = video && video.closest(".stage");
    if (stage && stage.requestFullscreen) stage.requestFullscreen();
    return;
  }

  // Live viewing has one entry point: a press. Rendering a screen must never
  // join a room — that would issue a viewer token and subscribe a real consumer
  // to the publisher on every navigation. "Watch all" is still one press, and
  // it resolves to the same call.
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
