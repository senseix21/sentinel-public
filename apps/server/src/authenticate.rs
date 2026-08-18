//! Turning an HTTP request into an [`Authentication`].
//!
//! This is the adapter half of ADR 009 Decision 1. It exists as its own module
//! for two reasons that are easy to get wrong if it lives inside a handler:
//!
//! - **Argon2id must not run on an executor thread.** Verification is
//!   memory-hard by design, so it runs on [`tokio::task::spawn_blocking`].
//! - **Unbounded concurrent verification is a memory-exhaustion vector.** Each
//!   in-flight verification costs [`HashParams::memory_bytes`], and an
//!   unauthenticated caller decides how many are in flight. A semaphore sized
//!   from an explicit memory budget bounds it. Without this, an attacker turns
//!   19 MiB-per-attempt into a denial of service against recording.
//!
//! A store outage is **not** an authentication failure. It returns
//! [`AuthenticationUnavailable`] so the adapter can answer `503` rather than
//! `401`, because telling an operator "bad credentials" during a database
//! outage sends them to the wrong runbook.

use std::{
    future::Future,
    net::IpAddr,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use tokio::sync::Semaphore;

use surveillance_auth::{
    clock::{Clock, Timestamp},
    principal::{Principal, PrincipalId},
    store::{PrincipalStore, TokenStore},
    token::PresentedToken,
    verify::{AuthFailure, Authentication, HashParams, StoredToken, Verifier},
};

use crate::trust_boundary::{ProofOutcome, TrustBoundary};

/// The scheme this server accepts, lowercased for comparison.
const BEARER_SCHEME: &str = "bearer";

/// Default memory budget for concurrent credential verification.
///
/// 256 MiB. At the default Argon2id parameters (19 MiB) that is 13 concurrent
/// verifications, which is ample for a 10-camera unit's operator traffic and
/// far below what would disturb recording.
pub const DEFAULT_VERIFY_MEMORY_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

/// How many credential verifications may run at once.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifyConcurrency(usize);

impl VerifyConcurrency {
    /// Derives the bound from a memory budget and the hash parameters.
    ///
    /// Always at least one: a budget too small for a single verification must
    /// still authenticate, slowly, rather than deadlock.
    #[must_use]
    pub fn from_budget(params: HashParams, budget_bytes: u64) -> Self {
        let per_verification = params.memory_bytes().max(1);
        let permits = budget_bytes / per_verification;
        Self(usize::try_from(permits).unwrap_or(usize::MAX).max(1))
    }

    #[must_use]
    pub const fn permits(self) -> usize {
        self.0
    }
}

/// Counts authentication outcomes for `/metrics`.
///
/// Labelled by failure reason only. Never by principal or token: those are
/// unbounded cardinality, and a token identifier in a metric is a token
/// identifier in every scrape and dashboard.
#[derive(Debug, Default)]
pub struct AuthMetrics {
    succeeded: AtomicU64,
    absent: AtomicU64,
    malformed: AtomicU64,
    unknown_token: AtomicU64,
    bad_secret: AtomicU64,
    expired: AtomicU64,
    revoked: AtomicU64,
    principal_disabled: AtomicU64,
    unknown_principal: AtomicU64,
    unavailable: AtomicU64,
    /// Successes that came from a proved proxy's assertion rather than from a
    /// verified bearer token. Counted separately because the two carry
    /// different evidence: one was checked here, the other was checked
    /// somewhere else and is trusted on the strength of the trust boundary. An
    /// operator who expects only service tokens and sees this climbing is
    /// looking at a misconfigured proxy.
    asserted: AtomicU64,
}

impl AuthMetrics {
    fn record(&self, outcome: &Authentication) {
        let counter = match outcome {
            Authentication::Authenticated(_) => &self.succeeded,
            Authentication::Refused(failure) => self.counter_for(*failure),
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    const fn counter_for(&self, failure: AuthFailure) -> &AtomicU64 {
        match failure {
            AuthFailure::Absent => &self.absent,
            AuthFailure::Malformed => &self.malformed,
            AuthFailure::UnknownToken => &self.unknown_token,
            AuthFailure::BadSecret => &self.bad_secret,
            AuthFailure::Expired => &self.expired,
            AuthFailure::Revoked => &self.revoked,
            AuthFailure::PrincipalDisabled => &self.principal_disabled,
            AuthFailure::UnknownPrincipal => &self.unknown_principal,
        }
    }

    fn record_unavailable(&self) {
        self.unavailable.fetch_add(1, Ordering::Relaxed);
    }

    /// Counts a success that arrived by proxy assertion.
    ///
    /// Called *in addition to* [`Self::record`], not instead of it, so
    /// `succeeded` remains the total across both paths and this is a strict
    /// subset of it.
    fn record_asserted(&self) {
        self.asserted.fetch_add(1, Ordering::Relaxed);
    }

    #[must_use]
    pub fn asserted(&self) -> u64 {
        self.asserted.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn succeeded(&self) -> u64 {
        self.succeeded.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn unavailable(&self) -> u64 {
        self.unavailable.load(Ordering::Relaxed)
    }

    /// The failure labels with zero counts, for a deployment with no identity
    /// store. The series must still be present: an absent series makes
    /// `rate()` empty rather than zero, and an alert on an empty series never
    /// fires.
    #[must_use]
    pub fn zeroed_failures() -> [(&'static str, u64); 8] {
        Self::default().failures()
    }

    /// Every failure counter, as `(label, value)` pairs for the metrics
    /// exporter. Bounded and stable.
    #[must_use]
    pub fn failures(&self) -> [(&'static str, u64); 8] {
        [
            (
                AuthFailure::Absent.as_str(),
                self.absent.load(Ordering::Relaxed),
            ),
            (
                AuthFailure::Malformed.as_str(),
                self.malformed.load(Ordering::Relaxed),
            ),
            (
                AuthFailure::UnknownToken.as_str(),
                self.unknown_token.load(Ordering::Relaxed),
            ),
            (
                AuthFailure::BadSecret.as_str(),
                self.bad_secret.load(Ordering::Relaxed),
            ),
            (
                AuthFailure::Expired.as_str(),
                self.expired.load(Ordering::Relaxed),
            ),
            (
                AuthFailure::Revoked.as_str(),
                self.revoked.load(Ordering::Relaxed),
            ),
            (
                AuthFailure::PrincipalDisabled.as_str(),
                self.principal_disabled.load(Ordering::Relaxed),
            ),
            (
                AuthFailure::UnknownPrincipal.as_str(),
                self.unknown_principal.load(Ordering::Relaxed),
            ),
        ]
    }
}

/// The identity store could not be consulted.
///
/// Deliberately distinct from every [`AuthFailure`]: this is an outage, and it
/// renders `503`, not `401`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthenticationUnavailable;

impl std::fmt::Display for AuthenticationUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the identity store could not be consulted")
    }
}

impl std::error::Error for AuthenticationUnavailable {}

/// Verifies presented credentials against a token store.
pub struct Authenticator<S, C> {
    store: S,
    clock: C,
    verifier: Arc<Verifier>,
    permits: Arc<Semaphore>,
    metrics: Arc<AuthMetrics>,
}

impl<S, C> Authenticator<S, C>
where
    S: TokenStore + PrincipalStore,
    C: Clock,
{
    #[must_use]
    pub fn new(
        store: S,
        clock: C,
        verifier: Arc<Verifier>,
        concurrency: VerifyConcurrency,
    ) -> Self {
        Self {
            store,
            clock,
            verifier,
            permits: Arc::new(Semaphore::new(concurrency.permits())),
            metrics: Arc::new(AuthMetrics::default()),
        }
    }

    #[must_use]
    pub fn metrics(&self) -> Arc<AuthMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Resolves the `Authorization` header into an [`Authentication`].
    ///
    /// # Errors
    /// Returns [`AuthenticationUnavailable`] when the store could not be read,
    /// or when the verification worker could not be scheduled. Neither is a
    /// credential problem and neither is reported as one.
    pub async fn authenticate(
        &self,
        authorization: Option<&str>,
    ) -> Result<Authentication, AuthenticationUnavailable> {
        let outcome = self.resolve(authorization).await?;
        self.metrics.record(&outcome);
        Ok(outcome)
    }

    async fn resolve(
        &self,
        authorization: Option<&str>,
    ) -> Result<Authentication, AuthenticationUnavailable> {
        let Some(header) = authorization else {
            return Ok(Authentication::Refused(AuthFailure::Absent));
        };
        let Some(credential) = strip_bearer(header) else {
            return Ok(Authentication::Refused(AuthFailure::Malformed));
        };
        let Ok(presented) = PresentedToken::parse(credential) else {
            return Ok(Authentication::Refused(AuthFailure::Malformed));
        };

        let Ok(stored): Result<Option<StoredToken>, _> = self.store.find(presented.id()).await
        else {
            // The store error is deliberately not carried into the response. It
            // is counted here and logged by the caller with context.
            self.metrics.record_unavailable();
            return Err(AuthenticationUnavailable);
        };

        let now = self.clock.now();
        self.verify_off_thread(stored, presented, now).await
    }

    /// Resolves a principal the trusted proxy has asserted it authenticated.
    ///
    /// **The caller must have proved the request's origin first.** There is no
    /// credential on this path to fall back on: whoever can set the header
    /// becomes the named principal. `SecurityContext::authenticate` is the only
    /// caller, and it gates this on
    /// [`ProofOutcome::proves_origin`](crate::trust_boundary::ProofOutcome::proves_origin).
    ///
    /// No Argon2id work happens here, so this takes no permit from the
    /// verification semaphore — there is nothing memory-hard to bound. The
    /// budget exists to cap concurrent *hashing*, and charging a plain indexed
    /// lookup against it would throttle the browser path for no reason.
    ///
    /// # Errors
    /// Returns [`AuthenticationUnavailable`] when the store cannot be consulted.
    pub async fn authenticate_asserted(
        &self,
        asserted: &str,
    ) -> Result<Authentication, AuthenticationUnavailable> {
        // A malformed assertion is the proxy's bug, not an attack, but it is
        // still refused rather than coerced into something valid.
        let Ok(id) = PrincipalId::new(asserted) else {
            let outcome = Authentication::Refused(AuthFailure::Malformed);
            self.metrics.record(&outcome);
            return Ok(outcome);
        };
        let Ok(found) = self.store.find_principal(&id).await else {
            self.metrics.record_unavailable();
            return Err(AuthenticationUnavailable);
        };
        let outcome = match found {
            // Enabled is checked here rather than in the store so that
            // "suspended" and "never existed" stay distinguishable in the
            // audit trail; the client cannot tell them apart either way.
            Some(principal) if principal.is_enabled() => Authentication::Authenticated(principal),
            Some(_) => Authentication::Refused(AuthFailure::PrincipalDisabled),
            None => Authentication::Refused(AuthFailure::UnknownPrincipal),
        };
        self.metrics.record(&outcome);
        if outcome.principal().is_some() {
            self.metrics.record_asserted();
        }
        Ok(outcome)
    }

    /// Runs the memory-hard verification on a blocking thread, under the
    /// concurrency bound.
    async fn verify_off_thread(
        &self,
        stored: Option<StoredToken>,
        presented: PresentedToken,
        now: Timestamp,
    ) -> Result<Authentication, AuthenticationUnavailable> {
        // Held for the duration of the blocking call. If the semaphore is
        // closed the server is shutting down, which is an unavailability, not
        // a credential failure.
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| AuthenticationUnavailable)?;

        let verifier = Arc::clone(&self.verifier);
        tokio::task::spawn_blocking(move || {
            verifier.verify(stored.as_ref(), presented.secret(), now)
        })
        .await
        .map_err(|_| AuthenticationUnavailable)
    }
}

/// An object-safe view of [`Authenticator`].
///
/// The HTTP adapter is generic over neither the store nor the clock, and making
/// it so would spread two type parameters across every routing function. This
/// trait keeps that generality where it belongs — at construction — without
/// leaking it into the adapter.
pub trait RequestAuthenticator: Send + Sync {
    /// Resolves the `Authorization` header.
    ///
    /// # Errors
    /// Returns [`AuthenticationUnavailable`] for a store or scheduling failure.
    fn authenticate<'a>(
        &'a self,
        authorization: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<Authentication, AuthenticationUnavailable>> + Send + 'a>>;

    /// Resolves a principal asserted by a proxy whose origin is already proved.
    ///
    /// See [`Authenticator::authenticate_asserted`] for the obligation this
    /// places on the caller.
    ///
    /// # Errors
    /// Returns [`AuthenticationUnavailable`] for a store failure.
    fn authenticate_asserted<'a>(
        &'a self,
        asserted: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Authentication, AuthenticationUnavailable>> + Send + 'a>>;

    /// The outcome counters, for `/metrics`.
    fn metrics(&self) -> Arc<AuthMetrics>;
}

impl<S, C> RequestAuthenticator for Authenticator<S, C>
where
    S: TokenStore + PrincipalStore,
    C: Clock,
{
    fn authenticate<'a>(
        &'a self,
        authorization: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<Authentication, AuthenticationUnavailable>> + Send + 'a>>
    {
        Box::pin(Self::authenticate(self, authorization))
    }

    fn authenticate_asserted<'a>(
        &'a self,
        asserted: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Authentication, AuthenticationUnavailable>> + Send + 'a>>
    {
        Box::pin(Self::authenticate_asserted(self, asserted))
    }

    fn metrics(&self) -> Arc<AuthMetrics> {
        Self::metrics(self)
    }
}

/// What the HTTP adapter needs in order to establish who is calling.
///
/// Bundled rather than passed as four more parameters: the routing functions
/// already take every owned boundary explicitly, and these four always travel
/// together.
pub struct SecurityContext<'a> {
    /// Where the server stops trusting the network.
    pub boundary: &'a TrustBoundary,
    /// `None` means no identity store is configured, which is only legal on a
    /// loopback listener — see [`Principal::loopback_development`].
    pub authenticator: Option<&'a dyn RequestAuthenticator>,
    /// Refuses mutations while the server drains.
    pub shutting_down: bool,
}

/// The owned half of [`SecurityContext`].
///
/// The listener owns these for its lifetime and lends a borrowed
/// [`SecurityContext`] per request, so that "is the server draining" is read
/// fresh on each request rather than captured once at startup.
pub struct SecurityOwner {
    boundary: TrustBoundary,
    authenticator: Option<Arc<dyn RequestAuthenticator>>,
}

impl SecurityOwner {
    #[must_use]
    pub const fn new(
        boundary: TrustBoundary,
        authenticator: Option<Arc<dyn RequestAuthenticator>>,
    ) -> Self {
        Self {
            boundary,
            authenticator,
        }
    }

    /// A loopback deployment with no identity store — the Phase 2–7 posture.
    #[must_use]
    pub const fn loopback_development() -> Self {
        Self::new(TrustBoundary::Loopback, None)
    }

    #[must_use]
    pub fn context(&self, shutting_down: bool) -> SecurityContext<'_> {
        SecurityContext {
            boundary: &self.boundary,
            authenticator: self.authenticator.as_deref(),
            shutting_down,
        }
    }

    #[must_use]
    pub const fn boundary(&self) -> &TrustBoundary {
        &self.boundary
    }

    /// Whether an identity store is configured. Part of the ADR 009 unlock
    /// condition for a non-loopback listener.
    #[must_use]
    pub const fn has_authenticator(&self) -> bool {
        self.authenticator.is_some()
    }

    /// The authentication counters, when an identity store is configured.
    #[must_use]
    pub fn metrics(&self) -> Option<Arc<AuthMetrics>> {
        self.authenticator
            .as_ref()
            .map(|authenticator| authenticator.metrics())
    }
}

/// What a request offered as evidence of who is calling.
///
/// Grouped into one type so that the *proof* travels with the assertion it
/// licenses. Passing `asserted_principal` as a bare `Option<&str>` alongside a
/// separate boolean invites a caller to supply the first and forget the second,
/// which is the authentication bypass this whole module is arranged to prevent.
#[derive(Clone, Copy, Debug)]
pub struct PresentedCredentials<'a> {
    authorization: Option<&'a str>,
    asserted_principal: Option<&'a str>,
    origin_proved: bool,
}

impl<'a> PresentedCredentials<'a> {
    /// The credentials of a request whose origin was **not** proved.
    ///
    /// Any asserted-principal header is dropped here rather than carried and
    /// ignored later, so there is no value downstream that could be read by
    /// mistake.
    #[must_use]
    pub const fn unproved(authorization: Option<&'a str>) -> Self {
        Self {
            authorization,
            asserted_principal: None,
            origin_proved: false,
        }
    }

    /// The credentials of a request the trust boundary proved came from the
    /// configured reverse proxy.
    #[must_use]
    pub const fn proved(
        authorization: Option<&'a str>,
        asserted_principal: Option<&'a str>,
    ) -> Self {
        Self {
            authorization,
            asserted_principal,
            origin_proved: true,
        }
    }

    /// Builds from a raw [`ProofOutcome`], discarding the assertion unless the
    /// outcome proves origin.
    #[must_use]
    pub fn from_outcome(
        outcome: ProofOutcome,
        authorization: Option<&'a str>,
        asserted_principal: Option<&'a str>,
    ) -> Self {
        if outcome.proves_origin() {
            Self::proved(authorization, asserted_principal)
        } else {
            Self::unproved(authorization)
        }
    }
}

impl SecurityContext<'_> {
    /// Resolves the request's identity.
    ///
    /// Precedence, and the reasoning for it:
    ///
    /// 1. A bearer token, when one is presented. It is a credential this server
    ///    verifies itself, so it is the stronger evidence, and letting a proxy
    ///    header override a presented token would mean a proxy bug could
    ///    silently replace a service identity.
    /// 2. A proxy-asserted principal, but only when the trust boundary proved
    ///    the request's origin. This is the browser path (ADR 009 Decision 6).
    /// 3. The loopback development identity, when no identity store exists.
    ///
    /// # Errors
    /// Returns [`AuthenticationUnavailable`] when an identity store is
    /// configured but could not be consulted. That is an outage, and the caller
    /// must render it as `503` rather than `401`.
    pub async fn authenticate(
        &self,
        credentials: PresentedCredentials<'_>,
        peer: Option<IpAddr>,
    ) -> Result<Authentication, AuthenticationUnavailable> {
        if let Some(authenticator) = self.authenticator {
            if credentials.authorization.is_some() {
                return authenticator.authenticate(credentials.authorization).await;
            }
            // `origin_proved` is re-checked here even though
            // `PresentedCredentials` already dropped the assertion without it.
            // The constructors are the guard; this is the belt. A future
            // constructor that sets the field without the proof would be caught
            // by the tests that drive this branch.
            if let Some(asserted) = credentials.asserted_principal {
                if credentials.origin_proved {
                    return authenticator.authenticate_asserted(asserted).await;
                }
            }
            return authenticator.authenticate(None).await;
        }
        // No identity store. The development identity is granted only to a
        // loopback peer — the same restriction Phase 7's `authorize` body
        // applied — so that this posture is never weaker than what it replaced.
        // A listener is loopback-bound whenever it holds, so a non-loopback
        // peer here means something unexpected reached the socket; absence of
        // evidence is not evidence of loopback.
        match peer {
            Some(address) if address.is_loopback() => Ok(Authentication::Authenticated(
                Principal::loopback_development(),
            )),
            _ => Ok(Authentication::Refused(AuthFailure::Absent)),
        }
    }
}

