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
  } finally {
    globalThis.document = previousDocument;
  }
});
