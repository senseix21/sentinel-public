//! Persistent settings and their audit trail.
//!
//! ADR 008 Decision 3: a settings change and its audit record commit in one
//! transaction. If the audit write fails, the settings change does not commit.
//! "Changed but unlogged" is unrepresentable rather than rare.
//!
//! Follows the `recording_segments` repository: a bounded pool, forward
//! migrations owned by the repository, and multi-statement writes inside an
//! explicit `pool.begin()`.

use sqlx::{postgres::PgPoolOptions, PgPool, Row};

use surveillance_common::types::CameraId;
use surveillance_config::{
    classification::{Origin, Scope, SettingId},
    repository::{
        AuditAction, AuditRecord, PersistedSettings, RejectedRow, RejectionReason, SettingsBatch,
    },
    resolution::{Layer, SettingKey},
    value::SettingValue,
};

use crate::{
    config::DatabaseConfig,
    recordings::{RepositoryError, MIGRATOR},
};

/// Reads and writes persistent operational settings.
pub struct PostgresSettingsRepository {
    pool: PgPool,
}

impl PostgresSettingsRepository {
    /// Creates a bounded pool without requiring `PostgreSQL` to be available.
    ///
    /// Lazy by default: ADR 008 requires the server to come up whether or not
    /// the settings store is reachable, because media preservation must not
    /// wait for configuration freshness.
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

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
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

    /// Loads every stored setting into a persisted layer.
    ///
    /// Rows that no longer name a setting, or that no longer parse, are
    /// returned in [`PersistedSettings::rejected`] rather than dropped. A
    /// stored configuration that is partly unreadable is something the operator
    /// needs told about; silently ignoring those rows would present a partial
    /// configuration as complete.
    ///
    /// # Errors
    /// Returns a classified `PostgreSQL` failure.
    pub async fn load_all(&self) -> Result<PersistedSettings, RepositoryError> {
        let rows = sqlx::query(
            "SELECT setting_key, camera_id, value, generation FROM settings \
             ORDER BY setting_key, camera_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;

        let mut layer = Layer::new(Origin::Persisted);
        let mut rejected = Vec::new();
        let mut generation = 0_u64;

        for row in rows {
            let key: String = row
                .try_get("setting_key")
                .map_err(RepositoryError::from_sqlx)?;
            let camera: Option<String> = row
                .try_get("camera_id")
                .map_err(RepositoryError::from_sqlx)?;
            let value: String = row.try_get("value").map_err(RepositoryError::from_sqlx)?;
            let stored_generation: i64 = row
                .try_get("generation")
                .map_err(RepositoryError::from_sqlx)?;
            generation = generation.max(u64::try_from(stored_generation).unwrap_or(0));

            let parsed_camera = camera.clone().map(CameraId::new).transpose();
            let interpreted = interpret(&key, parsed_camera.clone().ok().flatten(), &value);
            let reason = match (parsed_camera, interpreted) {
                (Err(_), _) => Some(RejectionReason::InvalidCamera),
                (Ok(_), Err(reason)) => Some(reason),
                (Ok(_), Ok((setting_key, parsed))) => {
                    let id = setting_key.id();
                    layer
                        .insert(setting_key, parsed)
                        .err()
                        .map(|_| RejectionReason::NotPersistable(id))
                }
            };
            if let Some(reason) = reason {
                rejected.push(RejectedRow {
                    key,
                    camera: camera.and_then(|name| CameraId::new(name).ok()),
                    reason,
                });
            }
        }

        Ok(PersistedSettings {
            layer,
            generation,
            rejected,
        })
    }

    /// Writes a batch and its audit rows in one transaction.
    ///
    /// Returns the committed generation. Per ADR 008 Decision 3, an audit write
    /// failure aborts the whole transaction: neither the settings change nor the
    /// audit row survives.
    ///
    /// # Errors
    /// Returns a classified `PostgreSQL` failure. On any error nothing is
    /// committed.
    pub async fn apply(&self, batch: &SettingsBatch) -> Result<u64, RepositoryError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(RepositoryError::from_sqlx)?;

        // One generation for the whole batch: a batch is one configuration
        // version, not one per field.
        let generation: i64 = sqlx::query_scalar("SELECT nextval('settings_generation_sequence')")
            .fetch_one(&mut *transaction)
            .await
            .map_err(RepositoryError::from_sqlx)?;

        for (key, value) in &batch.entries {
            if !key.id().persistable() {
                // Defence in depth: the planner refuses these, the schema
                // refuses them, and so does this.
                return Err(RepositoryError::invariant());
            }
            let Some(text) = value.persisted_text() else {
                return Err(RepositoryError::invariant());
            };

            if let Some(camera) = key.camera() {
                sqlx::query("INSERT INTO cameras(camera_id) VALUES ($1) ON CONFLICT DO NOTHING")
                    .bind(camera.as_str())
                    .execute(&mut *transaction)
                    .await
                    .map_err(RepositoryError::from_sqlx)?;
            }

            // The partial unique indexes distinguish global from camera-scoped
            // rows, so the conflict target must match the one that applies.
            let statement = if key.camera().is_some() {
                "INSERT INTO settings (setting_key, camera_id, value, generation) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (setting_key, camera_id) WHERE camera_id IS NOT NULL \
                 DO UPDATE SET value = EXCLUDED.value, generation = EXCLUDED.generation, \
                 updated_at = clock_timestamp()"
            } else {
                "INSERT INTO settings (setting_key, camera_id, value, generation) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (setting_key) WHERE camera_id IS NULL \
                 DO UPDATE SET value = EXCLUDED.value, generation = EXCLUDED.generation, \
                 updated_at = clock_timestamp()"
            };

            sqlx::query(statement)
                .bind(key.id().key())
                .bind(key.camera().map(CameraId::as_str))
                .bind(&text)
                .bind(generation)
                .execute(&mut *transaction)
                .await
                .map_err(RepositoryError::from_sqlx)?;
        }

        for record in &batch.audit {
            insert_audit(&mut transaction, generation, record).await?;
        }

        transaction
            .commit()
            .await
            .map_err(RepositoryError::from_sqlx)?;

        u64::try_from(generation).map_err(|_| RepositoryError::invariant())
    }