/// Extracts the credential from an `Authorization: Bearer <token>` header.
///
/// The scheme is matched case-insensitively (RFC 7235 says it is
/// case-insensitive) but the credential is not touched: a token differing only
/// in case is a different token.
fn strip_bearer(header: &str) -> Option<&str> {
    let (scheme, credential) = header.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case(BEARER_SCHEME) {
        return None;
    }
    let credential = credential.trim_start();
    if credential.is_empty() {
        return None;
    }
    Some(credential)
}

#[cfg(test)]
mod tests {
    use super::{
        strip_bearer, AuthMetrics, Authenticator, VerifyConcurrency,
        DEFAULT_VERIFY_MEMORY_BUDGET_BYTES,
    };
    use std::sync::Arc;
    use surveillance_auth::{
        clock::{FixedClock, Timestamp},
        principal::{CameraScope, Principal, PrincipalId, PrincipalKind, Role},
        store::InMemoryTokenStore,
        token::IssuedToken,
        verify::{AuthFailure, Authentication, HashParams, StoredToken, Verifier},
    };

    fn params() -> HashParams {
        HashParams::new(64, 1, 1).expect("cheap test parameters are valid")
    }

    fn verifier() -> Arc<Verifier> {
        Arc::new(Verifier::new(params()).expect("verifier builds"))
    }

