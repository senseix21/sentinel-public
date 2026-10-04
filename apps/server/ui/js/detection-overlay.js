// Canvas overlay that draws tracked person boxes over a <video> element.
//
// Normalised box coordinates arrive on a LiveKit data channel (topic
// "det/v1"). The overlay is lossy by design: it keeps the most recent packet
// and draws it on every animation frame, dropping it when it goes stale.
// Alignment to a specific frame is not achievable here (the published stream
// uses the adapter clock, independent of the detector's RTP clock) and we
// deliberately do not try — see the design spec section 2 (C-1).
//
// The server includes `results_are_fixture` in every packet. When true, the
// canvas stays blank. A caller can observe the flag via `isFixture()` and
// show a sibling badge in the tile.

export const DETECTION_TOPIC = "det/v1";
export const DETECTION_MAX_AGE_MS = 3000;

const BOX_STROKE = "rgba(40,220,100,";
const LABEL_FONT_SIZE_PX = 11;
const BOX_LINE_WIDTH_PX = 2;

/**
 * Given a <video> element using `object-fit: contain`, returns the content
 * rectangle (CSS pixels) that the video actually paints into — accounting
 * for pillarbox/letterbox bars. Returns null when the video has no
 * intrinsic size yet.
 */
export function contentRect(video) {
  const elW = video.clientWidth;
  const elH = video.clientHeight;
  const vW = video.videoWidth;
  const vH = video.videoHeight;
  if (vW === 0 || vH === 0 || elW === 0 || elH === 0) return null;
  const scale = Math.min(elW / vW, elH / vH);
  const cW = vW * scale;
  const cH = vH * scale;
  return { x: (elW - cW) / 2, y: (elH - cH) / 2, w: cW, h: cH };
}

function drawBox(ctx, det, rect, dpr) {
  const x = (rect.x + det.left * rect.w) * dpr;
  const y = (rect.y + det.top * rect.h) * dpr;
  const w = det.width * rect.w * dpr;
  const h = det.height * rect.h * dpr;
  // Opacity reflects confidence: 0.4 at zero, 1.0 at one. Low-confidence
  // boxes still render but are visually dim; a hidden box would misread as
  // "no person" rather than "the model is unsure".
  const alpha = 0.4 + Math.max(0, Math.min(1, det.confidence)) * 0.6;
  ctx.strokeStyle = BOX_STROKE + alpha + ")";
  ctx.lineWidth = BOX_LINE_WIDTH_PX * dpr;
  ctx.strokeRect(x, y, w, h);
  if (det.track_id !== null && det.track_id !== undefined) {
    ctx.fillStyle = ctx.strokeStyle;
    ctx.font = (LABEL_FONT_SIZE_PX * dpr) + "px monospace";
    ctx.fillText("#" + det.track_id, x + 4 * dpr, y + 14 * dpr);
  }
}

/**
 * One overlay per camera tile. The caller calls `attach(video)` after the
 * video element exists in the DOM, delivers packets via `onPacket(packet)`,
 * and calls `detach()` when the tile stops watching.
 *
 * The overlay owns one <canvas> element inserted immediately after the
 * <video>, sized to match it via a `ResizeObserver`. Rendering runs on
 * `requestAnimationFrame`; nothing is drawn while no packet has arrived,
 * while the latest packet is older than `DETECTION_MAX_AGE_MS`, or while
 * `results_are_fixture` is true.
 */
export class DetectionOverlay {
  constructor() {
    this._video = null;
    this._canvas = null;
    this._raf = null;
    this._resizeObserver = null;
    this._lastPacket = null;
    this._lastReceivedAt = 0;
    this._now = () => Date.now();
  }

  // Injected in tests to simulate elapsed time without wall-clock drift.
  _setClock(now) { this._now = now; }

  attach(videoEl) {
    if (this._video) this.detach();
    this._video = videoEl;

    const canvas = document.createElement("canvas");
    canvas.className = "detection-overlay";
    canvas.style.cssText =
      "position:absolute;inset:0;pointer-events:none";
    // The parent must be `position:relative` for `inset:0` to pin to it.
    // `.stage` already is in screens.css; this is defensive for other hosts.
    if (videoEl.parentElement) {
      const current = window.getComputedStyle(videoEl.parentElement).position;
      if (current === "static") {
        videoEl.parentElement.style.position = "relative";
      }
      videoEl.parentElement.insertBefore(canvas, videoEl.nextSibling);
    }
    this._canvas = canvas;

    this._resizeObserver = new ResizeObserver(() => this._resize());
    this._resizeObserver.observe(videoEl);
    this._resize();
    this._schedule();
  }

  detach() {
    if (this._raf !== null) {
      cancelAnimationFrame(this._raf);
      this._raf = null;
    }
    if (this._resizeObserver) {
      this._resizeObserver.disconnect();
      this._resizeObserver = null;
    }
    if (this._canvas) {
      this._canvas.remove();
      this._canvas = null;
    }
    this._video = null;
    this._lastPacket = null;
    this._lastReceivedAt = 0;
  }

  onPacket(packet) {
    if (!packet || !Array.isArray(packet.detections)) return;
    this._lastPacket = packet;
    this._lastReceivedAt = this._now();
  }

  isFixture() {
    return !!(this._lastPacket && this._lastPacket.results_are_fixture);
  }

  _resize() {
    if (!this._video || !this._canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const w = this._video.clientWidth;
    const h = this._video.clientHeight;
    this._canvas.width = Math.round(w * dpr);
    this._canvas.height = Math.round(h * dpr);
    this._canvas.style.width = w + "px";
    this._canvas.style.height = h + "px";
  }

  _schedule() {
    this._raf = requestAnimationFrame(() => {
      this._render();
      if (this._video) this._schedule();
    });
  }

  _render() {
    if (!this._canvas || !this._video) return;
    const ctx = this._canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, this._canvas.width, this._canvas.height);

    const packet = this._lastPacket;
    if (!packet) return;
    if (this._now() - this._lastReceivedAt > DETECTION_MAX_AGE_MS) return;
    if (packet.results_are_fixture) return;

    const rect = contentRect(this._video);
    if (!rect) return;

    const dpr = window.devicePixelRatio || 1;
    for (const det of packet.detections) {
      drawBox(ctx, det, rect, dpr);
    }
  }
}
