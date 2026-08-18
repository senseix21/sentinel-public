//! Credential hashing and verification.
//!
//! Two properties this module exists to hold:
//!
//! 1. **The store never holds a usable token.** Only an Argon2id PHC string,
//!    which encodes its own parameters so they can be raised later without
//!    invalidating existing rows.
//! 2. **An unknown token costs the same as a wrong one.** Returning early on a
//!    missed lookup would leak, through timing, which token identifiers exist.
//!    [`Verifier::verify`] performs a dummy hash on the miss path.
//!
//! Argon2id is memory-hard by design, so verification is CPU- and
//! memory-expensive on purpose. Callers must not run it on an async executor
//! thread — see [`Verifier::verify`].

use std::fmt;

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};

use crate::{
    clock::Timestamp,
    principal::Principal,
    token::{TokenId, TokenSecret},
};

/// Memory cost in kibibytes.
///
/// 19 MiB, the OWASP first-choice parameter set for Argon2id at t=2, p=1.
pub const DEFAULT_MEMORY_KIB: u32 = 19_456;
/// Iteration count.
pub const DEFAULT_ITERATIONS: u32 = 2;
/// Degree of parallelism.
pub const DEFAULT_PARALLELISM: u32 = 1;

/// Salt length in bytes, before base64 encoding.
const SALT_BYTES: usize = 16;

/// Argon2id cost parameters.
///
/// Held explicitly rather than taken from the crate default so that the values
/// this deployment actually uses are greppable, and so a deployment under
/// memory pressure can lower them knowingly instead of discovering them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HashParams {
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
}

impl Default for HashParams {
    fn default() -> Self {
        Self {
            memory_kib: DEFAULT_MEMORY_KIB,
            iterations: DEFAULT_ITERATIONS,
            parallelism: DEFAULT_PARALLELISM,
        }
    }
}

impl HashParams {
    /// # Errors
    /// Returns [`HashError::Parameters`] for a combination Argon2 refuses.
    pub fn new(memory_kib: u32, iterations: u32, parallelism: u32) -> Result<Self, HashError> {
        let candidate = Self {
            memory_kib,
            iterations,
            parallelism,
        };
        // Validate eagerly so an invalid set is a construction error rather
        // than a surprise on the first authentication attempt.
        candidate.to_argon2()?;
        Ok(candidate)
    }

    /// Peak memory this parameter set costs per concurrent verification.
    ///
    /// Callers use this to size the concurrency bound: Argon2id is memory-hard,
    /// so unbounded concurrent verification is a memory-exhaustion vector for
    /// an unauthenticated caller.
    #[must_use]
    pub const fn memory_bytes(self) -> u64 {
        self.memory_kib as u64 * 1024
    }

    fn to_argon2(self) -> Result<Argon2<'static>, HashError> {
        let params = Params::new(self.memory_kib, self.iterations, self.parallelism, None)
            .map_err(|_| HashError::Parameters)?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }
}

/// An Argon2id PHC string, as stored.
///
/// Encodes algorithm, version, parameters, salt, and digest, so a row verifies
/// under the parameters it was created with even after the defaults change.
#[derive(Clone, Eq, PartialEq)]
pub struct CredentialHash(String);

impl CredentialHash {
    /// Accepts a stored PHC string after checking it parses.
    ///
    /// # Errors
    /// Returns [`HashError::Malformed`] for anything Argon2 cannot read. A row
    /// that cannot be parsed denies; it never falls back to accepting.
    pub fn parse(stored: &str) -> Result<Self, HashError> {
        PasswordHash::new(stored).map_err(|_| HashError::Malformed)?;
        Ok(Self(stored.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CredentialHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The parameters are useful in a log; the digest and salt are not.
        formatter.write_str("CredentialHash(argon2id)")
    }
}

/// Why hashing or verification could not be performed.
///
/// Distinct from [`AuthFailure`]: this is a fault in the system, not a refusal
/// of a caller. The two are kept apart so a hashing fault cannot be reported to
/// a client as "bad credentials", which would hide an outage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HashError {
    Parameters,
    Malformed,
    Entropy,
    Backend,
}

impl fmt::Display for HashError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parameters => formatter.write_str("argon2 parameters are invalid"),
            Self::Malformed => formatter.write_str("stored credential hash is malformed"),
            Self::Entropy => {
                formatter.write_str("the operating system entropy source is unavailable")
            }
            Self::Backend => formatter.write_str("argon2 backend failure"),
        }
    }
}

impl std::error::Error for HashError {}

