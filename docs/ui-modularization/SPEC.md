# Operator UI modularization specification

Status: accepted for implementation

Date: 2026-08-18

## Problem

`apps/server/ui/dashboard.html` is a 2,444-line, 118,750-byte deployment
artifact that owns markup, styling, transport translation, authentication
behavior, mutable state, routing, metrics parsing, LiveKit sessions, playback
assembly, settings edits, camera operations, polling, and every screen renderer.
The browser executes it as one global program. A local edit therefore has no
enforced ownership boundary and can accidentally retain live sessions, erase
in-progress input, change polling, or expose transport details to a screen.

The operator UI needs source-level ownership boundaries before ROI, rules, or
analytics add more state. This change is an extraction, not a redesign.

## Known facts

- The UI has no Node, npm, bundler, or runtime dependency today.
- The server embeds the dashboard and LiveKit UMD bundle with `include_str!` and
  serves them from `/`, `/ui`, and `/ui/livekit-client.js`.
- All responses use `Cache-Control: no-store`; browser assets are same-origin
  and must work without internet access.
- ADR 010 makes the LiveKit room map the single owner of browser sessions and
  requires teardown before a route replaces attached video elements.
- Rust tests currently inspect JavaScript source text. Those tests can detect a
  spelling but cannot execute routing, range construction, parsing, or state
  transitions.
- The repository has authenticated browser fixtures, but they depend on local
  PostgreSQL, FFmpeg, MediaMTX, LiveKit, or browser availability and are not the
  deterministic unit-test boundary.

## Goals

1. Preserve the rendered UI, routes, API calls, authentication behavior,
   polling cadence, and LiveKit/playback lifecycle.
2. Use native ES modules with no frontend build step or package manager.
3. Give transport translation, routing, application state, LiveKit sessions,
   playback state, shared components, and screen rendering explicit owners.
4. Embed every shipped asset in the Rust binary and serve it only through an
   explicit allowlisted registry.
5. Execute deterministic JavaScript unit tests for metrics parsing, playback
   range logic, routing, and state transitions using the available system
   JavaScript runtime.
6. Test the stable HTTP asset contract in Rust instead of coupling Rust tests to
   JavaScript implementation strings.

## Non-goals

- No visual redesign, framework, TypeScript conversion, npm dependency,
  bundler, minifier, service worker, client-side cache, or external resource.
- No API shape changes, new server capability, new identity flow, or change to
  the development-only LiveKit token posture.
- No attempt to claim browser compatibility, camera scale, or live-session
  performance from unit tests.

## Target layout

```text
apps/server/ui/
├── index.html
├── styles/
│   ├── base.css
│   ├── layout.css
│   ├── components.css
│   └── screens.css
├── js/
│   ├── app.js
│   ├── api.js
│   ├── router.js
│   ├── state.js
│   ├── formatting.js
│   ├── metrics.js
│   ├── livekit.js
│   ├── playback.js
│   ├── components/
│   │   ├── banner.js
│   │   ├── camera-tile.js
│   │   └── empty-state.js
│   └── screens/
│       ├── dashboard.js
│       ├── live.js
│       ├── playback.js
│       ├── events.js
│       ├── heatmap.js
│       ├── storage.js
│       ├── cameras.js
│       ├── settings.js
│       └── system.js
└── vendor/
    └── livekit-client.umd.js
```

The additional event, heatmap, and storage modules reflect screens already
shipped by the monolith; omitting them would leave screen ownership incomplete.

## Ownership and contracts

### HTML and CSS

`index.html` owns only the static shell, inline SVG symbol definitions, four
same-origin stylesheet links, and one `type="module"` entry point. CSS files
retain selectors and declaration order from the original page. Splitting may
not rename classes or alter layout values during this slice.

### `api.js`

Owns fetch, response decoding, error classification, and authentication
recovery. It exports domain-oriented operations or a narrow request client.
Screens receive normalized data or `ApiError`; they do not parse raw responses
or decide what HTTP status means. Viewer tokens remain ephemeral inputs to
LiveKit and are never returned to a renderer.

### `router.js`

Owns hash parsing, known-screen selection, and route-change subscription. Route
parsing is pure and accepts an explicit hash in tests. Unknown screens resolve
to the dashboard exactly as before.

### `state.js`

Owns domain-specific stores rather than one writable global bag:

- system/settings store;
- fleet/metrics store;
- playback store;
- camera-detail and add-camera form stores;
- settings-edit store;
- UI presentation state such as live-grid density.

Stores expose intentional transitions and immutable snapshots where practical.
Polling writes through store methods. Screens read snapshots and invoke
commands; they do not replace unrelated domains.

### `metrics.js`