    fn principal(enabled: bool) -> Principal {
        Principal::new(
            PrincipalId::new("operator").expect("valid"),
            PrincipalKind::Human,
            "Operator",
            Role::Operator,
            CameraScope::All,
            enabled,
        )
        .expect("principal is valid")
    }

    /// Builds a store holding one token, and returns the rendered credential.
    fn fixture(enabled: bool) -> (InMemoryTokenStore, String, Arc<Verifier>) {
        let verifier = verifier();
        let token =
            IssuedToken::generate(Timestamp::from_unix_seconds(2_000)).expect("CSPRNG available");
        let rendered = token.render().expose().to_owned();
        let store = InMemoryTokenStore::new();
        store.insert(StoredToken {
            id: token.id().clone(),
            principal: principal(enabled),
            hash: verifier.hash(token.secret()).expect("hashing succeeds"),
            expires_at: token.expires_at(),
            revoked_at: None,
        });
        (store, rendered, verifier)
    }

    fn authenticator(
        store: InMemoryTokenStore,
        verifier: Arc<Verifier>,
        at: i64,
    ) -> Authenticator<InMemoryTokenStore, FixedClock> {
        Authenticator::new(
            store,
            FixedClock::at_unix_seconds(at),
            verifier,
            VerifyConcurrency::from_budget(params(), 1024 * 1024),
        )
    }

