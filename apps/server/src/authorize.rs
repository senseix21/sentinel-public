//! The single authorization chokepoint.
//!
//! ADR 008 Decision 1 established the seam. ADR 009 fills it.
//!
//! Three controls, because they answer three different questions:
//!
//! - [`MutatingSurface`] answers "is this deployment allowed to be
//!   *reconfigured* over HTTP at all". That is a deployment posture, and it
//!   defaults to off. It gates configuration mutation only — camera lifecycle
//!   and reads are governed by identity, not by a deployment toggle.
//! - Authentication answers "who is this". It happens in the HTTP adapter,
//!   because verifying an Argon2id credential is memory-hard and must not run
//!   on an async executor thread. The resolved [`Authentication`] is handed to
//!   this module.
//! - [`authorize`] answers "may this caller perform this action on this
//!   camera". That is a request property and it is decided here, once.
//!
//! [`Authorized`] has a private field and no public constructor, so it can only
//! be produced by [`authorize`]. Every protected handler takes one by value. A
//! handler that skips the check cannot obtain one and does not compile — which
//! is what makes "every protected endpoint reaches the chokepoint" provable
//! rather than a matter of review discipline.
//!
//! ## Why a denied camera answers 404
//!
//! [`Denied::NotInScope`] renders as `404`, not `403`. A `403` confirms that
//! the camera exists, which lets a scoped operator enumerate the estate.

use std::{
    fmt,
    net::IpAddr,
    sync::atomic::{AtomicU64, Ordering},
};

use surveillance_auth::{
    principal::{Principal, Role},
    verify::{AuthFailure, Authentication},
};
use surveillance_common::types::CameraId;
use surveillance_config::repository::Actor;

/// Whether this deployment exposes the mutating configuration surface.
///
/// Defaults to off. Loopback is not a security boundary on a multi-user host,
/// and a default that is safe only under the current listener configuration is
/// a latent defect.
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

/// What a caller is asking to do, and to which camera.
///
/// One variant per protected operation. Adding a route means adding a variant,
/// which means the role table below stops compiling until the new operation's
/// required role is stated — the enumeration is the point.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    /// List the cameras this principal can see.
    ListCameras,
    /// Read one camera's state and health.
    ViewCamera(CameraId),
    /// Search or read recording metadata for one camera.
    ViewRecordings(CameraId),
    /// Obtain a live viewer token for one camera.
    IssueViewerToken(CameraId),
    /// Remove footage from the system's control. Audited most carefully.
    ExportRecording(CameraId),
    /// Read effective configuration.
    ViewSettings,
    /// Read what this deployment can and cannot do.
    ///
    /// Authenticated rather than open, even though it exposes no footage and no
    /// identity: it describes the deployment's shape — which optional surfaces
    /// are wired in — and that is reconnaissance worth withholding from an
    /// anonymous caller.
    ViewSystemCapabilities,
    /// List identities and their tokens. Never their secrets.
    ViewIdentities,
    /// Bring one camera's pipeline up.
    StartCamera(CameraId),
    /// Take one camera's pipeline down.
    StopCamera(CameraId),
    /// Add a camera to the supervisor.
    AddCamera,
    /// Remove a camera from the supervisor.
    RemoveCamera(CameraId),
    /// Change global operational settings.
    UpdateGlobalSettings,
    /// Change one camera's operational settings.
    UpdateCameraSettings(CameraId),
    /// Create, modify, or revoke identities and tokens.
    ManageIdentities,
}

/// Which cameras an action touches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionScope<'a> {
    /// The action is not camera-specific.
    Global,
    /// The action touches exactly this camera.
    Camera(&'a CameraId),
}

