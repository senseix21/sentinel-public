// Isolated development fixture only. Replaces the selected MediaMTX publisher;
// the replacement FFmpeg and Chromium children are stopped on completion.
const assert = require('node:assert/strict');
const { statSync } = require('node:fs');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || 'playwright');

function loopbackUrl(value, protocol) {
  const url = new URL(value);
  assert.equal(url.protocol, protocol);
  assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname), 'fixture must use loopback');
  assert.ok(!url.username && !url.password && !url.search && !url.hash, 'credential-free fixture URL required');
  return url;
}

async function stopChild(child) {
  if (!child?.pid || child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, 'exit');
  child.kill('SIGTERM');
  let timer;
  try {
    await Promise.race([exited, new Promise((resolve) => {
      timer = setTimeout(() => { child.kill('SIGKILL'); resolve(); }, 3000);
    })]);
    await exited;
  } finally { clearTimeout(timer); }
}

(async () => {
  const source = process.env.SENTINEL_RECOVERY_SOURCE;
  assert.ok(source, 'SENTINEL_RECOVERY_SOURCE must name the local H.264 fixture file');
  assert.ok(statSync(source).isFile(), 'recovery source must be a local file');
  const base = loopbackUrl(process.env.SENTINEL_BROWSER_URL || 'http://127.0.0.1:3001', 'http:');
  const api = loopbackUrl(process.env.SENTINEL_MEDIAMTX_API || 'http://127.0.0.1:9997', 'http:');
  const rtsp = loopbackUrl(process.env.SENTINEL_RTSP_FIXTURE_URL || 'rtsp://127.0.0.1:8554/presence', 'rtsp:');
  const camera = process.env.SENTINEL_RECOVERY_CAMERA || 'qualification-01';
  const browser = await chromium.launch({ headless: true, channel: 'chromium' });
  let publisher;
  let cleaning;
  const cleanup = () => cleaning ||= (async () => {
    try { await stopChild(publisher); }
    finally { await browser.close(); }
  })();
  const interrupted = code => { cleanup().then(() => process.exit(code), error => {
    console.error(error); process.exit(1);
  }); };
  const onInterrupt = () => interrupted(130);
  const onTerminate = () => interrupted(143);
  process.once('SIGINT', onInterrupt);
  process.once('SIGTERM', onTerminate);
  try {
    const context = await browser.newContext();
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(base.href);
    await page.locator('[name=username]').fill(process.env.SENTINEL_DEV_USERNAME || 'operator');
    await page.locator('[name=password]').fill(process.env.SENTINEL_DEV_PASSWORD || 'sentinel-local');
    await page.locator('button[type=submit]').click();
    await page.waitForURL(url => url.pathname === '/');
    const health = async () => {
      const response = await page.request.get(new URL(`/api/cameras/${encodeURIComponent(camera)}/health`, base).href);
      assert.equal(response.status(), 200);
      return response.json();
    };
    const play = async target => {
      await target.goto(new URL('/#live', base).href);
      await target.waitForFunction(() => document.querySelector('[data-live-tile]')?.videoWidth > 0, null, { timeout: 45000 });
    };
    const frames = target => target.evaluate(() => document.querySelector('[data-live-tile]').getVideoPlaybackQuality().totalVideoFrames);
    await play(page);
    const other = await context.newPage();
    other.on('pageerror', error => errors.push(error.message));
    await play(other);
    const before = await health();
    assert.equal(before.connection_state, 'connected');
    const initialFrames = [await frames(page), await frames(other)];
    const sessionResponse = await page.request.get(new URL('/v3/rtspsessions/list', api).href);
    assert.equal(sessionResponse.status(), 200);
    const sessions = (await sessionResponse.json()).items;
    const publishers = sessions.filter(s => s.state === 'publish' && s.path === rtsp.pathname.slice(1));
    assert.equal(publishers.length, 1, 'expected exactly one fixture publisher');
    const kick = await page.request.post(new URL(`/v3/rtspsessions/kick/${publishers[0].id}`, api).href);
    assert.equal(kick.status(), 200);
    await page.waitForTimeout(1500);
    const down = await health();
    assert.notEqual(down.connection_state, 'connected', 'camera loss was not observed');
    // No shell, no extra camera reader: only the fixture publisher is replaced.
    publisher = spawn(process.env.FFMPEG_BIN || 'ffmpeg', ['-hide_banner', '-loglevel', 'error', '-re', '-stream_loop', '-1', '-i', source,
      '-c:v', 'copy', '-an', '-rtsp_transport', 'tcp', '-f', 'rtsp', rtsp.href], { stdio: ['ignore', 'ignore', 'inherit'] });
    let publisherError;
    publisher.on('error', error => { publisherError = error; });
    let after;
    const until = Date.now() + 75000;
    while (Date.now() < until) {
      assert.ifError(publisherError);
      assert.equal(publisher.exitCode, null, 'fixture publisher exited');
      after = await health();
      if (after.connection_epochs > before.connection_epochs && after.connection_state === 'connected'
          && after.detection_inferred > down.detection_inferred + 5
          && after.live_published_access_units > down.live_published_access_units + 10
          && after.recorder_completed_segments >= down.recorder_completed_segments + 2) break;
      await page.waitForTimeout(250);
    }
    assert.ok(after.connection_epochs > before.connection_epochs, 'camera did not reconnect');
    assert.ok(after.detection_inferred > down.detection_inferred + 5, 'inference stalled after reconnect');
    assert.ok(after.live_published_access_units > down.live_published_access_units + 10, 'live publication stalled after reconnect');
    assert.ok(after.recorder_completed_segments >= down.recorder_completed_segments + 2, 'recording stalled after reconnect');
    assert.equal(after.recorder_rejected, before.recorder_rejected);
    assert.equal(after.recorder_failed_segments, before.recorder_failed_segments);
    assert.equal(after.detection_state, 'Running');
    const resumedFrames = [await frames(page), await frames(other)];
    await page.waitForTimeout(6000);
    const finalFrames = [await frames(page), await frames(other)];
    for (let i = 0; i < 2; i++) {
      assert.ok(resumedFrames[i] > initialFrames[i] && finalFrames[i] > resumedFrames[i] + 20, `viewer ${i} did not resume decoding`);
    }
    const finalSessions = await page.request.get(new URL('/v3/rtspsessions/list', api).href);
    assert.equal(finalSessions.status(), 200);
    assert.equal((await finalSessions.json()).items.filter(s => s.state === 'read' && s.path === rtsp.pathname.slice(1)).length, 1,
      'consumers created an extra upstream camera connection');
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ result: 'passed', initialFrames, resumedFrames, finalFrames, before, down, after }));
  } finally {
    await cleanup();
    process.removeListener('SIGINT', onInterrupt);
    process.removeListener('SIGTERM', onTerminate);
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