    #[tokio::test]
    async fn a_valid_bearer_token_authenticates() {
        let (store, rendered, verifier) = fixture(true);
        let subject = authenticator(store, verifier, 1_000);
        let outcome = subject
            .authenticate(Some(&format!("Bearer {rendered}")))
            .await
            .expect("the store is available");
        assert_eq!(
            outcome.principal().map(|p| p.id().as_str().to_owned()),
            Some("operator".to_owned())
        );
        assert_eq!(subject.metrics().succeeded(), 1);
    }

    #[tokio::test]
    async fn the_bearer_scheme_is_matched_case_insensitively() {
        let (store, rendered, verifier) = fixture(true);
        let subject = authenticator(store, verifier, 1_000);
        for scheme in ["Bearer", "bearer", "BEARER", "BeArEr"] {
            let outcome = subject
                .authenticate(Some(&format!("{scheme} {rendered}")))
                .await
                .expect("the store is available");
            assert!(
                matches!(outcome, Authentication::Authenticated(_)),
                "scheme {scheme} should be accepted"
            );
        }
    }

    #[tokio::test]
    async fn an_absent_header_is_absent_not_malformed() {
        let (store, _, verifier) = fixture(true);
        let subject = authenticator(store, verifier, 1_000);
        let outcome = subject.authenticate(None).await.expect("available");
        assert_eq!(outcome.failure(), Some(AuthFailure::Absent));
    }

