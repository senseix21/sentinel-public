# Operator UI modularization execution plan

Date: 2026-08-18

Follow-up execution started: 2026-08-19 on
`refactor/complete-dashboard-modules` after PR #4 merged the static shell,
styles, pure utilities, playback helpers, theme support, and first embedded
registry.

Implementation status: complete. Every
screen renderer is an embedded ES module, the LiveKit and camera-tile owners
are extracted, JavaScript tests run from `make check`, the Rust source-string
corpus is gone, and the authenticated fixture plus headless Chrome rendered the
real URL successfully. The final repository gate and remaining external limits
are recorded in the dated context snapshot.

## Completion slice acceptance criteria

- `app.js` owns composition, polling, delegated DOM events, and boot only; it
  contains no screen renderer and no LiveKit `Room` registry.
- `livekit.js` is the sole owner of SDK loading and room/session lifetime, with
  executable tests for replacement, stale completion, stop, and stop-all.
- `camera-tile.js` and every existing screen under `js/screens/` contain their
  real renderer implementation rather than forwarding wrappers.
- The Rust test-only concatenation of JavaScript source is removed. Rust tests
  exercise exact registered routes, MIME types, bytes, no-store headers,
  unknown paths, traversal-shaped paths, and wrong methods.
- `make check` executes the JavaScript suite before the Rust gate.
- A real browser loads every module and renders through the authenticated
  fixture; navigation cleanup and light/dark toggling remain operational.
- `ROADMAP.md` and a new dated `context/` snapshot state what was verified and
  retain every external product/hardware gate.

The work is one behavior-preserving vertical slice. Each step leaves the server
able to compile, and focused checks run before moving to the next ownership
boundary.

## 1. Baseline and contract inventory

- Record branch, clean-tree state, monolith size, existing routes, asset tests,
  screen list, API endpoints, timers, and mutable state owners.
- Read `analysis.md`, `ROADMAP.md`, ADR 009, ADR 010, the Phase 8 UI spec, and
  the latest context snapshot where they constrain this change.
- Confirm available JavaScript runtime and authenticated fixture prerequisites.

Exit: known facts and unresolved external prerequisites are recorded; no UI
behavior has changed.

## 2. Static shell, CSS, and pure leaf modules

- Rename the shell to `index.html` and replace inline CSS/script with explicit
  same-origin links.
- Split CSS without altering selectors, declarations, or cascade order.
- Extract formatting, metrics, route parsing, and state transitions first.
- Add executable JavaScript tests before moving side-effecting ownership.

Focused verification: JavaScript unit tests plus the Rust server library test
that compiles the embedded HTML.

Exit: pure logic is independently executable and the page still has one
composition entry point.

## 3. Transport and lifecycle owners

- Extract API/error/authentication translation.
- Extract the lazy LiveKit SDK loader and room registry behind one service.
- Test room replacement, stale callbacks, per-camera stop, and stop-all.
- Extract playback filters, range/query construction, run assembly, and
  coverage; test gaps and timestamp precision.

Focused verification: JavaScript unit tests and the existing LiveKit/playback
Rust route tests.

Exit: screens do not own raw transport interpretation, LiveKit rooms, or
playback range algorithms.

## 4. Components and screen modules

- Extract banner, empty-state, and camera-tile renderers.
- Extract every existing screen into its own module, including event, heatmap,
  and storage screens beyond the initially proposed list.
- Pass a narrow render context; keep timers, DOM event binding, polling, and
  navigation in `app.js`.
- Preserve form state across polling and preserve live video reattachment during
  same-route repaint.

Focused verification: JavaScript tests, Rust server tests, and a manual static
comparison of shell/classes/routes/API calls against the baseline.

Exit: `app.js` is a composition root rather than a collection of screen
implementations.

## 5. Explicit Rust embedded-asset registry

- Replace the two standalone constants with a closed registry for HTML, CSS,
  application modules, and the vendor bundle.
- Serve exact GET paths with exact MIME types through the existing no-store
  response helper; preserve `/` and `/ui` aliases.
- Replace JavaScript-source-string tests with route/bytes/content-type/cache
  tests. Retain vendor digest verification against the embedded bundle.
- Test unknown, near-match, traversal-shaped, and wrong-method requests.

Focused verification: `cargo test -p surveillance-server health_http` (or the
smallest equivalent target discovered in the workspace).

Exit: no arbitrary filesystem serving exists and all imports are present in the
compiled binary.

## 6. Integration and final gate

- Run JavaScript unit tests.
- Run focused server tests.
- Run the authenticated browser fixture if PostgreSQL, FFmpeg, MediaMTX,
  LiveKit, and its browser driver are available; otherwise record precisely
  what is unavailable.
- Run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
```

- Run the repository dependency checks included by `make check` if the full
  gate is the canonical wrapper.
- Review ownership, cleanup, external-resource, credential, and no-store
  invariants.
- Update `ROADMAP.md` and a new dated context snapshot with commands, results,
  ignored/external checks, and remaining risks.

Exit: all locally runnable checks pass, documentation matches the shipped
implementation, and any external/browser gap is explicitly unverified.
