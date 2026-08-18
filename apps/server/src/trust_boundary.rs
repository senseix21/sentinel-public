//! Where the server stops trusting the network.
//!
//! ADR 009 Decision 3. The server does not terminate TLS; a reverse proxy does,
//! and the server binds a private address behind it.
//!
//! That arrangement is only safe if the server can *prove* a request came from
//! the proxy. Every "trusted header" design that omits the proof is an
//! authentication bypass, because anything that can reach the socket can then
//! claim to be any client. The threat model ranks this the highest-severity
//! design risk in Phase 8.
//!
//! The proof has two independent parts, and **both** must hold before any
//! forwarded header is read:
//!
//! 1. the connection's peer address is in a configured allowlist; and
//! 2. the request carries the configured shared secret in
//!    [`PROOF_HEADER`], compared in constant time.
//!
//! On failure the forwarded value is *discarded* — not defaulted to something
//! permissive — and the peer address is used as the client.

use std::{
    fmt,
    net::IpAddr,
    sync::atomic::{AtomicU64, Ordering},
};

use subtle::ConstantTimeEq;
use surveillance_common::secret::Secret;

/// The header carrying proof that a request traversed the trusted proxy.
pub const PROOF_HEADER: &str = "x-forwarded-proof";

/// The header carrying the original client address.
pub const FORWARDED_FOR_HEADER: &str = "x-forwarded-for";

/// The header by which a proved proxy asserts *who* it authenticated.
///
/// ADR 009 Decision 6. This is the browser authentication path: the proxy
/// terminates TLS, authenticates the human by whatever means its operator
/// chose, and names the resulting principal here. The server does not verify a
/// credential on this path — there is none — so the header is worth exactly as
/// much as the proof of origin that accompanies it, and
/// [`ProofOutcome::proves_origin`] is what decides whether it is read at all.
pub const AUTHENTICATED_PRINCIPAL_HEADER: &str = "x-authenticated-principal";

/// The smallest accepted proof secret, in bytes.
///
/// A short shared secret is guessable by anything that can reach the socket,
/// which is exactly the adversary this control exists to stop.
pub const MINIMUM_PROOF_SECRET_LEN: usize = 32;

/// The largest number of proxy addresses that may be allowlisted.
pub const MAX_TRUSTED_PEERS: usize = 16;

/// How this deployment establishes who a request is from.
pub enum TrustBoundary {
    /// No proxy. The peer address is the client address, and forwarded headers
    /// are ignored entirely.
    Loopback,
    /// A reverse proxy terminates TLS and forwards. Origin must be proved.
    ReverseProxy {
        peers: Vec<IpAddr>,
        proof: Secret<String>,
    },
    // A `DirectTls` variant is reserved by ADR 009. It is deliberately absent
    // rather than stubbed: an unimplemented variant that silently behaves like
    // `Loopback` would be worse than not having it.
}

impl TrustBoundary {
    /// Builds a reverse-proxy boundary.
    ///
    /// # Errors
    /// Returns [`TrustBoundaryError`] for an empty or oversized peer list, or a
    /// proof secret shorter than [`MINIMUM_PROOF_SECRET_LEN`].
    pub fn reverse_proxy(
        peers: Vec<IpAddr>,
        proof: Secret<String>,
    ) -> Result<Self, TrustBoundaryError> {
        if peers.is_empty() {
            return Err(TrustBoundaryError::NoTrustedPeers);
        }
        if peers.len() > MAX_TRUSTED_PEERS {
            return Err(TrustBoundaryError::TooManyTrustedPeers);
        }
        if proof.expose().len() < MINIMUM_PROOF_SECRET_LEN {
            return Err(TrustBoundaryError::ProofSecretTooShort);
        }
        Ok(Self::ReverseProxy { peers, proof })
    }

    /// Whether this boundary can carry a non-loopback listener (ADR 009).
    #[must_use]
    pub const fn permits_non_loopback_bind(&self) -> bool {
        matches!(self, Self::ReverseProxy { .. })
    }

