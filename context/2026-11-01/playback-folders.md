# Dashboard recording-folder selection — 2026-11-01

User authorized choosing existing server folders in Playback after plain
`make run`, without changing STATE_DIR/RECORDING_ROOT in terminal commands.
ADR 020 records the source-identity, filesystem-boundary and ownership decisions.

Implementation:

- Migration 0006 adds recording sources and composite `(source_id, recording_id)`
  and `(source_id, storage_locator)` uniqueness. Existing rows remain `primary`.
  A scoped repository shares the existing bounded database pool. All reads,
  reconciliation, inserts and lifecycle updates include source identity.
- Archive handles reject retention operations; primary retention queries filter
  to primary explicitly. Archives are indexed/read in place and never copied,
  moved, or deleted by this feature. Media topology and camera ownership do not
  change. `recording_source_id` accompanies export audit rows.
- Administrator-only folder discovery is bounded beneath approved roots. Default
  is the checkout's `.local` tree. Override with the platform path list in
  `SURVEILLANCE_RECORDING_BROWSE_ROOTS`. Canonical roots, symlinks, structured
  camera/year/month/day/segment identities, 5,000 discovery entries, depth two,
  eight approved roots and 32 source choices are checked. APIs return relative
  labels and opaque choices, never absolute paths; POST accepts only a known ID.
- One supervised worker, one permit and a capacity-one channel admit one index
  operation and no backlog. Busy admission returns 409 and increments a metric.
  Indexing is asynchronous; the dashboard polls camera coverage/counts/state.
  Cancellation is checked between files/pages, and the owner is joined. Archive probes
  have positive deadlines and kill-on-drop; the existing recorder probe policy
  is preserved; archive probe deadline and scan limit
  come from existing process configuration. DB recovery retries are capped with
  exponential backoff and jitter. Interrupted work becomes visibly interrupted.
- Archive URLs explicitly identify the source. Authorization occurs before source
  lookup; a source without the authorized camera has the same response as a
  missing source, and normal camera-scoped permissions remain in force. Source parsing reads
  HTTP headers only, including when an untrusted GET contains a binary body.
- Playback offers Choose existing folder / Use folder, a saved-source selector,
  source-specific cameras and UTC coverage dates, and indexing/partial/error state.
  Source selection is local browser state; registration/indexes persist in DB.
  Search responses from a previously selected source are discarded after a
  source/camera change. A page of results remains bounded; when more match, the
  UI asks for a narrower time range (automatic pagination is not implemented).

Validation and qualification:

- Workspace fmt, all-target/all-feature Clippy, 599 all-feature Rust tests and
  unstripped macOS release passed. 41 tests require external fixtures and remain
  ignored in the general gate. Plain macOS release retains the previously
  documented SQLx LINKEDIT limitation.
- 51 dashboard tests and 26 Python tests passed.
- PostgreSQL fixture: 15 tests passed, including duplicate IDs/paths across
  sources, empty camera scope, archive retention rejection, durable owner restart,
  actual playback and source-byte preservation.
- Real Chromium: selected two fixture folders with identical camera/recording
  IDs and different dates, played both at 320 px wide, preserved selection through
  reload, rejected arbitrary folder IDs and unauthenticated source APIs, and
  discarded a delayed old search after switching source. No browser exceptions.
- Debian: plain root `make run` on default ports reached readiness 200. Actual
  `.local/laptop-recordings` indexed five recordings and played at 1280 px wide;
  `.local/server-demo/recordings` indexed 1,372 playable recordings and played at
  640 px wide. Three unfinished files leave the latter visibly partial.
- The two media trees retained the same paths, sizes and modification timestamps
  before/after indexing (1,375 demo files including partials, five laptop files).
  The generated fixture additionally compares full source bytes after playback.
  Actual exports produced audit rows tagged archive-1/archive-2.
- Linux server suite: 142 tests passed. The default DB was backed up with mode
  0600 at `.local/server-run/before-recording-sources-2026-11-01.dump` before
  applying migration 0006. The old dashboard was already stopped; only its
  development PostgreSQL remained, and was stopped cleanly before qualification.

Limits: established SENTINEL camera/UTC H.264 segment layout only; not arbitrary
camera-vendor MP4 import or timestamp inference. Approval roots are deployment
configuration. No physical camera, production capacity, alternate browser,
multi-site/source ACL or packaging claim follows from these checks.

Final restart qualification: both actual source indexes persisted across a full
normal `make run` restart without reindexing. Chromium played both again
(1280 px laptop, 640 px demo), the picker listed exactly those two supported
folders, and missing camera coverage returned the same response as a missing
source. Header-only routing includes a regression for binary GET bodies.
Qualification instances were stopped afterward so normal ports remain available.
