//! Loading the trust boundary, and the rule that unlocks a routable listener.
//!
//! ADR 009 Decisions 2 and 3. The important thing here is not the parsing — it
//! is [`resolve_bind`], which makes the non-loopback unlock a *validation rule*
//! rather than a paragraph in a runbook.
//!
//! Phase 7 refused every non-loopback bind outright
//! (`EnvironmentConfigError::HealthBindNotLoopback`). That lock was deliberate,
//! and the handoff is explicit that it must be unlocked *together with* TLS and
//! authentication, never before. So the unlock is conditional and the
//! conditions are checked here, in one place, with a test per condition.

use std::{fmt, net::IpAddr, net::SocketAddr};

use surveillance_common::secret::Secret;

use crate::trust_boundary::{TrustBoundary, TrustBoundaryError, MINIMUM_PROOF_SECRET_LEN};

/// Comma-separated addresses of the reverse proxies permitted to forward.
pub const TRUSTED_PROXY_PEERS: &str = "SURVEILLANCE_TRUSTED_PROXY_PEERS";
/// The shared secret a forwarded request must carry.
pub const PROXY_PROOF_SECRET: &str = "SURVEILLANCE_PROXY_PROOF_SECRET";

/// Builds the trust boundary from the environment.
///
/// Absent configuration yields [`TrustBoundary::Loopback`] — the Phase 2–7
/// posture — rather than a permissive default.
///
/// # Errors
/// Returns [`SecurityConfigError`] when the two variables disagree (one set
/// without the other), when a peer address does not parse, or when the proof
/// secret is too short to be worth having.
pub fn load_trust_boundary(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<TrustBoundary, SecurityConfigError> {
    let peers = lookup(TRUSTED_PROXY_PEERS).filter(|value| !value.trim().is_empty());
    let proof = lookup(PROXY_PROOF_SECRET).filter(|value| !value.is_empty());

    match (peers, proof) {
        (None, None) => Ok(TrustBoundary::Loopback),
        // Half a control is not a control. A peer allowlist without a proof
        // secret would trust any request from the proxy's address, including
        // one from anything else that can spoof or share it.
        (Some(_), None) => Err(SecurityConfigError::ProofSecretMissing),
        (None, Some(_)) => Err(SecurityConfigError::TrustedPeersMissing),
        (Some(peers), Some(proof)) => {
            let parsed = parse_peers(&peers)?;
            TrustBoundary::reverse_proxy(parsed, Secret::new(proof))
                .map_err(SecurityConfigError::Boundary)
        }
    }
}

fn parse_peers(value: &str) -> Result<Vec<IpAddr>, SecurityConfigError> {
    let mut peers = Vec::new();
    for candidate in value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let address = candidate
            .parse::<IpAddr>()
            .map_err(|_| SecurityConfigError::InvalidPeerAddress)?;
        if !peers.contains(&address) {
            peers.push(address);
        }
    }
    if peers.is_empty() {
        return Err(SecurityConfigError::TrustedPeersMissing);
    }
    Ok(peers)
}

/// Decides whether the requested listener address may be used.
///
/// A loopback address is always permitted. A routable address requires **both**
/// ADR 009 conditions, and the error names which one is missing:
///
/// - a configured trust boundary, because otherwise every peer is anonymous and
///   TLS terminates nowhere; and
/// - a configured identity store with at least one enabled principal, because
///   otherwise the listener would fall back to
///   [`Principal::loopback_development`](surveillance_auth::principal::Principal::loopback_development),
///   which grants administrator to anyone who can reach the socket.
///
/// The second condition is the one that is easy to forget, and forgetting it
/// turns the development posture into an authentication bypass.
///
/// # Errors
/// Returns [`SecurityConfigError::BindRequiresTrustBoundary`] or
/// [`SecurityConfigError::BindRequiresAuthentication`].
pub fn resolve_bind(
    address: SocketAddr,
    boundary: &TrustBoundary,
    has_enabled_principal: bool,
) -> Result<SocketAddr, SecurityConfigError> {
    if address.ip().is_loopback() {
        return Ok(address);
    }
    if !boundary.permits_non_loopback_bind() {
        return Err(SecurityConfigError::BindRequiresTrustBoundary);
    }
    if !has_enabled_principal {
        return Err(SecurityConfigError::BindRequiresAuthentication);
    }
    Ok(address)
}

