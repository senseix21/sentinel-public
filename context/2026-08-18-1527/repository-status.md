# Repository status — 2026-08-18 15:27 +06

Base commit: `e65a318` (`docs: add Phase 8 handoff`), branch
`phase-8-handoff-and-config-metrics`, 8 commits ahead of `main`. **No source file
changed this session and nothing was committed.**

This snapshot exists for two reasons: the full gate was re-run against the tree
as it stands today, and an audit of that tree found a risk that outranks every
engineering item carried in `next-actions.md`.

Previous snapshot: `context/2026-08-17-2159/`. Its measurements are not repeated
here. Everything it recorded still stands, and all ten of its open items are
carried forward unchanged.

## What this session did, precisely

Ran `make check` on the dirty worktree, and audited what in that worktree is
actually under version control. No code was written, no test added, no
measurement taken of the running system. Read this as an inventory, not as
progress.

## Verified — full gate

`make check`, exit 0, on the working tree at 2026-08-18 15:27 +06:

| Step | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --workspace --all-features` | **43 suites, 509 passed, 0 failed, 34 ignored** |
| `cargo build --workspace --release` | succeeded |
| `make audit` | `cargo deny`: advisories ok, bans ok, licenses ok, sources ok. `cargo audit`: 461 crate dependencies scanned against 1217 advisories, **1 allowed warning** — `number_prefix` 0.4.0 unmaintained, RUSTSEC-2025-0119, reached via `indicatif` 0.17.11 ← `muxide` 0.2.5 ← `surveillance-recorder` |

Identical to the previous snapshot: 509 passed, 0 failed, 34 ignored. The 34
ignored tests are the ones needing FFmpeg/libx264, MediaMTX, PostgreSQL, or a
LiveKit server; their fixtures are in `scripts/phaseN/`. None of them ran.

`AGENTS.md` §"Current Repository Reality" still records **508** passed. It has
been one test stale since the reconnect regression test landed on 2026-08-17.
Correct it when this tree is committed, not before — the number in `AGENTS.md`
should describe a state someone else can check out.

## The dominant risk: this working tree is the only copy of five phases of work

`make check` passing above says nothing about what is *durable*. The gate ran
against files that mostly do not exist in git.

| | count |
|---|---|
| tracked files modified, unstaged | 53 (+6712 / −703) |
| staged | 1 rename: `crates/config/src/secret.rs` → `crates/common/src/secret.rs` |
| untracked files | 93, including **8124 lines of Rust** |

What is untracked is not scratch work. It is the implementation:

| Untracked path | What it is |
|---|---|
| `crates/auth/` | The entire Phase 8 auth crate — `clock.rs`, `lib.rs`, `principal.rs`, `store.rs`, `token.rs`, `verify.rs` |
| `apps/server/src/authenticate.rs`, `trust_boundary.rs`, `security_config.rs` | Bearer verification, the proved-origin proxy boundary, security configuration |
| `apps/server/ui/` | The operator dashboard, `dashboard.html`, 2424 lines |
| `migrations/0003_auth.sql`, `0004_access_audit.sql`, `0005_retire_livekit_cli_settings.sql` | Three of the five migrations |
| `crates/database/src/auth.rs`, `access_audit.rs` | Identity and access-audit repositories |
| `crates/livekit/src/native.rs`, `decode.rs`, `transcode.rs`, `build.rs` | ADR 011's pass-through path and ADR 012's transcode adapter |
| `crates/config/src/secret_store.rs`, `crates/media/src/sps.rs` | Secret store, SPS parsing |
| `docs/adr/009` … `012` | Identity/trust boundary, dashboard LiveKit client, native encoded publication, viewer-gated transcode |
| `docs/phase-5/`, `docs/phase-8/`, `docs/phase-9/` | Specs, plans, evidence, threat model, runbooks, dashboards, `alerts.yml` |
| `scripts/dev/`, `scripts/phase8/`, `scripts/phase9/`, `scripts/phase5/sps-corpus-fixture.sh` | The local stack and every Phase 8/9 fixture |
| `deny.toml`, `.cargo/audit.toml`, `config/last-known-good.toml` | Gate configuration (see below) |
| `context/2026-08-15-2300/` … `2026-08-17-2159/` | Six handoff snapshots, including the two `context/README.md` points readers at |

Three consequences, each checkable:

1. **`git checkout .`, `git clean -fd`, or a stash mishap destroys Phase 8.**
   There is no second copy. Nothing here has ever been pushed.
2. **`context/README.md` points at snapshots that do not exist in git.** The
   handoff chain the next engineer is told to read resolves only on this machine.
3. **A fresh clone cannot reproduce this gate result.** `.cargo/audit.toml` is
   untracked and carries the RUSTSEC-2023-0071 ignore (Marvin attack in `rsa`,
   reachable only through the never-compiled `sqlx-mysql`) with its
   justification. `deny.toml` is untracked too. Without them `make audit`
   behaves differently than it did above.

The committed state is at least *coherent*: `crates/auth` is listed in the root
`Cargo.toml`, but that edit is itself uncommitted, so `HEAD` still describes a
buildable Phase 7 workspace. The tree is not half-committed — it is five phases
behind, cleanly.

**This is a recommendation, not an action taken.** Per `CLAUDE.md` the worktree
was left exactly as found; nothing was added, committed, or stashed.

## Two untracked files that should stay out of git

`.playwright-mcp/` and `makerun-live.png` are run artifacts, not source, and
neither is ignored — a `git add -A` would sweep them in. `.gitignore` already has
an uncommitted change adding `/.local/` (development PostgreSQL cluster,
generated LiveKit config, per-run logs) and `/recordings/` (real media). Extend
it before staging anything broadly.

## Roadmap position

`ROADMAP.md` is the authority. Counts read from it today:

| Phase | State |
|---|---|
| 0–7 | Implemented |
| 8 — Security, Operability, Production Readiness | **Gate open**: 10 `[x]`, 13 `[~]`, 7 `[!]` |
| 9 — Capacity Qualification and Evidence-Led Scaling | **Not started**: 12 `[ ]` |
| Whole file | 37 `[ ]`, 23 `[~]`, 23 `[!]` |

Phase 8's seven `[!]` are blocked on things not present here: a confirmed Linux
deployment target (packaging, upgrade/rollback), a multi-host environment
(network partition), a nightly toolchain (`cargo-fuzz`), `promtool` (alert rules
have never been executed), and two Phase 7 concurrency qualifications that need
two concurrent writers against one database.

Phase 9 has no checked items but three evidence documents already drafted, all
untracked:

- `docs/phase-9/livekit-load-2026-08-15.md` — ten cameras, ten viewers.
- `docs/phase-9/live-grid-tiles-2026-08-16.md` — what N concurrent live tiles
  cost, answering ADR 010 Decision 5. Explicitly not a tile-count recommendation
  and not a capacity claim.
- `docs/phase-9/authenticated-ui-2026-08-16.md` — Playback and Settings driven
  through a trust-boundary proxy session, which is the only way the dashboard can
  have one (ADR 009 Decision 6).

Do not read those as Phase 9 progress. Phase 9 requires a documented workload and
repeatable measurement, and its first item — an unoptimized one-camera baseline —
is untouched.

## What remains unverified, carried forward unchanged

Nothing below was re-tested this session. All of it is from
`context/2026-08-17-2159/`, restated so this snapshot is readable on its own.

- **The 41% publish shortfall is open.** A 15.000 fps source, 8.85 published
  frames/s with a viewer attached. The leading candidate — distributor eviction —
  was eliminated by measurement at 1.7%. No replacement candidate.
- **No viewer has been attached across a reconnect.** Every reconnect measurement
  ran at `viewers` 0. What a subscriber sees during the keyframe resynchronisation
  is unknown.
- **The dashboard's own playback rendering.** Proven at the API — 11 274 711
  bytes, 900 frames, 60.000000 s, decoded clean — unproven in the page. The
  Playwright check timed out at 120 s and was killed.
- **No CPU-per-viewer number**, so ADR 012's second reversal criterion has nothing
  behind it.
- **One camera, one browser.** A Mac webcam at Constrained Baseline 1280x720,
  single slice. No representative High Profile camera, no H.265, no
  time-to-first-picture.
- **`promtool` is absent**, so `docs/phase-8/alerts.yml` has never been executed.
  `surveillance_livekit_timestamp_discontinuities_total` has no rule.
- **108 `RecordingStatus::Inconsistent` rows** whose files are present and probe
  cleanly, with no implemented operator path back.
- **One reconnect-mid-segment rejection** produced a decodable 224-frame file with
  a permanently `incomplete` row that the API 404s, counted by no metric.
- **Detection is `DeterministicFakeInference`.** No production model, no accuracy
  claim.
