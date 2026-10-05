# Detection Overlay — Design Spec

**Status:** approved for implementation planning  
**Date:** 2026-10-04  
**Scope:** Phase 9 — bounding-box overlay on live view with stable tracked IDs

---

## 1. What this builds

A client-side canvas overlay that draws a rectangle around each detected person in
the live view and keeps the box on that person as they move. Detections travel from
the server to the browser over a LiveKit data channel. The client renders them on a
`<canvas>` element positioned over the `<video>` element.

Nothing is burned into the video stream. The recording pipeline is unaffected. The
overlay is invisible when the detector is not running in production mode.

---

## 2. Load-bearing constraints

These are the facts that determine whether the design works or breaks. Every
subsequent decision follows from them.

**C-1 · Clock domains are irreconcilable.**  
`TranscodePublicationAdapter` reports `TimestampAuthority::AdapterClock`
(`crates/livekit/src/adapter.rs:162`). The codec's B-frame reordering breaks the
one-to-one mapping between an access unit's RTP timestamp and the frame that leaves
the encoder. Detection `SourceMediaTime` values are in the camera's RTP clock
domain. The published video stream is in the adapter's clock domain. There is no
bridge between them, and there is no safe one to construct.

Consequence: **frame-accurate overlay alignment is not achievable on this path.**
The overlay operates on recency: the client displays the most recently received
detection set provided it arrived within the staleness window (default 3 s). This
is the correct decision for surveillance. When a person is walking, a ~200 ms lag
between the detection and the frame it was inferred from is not perceptible; what
is perceptible is a box that jumps on arrival and teleports on the next arrival.
Recency + interpolation handles that. Frame-accurate sync does not.

**C-2 · The viewer token does not grant `can_publish_data`.**  
`crates/livekit/src/token.rs` sets `can_publish_data: false` on both viewer and
publisher tokens. The publisher token must be changed to `can_publish_data: true`
before `publish_data` calls will succeed. This is a one-line change; it is called
out here because it is a silent failure otherwise.

**C-3 · Detection rate is far below frame rate.**  
Sampling + motion gating reduces the inference rate to roughly 1–5 Hz. Drawing
only on arrival makes boxes teleport. The tracker provides stable IDs across frames;
coasting through missed detections (holding the last known position for up to
`max_frames_missing` frames) smooths the box trajectory through gaps. This is not
motion prediction — it is position holding. Kalman prediction is a future upgrade.

**C-4 · The fixture guard is non-negotiable.**  
`DeterministicFakeInference` fabricates detections. Drawing fabricated boxes over
live video would misrepresent system capability. The guard has two layers: the
server includes `results_are_fixture: true` in every data packet, and the client
discards any packet where that field is true before touching the canvas.

**C-5 · The video element uses `object-fit: contain`.**  
Confirmed in `apps/server/ui/styles/screens.css:15`. Black bars appear whenever the
camera aspect ratio differs from the tile. Normalized coordinates `[0..1]²` are
relative to the camera frame, not the element. The client must compute the content
rect (the area the actual video occupies, excluding bars) and map coordinates into
that region only.

---

## 3. What exists today

| Component | Location | State |
|-----------|----------|-------|
| `NormalizedBoundingBox` (`left`, `top`, `width`, `height`: `f32` in `[0..1]`) | `crates/detector/src/event.rs:169` | ✓ exists |
| `PersonDetection` (`confidence`, `bounding_box`) | `crates/detector/src/event.rs:226` | ✓ exists — gains `track_id` |
| `DetectionSet` (validated `Vec<PersonDetection>`, cap 64) | `crates/detector/src/event.rs:234` | ✓ exists |
| `SourceMediaTime` (`epoch`, `ticks`, `timebase_hz`) on every detection event | `crates/events/src/event.rs:131` | ✓ exists |
| `EventBus` in-process pub/sub, per-class bounded queues | `crates/events/src/bus.rs` | ✓ exists |
| `PersonDetectionObserved` domain event with `DetectionSet` + `SourceMediaTime` | `crates/events/src/event.rs` | ✓ exists |
| LiveKit room session, `publish_data` API | `crates/livekit/src/` | ✓ SDK present; **not yet used** |
| Live view client with `<video>` tiles | `apps/server/ui/js/screens/live.js` | ✓ exists — gains overlay |
| `results_are_fixture` in `GET /api/system` response | `apps/server/src/health_http.rs:737` | ✓ exists |
| `PersonTracker` | — | **new** |
| `DetectionRelay` | — | **new** |
| `detection-overlay.js` | — | **new** |

---

## 4. New components

### 4.1 PersonTracker (`crates/detector/src/tracker.rs`)