/// A security configuration that cannot be used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecurityConfigError {
    TrustedPeersMissing,
    ProofSecretMissing,
    InvalidPeerAddress,
    Boundary(TrustBoundaryError),
    BindRequiresTrustBoundary,
    BindRequiresAuthentication,
}

impl fmt::Display for SecurityConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrustedPeersMissing => write!(
                formatter,
                "{PROXY_PROOF_SECRET} is set but {TRUSTED_PROXY_PEERS} names no usable address"
            ),
            Self::ProofSecretMissing => write!(
                formatter,
                "{TRUSTED_PROXY_PEERS} is set without {PROXY_PROOF_SECRET}; \
                 a peer allowlist alone is not proof of origin"
            ),
            Self::InvalidPeerAddress => {
                write!(
                    formatter,
                    "{TRUSTED_PROXY_PEERS} contains an unparseable IP address"
                )
            }
            Self::Boundary(error) => write!(formatter, "{error}"),
            Self::BindRequiresTrustBoundary => write!(
                formatter,
                "a non-loopback listener requires a reverse-proxy trust boundary; \
                 set {TRUSTED_PROXY_PEERS} and {PROXY_PROOF_SECRET} \
                 (at least {MINIMUM_PROOF_SECRET_LEN} bytes)"
            ),
            Self::BindRequiresAuthentication => formatter.write_str(
                "a non-loopback listener requires a configured identity store with at least \
                 one enabled principal; without it every caller would be granted administrator",
            ),
        }
    }
}

impl std::error::Error for SecurityConfigError {}

#[cfg(test)]
mod tests {
    use super::{
        load_trust_boundary, resolve_bind, SecurityConfigError, PROXY_PROOF_SECRET,
        TRUSTED_PROXY_PEERS,
    };
    use crate::trust_boundary::{TrustBoundary, TrustBoundaryError, MINIMUM_PROOF_SECRET_LEN};
    use std::net::SocketAddr;

