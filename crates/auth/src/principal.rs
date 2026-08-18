//! Who is asking: validated identity, role, and camera scope.
//!
//! This module owns identity only. It deliberately knows nothing about HTTP,
//! about persistence, or about the actions a role permits — the role/action
//! table lives at the authorization chokepoint in the server, because that is
//! where the actions are defined.

use std::{collections::BTreeSet, error::Error, fmt};

use serde::Serialize;
use surveillance_common::types::CameraId;

/// The largest number of cameras one principal may be scoped to.
///
/// Bounded for the same reason metric labels are: a scope is read on every
/// request, and an unbounded set turns a configuration mistake into a
/// per-request cost. The 10-camera target leaves ample headroom.
pub const MAX_SCOPED_CAMERAS: usize = 256;

/// The largest accepted principal identifier, in bytes.
///
/// Chosen so that `principal:<kind>:<id>` always fits the 128-byte audit
/// `actor` column: the longest prefix is `principal:service:` at 18 bytes.
pub const MAX_PRINCIPAL_ID_LEN: usize = 64;

/// The largest accepted display name, in bytes.
pub const MAX_DISPLAY_NAME_LEN: usize = 128;

/// A validated, stable principal identifier.
///
/// The charset is restricted rather than merely length-bounded because this
/// value is rendered into audit rows and log lines. Restricting it at
/// construction means no downstream consumer has to escape it.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct PrincipalId(String);

impl PrincipalId {
    /// Accepts 1..=[`MAX_PRINCIPAL_ID_LEN`] bytes of ASCII alphanumerics,
    /// `-`, `_`, and `.`, not beginning or ending with a separator.
    ///
    /// # Errors
    /// Returns [`PrincipalIdError`] naming the specific violation.
    pub fn new(value: &str) -> Result<Self, PrincipalIdError> {
        if value.is_empty() {
            return Err(PrincipalIdError::Empty);
        }
        if value.len() > MAX_PRINCIPAL_ID_LEN {
            return Err(PrincipalIdError::TooLong);
        }
        let separator = |character: char| matches!(character, '-' | '_' | '.');
        if !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || separator(character))
        {
            return Err(PrincipalIdError::IllegalCharacter);
        }
        // `starts_with`/`ends_with` on a non-empty ASCII string cannot panic on
        // a char boundary, and emptiness was rejected above.
        if value.starts_with(separator) || value.ends_with(separator) {
            return Err(PrincipalIdError::BoundarySeparator);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PrincipalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Why a principal identifier was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrincipalIdError {
    Empty,
    TooLong,
    IllegalCharacter,
    BoundarySeparator,
}

impl fmt::Display for PrincipalIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("principal identifier is empty"),
            Self::TooLong => write!(
                formatter,
                "principal identifier exceeds {MAX_PRINCIPAL_ID_LEN} bytes"
            ),
            Self::IllegalCharacter => formatter
                .write_str("principal identifier accepts only alphanumerics, '-', '_', and '.'"),
            Self::BoundarySeparator => {
                formatter.write_str("principal identifier must not begin or end with a separator")
            }
        }
    }
}

impl Error for PrincipalIdError {}

/// Whether an identity belongs to a person or to a program.
///
/// The distinction is operational, not cosmetic: a service identity's
/// credential is stored somewhere a program can read it, which makes its
/// rotation story different, and audit review treats the two differently.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PrincipalKind {
    Human,
    Service,
}

impl PrincipalKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Service => "service",
        }
    }
}

impl fmt::Display for PrincipalKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What a principal is broadly permitted to do.
///
/// Three roles, because the operations divide three ways: watch, operate, and
/// administer. A fourth role would need an operation that none of these covers.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Read camera state and recordings; obtain a viewer token.
    Viewer,
    /// Everything a viewer may do, plus camera lifecycle and export.
    Operator,
    /// Everything an operator may do, plus configuration and identity changes.
    Administrator,
}

impl Role {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Operator => "operator",
            Self::Administrator => "administrator",
        }
    }

    /// Parses the stored representation.
    ///
    /// # Errors
    /// Returns [`RoleError`] for an unrecognised value. A row whose role cannot
    /// be parsed denies rather than defaulting: silently downgrading an unknown
    /// role to `Viewer` would be a quiet privilege change, and silently
    /// upgrading it would be worse.
    pub fn parse(value: &str) -> Result<Self, RoleError> {
        match value {
            "viewer" => Ok(Self::Viewer),
            "operator" => Ok(Self::Operator),
            "administrator" => Ok(Self::Administrator),
            _ => Err(RoleError::Unrecognised),
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// An unparseable stored role.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoleError {
    Unrecognised,
}

impl fmt::Display for RoleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unrecognised role")
    }
}