    /// Resolves who a request is from.
    ///
    /// `forwarded_for` and `proof` are the raw header values, if present.
    #[must_use]
    pub fn resolve(
        &self,
        peer: Option<IpAddr>,
        forwarded_for: Option<&str>,
        proof: Option<&str>,
    ) -> ResolvedClient {
        let resolved = self.decide(peer, forwarded_for, proof);
        count(resolved.outcome);
        resolved
    }

    fn decide(
        &self,
        peer: Option<IpAddr>,
        forwarded_for: Option<&str>,
        proof: Option<&str>,
    ) -> ResolvedClient {
        let Self::ReverseProxy {
            peers,
            proof: expected,
        } = self
        else {
            return ResolvedClient {
                client: peer,
                outcome: ProofOutcome::NotConfigured,
            };
        };

        let Some(peer_address) = peer else {
            return ResolvedClient {
                client: None,
                outcome: ProofOutcome::PeerNotAllowed,
            };
        };
        if !peers.contains(&peer_address) {
            return ResolvedClient {
                client: peer,
                outcome: ProofOutcome::PeerNotAllowed,
            };
        }
        let Some(presented) = proof else {
            return ResolvedClient {
                client: peer,
                outcome: ProofOutcome::ProofMissing,
            };
        };
        if !constant_time_eq(presented, expected.expose()) {
            return ResolvedClient {
                client: peer,
                outcome: ProofOutcome::ProofMismatch,
            };
        }

        // Origin is proved. Only now is the forwarded value read at all.
        match forwarded_for.and_then(rightmost_forwarded_address) {
            Some(client) => ResolvedClient {
                client: Some(client),
                outcome: ProofOutcome::Proved,
            },
            // Proved, but the proxy sent nothing usable. The peer — the proxy
            // itself — is the honest answer; inventing a client would be worse.
            None => ResolvedClient {
                client: peer,
                outcome: ProofOutcome::ProvedWithoutClient,
            },
        }
    }
}

impl fmt::Debug for TrustBoundary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loopback => formatter.write_str("TrustBoundary::Loopback"),
            Self::ReverseProxy { peers, .. } => formatter
                .debug_struct("TrustBoundary::ReverseProxy")
                .field("peers", peers)
                .finish_non_exhaustive(),
        }
    }
}

/// The result of establishing who a request is from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedClient {
    client: Option<IpAddr>,
    outcome: ProofOutcome,
}

impl ResolvedClient {
    #[must_use]
    pub const fn client(&self) -> Option<IpAddr> {
        self.client
    }

    #[must_use]
    pub const fn outcome(&self) -> ProofOutcome {
        self.outcome
    }
}

/// Why a forwarded client address was or was not accepted.
///
/// A bounded metric label. `PeerNotAllowed` and `ProofMismatch` firing on a
/// deployment that has a proxy configured means something is talking to the
/// server directly, which is worth an alert.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProofOutcome {
    /// No proxy configured; forwarded headers were never consulted.
    NotConfigured,
    /// Origin proved and a client address was forwarded.
    Proved,
    /// Origin proved, but no usable forwarded address was present.
    ProvedWithoutClient,
    /// The connection did not come from an allowlisted proxy.
    PeerNotAllowed,
    /// From an allowlisted peer, but with no proof header.
    ProofMissing,
    /// From an allowlisted peer, but the proof did not match.
    ProofMismatch,
}

impl ProofOutcome {
    /// Whether this outcome establishes that the request came from the trusted
    /// proxy.
    ///
    /// The gate for every proxy-asserted claim, including
    /// [`AUTHENTICATED_PRINCIPAL_HEADER`]. Only the two `Proved*` variants
    /// qualify: `NotConfigured` in particular does **not**, because a
    /// deployment with no proxy has nothing to distinguish the proxy's headers
    /// from any other caller's, and treating "no proxy" as "trusted" is the
    /// authentication bypass this module exists to prevent.
    #[must_use]
    pub const fn proves_origin(self) -> bool {
        matches!(self, Self::Proved | Self::ProvedWithoutClient)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::Proved => "proved",
            Self::ProvedWithoutClient => "proved_without_client",
            Self::PeerNotAllowed => "peer_not_allowed",
            Self::ProofMissing => "proof_missing",
            Self::ProofMismatch => "proof_mismatch",
        }
    }

    /// Whether the forwarded client address was trusted.
    #[must_use]
    pub const fn is_proved(self) -> bool {
        matches!(self, Self::Proved | Self::ProvedWithoutClient)
    }
}