    const PROOF: &str = "0123456789abcdef0123456789abcdef";

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |key| {
            owned
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, value)| value.clone())
        }
    }

    fn address(value: &str) -> SocketAddr {
        value.parse().expect("test address parses")
    }

    #[test]
    fn absent_configuration_is_loopback_not_permissive() {
        let boundary = load_trust_boundary(lookup(&[])).expect("absent configuration is legal");
        assert!(matches!(boundary, TrustBoundary::Loopback));
        assert!(!boundary.permits_non_loopback_bind());
    }

    #[test]
    fn a_full_configuration_builds_a_reverse_proxy_boundary() {
        let boundary = load_trust_boundary(lookup(&[
            (TRUSTED_PROXY_PEERS, "10.0.0.2, 10.0.0.3"),
            (PROXY_PROOF_SECRET, PROOF),
        ]))
        .expect("configuration is valid");
        assert!(boundary.permits_non_loopback_bind());
    }

    #[test]
    fn a_peer_allowlist_without_a_proof_secret_is_refused() {
        // This is the authentication-bypass shape the threat model ranks
        // highest: trusting an address without proving origin.
        assert_eq!(
            load_trust_boundary(lookup(&[(TRUSTED_PROXY_PEERS, "10.0.0.2")])).err(),
            Some(SecurityConfigError::ProofSecretMissing)
        );
    }

    #[test]
    fn a_proof_secret_without_a_peer_allowlist_is_refused() {
        assert_eq!(
            load_trust_boundary(lookup(&[(PROXY_PROOF_SECRET, PROOF)])).err(),
            Some(SecurityConfigError::TrustedPeersMissing)
        );
    }

    #[test]
    fn an_unparseable_peer_is_refused_rather_than_skipped() {
        // Skipping it would silently shrink the allowlist, and a shrunk
        // allowlist fails closed in a way that looks like an outage.
        assert_eq!(
            load_trust_boundary(lookup(&[
                (TRUSTED_PROXY_PEERS, "10.0.0.2, not-an-address"),
                (PROXY_PROOF_SECRET, PROOF),
            ]))
            .err(),
            Some(SecurityConfigError::InvalidPeerAddress)
        );
    }

    #[test]
    fn a_short_proof_secret_is_refused() {
        assert_eq!(
            load_trust_boundary(lookup(&[
                (TRUSTED_PROXY_PEERS, "10.0.0.2"),
                (PROXY_PROOF_SECRET, "short"),
            ]))
            .err(),
            Some(SecurityConfigError::Boundary(
                TrustBoundaryError::ProofSecretTooShort
            ))
        );
        assert!(load_trust_boundary(lookup(&[
            (TRUSTED_PROXY_PEERS, "10.0.0.2"),
            (PROXY_PROOF_SECRET, &"x".repeat(MINIMUM_PROOF_SECRET_LEN)),
        ]))
        .is_ok());
    }

    #[test]
    fn loopback_binds_without_any_security_configuration() {
        // The Phase 2-7 posture must keep working unchanged.
        for candidate in ["127.0.0.1:8080", "[::1]:8080"] {
            assert!(resolve_bind(address(candidate), &TrustBoundary::Loopback, false).is_ok());
        }
    }

    #[test]
    fn a_routable_bind_without_a_trust_boundary_is_refused() {
        assert_eq!(
            resolve_bind(address("0.0.0.0:8080"), &TrustBoundary::Loopback, true).err(),
            Some(SecurityConfigError::BindRequiresTrustBoundary)
        );
    }

    #[test]
    fn a_routable_bind_without_an_enabled_principal_is_refused() {
        // Without this the listener would fall back to the development
        // identity, which grants administrator to anything that connects.
        let boundary = load_trust_boundary(lookup(&[
            (TRUSTED_PROXY_PEERS, "10.0.0.2"),
            (PROXY_PROOF_SECRET, PROOF),
        ]))
        .expect("configuration is valid");
        assert_eq!(
            resolve_bind(address("10.0.0.5:8080"), &boundary, false).err(),
            Some(SecurityConfigError::BindRequiresAuthentication)
        );
    }

    #[test]
    fn a_routable_bind_is_permitted_only_when_both_conditions_hold() {
        let boundary = load_trust_boundary(lookup(&[
            (TRUSTED_PROXY_PEERS, "10.0.0.2"),
            (PROXY_PROOF_SECRET, PROOF),
        ]))
        .expect("configuration is valid");
        assert!(resolve_bind(address("10.0.0.5:8080"), &boundary, true).is_ok());
    }

    #[test]
    fn the_refusals_name_the_missing_condition() {
        // An operator hitting this at 3am needs to know which half is absent.
        let rendered = SecurityConfigError::BindRequiresTrustBoundary.to_string();
        assert!(rendered.contains(TRUSTED_PROXY_PEERS));
        assert!(rendered.contains(PROXY_PROOF_SECRET));

        let rendered = SecurityConfigError::BindRequiresAuthentication.to_string();
        assert!(rendered.contains("enabled principal"));
    }

    #[test]
    fn no_error_rendering_contains_the_proof_secret() {
        for error in [
            SecurityConfigError::TrustedPeersMissing,
            SecurityConfigError::ProofSecretMissing,
            SecurityConfigError::InvalidPeerAddress,
            SecurityConfigError::BindRequiresTrustBoundary,
            SecurityConfigError::BindRequiresAuthentication,
        ] {
            assert!(!error.to_string().contains(PROOF), "{error:?} leaked");
        }
    }
}
