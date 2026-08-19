# Repository status — 2026-08-19 15:36 +06

Branch: `refactor/complete-dashboard-modules`.

## Operator UI modularization

- The dashboard is a framework-free ES-module application under
  `apps/server/ui/`; `app.js` owns boot, polling, delegated events, and screen
  composition while screen renderers live under `js/screens/`.
- `livekit.js` is the sole owner of the lazy SDK and per-camera room map.
  `camera-tile.js` owns the shared live-camera tile.
- Rust serves HTML, CSS, JavaScript, and the vendored SDK from one exact
  compile-time registry. It never resolves a caller-supplied filesystem path.
- The previous test-only concatenation of JavaScript source was removed.
  Behavioral JavaScript tests and stable Rust asset-route tests replace it.
- Light and dark modes remain available without changing the product layout.

## Verification

- `node --test apps/server/ui/tests/*.test.js`: 18 passed.
- `cargo test -p surveillance-server health_http --lib`: 41 passed.
- The authenticated Phase 9 fixture started PostgreSQL, MediaMTX, FFmpeg,
  LiveKit, the server, and the proof-bearing login proxy. All registered assets
  and `/api/cameras`, `/api/system`, `/api/settings`, `/metrics`, `/live`, and
  `/ready` returned `200` through the authenticated origin.
- Headless Chrome logged in through the development proxy and rendered all
  nine hash-routed screens from the release binary. Main-content sizes ranged
  from 1,185 to 26,262 bytes; no screen rendered the failure boundary and no
  uncaught exception was observed. The theme changed from dark to light.
- `make check` passed: shell validation, 10 Python trust-boundary tests, 18
  JavaScript tests, formatting, workspace Clippy with warnings denied, the
  complete workspace test suite, release build, `cargo deny`, and `cargo audit`.
  Externally orchestrated tests remain ignored exactly as reported by Cargo;
  `cargo audit` retains the accepted `number_prefix` unmaintained warning.
