# Repository Status — 2026-08-16 01:51 +06

Commit at snapshot: `e65a318` ("docs: add Phase 8 handoff"). The worktree is
intentionally dirty: 67 changed or untracked paths, all Phase 8 and operator
dashboard work in flight. Nothing has been committed.

## Verified this session

`make check` run twice, both green. Final run:

```
cargo fmt --all --check                                            ok
cargo clippy --workspace --all-targets --all-features -- -D warnings  ok
cargo test --workspace --all-features   460 passed; 0 failed; 32 ignored
cargo build --workspace --release                                  ok
make audit                                                         ok
MAKE_EXIT=0
```

One allowed `cargo audit` warning: RUSTSEC-2025-0119, unmaintained
`number_prefix`, reached through `indicatif` ← `muxide` ← `surveillance-recorder`.
Not a vulnerability; recorded in `docs/phase-8/dependency-policy.md`.

Ignored tests are unchanged and still require FFmpeg/libx264, MediaMTX,
PostgreSQL, or a LiveKit server.

## What changed

**`apps/server/ui/dashboard.html`** — rebuilt to the reference operator design
as nine screens, and the Live View now owns N concurrent LiveKit rooms rather
than one. Both were user decisions ("honest empty states" and "multi-room
grid"); ADR 010 Decision 5 records the second.

**`apps/server/src/health_http.rs`** — two tests added:
`watch_all_only_targets_tiles_that_offer_a_watch_button` and
`the_dashboard_separates_an_unreadable_fleet_from_an_empty_one`. Both were
mutation-checked: each fails when the line it protects is reverted.

**`docs/phase-9/live-grid-tiles-2026-08-16.md`** — new; the tile-cost
measurement. **`docs/adr/010-…md`** Decision 5 amended to carry its result and
to restate that it is still not a tile-count claim. **`ROADMAP.md`** — two new
`[~]` items under "Operator dashboard".

## Defects found in the browser walk and fixed

All found by driving the page, not by reading it.

1. **An unreadable fleet rendered as a measured fleet of zero.** With a database
   configured, every `/api` call answers `401` and the dashboard sends no
   credential by design. The page turned that refusal into `0 / 0`,
   "no cameras configured", "no camera connected", "idle" on five surfaces, and
   pushed a `0` sample into the camera trend series. The System screen sat on
   "Loading capabilities…" forever, claiming a request was still in flight after
   it had already come back refused. `fleetCounts()` now carries
   `readable: state.fleetError === null`; every count, table, service row and
   the trend push is gated on it, and `state.systemError` gives the System
   screen a failure branch. This is the fix that mattered — the rest are
   cosmetic by comparison.
2. **The tile footer clipped its watch, stop and fullscreen controls at 5×
   density.** The LiveKit words were squeezed to 4–6px and rendered as two
   unreadable ellipsis fragments. Footers now carry `data-density`; at 4× and
   above the words are dropped and the status dot plus a `title` carry the
   state.
3. **"Watch all" joined rooms for cameras whose own tile offers no watch
   button.** A camera with publication disabled renders no button, but the bulk
   action selected every tile — issuing a viewer token and writing an audit row
   for a subscription that can never carry a track. `data-live-watchable` is now
   emitted from the same flag as the button, and the bulk action filters on it.
4. Camera identifiers in tables were real links rendered as plain text.
5. The idle overlay kept painting over a decoding video, because the user
   agent's `[hidden]` rule loses to any class rule that sets `display`.

## Measurement: what N live tiles cost

`docs/phase-9/live-grid-tiles-2026-08-16.md`. Host MacBookPro15,1, i9-9880H
@ 2.30 GHz 8c/16t, 32 GB, macOS 15.7.7, headless Chrome. Sixty-second windows,
cumulative CPU-seconds over each process tree divided by wall time
(`scripts/phase9/cpu-sample.py`) — macOS `ps -o %cpu` is a lifetime-decayed
average and would fold the connection storm into the steady state.

| Tiles subscribed | Browser CPU | Browser RSS | LiveKit CPU | Server CPU |
|---|---|---|---|---|
| 0 | 6.6 | 689 MB | 6.7 | 5.6 |
| 1 × 720p30 | 27.6 | 641 MB | 11.3 | 5.0 |
| 5 × 720p30 | 91.5 | 2916 MB | 29.8 | 4.5 |
| 5 × 720p30 + 1 × 360p (~1 fps) | 95.3 | 1765 MB | 37.3 | 5.9 |

Per tile over baseline: **≈0.17 core in the browser tree, ≈0.046 in the SFU**.
`surveillance-server` stayed between 4.5% and 5.9% of one core across every
configuration — the subscriber side is the binding constraint, which is what
ingest-once-distribute-many is supposed to produce.

**No tile-count claim is made.** Two homogeneous points, nothing above six, and
five of the six tracks came from an `lk --publish-demo` publisher rather than a
camera. RSS is deliberately not reported per tile: the readings are single
instants and not monotonic.

## Not verified

- **No automated browser test exists.** Everything above was driven manually in
  one browser, in one session. The roadmap items stay `[~]` for this reason.
- **Playback populated with real segments** and **the Settings write path**.
  Both need an authenticated session, which the dashboard cannot establish
  without a trust-boundary proxy in front of it.
- The Settings screen could not be screenshotted (4734px page); its content was
  verified through the DOM instead.
- Six *real* camera streams have never been on screen at once, and cannot be:
  `AddCameraRequest` carries no LiveKit field, so cameras added through the API
  are created with publication disabled.