    /// Counts audit rows, for tests and operational inspection.
    ///
    /// # Errors
    /// Returns a classified `PostgreSQL` failure.
    pub async fn audit_count(&self) -> Result<i64, RepositoryError> {
        sqlx::query_scalar("SELECT count(*) FROM settings_audit")
            .fetch_one(&self.pool)
            .await
            .map_err(RepositoryError::from_sqlx)
    }
}

/// Turns one stored row into a setting, or says why it cannot be one.
///
/// Rows are rejected rather than dropped so that a stored configuration which
/// is partly unreadable — after a rename, a downgrade, or a hand-edited table —
/// is visible instead of quietly presenting as complete.
fn interpret(
    key: &str,
    camera: Option<CameraId>,
    value: &str,
) -> Result<(SettingKey, SettingValue), RejectionReason> {
    let id = SettingId::from_key(key).ok_or(RejectionReason::UnknownKey)?;
    if !id.persistable() {
        return Err(RejectionReason::NotPersistable(id));
    }
    let setting_key = match camera {
        Some(camera) => {
            if id.scope() == Scope::Global {
                return Err(RejectionReason::WrongScope(id));
            }
            SettingKey::scoped(id, camera).map_err(|_| RejectionReason::WrongScope(id))?
        }
        None => SettingKey::unscoped(id),
    };
    let parsed = SettingValue::parse(id, value).map_err(|_| RejectionReason::Unparseable)?;
    Ok((setting_key, parsed))
}

