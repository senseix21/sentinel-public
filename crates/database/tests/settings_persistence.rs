//! Failure-matrix cases for P7.5 that need a real transaction.
//!
//! Transactional safety is not inferred from code structure. Every case here
//! commits or fails against the real store and then reads the result back.
//!
//! Run with `scripts/phase7/config-fixture.sh`, which provides
//! `PHASE4_DATABASE_URL`.

use std::{num::NonZeroU32, time::Duration};

use surveillance_common::types::CameraId;
use surveillance_config::{
    classification::{Origin, SettingId},
    repository::{Actor, AuditAction, AuditRecord, RejectionReason, SettingsBatch},
    resolution::{ResolvedSettings, SettingKey},
    secret::Secret,
    value::SettingValue,
};
use surveillance_database::{
    settings::{audit_records, PostgresSettingsRepository},
    DatabaseConfig,
};

fn database_url() -> String {
    std::env::var("PHASE4_DATABASE_URL").unwrap_or_else(|_| {
        panic!("PHASE4_DATABASE_URL is set by scripts/phase7/config-fixture.sh")
    })
}

fn repository() -> PostgresSettingsRepository {
    let config = DatabaseConfig::new(
        &database_url(),
        NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
        Duration::from_secs(2),
    )
    .unwrap_or_else(|error| panic!("valid database configuration: {error}"));
    PostgresSettingsRepository::connect_lazy(&config)
}

fn salt() -> Secret<String> {
    Secret::new("integration".to_owned())
}

fn camera() -> CameraId {
    CameraId::new("camera-01".to_owned()).unwrap_or_else(|error| panic!("valid camera: {error}"))
}

/// Clears both tables so each case starts from a known state.
async fn reset(repository: &PostgresSettingsRepository) {
    for statement in ["DELETE FROM settings_audit", "DELETE FROM settings"] {
        sqlx::query(statement)
            .execute(repository.pool())
            .await
            .unwrap_or_else(|error| panic!("reset {statement}: {error}"));
    }
}

fn batch(entries: Vec<(SettingKey, SettingValue, Option<SettingValue>)>) -> SettingsBatch {
    let audit = audit_records(
        &entries,
        &Actor::loopback(),
        Origin::Persisted,
        "applied",
        &salt(),
    );
    SettingsBatch {
        entries: entries
            .into_iter()
            .map(|(key, value, _)| (key, value))
            .collect(),
        audit,
    }
}

#[tokio::test]
#[ignore = "requires scripts/phase7/config-fixture.sh"]
async fn migrations_apply_from_empty_and_are_rerunnable() {
    let repository = repository();
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("empty database migrates: {error}"));
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("migration rerun succeeds: {error}"));
}

#[tokio::test]
#[ignore = "requires scripts/phase7/config-fixture.sh"]
async fn a_change_and_its_audit_row_commit_together() {
    let repository = repository();
    repository.migrate().await.expect("migrations apply");
    reset(&repository).await;

    let generation = repository
        .apply(&batch(vec![(
            SettingKey::unscoped(SettingId::StorageWatermarkWarning),
            SettingValue::Unsigned(85),
            None,
        )]))
        .await
        .expect("batch commits");
    assert!(generation > 0, "a committed batch has a generation");

    let loaded = repository.load_all().await.expect("load succeeds");
    let resolved = ResolvedSettings::resolve(vec![loaded.layer]);
    assert_eq!(
        resolved
            .effective(SettingId::StorageWatermarkWarning, None)
            .map(|value| value.value().render()),
        Some("85".to_owned())
    );
    assert_eq!(
        repository.audit_count().await.expect("audit readable"),
        1,
        "the change wrote exactly one audit row"
    );
}

#[tokio::test]
#[ignore = "requires scripts/phase7/config-fixture.sh"]
async fn an_audit_write_failure_rolls_back_the_settings_change() {
    let repository = repository();
    repository.migrate().await.expect("migrations apply");
    reset(&repository).await;

    // An audit row with neither a value nor a fingerprint on the new side
    // violates `settings_audit_set_has_a_new_side`. The settings write in the
    // same transaction must not survive it.
    let key = SettingKey::unscoped(SettingId::RetentionBatchFiles);
    let poisoned = SettingsBatch {
        entries: vec![(key.clone(), SettingValue::Unsigned(42))],
        audit: vec![AuditRecord {
            key,
            actor: Actor::loopback(),
            action: AuditAction::Set,
            origin: Origin::Persisted,
            outcome: "applied",
            old_value: None,
            new_value: None,
            old_fingerprint: None,
            new_fingerprint: None,
        }],
    };

    assert!(
        repository.apply(&poisoned).await.is_err(),
        "the constraint must reject the audit row"
    );

    let loaded = repository.load_all().await.expect("load succeeds");
    let resolved = ResolvedSettings::resolve(vec![loaded.layer]);
    assert!(
        resolved
            .effective(SettingId::RetentionBatchFiles, None)
            .is_none(),
        "the settings write must not have committed"
    );
    assert_eq!(
        repository.audit_count().await.expect("audit readable"),
        0,
        "neither side committed"
    );
}

