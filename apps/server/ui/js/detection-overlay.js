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
export const PRESENCE_TOPIC = "presence/v1";
export const FACE_TOPIC = "face/v1";
export const MAX_DETECTION_PACKET_BYTES = 16 * 1024;
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
  constructor({ mode = "person", cameraId = null, maxAgeMs = DETECTION_MAX_AGE_MS } = {}) {
    this._presence = null;
    this._presenceAt = 0;
    this._presenceSequence = null;
    this._presenceSession = null;
    this._faceSession = null;
    this._personUnavailable = false;
    this._mode = mode;
    this._maxAgeMs = Number.isFinite(maxAgeMs) && maxAgeMs > 0 && maxAgeMs <= 60000 ? maxAgeMs : DETECTION_MAX_AGE_MS;
    this._unavailable = false;
    this._cameraId = cameraId;
    this._epoch = null;
    this._sequence = null;
    this._video = null;
    this._canvas = null;
    this._raf = null;
    this._resizeObserver = null;
    this._lastPacket = null;
    this._lastReceivedAt = 0;
    this._now = () => Date.now();
    // Track-id bookkeeping for the person-count readout. `_sessionTrackIds`
    // grows across the lifetime of this overlay instance (one Watch session),
    // and is reset on `detach`.
    this._activeTrackIds = new Set();
    this._sessionTrackIds = new Set();
  }

  // Injected in tests to simulate elapsed time without wall-clock drift.
  _setClock(now) { this._now = now; }

  attach(videoEl) {
    if (this._video) this.detach();
    this._video = videoEl;
    this._insertCanvas(videoEl);
    this._resizeObserver = new ResizeObserver(() => this._resize());
    this._resizeObserver.observe(videoEl);
    this._resize();
    this._schedule();
  }

  detach() {
    this._tearDownDom();
    this._video = null;
    this._lastPacket = null;
    this._lastReceivedAt = 0;
    this._presence = null;
    this._presenceSession = null;
    this._faceSession = null;
    this._presenceSequence = null;
    this._activeTrackIds.clear();
    this._sessionTrackIds.clear();
    this._epoch = null;
    this._sequence = null;
  }

  // Re-point the overlay at a new <video> element without forgetting the
  // packet it last received. Called by the live-screen reconcile loop, which
  // carries the <video> node across re-renders but cannot carry the sibling
  // canvas. A packet that arrived moments before the reconcile stays visible
  // through it rather than blanking for the full staleness window.
  reattach(videoEl) {
    if (!videoEl) return;
    // Only skip the full teardown when the canvas is still in the live DOM
    // under the same video. After a `.stage` subtree is replaced, the old
    // canvas's parentNode is a detached node that still passes a truthiness
    // check — hence the explicit `isConnected`.
    const canvasLive =
      this._canvas &&
      this._canvas.isConnected &&
      this._canvas.parentNode === videoEl.parentNode;
    if (videoEl === this._video && canvasLive) return;
    this._tearDownDom();
    this._video = videoEl;
    this._insertCanvas(videoEl);
    this._resizeObserver = new ResizeObserver(() => this._resize());
    this._resizeObserver.observe(videoEl);
    this._resize();
    this._schedule();
  }

  _tearDownDom() {
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
  }

  _insertCanvas(videoEl) {
    const canvas = document.createElement("canvas");
    canvas.className = "detection-overlay";
    canvas.style.cssText = "position:absolute;inset:0;pointer-events:none";
    if (videoEl.parentElement) {
      const current = window.getComputedStyle(videoEl.parentElement).position;
      if (current === "static") {
        videoEl.parentElement.style.position = "relative";
      }
      videoEl.parentElement.insertBefore(canvas, videoEl.nextSibling);
    }
    this._canvas = canvas;
  }

  onPacket(packet) {
    if (this._mode !== "person") return false;
    if (!packet || !Array.isArray(packet.detections)) return;
    this._lastPacket = packet;
    this._lastReceivedAt = this._now();
    // Refresh active-id set from this frame; accumulate into session set.
    // Fixture packets would include fake track_ids; ignore them so counts
    // reflect only real detections.
    this._activeTrackIds.clear();
    if (!packet.results_are_fixture) {
      for (const d of packet.detections) {
        if (d.track_id !== null && d.track_id !== undefined) {
          this._activeTrackIds.add(d.track_id);
          this._sessionTrackIds.add(d.track_id);
        }
      }
    }
  }

  // Face packets are actual localized observations, not tracks or person boxes.
  onFacePacket(packet) {
    if (this._unavailable) return false;
    if (!["face", "person_face"].includes(this._mode) || !packet || packet.camera_id !== this._cameraId ||
        typeof packet.results_are_fixture !== "boolean" ||
        !Array.isArray(packet.detections) || packet.detections.length > 64 ||
        typeof packet.epoch !== "string" || !/^[1-9][0-9]{0,19}$/.test(packet.epoch) ||
        typeof packet.sequence !== "string" || !/^[1-9][0-9]{0,19}$/.test(packet.sequence)) return false;
    let incomingSession = null;
    if (this._mode === "person_face") {
      if (typeof packet.session !== "string" || !/^[1-9][0-9]{0,19}$/.test(packet.session) || BigInt(packet.session)>18446744073709551615n) return false;
      const session=BigInt(packet.session);
      if ((this._faceSession !== null && session<this._faceSession) || (this._presenceSession !== null && session<this._presenceSession)) return false;
      incomingSession=session;
    }
    const epoch = BigInt(packet.epoch), sequence = BigInt(packet.sequence);
    if (epoch > 18446744073709551615n || sequence > 18446744073709551615n) return false;
    if ((incomingSession === null || incomingSession === this._faceSession) && this._epoch !== null && (epoch < this._epoch ||
        (epoch === this._epoch && sequence <= this._sequence))) return false;
    for (const d of packet.detections) {
      if (!d || ![d.left, d.top, d.width, d.height, d.confidence].every(Number.isFinite) ||
          d.left < 0 || d.top < 0 || d.width <= 0 || d.height <= 0 ||
          d.left + d.width > 1.000001 || d.top + d.height > 1.000001 ||
          d.confidence < 0 || d.confidence > 1 || d.track_id !== undefined) return false;
    }
    if(incomingSession !== null) this._faceSession=incomingSession;
    this._epoch = epoch;
    this._sequence = sequence;
    this._lastPacket = packet;
    this._lastReceivedAt = this._now();
    return true;
  }

  onPresencePacket(packet) {
    if (this._mode !== "person_face" || !packet || packet.camera_id !== this._cameraId ||
        ![packet.available, packet.saturated, packet.results_are_fixture].every(v => typeof v === "boolean") ||
        ![packet.observed, packet.uncertain, packet.tentative].every(v => Number.isInteger(v) && v >= 0 && v <= 64) ||
        packet.observed + packet.uncertain + packet.tentative > 64 || (!packet.available && packet.observed !== 0)) return false;
    const valid = v => typeof v === "string" && /^[1-9][0-9]{0,19}$/.test(v) && BigInt(v) <= 18446744073709551615n;
    if (![packet.epoch, packet.session, packet.sequence].every(valid)) return false;
    const session=BigInt(packet.session), sequence=BigInt(packet.sequence);
    if(this._faceSession !== null && session<this._faceSession) return false;
    if (this._presenceSession !== null && (session < this._presenceSession || (session === this._presenceSession && sequence <= this._presenceSequence))) return false;
    if(this._presenceSession !== session && this._faceSession !== session) this._lastPacket=null;
    this._presenceSession=session; this._presenceSequence=sequence;
    this._presence=packet; this._presenceAt=this._now();
    return true;
  }

  setPersonUnavailable(value) { this._personUnavailable=value; }

  updateConfiguration(mode, maxAgeMs) {
    if (!["face", "person", "person_face"].includes(mode)) return;
    if (this._mode !== mode) {
      this._mode = mode;
      this._presence = null;
      this._presenceSession = null;
      this._faceSession = null;
      this._presenceSequence = null;
      this._lastPacket = null;
      this._epoch = null;
      this._sequence = null;
      this._activeTrackIds.clear();
      this._sessionTrackIds.clear();
    }
    if (Number.isFinite(maxAgeMs) && maxAgeMs > 0 && maxAgeMs <= 60000) this._maxAgeMs = maxAgeMs;
  }

  setUnavailable(value) {
    this._unavailable = value;
    if (value) {
      this._lastPacket = null;
      this._activeTrackIds.clear();
    }
  }

  faceUnavailable() {
    return this._unavailable || this.isFixture() || (!!this._lastPacket && this._now()-this._lastReceivedAt > this._maxAgeMs);
  }

  isFixture() {
    return !!(this._lastPacket && this._lastPacket.results_are_fixture);
  }

  // {active, cumulative} — distinct track IDs in the latest packet, and
  // distinct track IDs ever seen since this overlay was attached.
  personCounts() {
    const stale = this._now() - this._lastReceivedAt > this._maxAgeMs;
    if (this._mode === "person_face") {
      const p=this._presence;
      const available=!!p && p.available && !p.results_are_fixture && !this._personUnavailable && this._now()-this._presenceAt <= this._maxAgeMs;
      return {mode:"person_face", received:!!p, available, active:available ? p.observed : 0, uncertain:available ? p.uncertain : 0, tentative:available ? p.tentative : 0, saturated:!!p?.saturated, faces:stale || this.isFixture() ? 0 : (this._lastPacket?.detections.length || 0)};
    }
    if (this._mode === "face") return { mode: "face", active: stale || this.isFixture() ? 0 : (this._lastPacket?.detections.length || 0), cumulative: 0, received: this._lastPacket !== null };
    return {
      active: stale ? 0 : this._activeTrackIds.size,
      cumulative: this._sessionTrackIds.size,
    };
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
    if (this._now() - this._lastReceivedAt > this._maxAgeMs) return;
    if (packet.results_are_fixture) return;

    const rect = contentRect(this._video);
    if (!rect) return;

    const dpr = window.devicePixelRatio || 1;
    for (const det of packet.detections) {
      drawBox(ctx, det, rect, dpr);
    }
  }
}