impl Action {
    /// A bounded, low-cardinality label for audit rows and metrics.
    ///
    /// Deliberately excludes the camera id: this string is a metric label, and
    /// camera identity already has its own bounded label elsewhere.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ListCameras => "list_cameras",
            Self::ViewCamera(_) => "view_camera",
            Self::ViewRecordings(_) => "view_recordings",
            Self::IssueViewerToken(_) => "issue_viewer_token",
            Self::ExportRecording(_) => "export_recording",
            Self::ViewSettings => "view_settings",
            Self::ViewSystemCapabilities => "view_system_capabilities",
            Self::ViewIdentities => "view_identities",
            Self::StartCamera(_) => "start_camera",
            Self::StopCamera(_) => "stop_camera",
            Self::AddCamera => "add_camera",
            Self::RemoveCamera(_) => "remove_camera",
            Self::UpdateGlobalSettings => "update_global_settings",
            Self::UpdateCameraSettings(_) => "update_camera_settings",
            Self::ManageIdentities => "manage_identities",
        }
    }

    /// The camera this action touches, if any.
    #[must_use]
    pub const fn scope(&self) -> ActionScope<'_> {
        match self {
            Self::ListCameras
            | Self::ViewSettings
            | Self::ViewSystemCapabilities
            | Self::ViewIdentities
            | Self::AddCamera
            | Self::UpdateGlobalSettings
            | Self::ManageIdentities => ActionScope::Global,
            Self::ViewCamera(camera)
            | Self::ViewRecordings(camera)
            | Self::IssueViewerToken(camera)
            | Self::ExportRecording(camera)
            | Self::StartCamera(camera)
            | Self::StopCamera(camera)
            | Self::RemoveCamera(camera)
            | Self::UpdateCameraSettings(camera) => ActionScope::Camera(camera),
        }
    }

    /// The least privileged role permitted to perform this action.
    ///
    /// Roles are ordered, so "permitted" is `principal.role() >= required`.
    /// Exhaustive by construction: a new variant does not compile until it
    /// appears here.
    #[must_use]
    pub const fn required_role(&self) -> Role {
        match self {
            Self::ListCameras
            | Self::ViewCamera(_)
            | Self::ViewRecordings(_)
            | Self::IssueViewerToken(_)
            | Self::ViewSettings
            | Self::ViewSystemCapabilities
            // Viewer, not Operator. Watching recorded footage is what the
            // Viewer role is *for*, and `/recordings/playback` is the only way
            // media leaves this system — so requiring Operator would not make
            // export rarer, it would make the Viewer role useless.
            //
            // The separation that earns its keep is the audit category, not the
            // role: an export row records who, which camera, the exact range,
            // and how many bytes left. The threat model (B7) is explicit that
            // insider export cannot be prevented, only detected.
            //
            // Raise this to Operator if the product wants footage gated above
            // metadata. It is a one-line change and the exhaustive role matrix
            // covers whichever way it is set.
            | Self::ExportRecording(_) => Role::Viewer,

            Self::StartCamera(_)
            | Self::StopCamera(_)
            | Self::AddCamera
            | Self::RemoveCamera(_) => Role::Operator,

            Self::ViewIdentities
            | Self::UpdateGlobalSettings
            | Self::UpdateCameraSettings(_)
            | Self::ManageIdentities => Role::Administrator,
        }
    }

    /// Whether [`MutatingSurface`] gates this action.
    ///
    /// Only *reconfiguration* is gated, which is what ADR 008 Decision 1
    /// actually says. Extending the toggle to camera lifecycle or to reads
    /// would silently change Phase 3 and Phase 7 behavior, and would conflate a
    /// deployment posture with an identity decision.
    #[must_use]
    pub const fn requires_mutating_surface(&self) -> bool {
        matches!(
            self,
            Self::UpdateGlobalSettings | Self::UpdateCameraSettings(_) | Self::ManageIdentities
        )
    }

    /// Whether this action changes state, and so must be refused during
    /// shutdown.
    ///
    /// Reads stay available while the server drains: an operator watching a
    /// shutdown is exactly when observability matters most.
    #[must_use]
    pub const fn is_mutating(&self) -> bool {
        match self {
            Self::ListCameras
            | Self::ViewCamera(_)
            | Self::ViewRecordings(_)
            | Self::IssueViewerToken(_)
            | Self::ViewSettings
            | Self::ViewSystemCapabilities
            | Self::ViewIdentities
            // An export changes nothing in this system, so it is audited
            // rather than refused during a drain. Refusing it while draining
            // would block an operator retrieving footage during an incident,
            // which is when they are most likely to need it.
            | Self::ExportRecording(_) => false,
            Self::StartCamera(_)
            | Self::StopCamera(_)
            | Self::AddCamera
            | Self::RemoveCamera(_)
            | Self::UpdateGlobalSettings
            | Self::UpdateCameraSettings(_)
            | Self::ManageIdentities => true,
        }
    }
}

