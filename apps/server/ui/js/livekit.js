import { esc } from "./formatting.js";
import { DetectionOverlay, DETECTION_TOPIC } from "./detection-overlay.js";

export function createLiveKitOwner({
  ApiError, api, banner, errorBanner, sdkLoader = null,
  setRetryTimer = (callback, delay) => setTimeout(callback, delay),
  clearRetryTimer = (timer) => clearTimeout(timer),
}) {

// ------------------------------------------------------------- live viewing

// The SDK is half a megabyte, so it is fetched only when a live subscription is
// requested rather than on every page load. It is same-origin:
// `/ui/livekit-client.js` is a compile-time constant inside the server binary,
// not a CDN, so this page still issues no external request.
let sdkPromise = null;
function loadBrowserLiveKit() {
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
// `stopped` is operator intent, distinct from a disconnected room. Without it,
// the live-screen reconciler sees the intentionally empty room map on its next
// poll and immediately starts every stopped camera again.
const live = { rooms: new Map(), desired: new Set(), retries: new Map(), stopped: new Set() };

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

function cancelRetry(cameraId) {
  const retry = live.retries.get(cameraId);
  if (retry) clearRetryTimer(retry.timer);
  live.retries.delete(cameraId);
}

async function teardownLive(cameraId) {
  const session = live.rooms.get(cameraId);
  live.rooms.delete(cameraId);
  if (session && session.overlay) {
    try { session.overlay.detach(); } catch (error) { /* already detached */ }
  }
  if (session && session.track) {
    try { session.track.detach(); } catch (error) { /* already detached */ }
  }
  for (const video of liveVideos(cameraId)) {
    video.hidden = true;
    video.removeAttribute("src");
  }
  setFixtureBadge(cameraId, false);
  liveIdle(cameraId, true);
  liveNote(cameraId, "");
  if (!session) return;
  try { await session.room.disconnect(); } catch (error) { /* already gone */ }
}

// A tile-local "detection unavailable" badge, toggled by the overlay when
// `results_are_fixture` is seen. Hidden by default; the live.js markup renders
// the span with `.detection-badge.hidden` and we flip the class.
function setFixtureBadge(cameraId, show) {
  for (const video of liveVideos(cameraId)) {
    // Guard against test doubles that stand in for a `<video>` without the
    // full element surface; `closest` only exists on real DOM elements.
    if (typeof video.closest !== "function") continue;
    const tile = video.closest(".stage");
    if (!tile) continue;
    const badge = tile.querySelector(".detection-badge");
    if (badge) badge.classList.toggle("hidden", !show);
  }
}

async function stopLive(cameraId, remember = true) {
  if (remember) live.stopped.add(cameraId);
  live.desired.delete(cameraId);
  cancelRetry(cameraId);
  await teardownLive(cameraId);
}

async function stopAllLive(remember = true) {
  const cameraIds = new Set([...live.desired, ...live.rooms.keys(), ...live.retries.keys()]);
  if (remember) {
    for (const video of document.querySelectorAll("[data-live-tile][data-live-watchable]")) {
      cameraIds.add(video.dataset.liveTile);
    }
  }
  // Each stop removes its local room and media synchronously before awaiting
  // the network disconnect. Start all teardowns together so one slow LiveKit
  // peer cannot delay stopping the remaining tiles.
  await Promise.all([...cameraIds].map((cameraId) => stopLive(cameraId, remember)));
}

function scheduleReconnect(cameraId, priorAttempt = 0) {
  if (!live.desired.has(cameraId) || live.retries.has(cameraId)) return;
  const attempt = priorAttempt + 1;
  const delay = Math.min(30_000, 1_000 * (2 ** Math.min(attempt - 1, 5)));
  const timer = setRetryTimer(async () => {
    live.retries.delete(cameraId);
    if (!live.desired.has(cameraId) || live.rooms.has(cameraId)) return;
    const video = liveVideos(cameraId)[0];
    if (!video) {
      scheduleReconnect(cameraId);
      return;
    }
    await startLive(cameraId, video, attempt);
  }, delay);
  live.retries.set(cameraId, { timer, attempt });
  liveNote(cameraId, banner("warn", "Reconnecting to the live stream…",
    `Retrying automatically in ${Math.ceil(delay / 1_000)} second(s). Recording is unaffected.`));
}

// The fleet screens re-render every five seconds, which replaces the elements
// the tracks were attached to. The subscriptions are untouched by that — they
// belong to the rooms, not to the DOM — so the video is put back rather than
// re-negotiated. The live-screen reconciler separately starts missing sessions.
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

// Detection canvases live in the DOM beside the `<video>` and are therefore
// casualties of the live-screen re-render even though the video itself is
// carried across. This re-inserts the canvas next to each attached video
// without disturbing the overlay's packet state, so a detection that arrived
// one tick ago is not forgotten over the next render.
function reattachOverlays() {
  for (const [cameraId, session] of live.rooms) {
    if (!session.overlay) continue;
    const video = liveVideos(cameraId)[0];
    if (!video) continue;
    session.overlay.reattach(video);
  }
}

async function startLive(cameraId, video, retryAttempt = 0) {
  live.stopped.delete(cameraId);
  live.desired.add(cameraId);
  cancelRetry(cameraId);
  await teardownLive(cameraId);

  let sdk;
  try {
    sdk = await (sdkLoader ? sdkLoader() : loadBrowserLiveKit());
  } catch (error) {
    liveNote(cameraId, banner("bad", "The live player could not be loaded.",
      "This page serves the LiveKit browser SDK from the server binary itself. " +
      "That request failed, so no player exists to connect with."));
    scheduleReconnect(cameraId, retryAttempt);
    return;
  }

  let access;
  try {
    access = await api(`/api/cameras/${encodeURIComponent(cameraId)}/viewer-token`, { method: "POST" });
  } catch (error) {
    liveNote(cameraId, errorBanner(error));
    scheduleReconnect(cameraId, retryAttempt);
    return;
  }

  const room = new sdk.Room();
  const overlay = new DetectionOverlay();
  const session = { room, track: null, overlay, retryAttempt };
  live.rooms.set(cameraId, session);

  // Attaching on subscribe rather than on connect: a room can be joined while
  // nothing is publishing into it, and a `<video>` shown before a track exists
  // is a black rectangle an operator cannot distinguish from a dead camera.
  room.on(sdk.RoomEvent.TrackSubscribed, (track) => {
    if (track.kind !== sdk.Track.Kind.Video) return;
    session.track = track;
    track.attach(video);
    overlay.attach(video);
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
    overlay.detach();
    if (session.track === track) session.track = null;
    video.hidden = true;
    setFixtureBadge(cameraId, false);
    liveIdle(cameraId, true);
    liveNote(cameraId, banner("warn", "The live stream stopped.",
      "The publisher went away. Recording is independent of this and is " +
      "unaffected; the camera state above is reported separately."));
  });

  // Detection boxes travel on the room's data channel, topic "det/v1".
  // The overlay enforces staleness and the fixture guard; here we only
  // decode, filter by topic, and route.
  room.on(sdk.RoomEvent.DataReceived, (payload, _participant, _kind, topic) => {
    if (topic !== DETECTION_TOPIC) return;
    let packet;
    try { packet = JSON.parse(new TextDecoder().decode(payload)); }
    catch (error) { return; }
    overlay.onPacket(packet);
    setFixtureBadge(cameraId, !!packet.results_are_fixture);
  });

  room.on(sdk.RoomEvent.Disconnected, () => {
    if (live.rooms.get(cameraId) !== session) return;  // a newer session replaced this one
    live.rooms.delete(cameraId);
    video.hidden = true;
    liveIdle(cameraId, true);
    scheduleReconnect(cameraId, session.retryAttempt);
  });

  liveNote(cameraId, banner("info", "Connecting…",
    `Room ${esc(access.room)} on ${esc(access.endpoint)}.`));

  try {
    await room.connect(access.endpoint, access.token);
  } catch (error) {
    await teardownLive(cameraId);
    liveNote(cameraId, banner("bad", "Could not connect to the live server.",
      "The token was issued, but this browser could not reach the LiveKit " +
      "server at " + esc(access.endpoint) + ". Recording is unaffected."));
    scheduleReconnect(cameraId, retryAttempt);
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

async function startVisibleLive() {
  const videos = document.querySelectorAll("[data-live-tile][data-live-watchable]");
  await Promise.all([...videos].map(async (video) => {
    const cameraId = video.dataset.liveTile;
    if (live.stopped.has(cameraId) || live.rooms.has(cameraId) || live.retries.has(cameraId)) return;
    await startLive(cameraId, video);
  }));
}

  return {
    live, liveVideos,
    startLive, startVisibleLive, stopLive, stopAllLive,
    reattachLive, reattachOverlays,
    liveIdle,
  };
}
