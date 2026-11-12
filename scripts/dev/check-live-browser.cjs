// Manual development-stack qualification; Playwright is supplied externally.
// Uses the real viewer-token API in two tabs; neither may evict another viewer.
const assert = require('node:assert/strict');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || 'playwright');
const base = process.env.SENTINEL_BROWSER_URL || 'http://127.0.0.1:3001';

async function qualify() {
  const browser = await chromium.launch({ headless: true, channel: 'chromium' });
  try {
    const context = await browser.newContext();
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.context().addInitScript(() => {
      window.qualificationPeers = [];
      const Original = window.RTCPeerConnection;
      window.RTCPeerConnection = class extends Original {
        constructor(...args) {
          super(...args);
          window.qualificationPeers.push(this);
        }
      };
    });
    await page.goto(base);
    await page.locator('[name=username]').fill(process.env.SENTINEL_DEV_USERNAME || 'operator');
    await page.locator('[name=password]').fill(process.env.SENTINEL_DEV_PASSWORD || 'sentinel-local');
    await page.locator('button[type=submit]').click();
    await page.waitForURL((url) => url.pathname === '/');
    await page.goto(`${base}/#live`);
    await page.waitForFunction(() => [...document.querySelectorAll('[data-live-tile]')]
      .some((video) => !video.hidden && video.readyState >= 2 && video.videoWidth > 0),
    null, { timeout: 45_000 });
    const instrumentFaces = async (target) => {
      if (process.env.SENTINEL_EXPECT_FACE !== 'true' && process.env.SENTINEL_EXPECT_PRESENCE !== 'true') return;
      await target.evaluate(async () => {
        window.qualificationFacePackets = [];
        window.qualificationPresencePackets = [];
        const { DetectionOverlay } = await import('/ui/js/detection-overlay.js');
        const originalPresence = DetectionOverlay.prototype.onPresencePacket;
        DetectionOverlay.prototype.onPresencePacket = function(packet) {
          const accepted=originalPresence.call(this,packet);
          if (accepted) {
            window.qualificationPresencePackets.push({available:packet.available,fixture:packet.results_are_fixture,observed:packet.observed,uncertain:packet.uncertain});
            if(window.qualificationPresencePackets.length>32)window.qualificationPresencePackets.shift();
          }
          return accepted;
        };
        const original = DetectionOverlay.prototype.onFacePacket;
        DetectionOverlay.prototype.onFacePacket = function(packet) {
          const accepted = original.call(this, packet);
          if (accepted) {
            window.qualificationFacePackets.push({ camera:packet.camera_id, fixture:packet.results_are_fixture, count:packet.detections.length });
            if (window.qualificationFacePackets.length > 32) window.qualificationFacePackets.shift();
          }
          return accepted;
        };
      });
    };
    await instrumentFaces(page);
    const sample = (target = page) => target.evaluate(async () => {
      const video = [...document.querySelectorAll('[data-live-tile]')]
        .find((element) => !element.hidden && element.videoWidth > 0);
      const peers = window.qualificationPeers.filter((peer) => peer.connectionState === 'connected');
      let decoded = 0;
      for (const peer of peers) {
        for (const stat of (await peer.getStats()).values()) {
          if (stat.type === 'inbound-rtp' && stat.kind === 'video') decoded += stat.framesDecoded || 0;
        }
      }
      return { time: video?.currentTime, width: video?.videoWidth, height: video?.videoHeight,
        ready: video?.readyState, paused: video?.paused, decoded, connected: peers.length };
    });
    const first = await sample();
    const other = await page.context().newPage();
    other.on('pageerror', (error) => errors.push(error.message));
    await other.goto(`${base}/#live`);
    await other.waitForFunction(() => [...document.querySelectorAll('[data-live-tile]')]
      .some((video) => !video.hidden && video.readyState >= 2 && video.videoWidth > 0),
    null, { timeout: 45_000 });
    await instrumentFaces(other);
    const otherFirst = await sample(other);
    // Covers multiple five-second dashboard reconciles, not just first picture.
    await page.waitForTimeout(16_000);
    const last = await sample();
    assert.ok(last.connected > 0, 'no connected media peer');
    assert.ok(last.ready >= 2 && !last.paused, 'video is not playing');
    assert.ok(last.time > first.time + 5, 'playback did not advance');
    assert.ok(last.decoded > first.decoded, 'browser decoded no further frames');
    assert.deepEqual(errors, [], 'dashboard JavaScript failed');
    const otherLast = await sample(other);
    assert.ok(otherLast.connected > 0 && !otherLast.paused, 'second tab is not connected');
    assert.ok(otherLast.time > otherFirst.time + 5, 'second tab playback did not advance');
    assert.ok(otherLast.decoded > otherFirst.decoded, 'second tab stopped decoding');
    let faces;
    if (process.env.SENTINEL_EXPECT_FACE === 'true' || process.env.SENTINEL_EXPECT_PRESENCE === 'true') {
      faces = await page.evaluate(() => ({ packets:window.qualificationFacePackets, mode:document.querySelector('[data-live-tile]').dataset.detectionMode, readout:document.querySelector('.person-count').textContent }));
      assert.equal(faces.mode, process.env.SENTINEL_EXPECT_PRESENCE === 'true' ? 'person_face' : 'face');
      if (process.env.SENTINEL_EXPECT_FACE_UNAVAILABLE !== 'true') assert.ok(faces.packets.length > 0, 'no real face observations arrived over LiveKit');
      else assert.equal(faces.packets.length, 0, 'failed face model produced observations');
      assert.ok(faces.packets.every((packet) => !packet.fixture), 'fixture faces must not qualify');
      assert.ok(faces.readout.startsWith(process.env.SENTINEL_EXPECT_PRESENCE === 'true' ? 'People observed:' : 'Faces detected:'), 'capability readout mislabeled');
    }
    let presence;
    if(process.env.SENTINEL_EXPECT_PRESENCE === 'true') {
      presence=await page.evaluate(()=>window.qualificationPresencePackets);
      assert.ok(presence.length>0 && presence.some(p=>p.available), 'no fresh person presence received');
      assert.ok(presence.every(p=>!p.fixture),'fixture presence cannot qualify');
    }
    console.log(JSON.stringify({ result: 'passed', first, last, otherFirst, otherLast, faces, presence }));
  } finally {
    await browser.close();
  }
}

qualify().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