/// What the adapter knows about an incoming request.
///
/// `peer` is the transport-level source. `client` is who the request is
/// *from* — equal to `peer` unless a reverse proxy proved its origin, in which
/// case it is the forwarded address. Both are kept: the peer is what an
/// allowlist is checked against, the client is what an audit row should name.
#[derive(Clone, Debug)]
pub struct RequestContext {
    peer: Option<IpAddr>,
    client: Option<IpAddr>,
    authentication: Authentication,
    surface: MutatingSurface,
    shutting_down: bool,
}

impl RequestContext {
    #[must_use]
    pub const fn new(
        peer: Option<IpAddr>,
        client: Option<IpAddr>,
        authentication: Authentication,
        surface: MutatingSurface,
        shutting_down: bool,
    ) -> Self {
        Self {
            peer,
            client,
            authentication,
            surface,
            shutting_down,
        }
    }

    #[must_use]
    pub const fn peer(&self) -> Option<IpAddr> {
        self.peer
    }

    #[must_use]
    pub const fn client(&self) -> Option<IpAddr> {
        self.client
    }

    #[must_use]
    pub const fn authentication(&self) -> &Authentication {
        &self.authentication
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
    role: Role,
}

impl Authorized {
    #[must_use]
    pub const fn action(&self) -> &Action {
        &self.action
    }

    /// The identity to record in the audit trail.
    #[must_use]
    pub const fn actor(&self) -> &Actor {
        &self.actor
    }

    /// The granting role. A bounded metric label; never the principal id.
    #[must_use]
    pub const fn role(&self) -> Role {
        self.role
    }
}

/// Why a request was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Denied {
    /// The mutating surface is off for this deployment. Renders as `404`, not
    /// `403`: a disabled surface does not advertise itself.
    SurfaceDisabled,
    /// No usable credential. Renders as `401`. The specific [`AuthFailure`] is
    /// carried for audit and metrics and is *not* rendered to the client.
    Unauthenticated(AuthFailure),
    /// Authenticated, but the role does not permit the action. Renders `403`.
    Unauthorized,
    /// Authenticated and permitted by role, but the camera is outside the
    /// principal's scope. Renders `404` so that scope cannot be used to
    /// enumerate cameras.
    NotInScope,
    /// The server is shutting down and will not accept changes. Renders `503`.
    ShuttingDown,
}

impl fmt::Display for Denied {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SurfaceDisabled => formatter.write_str("configuration API is disabled"),
            Self::Unauthenticated(_) => formatter.write_str("not authenticated"),
            Self::Unauthorized => formatter.write_str("not authorized"),
            Self::NotInScope => formatter.write_str("not found"),
            Self::ShuttingDown => formatter.write_str("server is shutting down"),
        }
    }
}

/// Counts every call, so a test can prove a route reached this function.
///
/// The type system proves a handler cannot *skip* the check; this counter
/// proves a given route actually *runs* it.
static AUTHORIZE_CALLS: AtomicU64 = AtomicU64::new(0);

/// Reads the chokepoint call count.
#[must_use]
pub fn authorize_call_count() -> u64 {
    AUTHORIZE_CALLS.load(Ordering::Relaxed)
}