The tracker sits between inference and event publication inside `DetectionOwner`.
It receives a raw `DetectionSet` and returns a list of `(TrackId, PersonDetection)`
pairs that includes both matched detections and coasting tracks.

**Types**

```rust
/// A stable per-session identifier assigned to one tracked person.
/// Starts at 1; 0 is reserved and not issued. Wraps at u32::MAX (not expected
/// in practice — a session would need to track ~4 billion distinct entrances).
pub type TrackId = NonZeroU32;

pub struct TrackerConfig {
    /// Minimum IoU for a new detection to be assigned to an existing track.
    /// Default 0.3. Below this threshold the detection starts a new track.
    pub min_iou: f32,
    /// Number of consecutive frames a track may be unmatched before retirement.
    /// Default 5. At 2 Hz sampling that is 2.5 s of coasting.
    pub max_frames_missing: u32,
}

impl Default for TrackerConfig {
    fn default() -> Self { Self { min_iou: 0.3, max_frames_missing: 5 } }
}

pub struct PersonTracker {
    tracks: Vec<TrackState>,
    next_id: u32,           // next value to issue; starts at 1
    config: TrackerConfig,
}

struct TrackState {
    id: TrackId,
    last_box: NormalizedBoundingBox,
    last_confidence: Confidence,
    frames_missing: u32,
}
```

**Algorithm (`PersonTracker::update`)**

Input: `&DetectionSet` (the raw inference output for one frame).  
Output: `Vec<(TrackId, PersonDetection)>` — at most 64 entries total (same cap as
`DetectionSet::MAX_DETECTIONS_PER_EVENT`); includes both matched and coasting tracks.
If the combined count of matched + coasting tracks would exceed 64, drop coasting
tracks in order of highest `frames_missing` first (oldest coasting goes first).

1. **Match matrix.** Compute IoU between every existing track's `last_box` and
   every incoming detection. IoU is the standard intersection-over-union on
   `NormalizedBoundingBox`:
   ```
   inter_left   = max(a.left,          b.left)
   inter_top    = max(a.top,           b.top)
   inter_right  = min(a.left + a.width,  b.left + b.width)
   inter_bottom = min(a.top  + a.height, b.top  + b.height)
   inter_area   = max(0, inter_right - inter_left)
                * max(0, inter_bottom - inter_top)
   union_area   = a.width * a.height + b.width * b.height - inter_area
   iou          = if union_area > 0.0 { inter_area / union_area } else { 0.0 }
   ```

2. **Greedy match.** Sort (track_index, detection_index) pairs by IoU descending.
   Iterate; assign a pair if neither side has been assigned yet AND
   `iou >= config.min_iou`. O(n²) is correct here; n ≤ 64.

3. **Update matched tracks.** Set `last_box`, `last_confidence` from the detection;
   reset `frames_missing` to 0.

4. **Coast unmatched tracks.** Increment `frames_missing`. Tracks where
   `frames_missing <= config.max_frames_missing` are retained with their last known
   position and included in the output. Tracks that exceed the threshold are
   removed from `self.tracks` and their IDs are never reused in this session.

5. **Create new tracks** for unmatched incoming detections. Assign `next_id`,
   increment it (wrapping); push to `self.tracks`.

6. **Return.** All tracks in `self.tracks` (matched + coasting), each paired with a
   `PersonDetection` built from the track's current `last_box` and
   `last_confidence`. The `track_id` field on `PersonDetection` carries the ID.

**Integration point in `DetectionOwner`**

`PersonDetection` gains one field:

```rust
pub struct PersonDetection {
    pub confidence: Confidence,
    pub bounding_box: NormalizedBoundingBox,
    /// Stable cross-frame identity. `None` when the tracker is not running.
    pub track_id: Option<TrackId>,
}
```

In `DetectionOwner::run`, after inference and before publishing to the event bus,
pass the `DetectionSet` through `self.tracker.update(detections)` and reconstruct
the set from the returned tracked detections. The event bus receives tracked IDs.
Coasting detections (from `frames_missing > 0`) appear in the published event with
their last known positions; downstream consumers that do not want coasted detections
may check whether the position changed.

**Tracker instance lifetime.** One `PersonTracker` per `DetectionOwner` instance.
The tracker is always active in v1; there is no configuration flag to disable it.
IDs are stable within a detection session; they reset when the detection owner
restarts (camera reconnect, detector restart). This is acceptable — a reconnect
resets the visual session anyway.

### 4.2 DetectionRelay (`crates/livekit/src/relay.rs`)

