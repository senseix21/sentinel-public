# Architecture handoff

[ADR 018](../../docs/adr/018-isolated-playback-fuzzing.md) keeps libFuzzer/nightly
in a separate tooling workspace. The normal stable workspace gains only a
default-disabled `fuzzing` feature that exposes an in-memory entry point to the
actual playback parsers. Production unsafe policy is unchanged. The fuzz binary
alone owns a documented native driver FFI boundary to avoid a competing native
main symbol on macOS.

The production media topology is unchanged: one camera ingest; bounded recording,
detection and live subscriptions; detection facts separate from media. See the
[combined detection handoff](../2026-10-09-2336/architecture.md).