/// Decision counters for `/metrics`, one per outcome.
///
/// Five bounded labels. Not labelled by principal or by camera: the first is
/// unbounded cardinality and an identity in every scrape, the second already
/// has its own bounded label elsewhere.
static GRANTED: AtomicU64 = AtomicU64::new(0);
static DENIED_SURFACE: AtomicU64 = AtomicU64::new(0);
static DENIED_UNAUTHENTICATED: AtomicU64 = AtomicU64::new(0);
static DENIED_UNAUTHORIZED: AtomicU64 = AtomicU64::new(0);
static DENIED_SCOPE: AtomicU64 = AtomicU64::new(0);
static DENIED_SHUTDOWN: AtomicU64 = AtomicU64::new(0);

/// Every decision counter as `(label, value)`.
#[must_use]
pub fn decision_counts() -> [(&'static str, u64); 6] {
    [
        ("granted", GRANTED.load(Ordering::Relaxed)),
        ("surface_disabled", DENIED_SURFACE.load(Ordering::Relaxed)),
        (
            "unauthenticated",
            DENIED_UNAUTHENTICATED.load(Ordering::Relaxed),
        ),
        ("unauthorized", DENIED_UNAUTHORIZED.load(Ordering::Relaxed)),
        ("not_in_scope", DENIED_SCOPE.load(Ordering::Relaxed)),
        ("shutting_down", DENIED_SHUTDOWN.load(Ordering::Relaxed)),
    ]
}

fn count(outcome: Result<&Authorized, Denied>) {
    let counter = match outcome {
        Ok(_) => &GRANTED,
        Err(Denied::SurfaceDisabled) => &DENIED_SURFACE,
        Err(Denied::Unauthenticated(_)) => &DENIED_UNAUTHENTICATED,
        Err(Denied::Unauthorized) => &DENIED_UNAUTHORIZED,
        Err(Denied::NotInScope) => &DENIED_SCOPE,
        Err(Denied::ShuttingDown) => &DENIED_SHUTDOWN,
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

/// The single decision point for protected requests.
///
/// # Errors
/// Returns [`Denied`] with the reason.
///
/// Order matters and is deliberate:
///
/// 1. A disabled surface is reported before anything else, so a deployment that
///    has not opted in reveals nothing about who would have been allowed.
/// 2. Shutdown is reported before authentication, because it is true regardless
///    of who is asking.
/// 3. Authentication precedes authorization, so an anonymous caller learns
///    nothing about the role or scope required.
/// 4. Role precedes scope, so a principal without the role never learns whether
///    a camera exists.
pub fn authorize(request: &RequestContext, action: Action) -> Result<Authorized, Denied> {
    AUTHORIZE_CALLS.fetch_add(1, Ordering::Relaxed);
    let outcome = decide(request, action);
    count(outcome.as_ref().map_err(|denied| *denied));
    outcome
}

fn decide(request: &RequestContext, action: Action) -> Result<Authorized, Denied> {
    if action.requires_mutating_surface() && !request.surface.is_enabled() {
        return Err(Denied::SurfaceDisabled);
    }
    if request.shutting_down && action.is_mutating() {
        return Err(Denied::ShuttingDown);
    }

    let principal = match &request.authentication {
        Authentication::Authenticated(principal) => principal,
        Authentication::Refused(failure) => return Err(Denied::Unauthenticated(*failure)),
    };

    if principal.role() < action.required_role() {
        return Err(Denied::Unauthorized);
    }
    if let ActionScope::Camera(camera) = action.scope() {
        if !principal.scope().permits(camera) {
            return Err(Denied::NotInScope);
        }
    }

    Ok(Authorized {
        actor: audit_actor(principal),
        role: principal.role(),
        action,
    })
}

/// Builds the audit identity, falling back rather than panicking.
///
/// `Principal` bounds its identifier so that `principal:<kind>:<id>` fits the
/// 128-byte `actor` column, and `crates/auth` has a test that keeps the two
/// bounds linked. The fallback exists because an audit row with a degraded
/// actor is better than a refused request or a panic in a request path — and
/// `unknown-principal` is visibly wrong, so it cannot be mistaken for a real
/// identity.
fn audit_actor(principal: &Principal) -> Actor {
    Actor::new(&principal.audit_identity())
        .or_else(|_| Actor::new("unknown-principal"))
        .unwrap_or_else(|_| Actor::loopback())
}

#[cfg(test)]
mod tests {
    use super::{
        authorize, authorize_call_count, Action, ActionScope, Authorized, Denied, MutatingSurface,
        RequestContext,
    };
    use std::net::{IpAddr, Ipv4Addr};
    use surveillance_auth::{
        principal::{CameraScope, Principal, PrincipalId, PrincipalKind, Role},
        verify::{AuthFailure, Authentication},
    };
    use surveillance_common::types::CameraId;

    fn camera(value: &str) -> CameraId {
        CameraId::new(value).expect("test camera identifier is valid")
    }

    fn principal(role: Role, scope: CameraScope) -> Principal {
        Principal::new(
            PrincipalId::new("tester").expect("valid identifier"),
            PrincipalKind::Human,
            "Tester",
            role,
            scope,
            true,
        )
        .expect("principal is valid")
    }

    fn request(role: Role, scope: CameraScope, surface: MutatingSurface) -> RequestContext {
        RequestContext::new(
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            Authentication::Authenticated(principal(role, scope)),
            surface,
            false,
        )
    }

    fn anonymous(surface: MutatingSurface) -> RequestContext {
        RequestContext::new(
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            Authentication::Refused(AuthFailure::Absent),
            surface,
            false,
        )
    }

    /// Every action, for the exhaustive matrix tests. Kept in one place so a
    /// new variant is added once and every matrix test covers it.
    fn every_action() -> Vec<Action> {
        let subject = camera("camera-01");
        vec![
            Action::ListCameras,
            Action::ViewCamera(subject.clone()),
            Action::ViewRecordings(subject.clone()),
            Action::IssueViewerToken(subject.clone()),
            Action::ExportRecording(subject.clone()),
            Action::ViewSettings,
            Action::StartCamera(subject.clone()),
            Action::StopCamera(subject.clone()),
            Action::AddCamera,
            Action::RemoveCamera(subject.clone()),
            Action::UpdateGlobalSettings,
            Action::UpdateCameraSettings(subject),
            Action::ManageIdentities,
        ]
    }

    #[test]
    fn the_surface_is_disabled_by_default() {
        assert_eq!(MutatingSurface::default(), MutatingSurface::Disabled);
        assert!(!MutatingSurface::default().is_enabled());
    }

    #[test]
    fn a_disabled_surface_refuses_configuration_changes_even_for_an_administrator() {
        let context = request(
            Role::Administrator,
            CameraScope::All,
            MutatingSurface::Disabled,
        );
        assert_eq!(
            authorize(&context, Action::UpdateGlobalSettings).err(),
            Some(Denied::SurfaceDisabled)
        );
    }

    #[test]
    fn a_disabled_surface_is_reported_before_authentication() {
        // A deployment that has not opted in must not reveal who would have
        // been allowed.
        let context = anonymous(MutatingSurface::Disabled);
        assert_eq!(
            authorize(&context, Action::UpdateGlobalSettings).err(),
            Some(Denied::SurfaceDisabled)
        );
    }

    #[test]
    fn a_disabled_surface_does_not_block_reads_or_camera_lifecycle() {
        // Phase 3 camera lifecycle and Phase 7 reads predate the toggle and
        // must keep working; the toggle is about reconfiguration only.
        let context = request(Role::Operator, CameraScope::All, MutatingSurface::Disabled);
        assert!(authorize(&context, Action::ListCameras).is_ok());
        assert!(authorize(&context, Action::StartCamera(camera("camera-01"))).is_ok());
        assert!(authorize(&context, Action::ViewSettings).is_ok());
    }

    #[test]
    fn an_anonymous_caller_is_unauthenticated_for_every_action() {
        let context = anonymous(MutatingSurface::Enabled);
        for action in every_action() {
            assert_eq!(
                authorize(&context, action.clone()).err(),
                Some(Denied::Unauthenticated(AuthFailure::Absent)),
                "{} should require authentication",
                action.as_str()
            );
        }
    }

    #[test]
    fn the_authentication_failure_reason_survives_to_the_audit_trail() {
        for failure in [
            AuthFailure::Absent,
            AuthFailure::Malformed,
            AuthFailure::UnknownToken,
            AuthFailure::BadSecret,
            AuthFailure::Expired,
            AuthFailure::Revoked,
            AuthFailure::PrincipalDisabled,
        ] {
            let context = RequestContext::new(
                None,
                None,
                Authentication::Refused(failure),
                MutatingSurface::Enabled,
                false,
            );
            assert_eq!(
                authorize(&context, Action::ListCameras).err(),
                Some(Denied::Unauthenticated(failure))
            );
        }
    }

    #[test]
    fn the_role_matrix_is_exhaustive_and_monotonic() {
        // Every action, every role. A higher role must never be refused where a
        // lower one is granted; that would be a privilege inversion.
        for action in every_action() {
            let required = action.required_role();
            for role in [Role::Viewer, Role::Operator, Role::Administrator] {
                let context = request(role, CameraScope::All, MutatingSurface::Enabled);
                let outcome = authorize(&context, action.clone());
                if role >= required {
                    assert!(
                        outcome.is_ok(),
                        "{role} should be permitted {}",
                        action.as_str()
                    );
                } else {
                    assert_eq!(
                        outcome.err(),
                        Some(Denied::Unauthorized),
                        "{role} should be refused {}",
                        action.as_str()
                    );
                }
            }
        }
    }

    #[test]
    fn a_camera_outside_scope_is_reported_as_not_found_not_forbidden() {
        // 403 would confirm the camera exists, letting a scoped operator
        // enumerate the estate.
        let scope = CameraScope::only([camera("camera-01")]).expect("scope is valid");
        let context = request(Role::Administrator, scope, MutatingSurface::Enabled);

        assert!(authorize(&context, Action::ViewCamera(camera("camera-01"))).is_ok());
        assert_eq!(
            authorize(&context, Action::ViewCamera(camera("camera-02"))).err(),
            Some(Denied::NotInScope)
        );
    }

    #[test]
    fn scope_is_enforced_for_every_camera_scoped_action() {
        let scope = CameraScope::only([camera("camera-01")]).expect("scope is valid");
        let context = request(Role::Administrator, scope, MutatingSurface::Enabled);
        let outsider = camera("camera-99");

        for action in every_action() {
            if matches!(action.scope(), ActionScope::Global) {
                continue;
            }
            // Rebuild the same variant against a camera outside the scope.
            let rebuilt = match action {
                Action::ViewCamera(_) => Action::ViewCamera(outsider.clone()),
                Action::ViewRecordings(_) => Action::ViewRecordings(outsider.clone()),
                Action::IssueViewerToken(_) => Action::IssueViewerToken(outsider.clone()),
                Action::ExportRecording(_) => Action::ExportRecording(outsider.clone()),
                Action::StartCamera(_) => Action::StartCamera(outsider.clone()),
                Action::StopCamera(_) => Action::StopCamera(outsider.clone()),
                Action::RemoveCamera(_) => Action::RemoveCamera(outsider.clone()),
                Action::UpdateCameraSettings(_) => Action::UpdateCameraSettings(outsider.clone()),
                other => unreachable!("{} is global", other.as_str()),
            };
            assert_eq!(
                authorize(&context, rebuilt.clone()).err(),
                Some(Denied::NotInScope),
                "{} must enforce scope",
                rebuilt.as_str()
            );
        }
    }

    #[test]
    fn an_all_scope_reaches_a_camera_added_later() {
        let context = request(Role::Operator, CameraScope::All, MutatingSurface::Enabled);
        assert!(authorize(&context, Action::ViewCamera(camera("brand-new"))).is_ok());
    }

    #[test]
    fn an_empty_scope_reaches_no_camera_but_still_permits_global_reads() {
        let scope = CameraScope::only([]).expect("an empty scope is representable");
        let context = request(Role::Operator, scope, MutatingSurface::Enabled);
        assert_eq!(
            authorize(&context, Action::ViewCamera(camera("camera-01"))).err(),
            Some(Denied::NotInScope)
        );
        assert!(authorize(&context, Action::ListCameras).is_ok());
    }

    #[test]
    fn role_is_checked_before_scope() {
        // A viewer asking about an out-of-scope camera must learn "forbidden",
        // not "not found" — otherwise the two denials leak different facts.
        let scope = CameraScope::only([]).expect("scope is valid");
        let context = request(Role::Viewer, scope, MutatingSurface::Enabled);
        assert_eq!(
            authorize(&context, Action::StopCamera(camera("camera-01"))).err(),
            Some(Denied::Unauthorized)
        );
    }

    #[test]
    fn shutdown_refuses_mutations_but_allows_reads() {
        let context = RequestContext::new(
            None,
            None,
            Authentication::Authenticated(principal(Role::Administrator, CameraScope::All)),
            MutatingSurface::Enabled,
            true,
        );
        assert_eq!(
            authorize(&context, Action::UpdateGlobalSettings).err(),
            Some(Denied::ShuttingDown)
        );
        assert_eq!(
            authorize(&context, Action::StopCamera(camera("camera-01"))).err(),
            Some(Denied::ShuttingDown)
        );
        // Observability during a drain is exactly when it matters.
        assert!(authorize(&context, Action::ListCameras).is_ok());
        assert!(authorize(&context, Action::ViewCamera(camera("camera-01"))).is_ok());
    }

    #[test]
    fn every_action_declares_a_distinct_stable_label() {
        let mut labels: Vec<&str> = every_action().iter().map(Action::as_str).collect();
        let total = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), total, "action labels must be distinct");
    }

    #[test]
    fn the_action_label_never_contains_the_camera_identifier() {
        // It is a metric label; embedding the camera would make cardinality
        // unbounded and duplicate a label that already exists.
        let action = Action::ViewCamera(camera("camera-01"));
        assert!(!action.as_str().contains("camera-01"));
    }

    #[test]
    fn only_configuration_actions_are_gated_by_the_deployment_toggle() {
        for action in every_action() {
            let gated = action.requires_mutating_surface();
            let expected = matches!(
                action,
                Action::UpdateGlobalSettings
                    | Action::UpdateCameraSettings(_)
                    | Action::ManageIdentities
            );
            assert_eq!(gated, expected, "{} gating is wrong", action.as_str());
        }
    }

    #[test]
    fn reads_are_never_classified_as_mutating() {
        for action in every_action() {
            if action.required_role() == Role::Viewer {
                assert!(
                    !action.is_mutating(),
                    "{} is a read and must survive shutdown",
                    action.as_str()
                );
            }
        }
    }

    #[test]
    fn a_granted_action_carries_the_authenticated_identity_not_the_placeholder() {
        let context = request(Role::Operator, CameraScope::All, MutatingSurface::Enabled);
        let granted: Authorized =
            authorize(&context, Action::ListCameras).expect("operator may list cameras");
        assert_eq!(granted.actor().as_str(), "principal:human:tester");
        assert_ne!(
            granted.actor().as_str(),
            "loopback",
            "the Phase 7 placeholder must be gone"
        );
        assert_eq!(granted.role(), Role::Operator);
        assert_eq!(granted.action().as_str(), "list_cameras");
    }

    #[test]
    fn every_call_is_counted_including_refusals() {
        let before = authorize_call_count();
        let context = anonymous(MutatingSurface::Disabled);
        let _ = authorize(&context, Action::UpdateGlobalSettings);
        let _ = authorize(&context, Action::ListCameras);
        assert!(
            authorize_call_count() >= before + 2,
            "the chokepoint must count refusals as well as grants"
        );
    }
}