impl Error for RoleError {}

/// Which cameras a principal may act on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CameraScope {
    /// Every camera, including cameras added after the principal was created.
    All,
    /// Exactly the listed cameras. An empty set permits nothing, which is a
    /// legal and occasionally useful state (a suspended operator).
    Only(BTreeSet<CameraId>),
}

impl CameraScope {
    /// Builds a bounded explicit scope.
    ///
    /// # Errors
    /// Returns [`CameraScopeError::TooMany`] beyond [`MAX_SCOPED_CAMERAS`].
    pub fn only<I: IntoIterator<Item = CameraId>>(cameras: I) -> Result<Self, CameraScopeError> {
        let set: BTreeSet<CameraId> = cameras.into_iter().collect();
        if set.len() > MAX_SCOPED_CAMERAS {
            return Err(CameraScopeError::TooMany);
        }
        Ok(Self::Only(set))
    }

    /// Whether this scope covers `camera`.
    #[must_use]
    pub fn permits(&self, camera: &CameraId) -> bool {
        match self {
            Self::All => true,
            Self::Only(cameras) => cameras.contains(camera),
        }
    }

    /// Whether this scope covers every camera.
    #[must_use]
    pub const fn is_all(&self) -> bool {
        matches!(self, Self::All)
    }
}

/// A scope that cannot be represented.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CameraScopeError {
    TooMany,
}

impl fmt::Display for CameraScopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "scope exceeds {MAX_SCOPED_CAMERAS} cameras")
    }
}

impl Error for CameraScopeError {}

/// An authenticated identity and what it is allowed to reach.
///
/// `enabled` is separate from deleting the principal so that an account can be
/// suspended without orphaning the audit rows that reference it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Principal {
    id: PrincipalId,
    kind: PrincipalKind,
    display_name: String,
    role: Role,
    scope: CameraScope,
    enabled: bool,
}

impl Principal {
    /// # Errors
    /// Returns [`PrincipalError::DisplayNameTooLong`] beyond
    /// [`MAX_DISPLAY_NAME_LEN`], or [`PrincipalError::DisplayNameEmpty`] for a
    /// blank name.
    pub fn new(
        id: PrincipalId,
        kind: PrincipalKind,
        display_name: &str,
        role: Role,
        scope: CameraScope,
        enabled: bool,
    ) -> Result<Self, PrincipalError> {
        let trimmed = display_name.trim();
        if trimmed.is_empty() {
            return Err(PrincipalError::DisplayNameEmpty);
        }
        if trimmed.len() > MAX_DISPLAY_NAME_LEN {
            return Err(PrincipalError::DisplayNameTooLong);
        }
        Ok(Self {
            id,
            kind,
            display_name: trimmed.to_owned(),
            role,
            scope,
            enabled,
        })
    }

    /// The identity a deployment uses when it has no identity store.
    ///
    /// **A posture, not a credential.** It exists so that the loopback-only
    /// deployment Phases 2–7 shipped keeps working: refusing every request on a
    /// unit with no principals would be a regression, not a control.
    ///
    /// It is safe only because a routable listener is refused unless a trust
    /// boundary *and* an enabled principal exist (ADR 009). If that unlock rule
    /// is ever relaxed on its own, this becomes an authentication bypass.
    ///
    /// Built infallibly, and deliberately here rather than at the call site:
    /// this module owns the validators, so it can state that these literals
    /// satisfy them — and
    /// `the_development_identity_would_also_pass_the_validators` keeps that
    /// true. A caller outside this module still has to go through
    /// [`Principal::new`].
    ///
    /// The name is conspicuous on purpose: a reader who sees
    /// `principal:service:loopback-development` in an audit row knows that
    /// nobody authenticated.
    #[must_use]
    pub fn loopback_development() -> Self {
        Self {
            id: PrincipalId("loopback-development".to_owned()),
            kind: PrincipalKind::Service,
            display_name: "Loopback development access".to_owned(),
            role: Role::Administrator,
            scope: CameraScope::All,
            enabled: true,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &PrincipalId {
        &self.id
    }

    #[must_use]
    pub const fn kind(&self) -> PrincipalKind {
        self.kind
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub const fn role(&self) -> Role {
        self.role
    }

    #[must_use]
    pub const fn scope(&self) -> &CameraScope {
        &self.scope
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The audit identity: `principal:<kind>:<id>`.
    ///
    /// Constructed so that the audit trail distinguishes a human from a service
    /// without a join, and so that the Phase 7 placeholder value `loopback`
    /// cannot be confused with a real identity.
    #[must_use]
    pub fn audit_identity(&self) -> String {
        format!("principal:{}:{}", self.kind.as_str(), self.id)
    }
}

/// A principal that cannot be represented.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrincipalError {
    DisplayNameEmpty,
    DisplayNameTooLong,
}

impl fmt::Display for PrincipalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DisplayNameEmpty => formatter.write_str("display name is empty"),
            Self::DisplayNameTooLong => {
                write!(
                    formatter,
                    "display name exceeds {MAX_DISPLAY_NAME_LEN} bytes"
                )
            }
        }
    }
}

