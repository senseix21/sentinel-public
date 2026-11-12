# Playback fuzz tests

This separate workspace runs coverage-guided libFuzzer tests against production
playback code. It does not start the server, access recordings, run models,
spawn FFmpeg, or connect to PostgreSQL. See
[ADR 018](../docs/adr/018-isolated-playback-fuzzing.md) for scope and boundaries.

The normal workspace retains its stable toolchain. These commands explicitly
select the installed `nightly-2026-08-21` and require `cargo-fuzz` 0.13.2:

```sh
mkdir -p fuzz/corpus/playback_plan fuzz/corpus/playback_http
cp fuzz/seeds/playback_plan/* fuzz/corpus/playback_plan/
cp fuzz/seeds/playback_http/* fuzz/corpus/playback_http/

cargo +nightly-2026-08-21 fuzz run playback_plan --fuzz-dir fuzz -- \
  -max_total_time=60 -max_len=262 -timeout=5 -rss_limit_mb=512

cargo +nightly-2026-08-21 fuzz run playback_http --fuzz-dir fuzz --features http -- \
  -max_total_time=60 -max_len=16384 -timeout=5 -rss_limit_mb=1024
```

libFuzzer enables AddressSanitizer, coverage instrumentation, debug assertions,
and overflow checks by default. The time limit covers fuzz execution, not the
initial build. In particular, the HTTP target builds the server dependency tree
and takes substantially longer to compile than the storage-only target.

## What each target checks

- `playback_plan`: positive bounded recording ranges; half-open interval
  intersections; missing/gapped/overlapping/unordered/cross-camera segments;
  continuous coverage and preservation of the requested range. At most 32
  synthetic segments per input; all timestamps are representable around the
  Unix epoch. Integer interval arithmetic supplies the acceptance oracle.
- `playback_http`: arbitrary bytes and UTF-8 queries through the real request
  target, header/content-length, recording cursor, clip-duration, and time-range
  parsers. Accepted durations must be 1-60 seconds; accepted ranges must be
  positive and at most 24 hours; accepted cursors must survive encoding and
  decoding without changing identity or timestamp. The harness skips inputs
  above the default HTTP request limit of 16 KiB.

Seeds include before-epoch cursors, offset timestamps, reversed ranges,
duplicate duration/length fields, integer overflow, invalid UTF-8, and valid
requests. Arbitrary request input does not exercise the socket reader or
authorization. Duplicate header handling is exercised for crashes, not claimed
to have a complete HTTP conformance oracle.

## Reproduce a crash

Crashes are written under `fuzz/artifacts/<target>/`. Replay the exact file:

```sh
cargo +nightly-2026-08-21 fuzz run playback_http --fuzz-dir fuzz --features http \
  fuzz/artifacts/playback_http/crash-REPLACE_WITH_ACTUAL_HASH
```

For fixed bugs, the ordinary stable regression tests must also pass:

```sh
cargo test -p surveillance-server --lib recording_cursor_round_trips_across_unix_epoch
cargo test -p surveillance-storage
```

Generated corpora, crash files, coverage output, and nightly build artifacts are
ignored. Seed inputs, target source, and this workspace's lockfile are tracked.
Keep the generated corpus when extending a run. Do not interpret a clean bounded
run as proof that no bugs exist. H.264/MP4/FFmpeg fuzzing, socket framing,
configuration fuzzing, and capacity/failure qualification remain separate work.
