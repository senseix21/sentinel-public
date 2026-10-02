# Next Actions — 2026-10-01 21:35 +06

1. Decide the fate of `crates/analytics`, `crates/api`, `crates/rules`, and
   `apps/worker`: implement, remove, or re-scope the Phase 1 exit-gate wording.
   Phase 1 cannot close until then.
2. Run `make run` from an interactive terminal so macOS can prompt for camera
   access, and confirm capture, recording, segment finalization, and decode.
   Nothing in the pipeline has been exercised against a real frame on this host.
3. Execute recording-video slice 5: deterministic fMP4 input, retrieval of the
   full, 5-second, and 10-second variants through the authenticated boundary,
   FFprobe decode of every response, then duration variance, startup latency,
   CPU and RSS, delivered bytes, cancellation cleanup, and concurrent retrieval
   at a confirmed viewer workload. Add `start_recording` coverage to
   `apps/server/tests/playback_assembler.rs`, which has none.
4. Implement the section 8 observability for the new route: request mode,
   outcome, active count, and totals, with fixed label vocabularies and no
   recording or camera identifier as a label.
5. Raise `rust-toolchain.toml` past 1.92.0 deliberately, fixing the two
   `clippy::unused_async_trait_impl` hits in `crates/auth/src/store.rs` that
   Rust 1.98 reports. Pinning made the gate reproducible; it did not remove the
   debt.
6. Select and ADR a deterministic external RTSP test peer, then rebuild the
   authentication, reconnect, and two-camera isolation fixtures.
7. Close out the competing branch `feature-optional-recording-clip-duration`.
   It carries a second ADR 013 and the optional-`end` design that ADR 014
   supersedes; it must not merge alongside pull request #10.
8. Obtain the Phase 0 answers that gate everything else: representative camera
   models and sample streams per codec/profile, target hardware and Linux
   distribution, and accepted p90 latency, availability, and disk-full priority
   objectives.