impl Error for PrincipalError {}

#[cfg(test)]
mod tests {
    use super::{
        CameraScope, CameraScopeError, Principal, PrincipalError, PrincipalId, PrincipalIdError,
        PrincipalKind, Role, RoleError, MAX_DISPLAY_NAME_LEN, MAX_PRINCIPAL_ID_LEN,
        MAX_SCOPED_CAMERAS,
    };
    use surveillance_common::types::CameraId;

    fn camera(value: &str) -> CameraId {
        CameraId::new(value).expect("test camera identifier is valid")
    }

    fn principal_id(value: &str) -> PrincipalId {
        PrincipalId::new(value).expect("test principal identifier is valid")
    }

    #[test]
    fn a_principal_identifier_accepts_the_documented_charset() {
        for accepted in ["operator", "svc-scraper", "a.b_c-1", "x", "A1"] {
            assert!(
                PrincipalId::new(accepted).is_ok(),
                "{accepted} should be accepted"
            );
        }
    }

    #[test]
    fn a_principal_identifier_rejects_everything_else() {
        assert_eq!(PrincipalId::new("").err(), Some(PrincipalIdError::Empty));
        assert_eq!(
            PrincipalId::new(&"a".repeat(MAX_PRINCIPAL_ID_LEN + 1)).err(),
            Some(PrincipalIdError::TooLong)
        );
        for illegal in ["has space", "slash/es", "quote\"", "new\nline", "üñî"] {
            assert_eq!(
                PrincipalId::new(illegal).err(),
                Some(PrincipalIdError::IllegalCharacter),
                "{illegal} should be refused"
            );
        }
        for boundary in ["-lead", "trail-", ".dot", "under_"] {
            assert_eq!(
                PrincipalId::new(boundary).err(),
                Some(PrincipalIdError::BoundarySeparator),
                "{boundary} should be refused"
            );
        }
    }

    #[test]
    fn the_identifier_bound_keeps_the_audit_identity_within_the_actor_column() {
        // The audit `actor` column is bounded to 128 bytes; the longest prefix
        // is `principal:service:`. This test is what keeps the two bounds
        // linked when either changes.
        let longest = principal_id(&"a".repeat(MAX_PRINCIPAL_ID_LEN));
        let subject = Principal::new(
            longest,
            PrincipalKind::Service,
            "service",
            Role::Viewer,
            CameraScope::All,
            true,
        )
        .expect("principal is valid");
        assert!(
            subject.audit_identity().len() <= 128,
            "audit identity {} exceeds the actor column bound",
            subject.audit_identity().len()
        );
    }

    #[test]
    fn the_audit_identity_names_the_kind_and_cannot_collide_with_the_phase_7_placeholder() {
        let human = Principal::new(
            principal_id("alice"),
            PrincipalKind::Human,
            "Alice",
            Role::Operator,
            CameraScope::All,
            true,
        )
        .expect("principal is valid");
        assert_eq!(human.audit_identity(), "principal:human:alice");
        assert_ne!(human.audit_identity(), "loopback");
    }

    #[test]
    fn roles_round_trip_through_their_stored_representation() {
        for role in [Role::Viewer, Role::Operator, Role::Administrator] {
            assert_eq!(Role::parse(role.as_str()), Ok(role));
        }
    }

