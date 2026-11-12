# Playback fuzz qualification - 2026-10-31

## Result and scope

Two actual coverage-guided libFuzzer targets ran against production playback
code on arm64 macOS. After two cursor fixes, clean bounded runs completed:

- `playback_plan`: **441,505 inputs in 31 seconds**, 262-byte maximum input,
  32-segment maximum generated plan, five-second per-input timeout, 512 MiB RSS limit.
- `playback_http`: **164,460 inputs in 61 seconds**, 16,384-byte maximum input,
  five-second per-input timeout, 1,024 MiB RSS limit.

These are 605,965 clean-run executions, not a count of unique inputs. AddressSanitizer,
debug assertions, and overflow checks were enabled. No failures were reported in
those final runs. Earlier failing HTTP executions are excluded from that total.

The HTTP target exercises request targeting, header/content-length parsing,
recording duration queries, timestamp ranges, and cursor encode/decode. Its
cursor round-trip invariant found two bugs. The plan target checks range and
continuous-coverage acceptance with an integer-interval oracle. See
[fuzz/README.md](../../fuzz/README.md) and [ADR 018](../adr/018-isolated-playback-fuzzing.md).

No running database, camera, saved recording, model, or FFmpeg process is used
by either fuzz target. The extra native driver entry point described in the ADR
belongs exclusively to the isolated fuzz binary, not production code.

## Confirmed bug 1: pre-epoch cursor pagination

The 12-byte seed `-1.segment-1` decodes to a timestamp one nanosecond before the
Unix epoch. Re-encoding used Rust's signed hexadecimal formatting, which emits
two's complement for a negative integer. The decoder's signed radix parser
cannot parse that unsigned-looking 128-bit value. Thus valid pre-epoch cursors
did not survive pagination.

Fix: encode negative timestamps with an explicit minus sign and unsigned
magnitude. Positive cursor encoding is unchanged. The stable regression
`recording_cursor_round_trips_across_unix_epoch` failed before the fix and now
passes for timestamps on both sides of the epoch, including nanosecond precision.

This edge case does not explain the earlier searches against October 2026 footage.

## Confirmed bug 2: malformed cursor timestamp panic

After the first fix, mutation produced a cursor beginning with
`-1CCCCCCCCCCCCCCCCCCCCCC.`. The hexadecimal integer fits i128 but is far outside
the supported timestamp interval. Conversion through the pinned jiff/jiff-core
dependency panicked at its internal epoch-seconds assertion instead of returning
an error. The failure is reproducible in a normal stable regression test, not
only under instrumentation. The recording ID suffix is irrelevant to the panic
because timestamp conversion occurs first.

Fix: check the integer against `jiff::Timestamp::MIN` and `MAX` in nanoseconds
before calling the dependency constructor. Keep the constructor's Result check.
Out-of-range cursors are rejected normally by the existing API validation path.
`recording_cursor_rejects_extreme_timestamps_without_panicking` covers the
reproducer, both signs, i128 extremes, and valid timestamp boundaries.

This is a parser panic, not evidence of native memory corruption or a confirmed
whole-process crash through HTTP. The in-memory harness aborts on a panic so
libFuzzer can capture it; actual HTTP task supervision is a separate boundary.

## Reproduction and evidence

Run from the repository root with cargo-fuzz 0.13.2 and the installed
`nightly-2026-08-21` toolchain. Target source, curated seeds, and the separate
fuzz workspace's lockfile are retained in `fuzz/`; generated corpora and crash
artifacts are ignored.

```sh
cargo +nightly-2026-08-21 fuzz run playback_plan --fuzz-dir fuzz -- \
  -max_total_time=30 -max_len=262 -timeout=5 -rss_limit_mb=512
cargo +nightly-2026-08-21 fuzz run playback_http --fuzz-dir fuzz --features http -- \
  -max_total_time=60 -max_len=16384 -timeout=5 -rss_limit_mb=1024
cargo test -p surveillance-server --features fuzzing --lib recording_cursor
```

Copy the tracked seeds to the generated corpora as documented in the fuzz README
when starting from a clean checkout. The final HTTP run also retained the
generated corpus from the earlier run that discovered the second bug.

Raw local evidence under `target/fuzz-2026-10-30/` (the work started October 30
and finished October 31): `playback-plan.log`, `playback-http-before.log`,
`playback-http-after.log`, `playback-http-final.log`, `cursor-before-fix.log`,
`extreme-cursor-before-fix.log`, and `cursors-after-both-fixes.log`.
Both crashing inputs are retained as curated seeds.

## Workspace verification

- Formatting for the production workspace and isolated fuzz workspace: passed.
- All-target/all-feature workspace Clippy with warnings denied: passed.
- All-feature workspace tests: **592 passed, zero failed, 39 ignored**, with
  the existing local ONNX runtime supplied through `ORT_DYLIB_PATH`. Tests ran
  outside the sandbox because the camera child-cleanup test requires process
  inspection that the sandbox denies. External ignored tests were not executed.
- Normal `cargo build --workspace --release`: failed on the previously recorded
  macOS SQLx proc-macro `mis-aligned LINKEDIT string pool` problem.
- `CARGO_PROFILE_RELEASE_STRIP=none cargo build --workspace --release`: passed.
  This is an explicit local workaround, not a claim that the normal release
  gate or complete `make check` passes.

Logs are `clippy-final.log`, `tests-final.log`, `release.log`, and
`release-unstripped-final.log` under the evidence directory above. Initial
attempts without process-inspection permission or the ONNX runtime failed;
neither failure was hidden by weakening tests.

## Remaining coverage

This is bounded playback fuzzing, not complete API or media qualification.
Socket reads/framing, authentication, authorization, configuration, H.264
depayloading, MP4 assembly, FFmpeg/native decoder robustness, concurrency,
representative cameras, and sustained capacity are not exercised here. Header
parsing is checked for crashes, not complete HTTP conformance. The plan input
grammar bounds segment count and timestamps; larger production plans and the
full SystemTime domain require additional targets. A clean short run does not
prove that no other defects exist.