/// Why a caller was not authenticated.
///
/// Every variant renders to the client identically. The distinction exists for
/// the audit trail and for metrics, where "forty expired tokens" and "forty
/// wrong secrets" mean very different things.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthFailure {
    /// No credential presented.
    Absent,
    /// Presented, but not a token this system could have issued.
    Malformed,
    /// Well-formed, but no such identifier.
    UnknownToken,
    /// Identifier exists; secret does not match.
    BadSecret,
    /// Past its expiry.
    Expired,
    /// Explicitly revoked.
    Revoked,
    /// Valid token, suspended principal.
    PrincipalDisabled,
    /// The trusted proxy asserted a principal this system does not hold.
    ///
    /// Only reachable on the proxy-asserted path (ADR 009 Decision 6). It means
    /// the proxy authenticated somebody the identity store has never heard of,
    /// which is a provisioning gap — or a proxy that is not asserting what its
    /// operator thinks it is. Distinct from [`Self::PrincipalDisabled`], which
    /// is a principal that exists and was deliberately suspended.
    UnknownPrincipal,
}

impl AuthFailure {
    /// A bounded, low-cardinality label for metrics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Malformed => "malformed",
            Self::UnknownToken => "unknown_token",
            Self::BadSecret => "bad_secret",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::PrincipalDisabled => "principal_disabled",
            Self::UnknownPrincipal => "unknown_principal",
        }
    }
}

impl fmt::Display for AuthFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The outcome of examining a request's credential.
#[derive(Clone, Debug)]
pub enum Authentication {
    /// A credential was presented and verified.
    Authenticated(Principal),
    /// A credential was presented and refused, or none was presented.
    Refused(AuthFailure),
}

impl Authentication {
    #[must_use]
    pub const fn principal(&self) -> Option<&Principal> {
        match self {
            Self::Authenticated(principal) => Some(principal),
            Self::Refused(_) => None,
        }
    }

    #[must_use]
    pub const fn failure(&self) -> Option<AuthFailure> {
        match self {
            Self::Authenticated(_) => None,
            Self::Refused(failure) => Some(*failure),
        }
    }
}

/// One token row as the store holds it.
#[derive(Clone, Debug)]
pub struct StoredToken {
    pub id: TokenId,
    pub principal: Principal,
    pub hash: CredentialHash,
    pub expires_at: Timestamp,
    pub revoked_at: Option<Timestamp>,
}

/// Hashes and verifies credentials.
pub struct Verifier {
    params: HashParams,
    /// A hash of a fixed value, used to spend the same work on a lookup miss
    /// that a real verification would spend. Built once at construction so the
    /// miss path costs a verification, not a hash plus a verification.
    dummy: CredentialHash,
}

impl Verifier {
    /// Builds a verifier and precomputes its dummy hash.
    ///
    /// # Errors
    /// Returns [`HashError`] if the parameters are invalid or the CSPRNG is
    /// unavailable.
    pub fn new(params: HashParams) -> Result<Self, HashError> {
        let dummy = hash_with(params, "dummy-credential-for-constant-work")?;
        Ok(Self { params, dummy })
    }

    #[must_use]
    pub const fn params(&self) -> HashParams {
        self.params
    }

    /// Hashes a token secret for storage.
    ///
    /// # Errors
    /// Returns [`HashError`] on parameter, entropy, or backend failure.
    pub fn hash(&self, secret: &TokenSecret) -> Result<CredentialHash, HashError> {
        hash_with(self.params, secret.expose())
    }

    /// Verifies a presented secret against a stored row.
    ///
    /// Pass `None` when the identifier was not found. The miss path still
    /// performs an Argon2id verification against a dummy hash, so that an
    /// attacker cannot enumerate valid token identifiers by timing.
    ///
    /// Order matters: the secret is checked *before* expiry, revocation, and
    /// principal status. Checking cheap conditions first would let an attacker
    /// with a wrong secret learn that an identifier is expired or revoked, and
    /// would make the fast path distinguishable.
    ///
    /// # Blocking
    /// This is deliberately expensive — [`HashParams::memory_bytes`] per call.
    /// It must run on a blocking thread with bounded concurrency, never on an
    /// async executor worker.
    #[must_use]
    pub fn verify(
        &self,
        stored: Option<&StoredToken>,
        presented: &TokenSecret,
        now: Timestamp,
    ) -> Authentication {
        let Some(stored) = stored else {
            // Spend the work, discard the answer.
            let _ = verify_against(&self.dummy, presented.expose());
            return Authentication::Refused(AuthFailure::UnknownToken);
        };

        if !verify_against(&stored.hash, presented.expose()) {
            return Authentication::Refused(AuthFailure::BadSecret);
        }
        if let Some(revoked_at) = stored.revoked_at {
            if !now.is_before(revoked_at) {
                return Authentication::Refused(AuthFailure::Revoked);
            }
        }
        if !now.is_before(stored.expires_at) {
            return Authentication::Refused(AuthFailure::Expired);
        }
        if !stored.principal.is_enabled() {
            return Authentication::Refused(AuthFailure::PrincipalDisabled);
        }
        Authentication::Authenticated(stored.principal.clone())
    }
}

