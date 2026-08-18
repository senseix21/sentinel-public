//! Link arguments the `livekit` dependency needs on Apple platforms.
//!
//! libwebrtc ships Objective-C categories — `+[NSString
//! stringForAbslStringView:]` among them — in archive members that define no
//! other symbol. The linker drops such members by default, so the category is
//! absent at run time and the first `PeerConnectionFactory` construction dies
//! with `unrecognized selector sent to class`, taking the process with it: an
//! Objective-C exception is not a Rust panic and no supervisor can catch it.
//!
//! `-ObjC` tells the linker to load every archive member that defines an
//! Objective-C class or category. It is required for correctness here, not an
//! optimisation, and it is scoped to this binary rather than set globally so
//! that it is visible next to the dependency that needs it.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple") {
        println!("cargo:rustc-link-arg-bins=-ObjC");
    }
}