#[tokio::test]
#[ignore = "requires scripts/phase7/config-fixture.sh"]
async fn a_repeated_global_setting_updates_rather_than_duplicating() {
    // Postgres treats NULLs as distinct in a unique index, so this is the case
    // a plain (setting_key, camera_id) primary key would fail to prevent.
    let repository = repository();
    repository.migrate().await.expect("migrations apply");
    reset(&repository).await;

    for value in [80_u64, 85, 90] {
        repository
            .apply(&batch(vec![(
                SettingKey::unscoped(SettingId::StorageWatermarkWarning),
                SettingValue::Unsigned(value),
                None,
            )]))
            .await
            .expect("batch commits");
    }

    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM settings WHERE setting_key = 'storage.watermark_warning'",
    )
    .fetch_one(repository.pool())
    .await
    .expect("count readable");
    assert_eq!(rows, 1, "a global setting must have exactly one row");

    let loaded = repository.load_all().await.expect("load succeeds");
    let resolved = ResolvedSettings::resolve(vec![loaded.layer]);
    assert_eq!(
        resolved
            .effective(SettingId::StorageWatermarkWarning, None)
            .map(|value| value.value().render()),
        Some("90".to_owned())
    );
}

#[tokio::test]
#[ignore = "requires scripts/phase7/config-fixture.sh"]
async fn camera_scoped_and_global_rows_coexist() {
    let repository = repository();
    repository.migrate().await.expect("migrations apply");
    reset(&repository).await;

    repository
        .apply(&batch(vec![
            (
                SettingKey::unscoped(SettingId::CameraDetectionSampleFps),
                SettingValue::Unsigned(5),
                None,
            ),
            (
                SettingKey::scoped(SettingId::CameraDetectionSampleFps, camera())
                    .expect("camera scoped"),
                SettingValue::Unsigned(15),
                None,
            ),
        ]))
        .await
        .expect("batch commits");

    let loaded = repository.load_all().await.expect("load succeeds");
    let resolved = ResolvedSettings::resolve(vec![loaded.layer]);
    assert_eq!(
        resolved
            .effective(SettingId::CameraDetectionSampleFps, Some(&camera()))
            .map(|value| value.value().render()),
        Some("15".to_owned())
    );
    assert_eq!(
        resolved
            .effective(SettingId::CameraDetectionSampleFps, None)
            .map(|value| value.value().render()),
        Some("5".to_owned())
    );
}

#[tokio::test]
#[ignore = "requires scripts/phase7/config-fixture.sh"]
async fn one_batch_commits_under_one_generation() {
    let repository = repository();
    repository.migrate().await.expect("migrations apply");
    reset(&repository).await;

    let generation = repository
        .apply(&batch(vec![
            (
                SettingKey::unscoped(SettingId::StorageWatermarkWarning),
                SettingValue::Unsigned(85),
                None,
            ),
            (
                SettingKey::unscoped(SettingId::StorageWatermarkCritical),
                SettingValue::Unsigned(92),
                None,
            ),
        ]))
        .await
        .expect("batch commits");

    let distinct: i64 = sqlx::query_scalar("SELECT count(DISTINCT generation) FROM settings")
        .fetch_one(repository.pool())
        .await
        .expect("count readable");
    assert_eq!(distinct, 1, "a batch is one configuration version");

    let audited: i64 =
        sqlx::query_scalar("SELECT count(*) FROM settings_audit WHERE generation = $1")
            .bind(i64::try_from(generation).unwrap_or(0))
            .fetch_one(repository.pool())
            .await
            .expect("count readable");
    assert_eq!(audited, 2);
}

