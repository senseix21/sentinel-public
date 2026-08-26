import assert from "node:assert/strict";
import test from "node:test";
import { createLiveKitOwner } from "../js/livekit.js";

function owner(overrides = {}) {
  return createLiveKitOwner({
    ApiError: Error,
    api: async () => ({ endpoint: "wss://live.test", token: "secret", room: "camera", expires_in_seconds: 60 }),
    banner: (_kind, title) => title,
    errorBanner: (error) => String(error),
    sdkLoader: async () => ({ Room: class {}, RoomEvent: {}, Track: { Kind: { Video: "video" } } }),
    ...overrides,
  });
}

function fakeDocument(videos = []) {
  return {
    querySelectorAll(selector) {
      if (!selector.includes("data-live-tile")) return [];
      return selector.includes("data-live-watchable")
        ? videos.filter((video) => Object.hasOwn(video.dataset, "liveWatchable"))
        : videos;
    },
  };
}

test("stopAllLive disconnects and forgets every owned room", async () => {
  const previousDocument = globalThis.document;
  globalThis.document = { querySelectorAll: () => [] };
  try {
    const sessions = owner();
    const disconnected = [];
    sessions.live.rooms.set("north", { room: { disconnect: async () => disconnected.push("north") }, track: null });
    sessions.live.rooms.set("south", { room: { disconnect: async () => disconnected.push("south") }, track: null });

    await sessions.stopAllLive();

    assert.deepEqual(disconnected.sort(), ["north", "south"]);
    assert.equal(sessions.live.rooms.size, 0);
  } finally {
    globalThis.document = previousDocument;
  }
});

test("stopAllLive tears down every local room before slow peers disconnect", async () => {
  const previousDocument = globalThis.document;
  globalThis.document = { querySelectorAll: () => [] };
  let release;
  const blocked = new Promise((resolve) => { release = resolve; });
  try {
    const sessions = owner();
    sessions.live.rooms.set("north", { room: { disconnect: () => blocked }, track: null });
    sessions.live.rooms.set("south", { room: { disconnect: () => blocked }, track: null });

    const stopping = sessions.stopAllLive();

    assert.equal(sessions.live.rooms.size, 0);
    assert.deepEqual([...sessions.live.stopped].sort(), ["north", "south"]);
    release();
    await stopping;
  } finally {
    globalThis.document = previousDocument;
  }
});

test("stopLive detaches media before disconnecting its room", async () => {
  const previousDocument = globalThis.document;
  globalThis.document = { querySelectorAll: () => [] };
  try {
    const calls = [];
    const sessions = owner();
    sessions.live.rooms.set("north", {
      track: { detach: () => calls.push("detach") },
      room: { disconnect: async () => calls.push("disconnect") },
    });

    await sessions.stopLive("north");

    assert.deepEqual(calls, ["detach", "disconnect"]);
    assert.equal(sessions.live.rooms.has("north"), false);
    assert.equal(sessions.live.stopped.has("north"), true);
  } finally {
    globalThis.document = previousDocument;
  }
});

test("automatic live reconciliation respects stop until an explicit watch", async () => {
  const previousDocument = globalThis.document;
  const video = { dataset: { liveTile: "north", liveWatchable: "" }, hidden: true, removeAttribute() {} };
  globalThis.document = fakeDocument([video]);
  const connected = [];
  class Room {
    on() {}
    async connect() { connected.push("north"); }
    async disconnect() {}
  }
  try {
    const sessions = owner({
      sdkLoader: async () => ({ Room, RoomEvent: {}, Track: { Kind: { Video: "video" } } }),
    });

    await sessions.stopLive("north");
    await sessions.startVisibleLive();
    assert.deepEqual(connected, []);

    await sessions.startLive("north", video);
    assert.deepEqual(connected, ["north"]);
    assert.equal(sessions.live.stopped.has("north"), false);
  } finally {
    globalThis.document = previousDocument;
  }
});

test("temporary navigation teardown does not suppress later automatic viewing", async () => {
  const previousDocument = globalThis.document;
  globalThis.document = fakeDocument([]);
  try {
    const sessions = owner();
    sessions.live.desired.add("north");

    await sessions.stopAllLive(false);

    assert.equal(sessions.live.stopped.has("north"), false);
  } finally {
    globalThis.document = previousDocument;
  }
});

test("startVisibleLive automatically joins every visible watchable camera", async () => {
  const previousDocument = globalThis.document;
  const videos = [
    { dataset: { liveTile: "north", liveWatchable: "" }, removeAttribute() {} },
    { dataset: { liveTile: "south" }, removeAttribute() {} },
  ];
  globalThis.document = fakeDocument(videos);
  const connected = [];
  class Room {
    on() {}
    async connect(endpoint) { connected.push(endpoint); }
    async disconnect() {}
  }
  try {
    const sessions = owner({
      sdkLoader: async () => ({ Room, RoomEvent: {}, Track: { Kind: { Video: "video" } } }),
    });

    await sessions.startVisibleLive();

    assert.deepEqual(connected, ["wss://live.test"]);
    assert.equal(sessions.live.rooms.has("north"), true);
    assert.equal(sessions.live.rooms.has("south"), false);
  } finally {
    globalThis.document = previousDocument;
  }
});

test("a disconnected desired session schedules and performs a reconnect", async () => {
  const previousDocument = globalThis.document;
  const video = { dataset: { liveTile: "north", liveWatchable: "" }, hidden: true, removeAttribute() {} };
  globalThis.document = fakeDocument([video]);
  const scheduled = [];
  const rooms = [];
  const RoomEvent = { Disconnected: "disconnected" };
  class Room {
    constructor() { this.handlers = new Map(); rooms.push(this); }
    on(event, handler) { this.handlers.set(event, handler); }
    async connect() {}
    async disconnect() {}
  }
  try {
    const sessions = owner({
      sdkLoader: async () => ({ Room, RoomEvent, Track: { Kind: { Video: "video" } } }),
      setRetryTimer: (callback, delay) => { scheduled.push({ callback, delay }); return scheduled.length; },
      clearRetryTimer: () => {},
    });
    await sessions.startLive("north", video);

    rooms[0].handlers.get("disconnected")();
    assert.equal(scheduled.length, 1);
    assert.equal(scheduled[0].delay, 1_000);

    await scheduled[0].callback();
    assert.equal(rooms.length, 2);
    assert.equal(sessions.live.rooms.get("north").room, rooms[1]);
  } finally {
    globalThis.document = previousDocument;
  }
});

test("stopLive cancels reconnect intent", async () => {
  const previousDocument = globalThis.document;
  const video = { dataset: { liveTile: "north", liveWatchable: "" }, hidden: true, removeAttribute() {} };
  globalThis.document = fakeDocument([video]);
  const cleared = [];
  try {
    const sessions = owner({ clearRetryTimer: (timer) => cleared.push(timer) });
    sessions.live.desired.add("north");
    sessions.live.retries.set("north", { timer: 42, attempt: 1 });

    await sessions.stopLive("north");

    assert.equal(sessions.live.desired.has("north"), false);
    assert.equal(sessions.live.retries.has("north"), false);
    assert.deepEqual(cleared, [42]);
  } finally {
    globalThis.document = previousDocument;
  }
});