Owns Prometheus text parsing and metric queries. Parsing is pure, skips comments
and malformed samples as today, and preserves label/value behavior. Its table
is passed explicitly or held by a metrics store, never hidden in a screen.

### `livekit.js`

Owns SDK loading and the only `Map<cameraId, session>`. Its public lifecycle is
`start`, `stop`, `stopAll`, `reattach`, and status rendering callbacks. Starting
a camera supersedes only that camera. A stale connect/disconnect callback may
not clear its successor. Route change awaits `stopAll` before rendering the new
view. SDK loading remains lazy, same-origin, memoized, and retryable after load
failure.

### `playback.js`

Owns playback filter state, ISO range construction, contiguous-run assembly,
coverage calculation, and playback URL construction. Range creation must retain
microsecond-bearing server timestamps when assembling playback requests; search
parameters remain `start` and `end`.

### Components and screens

Components render reusable, side-effect-free fragments. Screen modules render
one screen and receive dependencies through an explicit context. DOM-wide event
delegation and polling orchestration remain in `app.js`; screen modules do not
start timers or own network sessions.

`app.js` is the composition root. It creates stores and services, binds DOM
events once, owns timers, coordinates polling, and enforces route cleanup.

## Embedded asset registry

Rust defines a closed registry entry containing:

- exact request path;
- MIME type;
- embedded bytes/text.

Only registry entries are served. There is no path concatenation, directory
walk, filesystem read, percent-decoded lookup, fallback, or prefix match. `/`
and `/ui` may alias the same HTML bytes; all other assets use explicit
`/ui/...` paths. All asset responses remain unauthenticated constant bytes and
carry `Cache-Control: no-store` through the existing response helper.

The registry includes the vendor bundle but preserves lazy loading. Missing,
near-match, traversal-shaped, and wrong-method requests do not resolve to an
asset.

## Security and failure behavior

- No asset contains an absolute or protocol-relative external URL or CSS
  `@import`.
- The HTML never stores credentials. A viewer token has one consumer:
  `Room.connect`.
- An API `401` retains the current login/session-expired behavior; `403`, `404`,
  conflicts, validation failures, and unavailable dependencies retain distinct
  operator messages.
- A failed module or stylesheet request is visible as a failed UI load and is
  never replaced with arbitrary content.
- LiveKit SDK failure clears its memoized promise so a later explicit watch can
  retry.
- Navigation, logout, and page teardown stop owned live sessions.
- Every timer is created by the composition root; this extraction introduces no
  detached per-screen polling.

## Acceptance criteria

- `dashboard.html` is removed and the target layout is present.
- `/` and `/ui` serve byte-identical `index.html`; every linked module,
  stylesheet, and vendor asset returns its exact embedded bytes, correct MIME
  type, and `Cache-Control: no-store`.
- An exhaustive Rust test covers the registry paths and rejects representative
  unknown/traversal paths and non-GET methods.
- Rust tests no longer inspect JavaScript implementation strings.
- JavaScript tests execute and cover:
  - Prometheus comments, labels, malformed lines, lookup, and sums;
  - root, named, camera-detail, and unknown hashes;
  - playback range parameter names, timestamp precision, contiguous gaps, and
    coverage edge cases;
  - independent store transitions and reset behavior;
  - LiveKit supersession and `stopAll` ownership with a fake SDK/room.
- Existing Rust focused tests pass, then the full formatting, Clippy, test, and
  release-build gate passes.
- The authenticated browser fixture is run when its documented local
  dependencies are available. If not available, the exact missing dependency
  is reported and no browser-validation claim is made.
- `ROADMAP.md` and the latest dated `context/` snapshot describe only verified
  behavior.

## Risks and mitigations

| Risk | Impact | Mitigation |
|---|---:|---|
| Import cycle or initialization-order change | High | Keep `app.js` as composition root; dependency injection into screens; pure leaf modules. |
| Live sessions survive navigation | High | One owner, awaited `stopAll` before render, fake-room transition tests, browser fixture. |
| CSS cascade changes during split | Medium | Preserve rule text and relative order of linked files; no selector/value edits. |
| Asset omitted from Rust registry | Medium | Derive an exhaustive expected-path test from the explicit registry and verify HTML imports resolve. |
| Tests pass without a real browser | Medium | Keep unit claims narrow and run the authenticated fixture where dependencies permit. |
| Node becomes an undeclared product dependency | Low | Use it only as a developer test runner if present; production build and runtime remain Node-free. |

## Reversal criteria

Recombine a boundary only if executable tests show that its public contract
creates circular ownership or makes lifecycle ordering less explicit. Consider
a build tool only when measured payload, browser-compatibility, or maintainable
static checking cannot be achieved with native modules; that requires a new
decision because it changes the build and dependency supply chain.