#[tokio::test]
#[ignore = "requires scripts/phase7/config-fixture.sh"]
async fn generations_increase_across_batches() {
    let repository = repository();
    repository.migrate().await.expect("migrations apply");
    reset(&repository).await;

    let first = repository
        .apply(&batch(vec![(
            SettingKey::unscoped(SettingId::RetentionBatchFiles),
            SettingValue::Unsigned(10),
            None,
        )]))
        .await
        .expect("commits");
    let second = repository
        .apply(&batch(vec![(
            SettingKey::unscoped(SettingId::RetentionBatchFiles),
            SettingValue::Unsigned(20),
            None,
        )]))
        .await
        .expect("commits");
    assert!(second > first, "generation must be monotonic");
}

#[tokio::test]
#[ignore = "requires scripts/phase7/config-fixture.sh"]
async fn a_corrupt_stored_row_is_reported_rather_than_dropped() {
    let repository = repository();
    repository.migrate().await.expect("migrations apply");
    reset(&repository).await;

    // A key that no longer names a setting, as a rename would leave behind.
    sqlx::query("INSERT INTO settings (setting_key, value, generation) VALUES ($1, $2, 1)")
        .bind("retention.max_agee")
        .bind("14d")
        .execute(repository.pool())
        .await
        .expect("insert succeeds");

    // A value that no longer parses for its setting.
    sqlx::query("INSERT INTO settings (setting_key, value, generation) VALUES ($1, $2, 1)")
        .bind("retention.interval")
        .bind("not-a-duration")
        .execute(repository.pool())
        .await
        .expect("insert succeeds");

    let loaded = repository.load_all().await.expect("load succeeds");
    assert_eq!(loaded.rejected.len(), 2, "both bad rows are reported");
    assert!(loaded
        .rejected
        .iter()
        .any(|row| row.reason == RejectionReason::UnknownKey));
    assert!(loaded
        .rejected
        .iter()
        .any(|row| row.reason == RejectionReason::Unparseable));
    assert_eq!(
        loaded.layer.len(),
        0,
        "an unreadable row supplies no value at all"
    );
}

#[tokio::test]
#[ignore = "requires scripts/phase7/config-fixture.sh"]
async fn the_schema_refuses_a_secret_key() {
    let repository = repository();
    repository.migrate().await.expect("migrations apply");
    reset(&repository).await;

    // Defence in depth: the planner refuses these and so does the repository,
    // but the schema is the last line and must hold on its own.
    let refused =
        sqlx::query("INSERT INTO settings (setting_key, value, generation) VALUES ($1, $2, 1)")
            .bind("camera.rtsp.password")
            .bind("hunter2")
            .execute(repository.pool())
            .await;
    assert!(
        refused.is_err(),
        "the settings table must never accept a credential"
    );
}

#[tokio::test]
#[ignore = "requires scripts/phase7/config-fixture.sh"]
async fn an_audit_row_carrying_both_representations_is_refused() {
    let repository = repository();
    repository.migrate().await.expect("migrations apply");
    reset(&repository).await;

    let refused = sqlx::query(
        "INSERT INTO settings_audit \
         (generation, actor, action, setting_key, origin, outcome, new_value, new_fingerprint) \
         VALUES (1, 'loopback', 'set', 'retention.interval', 'persisted', 'applied', '30s', $1)",
    )
    .bind("0".repeat(64))
    .execute(repository.pool())
    .await;
    assert!(
        refused.is_err(),
        "a row must carry a value or a fingerprint, never both"
    );
}

#[tokio::test]
#[ignore = "requires scripts/phase7/config-fixture.sh"]
async fn an_empty_store_loads_as_empty_rather_than_failing() {
    let repository = repository();
    repository.migrate().await.expect("migrations apply");
    reset(&repository).await;

    let loaded = repository
        .load_all()
        .await
        .expect("an empty store is valid");
    assert_eq!(loaded.generation, 0);
    assert!(loaded.layer.is_empty());
    assert!(loaded.rejected.is_empty());
}

#[tokio::test]
#[ignore = "requires scripts/phase7/config-fixture.sh"]
async fn an_unreachable_store_fails_the_load_without_panicking() {
    // The caller turns this into Stale; it must be an error, not a panic, and
    // must not block the server from starting.
    let config = DatabaseConfig::new(
        "postgres://127.0.0.1:1/none",
        NonZeroU32::new(1).unwrap_or(NonZeroU32::MIN),
        Duration::from_millis(250),
    )
    .unwrap_or_else(|error| panic!("valid configuration: {error}"));
    let repository = PostgresSettingsRepository::connect_lazy(&config);
    assert!(
        repository.load_all().await.is_err(),
        "an unreachable store reports an error"
    );
}