/// Proof-of-origin counters for `/metrics`.
///
/// Six bounded labels from a six-variant enum. `peer_not_allowed` or
/// `proof_mismatch` rising on a deployment with a proxy configured means
/// something is reaching the server directly, which the P8.7 alert watches for.
static NOT_CONFIGURED: AtomicU64 = AtomicU64::new(0);
static PROVED: AtomicU64 = AtomicU64::new(0);
static PROVED_WITHOUT_CLIENT: AtomicU64 = AtomicU64::new(0);
static PEER_NOT_ALLOWED: AtomicU64 = AtomicU64::new(0);
static PROOF_MISSING: AtomicU64 = AtomicU64::new(0);
static PROOF_MISMATCH: AtomicU64 = AtomicU64::new(0);

/// Every proof counter as `(label, value)`.
#[must_use]
pub fn proof_counts() -> [(&'static str, u64); 6] {
    [
        (
            ProofOutcome::NotConfigured.as_str(),
            NOT_CONFIGURED.load(Ordering::Relaxed),
        ),
        (
            ProofOutcome::Proved.as_str(),
            PROVED.load(Ordering::Relaxed),
        ),
        (
            ProofOutcome::ProvedWithoutClient.as_str(),
            PROVED_WITHOUT_CLIENT.load(Ordering::Relaxed),
        ),
        (
            ProofOutcome::PeerNotAllowed.as_str(),
            PEER_NOT_ALLOWED.load(Ordering::Relaxed),
        ),
        (
            ProofOutcome::ProofMissing.as_str(),
            PROOF_MISSING.load(Ordering::Relaxed),
        ),
        (
            ProofOutcome::ProofMismatch.as_str(),
            PROOF_MISMATCH.load(Ordering::Relaxed),
        ),
    ]
}

fn count(outcome: ProofOutcome) {
    let counter = match outcome {
        ProofOutcome::NotConfigured => &NOT_CONFIGURED,
        ProofOutcome::Proved => &PROVED,
        ProofOutcome::ProvedWithoutClient => &PROVED_WITHOUT_CLIENT,
        ProofOutcome::PeerNotAllowed => &PEER_NOT_ALLOWED,
        ProofOutcome::ProofMissing => &PROOF_MISSING,
        ProofOutcome::ProofMismatch => &PROOF_MISMATCH,
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

/// A trust boundary that cannot be represented.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustBoundaryError {
    NoTrustedPeers,
    TooManyTrustedPeers,
    ProofSecretTooShort,
}

impl fmt::Display for TrustBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoTrustedPeers => {
                formatter.write_str("a reverse-proxy boundary needs at least one trusted peer")
            }
            Self::TooManyTrustedPeers => {
                write!(formatter, "at most {MAX_TRUSTED_PEERS} trusted peers")
            }
            Self::ProofSecretTooShort => write!(
                formatter,
                "the proof secret must be at least {MINIMUM_PROOF_SECRET_LEN} bytes"
            ),
        }
    }
}

impl std::error::Error for TrustBoundaryError {}

/// Takes the **rightmost** entry of `X-Forwarded-For`.
///
/// The leftmost entry is whatever the original client claimed and is fully
/// attacker-controlled; only the rightmost entry was appended by the proxy we
/// just proved we are talking to. Taking the leftmost is the classic spoofing
/// bug in this header.
fn rightmost_forwarded_address(header: &str) -> Option<IpAddr> {
    header
        .rsplit(',')
        .map(str::trim)
        .find(|candidate| !candidate.is_empty())
        .and_then(|candidate| {
            // Tolerate a bracketed IPv6 literal and an appended port, both of
            // which proxies emit.
            candidate
                .parse::<IpAddr>()
                .ok()
                .or_else(|| strip_port(candidate).and_then(|host| host.parse::<IpAddr>().ok()))
        })
}

