# CLAUDE.md

The full engineering policy for this repository lives in `AGENTS.md`. Read it —
it is the governing document for architecture, Rust design rules, domain
requirements, and evidence standards.

@AGENTS.md

This file adds what `AGENTS.md` cannot: verified commands, the current state of
the tree, and the rules that get broken most often here.

## Commands

```bash
make check    # the full gate: fmt --check, clippy -D warnings, test, release build
make help     # manual development targets and their overridable variables
```

`make check` is the completion gate. Run it before claiming a phase or slice is
done. Individual steps:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
```

Tests that need external dependencies are `#[ignore]`d, not deleted. Run them
explicitly and report which ones you could not run:

```bash
cargo test -p surveillance-detector --test ffmpeg_decode -- --ignored  # needs ffmpeg + libx264
scripts/phase0/retina-rtsp-fixture.sh    # needs MediaMTX (also `make fixture`)
scripts/phase3/multi-camera-fixture.sh   # ten synthetic sources
scripts/phase4/postgres-fixture.sh       # needs PostgreSQL
scripts/phase5/livekit-fixture.sh        # needs a LiveKit server
```

Local media loop: `make mediamtx`, then `make webcam`, then `make server`, then
`make health` / `make verify-recordings`.

## Repository reality

`AGENTS.md` §"Current Repository Reality" carries the verified state of the tree
and its last-verified date. Read it there; re-verify before relying on a specific
claim. `ROADMAP.md` is the authority on per-phase status.

### Where code actually lives

| Path | State |
|---|---|
| `apps/server` | The runnable binary. Pipeline wiring, HTTP health/metrics/API, retention owner. Largest crate. |
| `crates/camera` `media` `recorder` `storage` `database` `events` `detector` `livekit` `config` `common` | Real implementations. |
| `crates/api` `analytics` `rules`, `apps/worker` | Empty stubs. The HTTP surface is in `apps/server/src/health_http.rs`, not `crates/api`. |
| `src/` | The original `cam_rtsp` prototype. Reference only — not an architectural constraint. |
| `docs/adr/` | Accepted decisions. Outrank this file and `analysis.md`. |
| `docs/phase-N/` | Per-phase contracts and evidence. |
| `context/YYYY-MM-DD[-HHMM]/` | Dated session snapshots; `context/README.md` points at the latest. |

Detection currently runs `DeterministicFakeInference` — the only implementation
of the `PersonInference` trait. There is no production model. Do not describe
detection as a working capability; it is a wired seam with a fixture behind it.

## Working conventions

- Update `ROADMAP.md` checkboxes only with evidence, using its legend
  (`[ ]` `[~]` `[x]` `[!]`). `[!]` is for items blocked on external hardware or
  a product decision — prefer it over silently leaving an item unchecked.
- Put session state in a new dated `context/` snapshot and repoint
  `context/README.md`. Never rewrite an old snapshot to look current.
- The worktree is often intentionally dirty with in-flight phase work. Preserve
  it. Do not commit unless asked.
- A material or hard-to-reverse choice needs an ADR in `docs/adr/` with options,
  trade-offs, decision, and reversal criteria.

## Rules that get broken here

These are drawn from `AGENTS.md`; they are repeated because they are the ones
that actually go wrong in this codebase.

- **One upstream connection per camera.** Adding a consumer must never add an
  RTSP connection. Consumers attach to the media distributor.
- **Every queue is bounded** with a documented capacity, overload policy, and
  metric. Recording preserves and surfaces backlog; detection and live viewing
  drop stale items to stay fresh.
- **No media on the domain event bus.** No packets, no frames, no unbounded
  binary payloads.
- **No detached tasks.** Every spawned task has an owner, a shutdown signal, and
  a join/error policy. Never block a Tokio worker with process, filesystem,
  database, or media work.
- **No fabricated metadata.** Derive codec, dimensions, timestamps, and size
  from actual media and filesystem state.
- **No `unwrap`/`expect`/`panic!`** in production paths without a locally proven
  and documented invariant.
- **Never weaken or delete a test to get a green run.** If something cannot be
  verified, say exactly what remains unverified.
- **No capacity claims from architecture.** A camera-count claim requires a
  documented workload and repeatable measurements.

## Reporting

State what was measured and what was assumed, separately. Name the exact command
and its result. If a check could not run — missing camera, GPU, LiveKit, or
database — say so explicitly rather than omitting it. Never present an
uncompiled or unrun path as working.