The relay is a sub-task owned by `Session` inside `TranscodePublicationAdapter`.
It subscribes to `PersonDetectionObserved` events from the `EventBus`, serializes
each to the wire format (§5), and sends it over the LiveKit room via `publish_data`.

**Task lifecycle**

The relay is started when `Session` connects (`Session::connect` or equivalent)
and cancelled when the session ends. It does not gate on viewer count — the cost of
sending ~300-byte JSON packets at 1–5 Hz when no viewer is present is negligible,
and checking viewer count introduces a window where an early viewer misses a frame.

**Shutdown contract.** The relay task is owned by a `JoinHandle` stored in
`Session`. When `stop_publishing` is called, the relay's `shutdown` channel is
signalled and `self.relay_handle.await` is called. The relay must be cancellation-
safe (it is — the select loop has no cross-await state).

**Structure**

```rust
pub struct DetectionRelay {
    event_sub: EventSubscription,
    room: Room,
    camera_id: CameraId,
}

impl DetectionRelay {
    pub fn new(event_sub: EventSubscription, room: Room, camera_id: CameraId) -> Self { ... }

    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                result = self.event_sub.receive() => {
                    match result {
                        Ok(event) => self.handle_event(event).await,
                        Err(_) => break,  // subscription closed
                    }
                }
            }
        }
    }

    async fn handle_event(&self, event: DomainEvent) {
        let EventPayload::PersonDetectionObserved { detections, .. } = &event.payload
        else { return };
        let msg = DetectionPacket::from_event(&event, &self.camera_id);
        let json = serde_json::to_vec(&msg).unwrap_or_default();
        let _ = self.room.local_participant()
            .publish_data(json, DataPacketKind::Lossy, true, vec![], Some(DETECTION_TOPIC))
            .await;
    }
}
```

`DETECTION_TOPIC = "det/v1"` — a string constant that lets the JS client filter
data packets without parsing every message.

**Token change required.**  
`crates/livekit/src/token.rs`: change `can_publish_data: false` to
`can_publish_data: true` in `PublisherToken::build`. The viewer token retains
`can_publish_data: false` (viewers receive, not send).

**Overflow policy.** If the detection event subscription is slow to drain, the
event bus drops the oldest detection events (per the existing `Detection`-class
overflow policy). The relay does not add backpressure. A dropped event means one
missed data packet; the client's staleness window absorbs that.

### 4.3 Client overlay (`apps/server/ui/js/detection-overlay.js`)

A self-contained ES module. Instantiated once per live tile; attached to the
`<video>` element when watching starts, detached when watching stops.

**Overlay element.** A `<canvas>` element inserted immediately after the
`<video>` element, with `position: absolute; inset: 0; pointer-events: none`.
The canvas does not intercept clicks.

**Size and resize.** A `ResizeObserver` on the video element keeps the canvas
the same pixel size as the element, accounting for `devicePixelRatio`:

```javascript
const { width, height } = video.getBoundingClientRect();
canvas.width  = Math.round(width  * devicePixelRatio);
canvas.height = Math.round(height * devicePixelRatio);
canvas.style.cssText = `position:absolute;inset:0;width:${width}px;height:${height}px;pointer-events:none`;
```

**Content rect (letterbox math).** `object-fit: contain` centers the video within
the element, adding bars on two sides when aspect ratios differ. The normalized box
coordinates are relative to the video frame, not the element. Transform:

```javascript
function contentRect(video) {
    const elW = video.clientWidth,  elH = video.clientHeight;
    const vW  = video.videoWidth,   vH  = video.videoHeight;
    if (vW === 0 || vH === 0) return null;
    const scale = Math.min(elW / vW, elH / vH);
    const cW = vW * scale, cH = vH * scale;
    return {
        x: (elW - cW) / 2,
        y: (elH - cH) / 2,
        w: cW,
        h: cH,
        dpr: devicePixelRatio,
    };
}
```

All canvas coordinates are multiplied by `rect.dpr` to draw at physical pixels.

**Rendering loop.** A `requestAnimationFrame` loop runs while the overlay is
attached. Each frame:

1. Compute `contentRect`; skip if null (video not ready).
2. Check staleness: if `Date.now() - lastReceivedAt > MAX_AGE_MS`, clear and return.
3. Check fixture guard: if `lastPacket.results_are_fixture`, clear and return.
4. For each detection in `lastPacket.detections`, call `drawBox`.
5. Schedule next frame.

`MAX_AGE_MS = 3000`. If no detection packet arrives for 3 s (camera stopped,
detector degraded, relay crashed), the canvas goes blank rather than showing stale
boxes indefinitely.

**Box rendering:**

