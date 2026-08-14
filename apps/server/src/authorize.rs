//! The single authorization chokepoint for mutating configuration requests.
//!
//! ADR 008 Decision 1. Two controls, because they answer different questions:
//!
//! - [`MutatingSurface`] answers "is this deployment allowed to be reconfigured
//!   over HTTP at all". That is a deployment posture, and it defaults to off.
//! - [`authorize`] answers "may this caller perform this action". That is a
//!   request property.
//!
//! [`Authorized`] has a private field and no public constructor, so it can only
//! be produced by [`authorize`]. Every mutating handler takes one by value. A
//! handler that skips the check cannot obtain one and does not compile — which
//! is what makes "every mutating endpoint reaches the chokepoint" provable
//! rather than a matter of review discipline.
//!
//! In Phase 7 the body grants on loopback. Phase 8 grows an identity model. The
//! seam does not move; only the body does.

use std::{
    fmt,
    net::IpAddr,
    sync::atomic::{AtomicU64, Ordering},
};

use surveillance_config::repository::Actor;

/// Whether this deployment exposes the mutating configuration surface.
///
/// Defaults to off. Loopback is not a security boundary on a multi-user host,
/// and the next phase binds beyond loopback; a default that is safe only under
/// the current listener configuration is a latent defect.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MutatingSurface {
    /// The default, per ADR 008: a deployment must opt in.
    #[default]
    Disabled,
    Enabled,
}

impl MutatingSurface {
    #[must_use]
    pub const fn from_enabled(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }

    #[must_use]
    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// What a caller is asking to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    /// Change global operational settings.
    UpdateGlobalSettings,
    /// Change one camera's operational settings.
    UpdateCameraSettings,
}

impl Action {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UpdateGlobalSettings => "update_global_settings",
            Self::UpdateCameraSettings => "update_camera_settings",
        }
    }
}

/// What the adapter knows about an incoming request.
#[derive(Clone, Copy, Debug)]
pub struct RequestContext {
    peer: Option<IpAddr>,
    surface: MutatingSurface,
    shutting_down: bool,
}

impl RequestContext {
    #[must_use]
    pub const fn new(peer: Option<IpAddr>, surface: MutatingSurface, shutting_down: bool) -> Self {
        Self {
            peer,
            surface,
            shutting_down,
        }
    }

    #[must_use]
    pub const fn surface(&self) -> MutatingSurface {
        self.surface
    }

    #[must_use]
    pub const fn shutting_down(&self) -> bool {
        self.shutting_down
    }
}

/// Proof that [`authorize`] granted a specific action.
///
/// Constructible only inside this module. Passed by value so a handler cannot
/// hold one and reuse it for a second, unauthorized action.
#[derive(Debug)]
pub struct Authorized {
    action: Action,
    actor: Actor,
}

impl Authorized {
    #[must_use]
    pub const fn action(&self) -> Action {
        self.action
    }

    /// The identity to record in the audit trail.
    #[must_use]
    pub const fn actor(&self) -> &Actor {
        &self.actor
    }
}

/// Why a request was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Denied {
    /// The mutating surface is off for this deployment. Handlers render this as
    /// `404`, not `403`: a disabled surface does not advertise itself.
    SurfaceDisabled,
    /// The caller is not permitted.
    Unauthorized,
    /// The server is shutting down and will not accept changes.
    ShuttingDown,
}

impl fmt::Display for Denied {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SurfaceDisabled => formatter.write_str("configuration API is disabled"),
            Self::Unauthorized => formatter.write_str("not authorized"),
            Self::ShuttingDown => formatter.write_str("server is shutting down"),
        }
    }
}

/// Counts every call, so a test can prove a route reached this function.
///
/// Failure-matrix item 12 requires proof by test rather than by inspection. The
/// type system proves a handler cannot *skip* the check; this counter proves a
/// given route actually *runs* it.
static AUTHORIZE_CALLS: AtomicU64 = AtomicU64::new(0);

/// Reads the chokepoint call count.
#[must_use]
pub fn authorize_call_count() -> u64 {
    AUTHORIZE_CALLS.load(Ordering::Relaxed)
}

