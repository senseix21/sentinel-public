//! Access, camera, export, and identity audit.
//!
//! P8.6. Extends the `settings_audit` shape of ADR 008 to the rest of the
//! surface. The record type lives here rather than in a domain crate because it
//! is persistence-shaped and carries no policy: what *is* auditable is decided
//! by the authorization chokepoint, which is where every protected action
//! already passes.
//!
//! Two rules that differ between reads and writes, stated because they are a
//! deliberate trade rather than an oversight:
//!
//! - An audit failure on a **mutating** operation fails the operation. Same as
//!   Phase 7: "changed but unlogged" stays unrepresentable.
//! - An audit failure on a **read** is counted and the read proceeds. Refusing
//!   reads because the audit store is down would turn an audit outage into an
//!   observability outage during exactly the incident that needs both.

use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool};

use surveillance_common::secret::Fingerprint;

use crate::{
    config::DatabaseConfig,
    recordings::{RepositoryError, MIGRATOR},
};

/// What kind of event a row records. Mirrors the schema's CHECK constraint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditCategory {
    Authentication,
    Authorization,
    Camera,
    Export,
    Identity,
}

impl AuditCategory {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::Authorization => "authorization",
            Self::Camera => "camera",
            Self::Export => "export",
            Self::Identity => "identity",
        }
    }
}

/// How the operation ended. Mirrors the schema's CHECK constraint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditOutcome {
    /// Authorization granted; the operation itself may still fail later.
    Granted,
    /// Authorization refused.
    Denied,
    /// The operation completed.
    Succeeded,
    /// The operation was attempted and failed.
    Failed,
}

impl AuditOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    /// Whether this outcome represents a refusal or a failure, for the alert
    /// that watches authentication-failure rate.
    #[must_use]
    pub const fn is_negative(self) -> bool {
        matches!(self, Self::Denied | Self::Failed)
    }
}

/// The actor recorded when nobody authenticated.
///
/// A literal rather than NULL: a NULL actor would be excluded from every
/// `GROUP BY actor` report, which is exactly the row an incident review needs
/// to see.
pub const ANONYMOUS_ACTOR: &str = "anonymous";

/// What left the system, on an export row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExportExtent {
    pub range_start: DateTime<Utc>,
    pub range_end: DateTime<Utc>,
    pub byte_count: i64,
}

/// One audit row, before it is written.
///
/// Built with [`AccessAuditRecord::new`] so category, outcome, and the export
/// columns cannot disagree in the way the schema forbids — the type refuses the
/// combination before the database has to.
#[derive(Clone, Debug)]
pub struct AccessAuditRecord {
    category: AuditCategory,
    action: String,
    actor: String,
    camera_id: Option<String>,
    outcome: AuditOutcome,
    reason: Option<&'static str>,
    client_address: Option<std::net::IpAddr>,
    export: Option<ExportExtent>,
    detail_fingerprint: Option<Fingerprint>,
}

impl AccessAuditRecord {
    /// Builds a non-export record.
    #[must_use]
    pub fn new(category: AuditCategory, action: &str, actor: &str, outcome: AuditOutcome) -> Self {
        Self {
            category,
            action: truncate(action, 64),
            actor: truncate(actor, 128),
            camera_id: None,
            outcome,
            reason: None,
            client_address: None,
            export: None,
            detail_fingerprint: None,
        }
    }

    /// Builds an export record, which must carry its extent.
    #[must_use]
    pub fn export(actor: &str, camera: &str, outcome: AuditOutcome, extent: ExportExtent) -> Self {
        let mut record = Self::new(AuditCategory::Export, "export_recording", actor, outcome);
        record.camera_id = Some(truncate(camera, 64));
        record.export = Some(extent);
        record
    }

    #[must_use]
    pub fn with_camera(mut self, camera: &str) -> Self {
        self.camera_id = Some(truncate(camera, 64));
        self
    }

    /// Attaches a bounded reason label.
    ///
    /// `&'static str` on purpose: it makes it impossible to pass free text from
    /// a request, which is how an audit column becomes a log-injection sink.
    #[must_use]
    pub const fn with_reason(mut self, reason: &'static str) -> Self {
        self.reason = Some(reason);
        self
    }