    #[test]
    fn an_unknown_stored_role_is_refused_rather_than_defaulted() {
        // Defaulting either direction is a silent privilege change.
        assert_eq!(
            Role::parse("superuser").err(),
            Some(RoleError::Unrecognised)
        );
        assert_eq!(Role::parse("").err(), Some(RoleError::Unrecognised));
        assert_eq!(Role::parse("Viewer").err(), Some(RoleError::Unrecognised));
    }

    #[test]
    fn roles_order_from_least_to_most_privileged() {
        assert!(Role::Viewer < Role::Operator);
        assert!(Role::Operator < Role::Administrator);
    }

    #[test]
    fn an_all_scope_permits_any_camera_including_unseen_ones() {
        let scope = CameraScope::All;
        assert!(scope.permits(&camera("camera-01")));
        assert!(scope.permits(&camera("added-later")));
        assert!(scope.is_all());
    }

    #[test]
    fn an_explicit_scope_permits_only_its_members() {
        let scope =
            CameraScope::only([camera("camera-01"), camera("camera-02")]).expect("scope is valid");
        assert!(scope.permits(&camera("camera-01")));
        assert!(scope.permits(&camera("camera-02")));
        assert!(!scope.permits(&camera("camera-03")));
        assert!(!scope.is_all());
    }

    #[test]
    fn an_empty_scope_permits_nothing_and_is_legal() {
        let scope = CameraScope::only([]).expect("an empty scope is representable");
        assert!(!scope.permits(&camera("camera-01")));
    }

    #[test]
    fn a_scope_is_bounded() {
        let too_many = (0..=MAX_SCOPED_CAMERAS).map(|index| camera(&format!("camera-{index:04}")));
        assert_eq!(
            CameraScope::only(too_many).err(),
            Some(CameraScopeError::TooMany)
        );
    }

    #[test]
    fn a_scope_at_the_bound_is_accepted() {
        let exactly = (0..MAX_SCOPED_CAMERAS).map(|index| camera(&format!("camera-{index:04}")));
        assert!(CameraScope::only(exactly).is_ok());
    }

    #[test]
    fn a_display_name_is_trimmed_and_bounded() {
        let subject = Principal::new(
            principal_id("alice"),
            PrincipalKind::Human,
            "  Alice  ",
            Role::Viewer,
            CameraScope::All,
            true,
        )
        .expect("principal is valid");
        assert_eq!(subject.display_name(), "Alice");

        assert_eq!(
            Principal::new(
                principal_id("alice"),
                PrincipalKind::Human,
                "   ",
                Role::Viewer,
                CameraScope::All,
                true,
            )
            .err(),
            Some(PrincipalError::DisplayNameEmpty)
        );
        assert_eq!(
            Principal::new(
                principal_id("alice"),
                PrincipalKind::Human,
                &"n".repeat(MAX_DISPLAY_NAME_LEN + 1),
                Role::Viewer,
                CameraScope::All,
                true,
            )
            .err(),
            Some(PrincipalError::DisplayNameTooLong)
        );
    }

    #[test]
    fn the_development_identity_would_also_pass_the_validators() {
        // `loopback_development` bypasses `new` by construction. This test is
        // what keeps that bypass honest: if a bound ever tightens past these
        // literals, this fails rather than shipping an unvalidated identity.
        let subject = Principal::loopback_development();
        assert!(PrincipalId::new(subject.id().as_str()).is_ok());
        assert!(Principal::new(
            PrincipalId::new(subject.id().as_str()).expect("identifier is valid"),
            subject.kind(),
            subject.display_name(),
            subject.role(),
            subject.scope().clone(),
            subject.is_enabled(),
        )
        .is_ok());
    }

    #[test]
    fn the_development_identity_is_conspicuous_in_an_audit_row() {
        let subject = Principal::loopback_development();
        assert_eq!(
            subject.audit_identity(),
            "principal:service:loopback-development"
        );
        // An operator reading the trail must be able to tell this from a real
        // identity at a glance.
        assert!(subject.audit_identity().contains("development"));
        assert!(subject.audit_identity().len() <= 128);
    }

    #[test]
    fn a_disabled_principal_reports_itself_disabled() {
        let subject = Principal::new(
            principal_id("suspended"),
            PrincipalKind::Human,
            "Suspended",
            Role::Administrator,
            CameraScope::All,
            false,
        )
        .expect("principal is valid");
        assert!(!subject.is_enabled());
    }
}
