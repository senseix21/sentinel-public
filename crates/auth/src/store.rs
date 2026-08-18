//! The token lookup boundary.
//!
//! A trait, because there are two real implementations: `PostgreSQL` in
//! `surveillance-database`, and the in-memory store below that lets the
//! verification tests run without a database. It is not a trait per struct.
//!
//! Lookup is by [`TokenId`] alone. The store must not be asked to compare
//! secrets — that is [`Verifier`](crate::verify::Verifier)'s job, and keeping
//! it there is what makes "the store never holds a usable token" checkable in
//! one place.

use std::{collections::HashMap, error::Error, fmt, future::Future, sync::RwLock};

use crate::{
    principal::{Principal, PrincipalId},
    token::TokenId,
    verify::StoredToken,
};

/// Reads token rows.
pub trait TokenStore: Send + Sync {
    /// Why a lookup failed. Distinct from "not found", which is `Ok(None)`:
    /// a database outage must not be indistinguishable from a bad token.
    type Error: Error + Send + Sync + 'static;

    /// Finds one token row by its public identifier.
    fn find(
        &self,
        id: &TokenId,
    ) -> impl Future<Output = Result<Option<StoredToken>, Self::Error>> + Send;
}

/// Reads principal rows by identifier, with no credential involved.
///
/// Separate from [`TokenStore`] on purpose. A token lookup is the *end* of a
/// credential check; this is a lookup with no credential at all, used only on
/// the proxy-asserted path (ADR 009 Decision 6) where the proxy has already
/// authenticated the human and the server's remaining job is to decide what
/// that named human may do.
///
/// Because this trait can produce a `Principal` without verifying anything, a
/// caller must not reach it until the request's origin has been *proved*. That
/// obligation is discharged in exactly one place —
/// `SecurityContext::authenticate` — and the type carries no proof of its own,
/// so the doc comment is the contract. Widening the set of callers is a
/// security change, not a refactor.
pub trait PrincipalStore: Send + Sync {
    /// Why a lookup failed. As with [`TokenStore::Error`], distinct from
    /// "no such principal", which is `Ok(None)`.
    type Error: Error + Send + Sync + 'static;

    /// Finds one principal by identifier, including its role and camera scope.
    ///
    /// Returns the row whether or not it is enabled; the caller decides what a
    /// disabled principal means, so that "suspended" and "never existed" stay
    /// distinguishable in the audit trail.
    fn find_principal(
        &self,
        id: &PrincipalId,
    ) -> impl Future<Output = Result<Option<Principal>, Self::Error>> + Send;
}

/// An in-memory store for tests.
///
/// Not for production use: it has no durability and no revocation propagation
/// across processes.
#[derive(Debug, Default)]
pub struct InMemoryTokenStore {
    rows: RwLock<HashMap<String, StoredToken>>,
    principals: RwLock<HashMap<String, Principal>>,
}

impl InMemoryTokenStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces a row.
    ///
    /// # Panics
    /// Never. A poisoned lock is recovered from rather than propagated: the
    /// guarded value is a plain map with no invariant that a panicking writer
    /// could have broken halfway.
    pub fn insert(&self, token: StoredToken) {
        let mut rows = self.rows.write().unwrap_or_else(|poisoned| {
            self.rows.clear_poison();
            poisoned.into_inner()
        });
        rows.insert(token.id.as_str().to_owned(), token);
    }

    /// Inserts or replaces a principal, for the proxy-asserted path.
    ///
    /// # Panics
    /// Never; a poisoned lock is recovered from, as in [`Self::insert`].
    pub fn insert_principal(&self, principal: Principal) {
        let mut principals = self.principals.write().unwrap_or_else(|poisoned| {
            self.principals.clear_poison();
            poisoned.into_inner()
        });
        principals.insert(principal.id().as_str().to_owned(), principal);
    }

    /// Number of rows held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.read().map_or(0, |rows| rows.len())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl TokenStore for InMemoryTokenStore {
    type Error = InMemoryStoreError;

    async fn find(&self, id: &TokenId) -> Result<Option<StoredToken>, Self::Error> {
        let rows = self.rows.read().map_err(|_| InMemoryStoreError::Poisoned)?;
        Ok(rows.get(id.as_str()).cloned())
    }
}

impl PrincipalStore for InMemoryTokenStore {
    type Error = InMemoryStoreError;

    async fn find_principal(&self, id: &PrincipalId) -> Result<Option<Principal>, Self::Error> {
        let principals = self
            .principals
            .read()
            .map_err(|_| InMemoryStoreError::Poisoned)?;
        Ok(principals.get(id.as_str()).cloned())
    }
}

/// The only way an in-memory lookup can fail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InMemoryStoreError {
    Poisoned,
}

impl fmt::Display for InMemoryStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("in-memory token store lock was poisoned")
    }
}

impl Error for InMemoryStoreError {}

#[cfg(test)]
mod tests {
    use super::{InMemoryTokenStore, TokenStore};
    use crate::{
        clock::Timestamp,
        principal::{CameraScope, Principal, PrincipalId, PrincipalKind, Role},
        token::{IssuedToken, TokenSecret},
        verify::{HashParams, StoredToken, Verifier},
    };

    fn row(id_source: &IssuedToken) -> StoredToken {
        let verifier = Verifier::new(HashParams::new(64, 1, 1).expect("valid")).expect("builds");
        StoredToken {
            id: id_source.id().clone(),
            principal: Principal::new(
                PrincipalId::new("svc").expect("valid"),
                PrincipalKind::Service,
                "Service",
                Role::Viewer,
                CameraScope::All,
                true,
            )
            .expect("valid"),
            hash: verifier
                .hash(&TokenSecret::new(&"ab".repeat(32)).expect("valid"))
                .expect("hashing succeeds"),
            expires_at: Timestamp::from_unix_seconds(2_000),
            revoked_at: None,
        }
    }

    fn issued() -> IssuedToken {
        IssuedToken::generate(Timestamp::from_unix_seconds(2_000)).expect("CSPRNG available")
    }

    #[test]
    fn a_stored_row_is_found_by_its_identifier() {
        let store = InMemoryTokenStore::new();
        let token = issued();
        store.insert(row(&token));

        let found = futures_executor_block_on(store.find(token.id()));
        assert!(found.expect("lookup succeeds").is_some());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn an_absent_identifier_is_not_found_rather_than_an_error() {
        // The distinction matters: an outage must not look like a bad token.
        let store = InMemoryTokenStore::new();
        let absent = issued();
        let found = futures_executor_block_on(store.find(absent.id()));
        assert!(found.expect("lookup succeeds").is_none());
        assert!(store.is_empty());
    }

    /// Polls a future to completion without an async runtime.
    ///
    /// The in-memory store never awaits anything, so it is ready on the first
    /// poll. This keeps `surveillance-auth` free of a Tokio dependency it would
    /// otherwise carry only for tests.
    fn futures_executor_block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::task::{Context, Poll, Waker};

        let mut context = Context::from_waker(Waker::noop());
        let mut pinned = std::pin::pin!(future);
        match pinned.as_mut().poll(&mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("the in-memory store never yields"),
        }
    }
}
