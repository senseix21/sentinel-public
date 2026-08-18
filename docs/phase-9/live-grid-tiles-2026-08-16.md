# What N concurrent live tiles cost the dashboard — 2026-08-16

ADR 010 Decision 5 committed the dashboard's Live View to a grid of concurrent
LiveKit rooms and refused to state a tile count until one was measured. This is
that measurement. **It is not a tile-count recommendation and not a capacity
claim.** It records what four configurations cost on one machine, so that a
later decision has numbers under it instead of architecture.

## Host

| | |
|---|---|
| Machine | MacBookPro15,1 |
| CPU | Intel Core i9-9880H @ 2.30 GHz, 8 cores / 16 threads |
| Memory | 32 GB |
| OS | macOS 15.7.7 |
| Browser | Google Chrome, launched by Playwright, headless |
| LiveKit | `livekit-server` from `scripts/phase9/livekit-load-fixture.sh` |
| `lk` CLI | 2.18.2 |

## What was actually subscribed to

`scripts/phase9/livekit-load-fixture.sh` with `CAMERAS=6 VIEWERS=0 WINDOW=1800`
brings up six camera pipelines, but only `camera-01` publishes to LiveKit — the
constraint recorded in `docs/phase-9/livekit-load-2026-08-15.md`:
`AddCameraRequest` carried no LiveKit field, so cameras added through the API
were created with publication disabled. Six *camera* streams therefore could not
be put on the screen at all when this run was taken.

That limitation was removed after this run: `AddCameraRequest` now carries
`live_publication_enabled: Option<bool>`, and a camera added through the form
with publication on was observed publishing — see
`docs/phase-9/authenticated-ui-2026-08-16.md`. **The numbers below were not
re-measured on that binary and do not change**: what follows is still five demo
publishers and one camera.

To get N real tracks into the page anyway, five `lk room join --publish-demo`
publishers were attached to the rooms the dashboard subscribes to
(`loadtest-camera-0N-live`, per `crates/livekit/src/domain.rs:117`). So:

- Tiles 2–6 carry the `lk` **demo** loop at **1280×720, 30 fps**, confirmed
  from `getRTCStatsReport()` on each subscribed track.
- Tile 1 carries the real `camera-01` publication at **640×360**, which under
  the live-queue decimation described in the 2026-08-15 run delivers roughly one
  frame per second.

This measures **the browser and SFU cost of N subscribed tiles**, which is what
the grid decision turns on. It does **not** measure the server's per-camera
publish cost at N — that path does not exist yet.

Because the demo tiles are 720p30 and the camera publishes 360p at about 1 fps,
these numbers are an **upper bound** relative to what this pipeline produces
today, and a **lower bound** relative to a fleet of real 720p30 cameras that
also pay the publish-side cost.

## Method

`scripts/phase9/cpu-sample.py` takes cumulative CPU-seconds for a process and
its children at two instants and divides by the wall time between them, because
macOS `ps -o %cpu` is a lifetime-decayed average and would fold connection
storms into a steady-state number. Each configuration was held for a 60-second
window after tracks were confirmed decoding. RSS is a single reading at the end
of the window, summed over the browser process tree.

Three trees were sampled: the Playwright Chrome root and its children, the
`livekit-server` process, and the `surveillance-server` process and its children.

## Results

CPU is percent of one core; 100 means one core saturated for the whole window.

| Tiles subscribed | Browser CPU | Browser RSS | LiveKit CPU | LiveKit RSS | Server CPU |
|---|---|---|---|---|---|
| 0 | 6.6 | 689 MB | 6.7 | 170 MB | 5.6 |
| 1 × 720p30 | 27.6 | 641 MB | 11.3 | — | 5.0 |
| 5 × 720p30 | 91.5 | 2916 MB | 29.8 | — | 4.5 |
| 5 × 720p30 + 1 × 360p (~1 fps) | 95.3 | 1765 MB | 37.3 | — | 5.9 |

Deltas over the zero-tile baseline, on the two comparable homogeneous points:

| | 1 tile | 5 tiles | per tile |
|---|---|---|---|
| Browser CPU | +21.0 | +84.9 | ≈ 17 points ≈ 0.17 core |
| LiveKit CPU | +4.6 | +23.1 | ≈ 4.6 points ≈ 0.046 core |

## What the numbers say

**The browser is the binding constraint, not the server.** Across every
configuration the `surveillance-server` process stayed between 4.5% and 5.9% of
one core — subscribing viewers cost it nothing measurable, which is what the
ingest-once-distribute-many topology is supposed to produce. The cost lands in
the subscriber's decode path and, secondarily, in the SFU.

At roughly 0.17 core per 720p30 tile, five tiles already consume about 0.92 of
one core in the browser tree on this host. Chrome spreads decode across
processes, so this is not a single-core wall, but it does mean a 5×N grid of
720p30 tiles is a client-side sizing question that the operator's machine
answers — not something this server can promise.

**RSS is not reported per tile.** The readings are not monotonic (2916 MB at
five tiles, 1765 MB at six) because they are single instants against Chrome's
own GC timing. Memory grows substantially with tiles; how much per tile is not
established by four single readings and no claim is made.

## What this does not establish

- No tile-count recommendation. Two homogeneous points do not establish that
  the per-tile cost stays linear past five, and nothing above six was run.
- Nothing about real cameras at 720p30. The only real camera stream in the run
  was 360p at about 1 fps.
- Nothing about the server at N publishing cameras. That was unreachable when
  this run was taken; `AddCameraRequest` can carry LiveKit configuration now,
  but no multi-publishing-camera run has been measured on it.
- Nothing about a non-headless browser, a different GPU or decode path, or any
  machine other than the one above.
- No latency percentile. `framesDecoded`, `frameWidth`, `frameHeight` and
  `framesPerSecond` were read to confirm the tracks were live; end-to-end glass
  latency was not instrumented.

## Reproducing

```bash
CAMERAS=6 VIEWERS=0 WINDOW=1800 scripts/phase9/livekit-load-fixture.sh &
# once http://127.0.0.1:3300/live answers, attach demo publishers:
for n in 02 03 04 05 06; do
  lk room join --url ws://127.0.0.1:7880 --api-key devkey \
    --api-secret development-secret-of-at-least-32-bytes \
    --identity "demo-pub-$n" --publish-demo "loadtest-camera-$n-live" &
done
# open http://127.0.0.1:3300/#/live, press watch on each tile, then:
python3 scripts/phase9/cpu-sample.py start /tmp/s.json <chrome> <livekit> <server>
python3 scripts/phase9/cpu-sample.py stop  /tmp/s.json <chrome> <livekit> <server>
```
