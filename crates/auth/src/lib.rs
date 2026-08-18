//! Identity, credentials, and token lifecycle.
//!
//! This crate owns *who is asking*. It deliberately does not own:
//!
//! - **what they may do** — the role/action table lives at the authorization
//!   chokepoint in `apps/server`, next to the actions it governs;
//! - **where rows live** — `surveillance-database` implements
//!   [`store::TokenStore`];
//! - **how a request carries a credential** — that is the HTTP adapter's job.
//!
//! ADR 009 Decision 1 records why bearer tokens, and why Argon2id even though
//! the tokens are high-entropy.
//!
//! ## The two halves of a token
//!
//! A token is `srv_<id>_<secret>`. The store indexes `id` and holds only an
//! Argon2id hash of `secret`. Reading the entire token table yields nothing an
//! attacker can authenticate with.
//!
//! ## Cost
//!
//! Verification is memory-hard on purpose. It must run on a blocking thread
//! with bounded concurrency; see [`verify::Verifier::verify`] and
//! [`verify::HashParams::memory_bytes`].

pub mod clock;
pub mod principal;
pub mod store;
pub mod token;
pub mod verify;

pub use clock::{Clock, FixedClock, SystemClock, Timestamp};
pub use principal::{CameraScope, Principal, PrincipalId, PrincipalKind, Role};
pub use store::{InMemoryTokenStore, TokenStore};
pub use token::{IssuedToken, PresentedToken, TokenId, TokenSecret};
pub use verify::{AuthFailure, Authentication, CredentialHash, HashParams, StoredToken, Verifier};