    #[tokio::test]
    async fn a_non_bearer_or_unparseable_credential_is_malformed() {
        let (store, rendered, verifier) = fixture(true);
        let subject = authenticator(store, verifier, 1_000);
        for header in [
            "Basic dXNlcjpwYXNz".to_owned(),
            "Bearer".to_owned(),
            "Bearer ".to_owned(),
            "Bearer not-a-token".to_owned(),
            rendered.clone(),
        ] {
            let outcome = subject
                .authenticate(Some(&header))
                .await
                .expect("available");
            assert_eq!(
                outcome.failure(),
                Some(AuthFailure::Malformed),
                "header {header:?} should be malformed"
            );
        }
    }

    #[tokio::test]
    async fn a_well_formed_token_that_was_never_issued_is_unknown() {
        let (store, _, verifier) = fixture(true);
        let subject = authenticator(store, verifier, 1_000);
        let stranger =
            IssuedToken::generate(Timestamp::from_unix_seconds(2_000)).expect("CSPRNG available");
        let outcome = subject
            .authenticate(Some(&format!("Bearer {}", stranger.render().expose())))
            .await
            .expect("available");
        assert_eq!(outcome.failure(), Some(AuthFailure::UnknownToken));
    }

    #[tokio::test]
    async fn an_expired_token_is_refused_by_the_injected_clock() {
        let (store, rendered, verifier) = fixture(true);
        // The fixture token expires at 2_000; ask at 2_000.
        let subject = authenticator(store, verifier, 2_000);
        let outcome = subject
            .authenticate(Some(&format!("Bearer {rendered}")))
            .await
            .expect("available");
        assert_eq!(outcome.failure(), Some(AuthFailure::Expired));
    }