/// The single decision point for mutating configuration requests.
///
/// # Errors
/// Returns [`Denied`] with the reason. Order matters: a disabled surface is
/// reported before authorization, so a deployment that has not opted in reveals
/// nothing about who would have been allowed.
pub fn authorize(request: &RequestContext, action: Action) -> Result<Authorized, Denied> {
    AUTHORIZE_CALLS.fetch_add(1, Ordering::Relaxed);

    if !request.surface.is_enabled() {
        return Err(Denied::SurfaceDisabled);
    }
    if request.shutting_down {
        return Err(Denied::ShuttingDown);
    }
    match request.peer {
        Some(peer) if peer.is_loopback() => Ok(Authorized {
            action,
            actor: Actor::loopback(),
        }),
        // No peer address means the adapter could not establish who is calling.
        // Absence of evidence is not evidence of loopback.
        _ => Err(Denied::Unauthorized),
    }
}

#[cfg(test)]
mod tests {
    use super::{authorize, authorize_call_count, Action, Denied, MutatingSurface, RequestContext};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    fn loopback() -> IpAddr {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    }

    #[test]
    fn the_surface_is_disabled_by_default() {
        assert_eq!(MutatingSurface::default(), MutatingSurface::Disabled);
        assert!(!MutatingSurface::default().is_enabled());
    }

    #[test]
    fn a_disabled_surface_refuses_even_a_loopback_caller() {
        let request = RequestContext::new(Some(loopback()), MutatingSurface::Disabled, false);
        assert_eq!(
            authorize(&request, Action::UpdateGlobalSettings).err(),
            Some(Denied::SurfaceDisabled)
        );
    }

    #[test]
    fn an_enabled_surface_grants_a_loopback_caller() {
        let request = RequestContext::new(Some(loopback()), MutatingSurface::Enabled, false);
        let granted =
            authorize(&request, Action::UpdateGlobalSettings).expect("loopback is granted");
        assert_eq!(granted.action(), Action::UpdateGlobalSettings);
        assert_eq!(granted.actor().as_str(), "loopback");
    }

    #[test]
    fn ipv6_loopback_is_granted_too() {
        let request = RequestContext::new(
            Some(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            MutatingSurface::Enabled,
            false,
        );
        assert!(authorize(&request, Action::UpdateCameraSettings).is_ok());
    }

    #[test]
    fn a_non_loopback_caller_is_refused() {
        let request = RequestContext::new(
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))),
            MutatingSurface::Enabled,
            false,
        );
        assert_eq!(
            authorize(&request, Action::UpdateGlobalSettings).err(),
            Some(Denied::Unauthorized)
        );
    }

    #[test]
    fn an_unknown_peer_is_refused_rather_than_assumed_local() {
        let request = RequestContext::new(None, MutatingSurface::Enabled, false);
        assert_eq!(
            authorize(&request, Action::UpdateGlobalSettings).err(),
            Some(Denied::Unauthorized)
        );
    }

    #[test]
    fn shutdown_refuses_changes() {
        let request = RequestContext::new(Some(loopback()), MutatingSurface::Enabled, true);
        assert_eq!(
            authorize(&request, Action::UpdateGlobalSettings).err(),
            Some(Denied::ShuttingDown)
        );
    }

    #[test]
    fn a_disabled_surface_is_reported_before_authorization() {
        // A deployment that has not opted in must not reveal who would have
        // been allowed.
        let request = RequestContext::new(
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))),
            MutatingSurface::Disabled,
            false,
        );
        assert_eq!(
            authorize(&request, Action::UpdateGlobalSettings).err(),
            Some(Denied::SurfaceDisabled)
        );
    }

    #[test]
    fn every_call_is_counted() {
        let before = authorize_call_count();
        let request = RequestContext::new(Some(loopback()), MutatingSurface::Disabled, false);
        let _ = authorize(&request, Action::UpdateGlobalSettings);
        let _ = authorize(&request, Action::UpdateCameraSettings);
        assert!(
            authorize_call_count() >= before + 2,
            "the chokepoint must count refusals as well as grants"
        );
    }
}