async fn insert_audit(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    generation: i64,
    record: &AuditRecord,
) -> Result<(), RepositoryError> {
    sqlx::query(
        "INSERT INTO settings_audit \
         (generation, actor, action, setting_key, camera_id, origin, outcome, \
          old_value, new_value, old_fingerprint, new_fingerprint) \
         VALUES ($1, $2, $3::settings_action, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(generation)
    .bind(record.actor.as_str())
    .bind(record.action.as_str())
    .bind(record.key.id().key())
    .bind(record.key.camera().map(CameraId::as_str))
    .bind(record.origin.as_str())
    .bind(record.outcome)
    .bind(record.old_value.as_deref())
    .bind(record.new_value.as_deref())
    .bind(record.old_fingerprint.as_deref())
    .bind(record.new_fingerprint.as_deref())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(RepositoryError::from_sqlx)
}

/// Builds the audit rows for a planned batch.
///
/// Kept beside the writer so that the representation the schema requires — one
/// side plaintext or fingerprint, never both — is chosen in exactly one place.
#[must_use]
pub fn audit_records(
    entries: &[(SettingKey, SettingValue, Option<SettingValue>)],
    actor: &surveillance_config::repository::Actor,
    origin: Origin,
    outcome: &'static str,
    salt: &surveillance_config::secret::Secret<String>,
) -> Vec<AuditRecord> {
    entries
        .iter()
        .map(|(key, new_value, old_value)| {
            let new = new_value.audit_representation(salt);
            let old = old_value
                .as_ref()
                .map(|value| value.audit_representation(salt));
            AuditRecord {
                key: key.clone(),
                actor: actor.clone(),
                action: AuditAction::Set,
                origin,
                outcome,
                old_value: old
                    .as_ref()
                    .and_then(|side| side.value_column().map(str::to_owned)),
                new_value: new.value_column().map(str::to_owned),
                old_fingerprint: old
                    .as_ref()
                    .and_then(surveillance_config::AuditRepresentation::fingerprint_column),
                new_fingerprint: new.fingerprint_column(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::audit_records;
    use surveillance_config::{
        classification::{Origin, SettingId},
        repository::{Actor, AuditAction},
        resolution::SettingKey,
        secret::Secret,
        value::SettingValue,
    };

    fn salt() -> Secret<String> {
        Secret::new("unit".to_owned())
    }

    #[test]
    fn a_public_change_records_plaintext_on_both_sides() {
        let entries = vec![(
            SettingKey::unscoped(SettingId::StorageWatermarkWarning),
            SettingValue::Unsigned(85),
            Some(SettingValue::Unsigned(80)),
        )];
        let records = audit_records(
            &entries,
            &Actor::loopback(),
            Origin::Persisted,
            "applied",
            &salt(),
        );
        let record = records.first().expect("one record");
        assert_eq!(record.old_value.as_deref(), Some("80"));
        assert_eq!(record.new_value.as_deref(), Some("85"));
        assert!(record.old_fingerprint.is_none());
        assert!(record.new_fingerprint.is_none());
        assert_eq!(record.action, AuditAction::Set);
    }

    #[test]
    fn a_first_time_change_has_no_old_side() {
        let entries = vec![(
            SettingKey::unscoped(SettingId::RetentionBatchFiles),
            SettingValue::Unsigned(50),
            None,
        )];
        let records = audit_records(
            &entries,
            &Actor::loopback(),
            Origin::Persisted,
            "applied",
            &salt(),
        );
        let record = records.first().expect("one record");
        assert!(record.old_value.is_none());
        assert!(record.old_fingerprint.is_none());
        assert_eq!(record.new_value.as_deref(), Some("50"));
    }

    #[test]
    fn a_secret_change_records_only_fingerprints() {
        // Secrets are refused before reaching persistence, but the audit
        // builder must still never emit a plaintext credential if one arrives.
        let entries = vec![(
            SettingKey::unscoped(SettingId::CameraRtspPassword),
            SettingValue::Secret(Secret::new("hunter2".to_owned())),
            Some(SettingValue::Secret(Secret::new("hunter1".to_owned()))),
        )];
        let records = audit_records(
            &entries,
            &Actor::loopback(),
            Origin::Persisted,
            "applied",
            &salt(),
        );
        let record = records.first().expect("one record");
        assert!(record.old_value.is_none());
        assert!(record.new_value.is_none());
        assert_eq!(
            record.new_fingerprint.as_ref().map(String::len),
            Some(64),
            "a secret side must be a fingerprint"
        );
        let rendered = format!("{record:?}");
        assert!(!rendered.contains("hunter2"), "audit debug leaked a secret");
    }

    #[test]
    fn each_side_carries_exactly_one_representation() {
        // Mirrors the two CHECK constraints in migrations/0002_settings.sql.
        let entries = vec![
            (
                SettingKey::unscoped(SettingId::RetentionBatchFiles),
                SettingValue::Unsigned(50),
                Some(SettingValue::Unsigned(100)),
            ),
            (
                SettingKey::unscoped(SettingId::CameraRtspPassword),
                SettingValue::Secret(Secret::new("new".to_owned())),
                Some(SettingValue::Secret(Secret::new("old".to_owned()))),
            ),
        ];
        for record in audit_records(
            &entries,
            &Actor::loopback(),
            Origin::Persisted,
            "applied",
            &salt(),
        ) {
            assert!(
                record.old_value.is_none() || record.old_fingerprint.is_none(),
                "old side carries both representations"
            );
            assert!(
                record.new_value.is_none() || record.new_fingerprint.is_none(),
                "new side carries both representations"
            );
            assert!(
                record.new_value.is_some() || record.new_fingerprint.is_some(),
                "a set must record what it set"
            );
        }
    }
}