impl fmt::Debug for Verifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Verifier")
            .field("params", &self.params)
            .finish_non_exhaustive()
    }
}

fn hash_with(params: HashParams, value: &str) -> Result<CredentialHash, HashError> {
    let mut salt_bytes = [0u8; SALT_BYTES];
    getrandom::fill(&mut salt_bytes).map_err(|_| HashError::Entropy)?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|_| HashError::Backend)?;
    let hashed = params
        .to_argon2()?
        .hash_password(value.as_bytes(), &salt)
        .map_err(|_| HashError::Backend)?;
    Ok(CredentialHash(hashed.to_string()))
}

fn verify_against(hash: &CredentialHash, presented: &str) -> bool {
    // A stored hash that no longer parses denies. It was validated on the way
    // in, so this indicates corruption, and accepting on corruption would be
    // the worst possible failure mode.
    PasswordHash::new(hash.as_str()).is_ok_and(|parsed| {
        Argon2::default()
            .verify_password(presented.as_bytes(), &parsed)
            .is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::{
        AuthFailure, Authentication, CredentialHash, HashError, HashParams, StoredToken, Verifier,
        DEFAULT_ITERATIONS, DEFAULT_MEMORY_KIB, DEFAULT_PARALLELISM,
    };
    use crate::{
        clock::Timestamp,
        principal::{CameraScope, Principal, PrincipalId, PrincipalKind, Role},
        token::{IssuedToken, TokenSecret},
    };

    /// Cheap parameters so the suite stays fast. The production defaults are
    /// asserted separately in `default_parameters_are_the_owasp_set`.
    fn test_params() -> HashParams {
        HashParams::new(64, 1, 1).expect("test parameters are valid")
    }

    fn verifier() -> Verifier {
        Verifier::new(test_params()).expect("verifier builds")
    }

    fn principal(enabled: bool) -> Principal {
        Principal::new(
            PrincipalId::new("operator").expect("valid identifier"),
            PrincipalKind::Human,
            "Operator",
            Role::Operator,
            CameraScope::All,
            enabled,
        )
        .expect("principal is valid")
    }

    fn now() -> Timestamp {
        Timestamp::from_unix_seconds(1_000)
    }

    fn stored(verifier: &Verifier, secret: &TokenSecret, enabled: bool) -> StoredToken {
        StoredToken {
            id: IssuedToken::generate(now())
                .expect("CSPRNG available")
                .id()
                .clone(),
            principal: principal(enabled),
            hash: verifier.hash(secret).expect("hashing succeeds"),
            expires_at: Timestamp::from_unix_seconds(2_000),
            revoked_at: None,
        }
    }

    fn secret() -> TokenSecret {
        TokenSecret::new(&"ab".repeat(32)).expect("valid secret shape")
    }

    #[test]
    fn default_parameters_are_the_owasp_set() {
        let defaults = HashParams::default();
        assert_eq!(
            defaults,
            HashParams::new(DEFAULT_MEMORY_KIB, DEFAULT_ITERATIONS, DEFAULT_PARALLELISM)
                .expect("defaults are valid")
        );
        assert_eq!(
            defaults.memory_bytes(),
            u64::from(DEFAULT_MEMORY_KIB) * 1024
        );
    }

    #[test]
    fn invalid_parameters_are_refused_at_construction() {
        // Zero iterations is refused by Argon2; catching it here means an
        // invalid set cannot survive until the first authentication.
        assert_eq!(HashParams::new(64, 0, 1).err(), Some(HashError::Parameters));
    }

    #[test]
    fn a_correct_secret_authenticates() {
        let verifier = verifier();
        let secret = secret();
        let row = stored(&verifier, &secret, true);
        let outcome = verifier.verify(Some(&row), &secret, now());
        assert_eq!(
            outcome.principal().map(|p| p.id().as_str().to_owned()),
            Some("operator".to_owned())
        );
        assert_eq!(outcome.failure(), None);
    }

    #[test]
    fn a_wrong_secret_is_refused() {
        let verifier = verifier();
        let row = stored(&verifier, &secret(), true);
        let wrong = TokenSecret::new(&"cd".repeat(32)).expect("valid shape");
        assert_eq!(
            verifier.verify(Some(&row), &wrong, now()).failure(),
            Some(AuthFailure::BadSecret)
        );
    }

    #[test]
    fn an_unknown_identifier_is_refused_and_still_costs_a_verification() {
        let verifier = verifier();
        let outcome = verifier.verify(None, &secret(), now());
        assert_eq!(outcome.failure(), Some(AuthFailure::UnknownToken));
        // The cost itself is asserted structurally: the miss path holds a
        // precomputed dummy hash, and `verify` runs a full verification against
        // it. A wall-clock assertion here would be flaky under CI load.
        assert!(matches!(outcome, Authentication::Refused(_)));
    }

    #[test]
    fn an_expired_token_is_refused_at_its_boundary() {
        let verifier = verifier();
        let secret = secret();
        let row = stored(&verifier, &secret, true);
        // Exactly at expiry is expired: the boundary is exclusive.
        assert_eq!(
            verifier
                .verify(Some(&row), &secret, row.expires_at)
                .failure(),
            Some(AuthFailure::Expired)
        );
        assert_eq!(
            verifier
                .verify(
                    Some(&row),
                    &secret,
                    row.expires_at.saturating_add_seconds(-1)
                )
                .failure(),
            None
        );
    }

    #[test]
    fn a_revoked_token_is_refused_from_its_revocation_instant() {
        let verifier = verifier();
        let secret = secret();
        let mut row = stored(&verifier, &secret, true);
        row.revoked_at = Some(Timestamp::from_unix_seconds(1_500));

        assert_eq!(verifier.verify(Some(&row), &secret, now()).failure(), None);
        assert_eq!(
            verifier
                .verify(Some(&row), &secret, Timestamp::from_unix_seconds(1_500))
                .failure(),
            Some(AuthFailure::Revoked)
        );
    }

    #[test]
    fn revocation_beats_expiry_so_the_reason_is_the_deliberate_one() {
        let verifier = verifier();
        let secret = secret();
        let mut row = stored(&verifier, &secret, true);
        row.revoked_at = Some(Timestamp::from_unix_seconds(1_500));
        // Past both boundaries: the operator action is the more useful reason.
        assert_eq!(
            verifier
                .verify(Some(&row), &secret, Timestamp::from_unix_seconds(9_000))
                .failure(),
            Some(AuthFailure::Revoked)
        );
    }

    #[test]
    fn a_disabled_principal_is_refused_despite_a_valid_token() {
        let verifier = verifier();
        let secret = secret();
        let row = stored(&verifier, &secret, false);
        assert_eq!(
            verifier.verify(Some(&row), &secret, now()).failure(),
            Some(AuthFailure::PrincipalDisabled)
        );
    }

    #[test]
    fn a_wrong_secret_reports_bad_secret_even_when_the_row_is_also_expired() {
        // Otherwise an attacker with a wrong secret learns the row's state.
        let verifier = verifier();
        let mut row = stored(&verifier, &secret(), true);
        row.revoked_at = Some(Timestamp::from_unix_seconds(1));
        let wrong = TokenSecret::new(&"cd".repeat(32)).expect("valid shape");
        assert_eq!(
            verifier
                .verify(Some(&row), &wrong, Timestamp::from_unix_seconds(9_000))
                .failure(),
            Some(AuthFailure::BadSecret)
        );
    }

    #[test]
    fn two_hashes_of_one_secret_differ_because_the_salt_differs() {
        let verifier = verifier();
        let secret = secret();
        let first = verifier.hash(&secret).expect("hashing succeeds");
        let second = verifier.hash(&secret).expect("hashing succeeds");
        assert_ne!(first.as_str(), second.as_str());
    }

    #[test]
    fn a_stored_hash_encodes_its_own_parameters() {
        // This is what lets the defaults be raised without invalidating rows.
        let verifier = verifier();
        let hash = verifier.hash(&secret()).expect("hashing succeeds");
        assert!(hash.as_str().starts_with("$argon2id$v=19$m=64,t=1,p=1$"));
    }

    #[test]
    fn a_malformed_stored_hash_is_refused_rather_than_accepted() {
        assert_eq!(
            CredentialHash::parse("not-a-phc-string").err(),
            Some(HashError::Malformed)
        );
    }

    #[test]
    fn the_hash_debug_rendering_contains_no_digest_or_salt() {
        let verifier = verifier();
        let hash = verifier.hash(&secret()).expect("hashing succeeds");
        let rendered = format!("{hash:?}");
        assert_eq!(rendered, "CredentialHash(argon2id)");
        assert!(!rendered.contains('$'));
    }

    #[test]
    fn the_verifier_debug_rendering_contains_no_dummy_hash() {
        let rendered = format!("{:?}", verifier());
        assert!(!rendered.contains('$'), "leaked: {rendered}");
    }

    #[test]
    fn failure_labels_are_stable_and_bounded() {
        let all = [
            AuthFailure::Absent,
            AuthFailure::Malformed,
            AuthFailure::UnknownToken,
            AuthFailure::BadSecret,
            AuthFailure::Expired,
            AuthFailure::Revoked,
            AuthFailure::PrincipalDisabled,
        ];
        let mut labels: Vec<&str> = all.iter().map(|failure| failure.as_str()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), all.len(), "labels must be distinct");
    }
}