```javascript
function drawBox(ctx, det, rect) {
    const x = (rect.x + det.left  * rect.w) * rect.dpr;
    const y = (rect.y + det.top   * rect.h) * rect.dpr;
    const w = det.width  * rect.w * rect.dpr;
    const h = det.height * rect.h * rect.dpr;
    const alpha = 0.4 + det.confidence * 0.6;   // dim at low confidence

    ctx.strokeStyle = `rgba(40, 220, 100, ${alpha})`;
    ctx.lineWidth   = 2 * rect.dpr;
    ctx.strokeRect(x, y, w, h);

    if (det.track_id !== null) {
        ctx.font      = `${11 * rect.dpr}px monospace`;
        ctx.fillStyle = ctx.strokeStyle;
        ctx.fillText(`#${det.track_id}`, x + 4 * rect.dpr, y + 14 * rect.dpr);
    }
}
```

**Fixture indicator.** When `results_are_fixture` is true, the canvas is blank
and a non-intrusive badge reads "detection unavailable" in the top-right corner of
the tile. This badge is separate from the canvas (a DOM element), always present,
and toggled by a CSS class. It must not appear during normal operation — the class
is removed as soon as the first non-fixture packet arrives.

**LiveKit data channel wiring.** In `apps/server/ui/js/livekit.js`, after the room
connects, subscribe to `RoomEvent.DataReceived`:

```javascript
room.on(RoomEvent.DataReceived, (payload, participant, kind, topic) => {
    if (topic !== 'det/v1') return;
    const packet = JSON.parse(new TextDecoder().decode(payload));
    overlay.onPacket(packet);
});
```

`overlay.onPacket` stores `lastPacket` and `lastReceivedAt = Date.now()`. The rAF
loop picks it up on the next frame.

---

## 5. Wire format

One JSON object per detection event. Sent over the LiveKit data channel with
`DataPacketKind.Lossy` (unreliable, unordered). Topic string: `"det/v1"`.

```typescript
interface DetectionPacket {
    // Camera this detection belongs to.
    camera_id: string;

    // Connection epoch from SourceMediaTime. Used to detect stream resets.
    epoch: number;           // u64 encoded as number; safe ≤ 2^53

    // RTP clock ticks and timebase of the inferred frame.
    // Informational only — the client cannot align these to rendered frames
    // because the published video stream uses an independent adapter clock
    // (see constraint C-1). Included for logging and future use.
    media_ticks:  number;    // i64 encoded as number
    timebase_hz:  number;    // u32

    // If true the client MUST NOT render any boxes. The server includes this
    // field in every packet so the client can enforce the guard per-packet
    // without polling /api/system.
    results_are_fixture: boolean;

    // Detections for this frame. May be empty (no persons detected).
    // Includes coasted tracks (frames_missing > 0) at their last known position.
    detections: DetectionEntry[];
}

interface DetectionEntry {
    // Stable cross-frame identity. null when tracker is not running.
    track_id:   number | null;

    // NormalizedBoundingBox — all values in [0.0, 1.0] of the camera frame.
    left:       number;
    top:        number;
    width:      number;
    height:     number;