    #[must_use]
    pub const fn with_client(mut self, client: Option<std::net::IpAddr>) -> Self {
        self.client_address = client;
        self
    }

    /// Attaches a salted fingerprint of something sensitive.
    ///
    /// A fingerprint shows that a value was involved, and that the same value
    /// recurred, without revealing it.
    #[must_use]
    pub const fn with_fingerprint(mut self, fingerprint: Fingerprint) -> Self {
        self.detail_fingerprint = Some(fingerprint);
        self
    }

    #[must_use]
    pub const fn category(&self) -> AuditCategory {
        self.category
    }

    #[must_use]
    pub const fn outcome(&self) -> AuditOutcome {
        self.outcome
    }

    #[must_use]
    pub fn actor(&self) -> &str {
        &self.actor
    }

    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }

    #[must_use]
    pub fn camera_id(&self) -> Option<&str> {
        self.camera_id.as_deref()
    }

    #[must_use]
    pub const fn reason(&self) -> Option<&'static str> {
        self.reason
    }

    #[must_use]
    pub const fn export_extent(&self) -> Option<ExportExtent> {
        self.export
    }
}

/// Writes access audit rows.
pub struct PostgresAccessAuditRepository {
    pool: PgPool,
}

impl PostgresAccessAuditRepository {
    #[must_use]
    pub fn connect_lazy(config: &DatabaseConfig) -> Self {
        let pool = PgPoolOptions::new()
            .max_connections(config.maximum_connections())
            .acquire_timeout(config.acquire_timeout())
            .connect_lazy_with(config.connect_options());
        Self { pool }
    }

    #[must_use]
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Applies repository-owned forward migrations.
    ///
    /// # Errors
    /// Returns a migration failure without exposing the DSN.
    pub async fn migrate(&self) -> Result<(), RepositoryError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(RepositoryError::migration)
    }

    /// Writes one row.
    ///
    /// # Errors
    /// Returns a classified `PostgreSQL` failure. The caller decides whether
    /// that failure is fatal: see this module's header for the read/write rule.
    pub async fn record(&self, record: &AccessAuditRecord) -> Result<(), RepositoryError> {
        let extent = record.export;
        sqlx::query(
            "INSERT INTO access_audit \
                 (category, action, actor, camera_id, outcome, reason, client_address, \
                  range_start, range_end, byte_count, detail_fingerprint) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::text::inet, $8, $9, $10, $11)",
        )
        .bind(record.category.as_str())
        .bind(&record.action)
        .bind(&record.actor)
        .bind(record.camera_id.as_deref())
        .bind(record.outcome.as_str())
        .bind(record.reason)
        .bind(record.client_address.map(|address| address.to_string()))
        .bind(extent.map(|extent| extent.range_start))
        .bind(extent.map(|extent| extent.range_end))
        .bind(extent.map(|extent| extent.byte_count))
        .bind(record.detail_fingerprint.map(|print| print.to_hex()))
        .execute(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;
        Ok(())
    }
}

