import { esc } from "./formatting.js";

export function createLiveKitOwner({ ApiError, api, banner, errorBanner, sdkLoader = null }) {

// ------------------------------------------------------------- live viewing

// The SDK is half a megabyte and live viewing is opt-in, so it is fetched on the
// first press rather than on every page load. It is same-origin:
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
    sdk = await (sdkLoader ? sdkLoader() : loadBrowserLiveKit());
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

  return { live, liveVideos, startLive, stopLive, stopAllLive, reattachLive };
}