    // Model confidence in [0.0, 1.0].
    confidence: number;
}
```

**Size.** A typical packet with two detections is ~220 bytes. At 5 Hz across 20
cameras that is 22 KB/s per viewer — negligible.

**Versioning.** The topic `"det/v1"` is the version discriminator. A future
breaking change increments to `"det/v2"` and both topics can coexist during a
transition.

---

## 6. Staleness and alignment

The client holds the single most recent `DetectionPacket` per camera. On each
`requestAnimationFrame` tick it checks two conditions:

| Condition | Source | Action if true |
|-----------|--------|----------------|
| `Date.now() - lastReceivedAt > 3000` | client clock | clear canvas, do not draw |
| `packet.results_are_fixture === true` | server field | clear canvas, show badge |

If both conditions are false, draw all detections in `packet.detections`.

**Why 3 s.** At 1 Hz detection rate, the expected inter-packet gap is 1 s. The 3 s
window allows two consecutive missed packets (dropped network, transient overload)
before the overlay goes blank, without persisting stale boxes through a significant
pause (person left frame, detector degraded). This is a configurable constant
`DETECTION_MAX_AGE_MS` in `detection-overlay.js`.

**Coasting duration.** `PersonTracker.config.max_frames_missing = 5`. At a 2 Hz
sample rate that is 2.5 s of coasting before a track is retired. The client's 3 s
staleness window is slightly longer, so a coasted position will still be visible
until the track retires and the position disappears from the packet.

---

## 7. Files changed or created

```
crates/detector/src/event.rs          PersonDetection gains track_id: Option<TrackId>
crates/detector/src/tracker.rs        NEW — PersonTracker, TrackerConfig, TrackId
crates/detector/src/lib.rs            pub mod tracker
crates/detector/src/detector.rs       DetectionOwner gains PersonTracker; calls update()
crates/livekit/src/relay.rs           NEW — DetectionRelay
crates/livekit/src/lib.rs             pub mod relay
crates/livekit/src/transcode.rs       Session gains relay_handle; starts/stops relay
crates/livekit/src/token.rs           publisher token: can_publish_data = true
apps/server/src/                      wire EventSubscription into relay construction
apps/server/ui/js/detection-overlay.js  NEW
apps/server/ui/js/livekit.js          subscribe RoomEvent.DataReceived, call overlay
apps/server/ui/js/screens/live.js     instantiate DetectionOverlay per tile
apps/server/ui/styles/screens.css     position canvas over video tile
```

No new crates. No changes to the recording path, event bus schema, or camera
lifecycle.

---

## 8. Testing

**`PersonTracker` unit tests** (`crates/detector/src/tracker.rs` inline tests):

- IoU of two identical boxes is 1.0; two non-overlapping boxes is 0.0.
- One person tracked across five frames with stable ID.
- Two persons tracked; IDs do not swap when boxes move apart and back.
- Person exits (no detection): ID coasts for `max_frames_missing` frames then
  retires; the retired ID is never reused.
- Person re-enters after retirement: assigned a new (higher) ID.
- Frame with zero detections: all existing tracks coast; none are matched.
- Frame with more detections than existing tracks: new tracks created for excess.

**`DetectionRelay` integration test** (new, lives beside existing room_fixture.rs
pattern or a separate `relay_fixture.rs`):

- Relay sends a `det/v1` data packet when a `PersonDetectionObserved` event is
  published; packet parses as valid `DetectionPacket`.
- Relay sends `results_are_fixture: true` in the packet when the detection is from
  `DeterministicFakeInference`; client-side guard asserted separately.
- Relay stops sending after shutdown signal; no panic on drop.

**Client tests** (`apps/server/ui/js/detection-overlay.test.js`, plain JS,
no framework required):

- `contentRect` returns the correct letterbox region for a 16:9 camera in a 4:3
  container, a 4:3 camera in a 16:9 container, and a 1:1 container.
- `drawBox` maps `(0,0,1,1)` to the full content rect and `(0.5,0.5,0.1,0.1)` to
  the correct pixel position within it.
- A packet with `results_are_fixture: true` clears the canvas rather than drawing.
- A packet older than `MAX_AGE_MS` clears the canvas rather than drawing.
- A `track_id: null` detection draws a box without an ID label; a numeric ID draws
  the label.

---

## 9. Non-requirements for this spec

The following are explicitly out of scope:

- **Kalman prediction.** Coasting at last known position is the v1 prediction
  strategy. Kalman-filter motion prediction is a named upgrade path but not part
  of this slice.
- **Configurable confidence threshold.** The model's confidence is encoded in
  box opacity; there is no UI slider in v1.
- **Per-person colour coding.** All boxes are the same colour (green). Assigning
  distinct colours per `track_id` is a future cosmetic change.
- **ROI zone visualisation.** ROI geometry does not exist yet. Overlaying detection
  zones is a separate spec.
- **Track IDs persistent across sessions.** IDs reset on every camera reconnect.
  Cross-session identity requires a persistent store and is outside this slice.
- **Accuracy claims.** The 73.7% recall / 89.7% precision figures are from COCO
  photographs against a debug build. They are regression guards, not grades. Boxes
  will visibly drop out on real footage; that is the model, not the overlay.
- **Multi-camera ID unification.** The same person appearing on two cameras is two
  separate tracks with two separate IDs. Cross-camera re-identification is outside
  scope.
- **Frame-accurate timestamp alignment.** Impossible on this architecture (C-1).
  If the adapter clock constraint is ever removed, the `media_ticks` field in the
  wire format is already there for the client to use.

---

## 10. Open questions resolved by this spec

| Question | Resolution |
|----------|------------|
| Is tracking required? | Yes. In-session IoU tracker in v1. |
| Where does the tracker live? | `crates/detector/src/tracker.rs`, between inference and event publication. |
| Coordinate system for normalised boxes? | Camera-frame `[0..1]²`; already the format of `NormalizedBoundingBox`. |
| Data transport? | LiveKit `publish_data` (lossy), topic `"det/v1"`. |
| Frame-accurate sync? | Not achievable (adapter clock). Recency-based with 3 s staleness window. |
| `results_are_fixture` enforcement? | Server includes field in every packet; client enforces on every frame. |