fn strip_port(candidate: &str) -> Option<&str> {
    if let Some(rest) = candidate.strip_prefix('[') {
        return rest.split_once(']').map(|(host, _)| host);
    }
    // Only strip a port from something with exactly one colon; more than one
    // means a bare IPv6 literal, whose colons are part of the address.
    let mut parts = candidate.split(':');
    let host = parts.next()?;
    let port = parts.next()?;
    if parts.next().is_some() || port.is_empty() {
        return None;
    }
    Some(host)
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    // Length is not secret — a shared secret's length is a deployment fact, not
    // a per-request one — but the comparison itself must not short-circuit.
    left.len() == right.len() && bool::from(left.ct_eq(right))
}

#[cfg(test)]
mod tests {
    use super::{
        constant_time_eq, rightmost_forwarded_address, ProofOutcome, TrustBoundary,
        TrustBoundaryError, MAX_TRUSTED_PEERS, MINIMUM_PROOF_SECRET_LEN,
    };
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use surveillance_common::secret::Secret;

    const PROOF: &str = "0123456789abcdef0123456789abcdef";

    fn proxy() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))
    }

    fn stranger() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7))
    }

    fn boundary() -> TrustBoundary {
        TrustBoundary::reverse_proxy(vec![proxy()], Secret::new(PROOF.to_owned()))
            .expect("boundary is valid")
    }

    #[test]
    fn a_proof_secret_must_be_long_enough_to_matter() {
        assert_eq!(
            TrustBoundary::reverse_proxy(vec![proxy()], Secret::new("short".to_owned())).err(),
            Some(TrustBoundaryError::ProofSecretTooShort)
        );
        assert!(TrustBoundary::reverse_proxy(
            vec![proxy()],
            Secret::new("x".repeat(MINIMUM_PROOF_SECRET_LEN))
        )
        .is_ok());
    }

    #[test]
    fn a_proxy_boundary_needs_a_bounded_non_empty_peer_list() {
        assert_eq!(
            TrustBoundary::reverse_proxy(Vec::new(), Secret::new(PROOF.to_owned())).err(),
            Some(TrustBoundaryError::NoTrustedPeers)
        );
        let too_many = (0..=MAX_TRUSTED_PEERS)
            .map(|index| {
                IpAddr::V4(Ipv4Addr::new(
                    10,
                    0,
                    0,
                    u8::try_from(index).unwrap_or(u8::MAX),
                ))
            })
            .collect();
        assert_eq!(
            TrustBoundary::reverse_proxy(too_many, Secret::new(PROOF.to_owned())).err(),
            Some(TrustBoundaryError::TooManyTrustedPeers)
        );
    }

    #[test]
    fn a_proved_request_takes_the_forwarded_client() {
        let resolved = boundary().resolve(Some(proxy()), Some("203.0.113.9"), Some(PROOF));
        assert_eq!(resolved.outcome(), ProofOutcome::Proved);
        assert_eq!(
            resolved.client(),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)))
        );
    }

    #[test]
    fn a_spoofed_header_from_a_stranger_is_discarded() {
        // The whole point: anything that reaches the socket directly must not
        // be able to claim an identity by setting a header.
        let resolved = boundary().resolve(Some(stranger()), Some("203.0.113.9"), Some(PROOF));
        assert_eq!(resolved.outcome(), ProofOutcome::PeerNotAllowed);
        assert_eq!(
            resolved.client(),
            Some(stranger()),
            "the real peer must survive, not the claimed client"
        );
    }

    #[test]
    fn an_allowlisted_peer_without_the_proof_is_refused() {
        let resolved = boundary().resolve(Some(proxy()), Some("203.0.113.9"), None);
        assert_eq!(resolved.outcome(), ProofOutcome::ProofMissing);
        assert_eq!(resolved.client(), Some(proxy()));
    }

    #[test]
    fn an_allowlisted_peer_with_a_wrong_proof_is_refused() {
        let resolved = boundary().resolve(Some(proxy()), Some("203.0.113.9"), Some("wrong"));
        assert_eq!(resolved.outcome(), ProofOutcome::ProofMismatch);
        assert_eq!(resolved.client(), Some(proxy()));
    }

    #[test]
    fn an_unknown_peer_cannot_be_proved() {
        let resolved = boundary().resolve(None, Some("203.0.113.9"), Some(PROOF));
        assert_eq!(resolved.outcome(), ProofOutcome::PeerNotAllowed);
        assert_eq!(resolved.client(), None);
    }

    #[test]
    fn a_loopback_boundary_never_consults_forwarded_headers() {
        let resolved =
            TrustBoundary::Loopback.resolve(Some(stranger()), Some("203.0.113.9"), Some(PROOF));
        assert_eq!(resolved.outcome(), ProofOutcome::NotConfigured);
        assert_eq!(resolved.client(), Some(stranger()));
    }

    #[test]
    fn a_proved_request_with_no_usable_forwarded_value_falls_back_to_the_proxy() {
        for header in [None, Some(""), Some("not-an-address"), Some(" , ")] {
            let resolved = boundary().resolve(Some(proxy()), header, Some(PROOF));
            assert_eq!(
                resolved.outcome(),
                ProofOutcome::ProvedWithoutClient,
                "header {header:?}"
            );
            assert_eq!(resolved.client(), Some(proxy()));
        }
    }

    #[test]
    fn the_rightmost_forwarded_entry_wins() {
        // The leftmost entry is attacker-supplied; only the rightmost was
        // appended by the proxy whose origin we proved.
        assert_eq!(
            rightmost_forwarded_address("203.0.113.1, 203.0.113.2, 203.0.113.3"),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 3)))
        );
    }

    #[test]
    fn a_forged_leftmost_entry_cannot_win_even_when_it_is_the_only_valid_one() {
        // An attacker sets `X-Forwarded-For: 127.0.0.1` hoping to look local.
        // The proxy appends its view; the rightmost is what we take.
        let resolved =
            boundary().resolve(Some(proxy()), Some("127.0.0.1, 203.0.113.9"), Some(PROOF));
        assert_eq!(
            resolved.client(),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)))
        );
    }

    #[test]
    fn forwarded_addresses_tolerate_ports_and_ipv6_literals() {
        assert_eq!(
            rightmost_forwarded_address("203.0.113.9:41234"),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)))
        );
        assert_eq!(
            rightmost_forwarded_address("2001:db8::1"),
            Some(IpAddr::V6(
                "2001:db8::1".parse::<Ipv6Addr>().expect("valid")
            ))
        );
        assert_eq!(
            rightmost_forwarded_address("[2001:db8::1]:443"),
            Some(IpAddr::V6(
                "2001:db8::1".parse::<Ipv6Addr>().expect("valid")
            ))
        );
    }

    #[test]
    fn only_a_reverse_proxy_boundary_permits_a_non_loopback_bind() {
        assert!(!TrustBoundary::Loopback.permits_non_loopback_bind());
        assert!(boundary().permits_non_loopback_bind());
    }

    #[test]
    fn the_debug_rendering_never_contains_the_proof_secret() {
        let rendered = format!("{:?}", boundary());
        assert!(!rendered.contains(PROOF), "proof leaked into: {rendered}");
    }

    #[test]
    fn constant_time_comparison_agrees_with_equality() {
        assert!(constant_time_eq(PROOF, PROOF));
        assert!(!constant_time_eq(PROOF, "wrong"));
        assert!(!constant_time_eq(PROOF, &PROOF[..PROOF.len() - 1]));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn proof_outcome_labels_are_distinct_and_bounded() {
        let all = [
            ProofOutcome::NotConfigured,
            ProofOutcome::Proved,
            ProofOutcome::ProvedWithoutClient,
            ProofOutcome::PeerNotAllowed,
            ProofOutcome::ProofMissing,
            ProofOutcome::ProofMismatch,
        ];
        let mut labels: Vec<&str> = all.iter().map(|outcome| outcome.as_str()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), all.len());
        assert!(ProofOutcome::Proved.is_proved());
        assert!(!ProofOutcome::PeerNotAllowed.is_proved());
    }
}
