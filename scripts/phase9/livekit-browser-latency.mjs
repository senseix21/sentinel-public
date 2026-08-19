#!/usr/bin/env node

import { mkdtemp, rm } from "node:fs/promises";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

const chromePath = process.env.CHROME_PATH || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const baseUrl = process.env.DASHBOARD_URL;
const username = process.env.DEV_USERNAME;
const password = process.env.DEV_PASSWORD;
const trials = Number(process.env.TRIALS || 10);
if (!baseUrl || !username || !password || !Number.isInteger(trials) || trials < 2) {
  throw new Error("DASHBOARD_URL, DEV_USERNAME, DEV_PASSWORD, and TRIALS>=2 are required");
}

const profile = await mkdtemp(join(tmpdir(), "surveillance-live-latency."));
const port = 9317 + (process.pid % 200);
const chrome = spawn(chromePath, [
  "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
  `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, "about:blank",
], { stdio: "ignore" });

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function json(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: ${response.status}`);
  return response.json();
}

let socket;
let sequence = 0;
const pending = new Map();
async function command(method, params = {}) {
  const id = ++sequence;
  socket.send(JSON.stringify({ id, method, params }));
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
}
async function evaluate(expression) {
  const response = await command("Runtime.evaluate", {
    expression, awaitPromise: true, returnByValue: true,
  });
  if (response.result.exceptionDetails) {
    const detail = response.result.exceptionDetails;
    throw new Error(detail.exception?.description || detail.text);
  }
  return response.result.result.value;
}
async function waitForChrome() {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      const pages = await json(`http://127.0.0.1:${port}/json`);
      const page = pages.find((entry) => entry.type === "page");
      if (page) return page;
    } catch { /* startup */ }
    await sleep(100);
  }
  throw new Error("Chrome DevTools endpoint did not become ready");
}

function percentile(samples, fraction) {
  const ordered = [...samples].sort((a, b) => a - b);
  return ordered[Math.max(0, Math.ceil(ordered.length * fraction) - 1)];
}

const metricNames = [
  "surveillance_livekit_stale_drops_total",
  "surveillance_camera_live_evictions_total",
  "surveillance_livekit_adapter_drops_total",
  "surveillance_livekit_published_frames_total",
  "surveillance_livekit_adapter_restarts_total",
];
function metricTotals(text) {
  return Object.fromEntries(metricNames.map((name) => {
    const total = text.split("\n")
      .filter((line) => line.startsWith(`${name}{`) || line.startsWith(`${name} `))
      .reduce((sum, line) => sum + Number(line.trim().split(/\s+/).at(-1)), 0);
    return [name, total];
  }));
}

try {
  const page = await waitForChrome();
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (!message.id || !pending.has(message.id)) return;
    const waiter = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) waiter.reject(new Error(message.error.message));
    else waiter.resolve(message);
  });
  await command("Page.enable");
  await command("Runtime.enable");
  await command("Page.navigate", { url: `${baseUrl.replace(/\/$/, "")}/login` });
  await sleep(500);
  await evaluate(`(() => {
    document.querySelector('#username').value = ${JSON.stringify(username)};
    document.querySelector('#password').value = ${JSON.stringify(password)};
    document.querySelector('form').requestSubmit();
  })()`);
  await sleep(1000);
  await command("Page.navigate", { url: `${baseUrl.replace(/\/$/, "")}/#/live` });
  await sleep(1500);
  const metricsBefore = metricTotals(await evaluate("fetch('/metrics').then((response) => response.text())"));

  const samples = [];
  for (let index = 0; index < trials; index += 1) {
    const warm = index % 2 === 1;
    if (index > 0) await sleep(warm ? 500 : 6000);
    let sample;
    try {
      sample = await evaluate(`(async () => {
      const wait = (test, timeout = 10000) => new Promise((resolve, reject) => {
        const started = performance.now();
        const poll = () => test() ? resolve(test()) : performance.now() - started > timeout
          ? reject(new Error('browser fixture timeout')) : setTimeout(poll, 25);
        poll();
      });
      const button = await wait(() => document.querySelector('[data-act="watch-live"]'));
      const video = document.querySelector('[data-live-tile="' + button.dataset.camera + '"]');
      const started = performance.now();
      const firstFrame = new Promise((resolve) => {
        if (video.requestVideoFrameCallback) video.requestVideoFrameCallback(() => resolve(performance.now()));
        else video.addEventListener('playing', () => resolve(performance.now()), { once: true });
      });
      button.click();
      const first = await Promise.race([
        firstFrame,
        new Promise((_, reject) => setTimeout(() => reject(new Error('first frame timeout')), 10000)),
      ]);
      const gaps = [];
      let prior = first;
      const until = performance.now() + 2000;
      await new Promise((resolve) => {
        const frame = (now) => {
          gaps.push(now - prior); prior = now;
          if (now >= until) resolve(); else video.requestVideoFrameCallback(frame);
        };
        if (video.requestVideoFrameCallback) video.requestVideoFrameCallback(frame);
        else setTimeout(resolve, 2000);
      });
      document.querySelector('[data-act="stop-live"][data-camera="' + button.dataset.camera + '"]').click();
      return { warm: ${warm}, first_frame_ms: first - started, max_frame_gap_ms: gaps.length ? Math.max(...gaps) : null };
      })()`);
    } catch (error) {
      sample = { warm, first_frame_ms: 10000, max_frame_gap_ms: null, timed_out: true };
      await evaluate(`(() => document.querySelector('[data-act="stop-live"]')?.click())()`);
    }
    samples.push(sample);
  }

  const summarize = (warm) => {
    const selected = samples.filter((sample) => sample.warm === warm);
    const first = selected.map((sample) => sample.first_frame_ms);
    return {
      count: selected.length,
      timed_out: selected.filter((sample) => sample.timed_out).length,
      first_frame_ms: { p50: percentile(first, 0.5), p90: percentile(first, 0.9), max: Math.max(...first) },
      max_frame_gap_ms: Math.max(...selected.map((sample) => sample.max_frame_gap_ms || 0)),
    };
  };
  const metricsAfter = metricTotals(await evaluate("fetch('/metrics').then((response) => response.text())"));
  const metricDeltas = Object.fromEntries(metricNames.map((name) => [name, metricsAfter[name] - metricsBefore[name]]));
  process.stdout.write(`${JSON.stringify({ workload: { trials, fixture: "authenticated-ui" }, cold: summarize(false), warm: summarize(true), metric_deltas: metricDeltas, samples }, null, 2)}\n`);
} finally {
  if (socket) socket.close();
  chrome.kill("SIGTERM");
  await Promise.race([
    new Promise((resolve) => chrome.once("exit", resolve)),
    sleep(3000),
  ]);
  await rm(profile, { recursive: true, force: true });
}
