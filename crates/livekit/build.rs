//! Link arguments this crate's own test binaries need on Apple platforms.
//!
//! Same defect and same fix as `apps/server/build.rs`: libwebrtc ships
//! Objective-C classes and categories in archive members that define no other
//! symbol, the linker drops them, and the first `PeerConnectionFactory`
//! construction dies with `unrecognized selector sent to class` — an
//! Objective-C exception, so `SIGABRT` rather than a catchable Rust panic.
//!
//! `apps/server/build.rs` emits `rustc-link-arg-bins`, which covers the server
//! and nothing else. The integration tests in `tests/` link their own binaries,
//! so without this they abort inside
//! `+[RTCDefaultVideoEncoderFactory supportedCodecs]` before reaching whatever
//! they were written to check. That failure looks like a broken fixture and is
//! not one, which is the reason this file exists rather than the test being
//! left to fail.
//!
//! Scoped to test targets deliberately: this crate builds no binary, and
//! setting the flag globally would attach it to every dependent's link line
//! whether or not it touches libwebrtc. Adding a benchmark or an example here
//! means adding the matching `rustc-link-arg-*` line — Cargo rejects the
//! instruction outright when no such target exists, so the omission cannot be
//! silent.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple") {
        println!("cargo:rustc-link-arg-tests=-ObjC");
    }
}