    #[tokio::test]
    async fn a_disabled_principal_is_refused() {
        let (store, rendered, verifier) = fixture(false);
        let subject = authenticator(store, verifier, 1_000);
        let outcome = subject
            .authenticate(Some(&format!("Bearer {rendered}")))
            .await
            .expect("available");
        assert_eq!(outcome.failure(), Some(AuthFailure::PrincipalDisabled));
    }

    #[tokio::test]
    async fn every_failure_reason_is_counted_separately() {
        let (store, rendered, verifier) = fixture(true);
        let subject = authenticator(store, verifier, 1_000);
        let _ = subject.authenticate(None).await;
        let _ = subject.authenticate(Some("Bearer nope")).await;
        let _ = subject
            .authenticate(Some(&format!("Bearer {rendered}")))
            .await;

        let metrics = subject.metrics();
        let failures: Vec<(&str, u64)> = metrics.failures().to_vec();
        assert_eq!(
            failures
                .iter()
                .find(|(label, _)| *label == "absent")
                .map(|(_, count)| *count),
            Some(1)
        );
        assert_eq!(
            failures
                .iter()
                .find(|(label, _)| *label == "malformed")
                .map(|(_, count)| *count),
            Some(1)
        );
        assert_eq!(metrics.succeeded(), 1);
    }