/// Trims to a byte bound without splitting a UTF-8 character.
///
/// The inputs are already bounded by their own validators, so this is a
/// belt-and-braces guard: it keeps a future caller from turning a schema
/// constraint violation into a failed audit write, which under the mutating
/// rule would fail the operation being audited.
fn truncate(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        truncate, AccessAuditRecord, AuditCategory, AuditOutcome, ExportExtent, ANONYMOUS_ACTOR,
    };
    use chrono::{DateTime, Utc};
    use surveillance_common::secret::{Fingerprint, Secret};

    fn instant(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).expect("representable")
    }

    fn extent() -> ExportExtent {
        ExportExtent {
            range_start: instant(1_000),
            range_end: instant(2_000),
            byte_count: 4_096,
        }
    }

    #[test]
    fn category_and_outcome_labels_match_the_schema_constraints() {
        // If these drift, every insert fails the CHECK at runtime instead of
        // here.
        let categories = [
            AuditCategory::Authentication,
            AuditCategory::Authorization,
            AuditCategory::Camera,
            AuditCategory::Export,
            AuditCategory::Identity,
        ];
        let expected = [
            "authentication",
            "authorization",
            "camera",
            "export",
            "identity",
        ];
        let rendered: Vec<&str> = categories.iter().map(|c| c.as_str()).collect();
        assert_eq!(rendered, expected);

        let outcomes = [
            AuditOutcome::Granted,
            AuditOutcome::Denied,
            AuditOutcome::Succeeded,
            AuditOutcome::Failed,
        ];
        let rendered: Vec<&str> = outcomes.iter().map(|o| o.as_str()).collect();
        assert_eq!(rendered, ["granted", "denied", "succeeded", "failed"]);
    }

    #[test]
    fn a_non_export_record_carries_no_export_columns() {
        // The schema refuses a partially-filled export row; this refuses it a
        // layer earlier.
        let record = AccessAuditRecord::new(
            AuditCategory::Authorization,
            "stop_camera",
            "principal:human:alice",
            AuditOutcome::Denied,
        );
        assert!(record.export_extent().is_none());
    }

    #[test]
    fn an_export_record_always_carries_its_extent() {
        let record = AccessAuditRecord::export(
            "principal:human:alice",
            "camera-01",
            AuditOutcome::Succeeded,
            extent(),
        );
        assert_eq!(record.category(), AuditCategory::Export);
        assert_eq!(record.export_extent(), Some(extent()));
        assert_eq!(record.camera_id(), Some("camera-01"));
        // Who, which camera, exact range, and how many bytes left the system.
        let recorded = record.export_extent().expect("present");
        assert_eq!(recorded.byte_count, 4_096);
        assert!(recorded.range_end >= recorded.range_start);
    }

    #[test]
    fn an_anonymous_actor_is_a_literal_not_a_null() {
        // A NULL actor would drop out of every GROUP BY actor report, which is
        // the row an incident review most needs to see.
        let record = AccessAuditRecord::new(
            AuditCategory::Authentication,
            "list_cameras",
            ANONYMOUS_ACTOR,
            AuditOutcome::Denied,
        );
        assert_eq!(record.actor(), "anonymous");
        assert!(!record.actor().is_empty());
    }

    #[test]
    fn a_fingerprint_is_recorded_rather_than_a_value() {
        let salt = Secret::new("deployment".to_owned());
        let fingerprint = Fingerprint::of(&salt, b"a-real-secret");
        let record = AccessAuditRecord::new(
            AuditCategory::Identity,
            "manage_identities",
            "principal:human:alice",
            AuditOutcome::Succeeded,
        )
        .with_fingerprint(fingerprint);
        let rendered = format!("{record:?}");
        assert!(!rendered.contains("a-real-secret"));
        assert!(rendered.contains(&fingerprint.to_hex()));
    }

    #[test]
    fn the_fingerprint_matches_the_schema_shape() {
        let salt = Secret::new("deployment".to_owned());
        let rendered = Fingerprint::of(&salt, b"value").to_hex();
        assert_eq!(rendered.len(), 64);
        assert!(rendered
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn bounded_fields_are_trimmed_to_the_column_bound() {
        let record = AccessAuditRecord::new(
            AuditCategory::Camera,
            &"a".repeat(200),
            &"b".repeat(300),
            AuditOutcome::Failed,
        )
        .with_camera(&"c".repeat(200));
        assert_eq!(record.action().len(), 64);
        assert_eq!(record.actor().len(), 128);
        assert_eq!(record.camera_id().map(str::len), Some(64));
    }

    #[test]
    fn truncation_never_splits_a_utf8_character() {
        // 'é' is two bytes; a naive slice at an odd boundary would panic.
        let value = "é".repeat(40);
        let trimmed = truncate(&value, 21);
        assert!(trimmed.len() <= 21);
        assert!(std::str::from_utf8(trimmed.as_bytes()).is_ok());
    }

    #[test]
    fn negative_outcomes_are_identifiable_for_the_failure_rate_alert() {
        assert!(AuditOutcome::Denied.is_negative());
        assert!(AuditOutcome::Failed.is_negative());
        assert!(!AuditOutcome::Granted.is_negative());
        assert!(!AuditOutcome::Succeeded.is_negative());
    }
}
