# ADR 018: Isolated coverage-guided playback fuzzing

Date: 2026-10-30
Status: accepted for development tooling.

## Context

The user requested fuzz testing after qualification of the recording playback
API. Hand-selected parser and range tests do not explore arbitrary malformed
requests, timestamp boundaries, or segment combinations. The normal workspace
pins stable Rust; changing that build contract is unnecessary for this work.

## Decision

Keep libFuzzer in a separate `fuzz/` Cargo workspace with its own lockfile and
an explicitly selected installed nightly (`nightly-2026-08-21` for this run).
Pin its only added direct tooling dependency, `libfuzzer-sys`, to 0.4.10.
The production workspace and toolchain are unchanged.

Two targets cover the current playback slice:

- `playback_plan` calls the production recording range and playback-plan
  constructors with bounded generated metadata. An integer-interval oracle
  checks range validation, half-open overlap, camera isolation, and continuous
  coverage. At most 32 generated segments are retained per input.
- `playback_http` calls the actual private HTTP/playback parsers through a
  documented development-only `fuzzing` feature entry point. It covers request
  targeting, header/content-length parsing, clip duration, timestamp ranges, and
  cursor round trips. Inputs above the default 16 KiB request bound are skipped.

These targets do not bind sockets, start FFmpeg, run models, read recordings,
or connect to a database. Assertions live only in the fuzz harness/feature.
Use explicit time, input-size, per-input timeout, and memory limits for runs.
Nightly and libFuzzer are not requirements of `make check`.

## Trade-offs and limits

The HTTP target links the server dependencies and is slower to build than the
storage-only target. A default-disabled feature is a smaller change than
extracting or duplicating parsers purely for tooling. The harness uses the
configured-default request bound, not every possible runtime configuration.
On macOS a transitive native archive supplied a competing `main`; the HTTP
target therefore owns its Rust entry point and calls `LLVMFuzzerRunDriver`
through a documented, narrowly scoped unsafe FFI boundary. Writable argument
buffers and their pointer vector live for the entire driver call. This test
binary is outside the production workspace; production unsafe policy is unchanged.
It does not exercise socket reading, authorization, MP4 demuxing, foreign codec
libraries, H.264 depayloading, or long-running concurrent workloads. These need
their own targets and qualification. A bounded clean run is not a proof that
there are no bugs.

Keep hand-curated seed cases in version control, generated corpora and crash
artifacts local, and convert each confirmed bug to a stable deterministic
regression test before fixing it.

## Reversal

Remove `fuzz/`, the `fuzzing` feature, and its HTTP entry point. No deployment,
configuration, storage, or wire-format migration is required by the tooling.