    #[test]
    fn concurrency_is_derived_from_an_explicit_memory_budget() {
        // The production defaults: 19 MiB per verification against 256 MiB.
        let production = VerifyConcurrency::from_budget(
            HashParams::default(),
            DEFAULT_VERIFY_MEMORY_BUDGET_BYTES,
        );
        assert_eq!(production.permits(), 13);
    }

    #[test]
    fn a_budget_smaller_than_one_verification_still_permits_one() {
        // Slow beats deadlocked.
        let starved = VerifyConcurrency::from_budget(HashParams::default(), 1);
        assert_eq!(starved.permits(), 1);
    }

    #[test]
    fn metric_labels_are_bounded_and_distinct() {
        let metrics = AuthMetrics::default();
        let mut labels: Vec<&str> = metrics.failures().iter().map(|(label, _)| *label).collect();
        let total = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), total);
    }

    #[test]
    fn bearer_extraction_handles_the_documented_shapes() {
        assert_eq!(strip_bearer("Bearer abc"), Some("abc"));
        assert_eq!(strip_bearer("  Bearer   abc  "), Some("abc"));
        assert_eq!(strip_bearer("Bearer"), None);
        assert_eq!(strip_bearer("Basic abc"), None);
        assert_eq!(strip_bearer(""), None);
        // The credential's case is preserved: a differently-cased token is a
        // different token.
        assert_eq!(strip_bearer("Bearer AbC"), Some("AbC"));
    }
}
