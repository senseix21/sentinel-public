//! Composes and owns the SENTINEL server process from startup through joined shutdown.

use std::{error::Error, path::PathBuf, sync::Arc};

use surveillance_config::{
    cache::Cache, cache::StaleRetry, classification::SettingId, secret::Secret,
};
use surveillance_database::PostgresSettingsRepository;

use surveillance_auth::{
    clock::SystemClock,
    verify::{HashParams, Verifier},
};
use surveillance_config::settings::OneCameraConfig;
use surveillance_database::{
    PostgresAccessAuditRepository, PostgresAuthRepository, PostgresRecordingRepository,
};
use surveillance_events::bus::EventBus;
use surveillance_livekit::token::DevelopmentTokenService;
use surveillance_server::{
    authenticate::{
        Authenticator, RequestAuthenticator, SecurityOwner, VerifyConcurrency,
        DEFAULT_VERIFY_MEMORY_BUDGET_BYTES,
    },
    authorize::MutatingSurface,
    camera_supervisor::CameraSupervisor,
    health_http::{serve_health, CameraDefaults},
    metadata_writer::MetadataWriter,
    phase4_metrics::Phase4Metrics,
    playback::PlaybackAssembler,
    reconciliation_owner::ReconciliationOwner,
    retention_owner::RetentionOwner,
    runtime_config::{
        bootstrap_count, bootstrap_duration, bootstrap_flag, bootstrap_path, bootstrap_secret,
        load_configuration_layers, load_database_from_environment, load_ffmpeg_from_environment,
        load_from_environment, load_health_bind_from_environment, load_retention,
        load_supervision_from_environment,
    },
    security_config::{load_trust_boundary, resolve_bind},
    settings_service::SettingsService,
};
use surveillance_storage::{
    reconciliation::discover_reconciliation_candidates, retention::SafeRecordingDelete,
    segment_store::SegmentFileStore,
};
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

#[tokio::main]
#[allow(
    clippy::too_many_lines,
    reason = "the process owner keeps startup and reverse-order joined shutdown auditable"
)]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init()?;

    let configuration = load_configuration_layers()?;
    let bootstrap_camera = load_from_environment(&configuration.bootstrap)?;
    // Kept before validation consumes the settings. `OneCameraConfig` holds a
    // validated `LiveKitConfig`, which is camera-scoped and cannot be turned
    // back into the deployment template, so the API's add path would have
    // nothing to stamp onto a new camera.
    let livekit_template = bootstrap_camera.livekit.clone();
    let config = Arc::new(OneCameraConfig::validate(bootstrap_camera)?);
    let health_bind = load_health_bind_from_environment()?;
    let supervision = load_supervision_from_environment()?;
    let database = load_database_from_environment()?;
    let retention = load_retention(&configuration.bootstrap)?;
    let ffmpeg = load_ffmpeg_from_environment();
    let phase4_metrics = Phase4Metrics::default();
    let event_bus = EventBus::new();
    let viewer_tokens = config
        .livekit
        .as_ref()
        .map(|livekit| {
            DevelopmentTokenService::new(
                livekit,
                livekit.deployment_namespace.clone(),
                bootstrap_duration(&configuration.bootstrap, SettingId::LiveKitViewerTokenTtl)
                    .unwrap_or(std::time::Duration::from_secs(5 * 60)),
            )
        })
        .transpose()?;
    tracing::info!(
        camera_id = %config.camera_id,
        endpoint = %config.source.endpoint(),
        bounded_memory_bytes = config.bounded_memory_bytes,
        "validated initial camera configuration"
    );
    let mut metadata_owner = None;
    let mut reconciliation_owner = None;
    let mut retention_owner = None;
    let mut recording_repository = None;
    let mut playback_assembler = None;
    // Borrowed rather than moved: the identity store below needs the same
    // configuration, and a per-unit PostgreSQL is one database, not two.
    let metadata_handle = if let Some(database) = database.as_ref() {
        let repository = PostgresRecordingRepository::connect_lazy(database);
        let reconciliation = ReconciliationOwner::new(
            repository.clone(),
            config.layout.clone(),
            SegmentFileStore::new(config.layout.clone(), config.ffprobe.clone()),
            config.reconciliation_limit,
            bootstrap_count(
                &configuration.bootstrap,
                SettingId::ReconciliationQueueItems,
                256,
            ),
            bootstrap_duration(&configuration.bootstrap, SettingId::ReconciliationInterval)
                .unwrap_or(std::time::Duration::from_secs(120)),
            bootstrap_duration(&configuration.bootstrap, SettingId::ReconciliationBackoff)
                .unwrap_or(std::time::Duration::from_secs(30)),
        )
        .with_metrics(phase4_metrics.clone());
        // Reconciliation is recovery/repair work, not a prerequisite for the
        // control plane. Running a complete pass here made `/live` and the
        // login page wait for one sequential ffprobe per durable segment. The
        // owned task below starts its first bounded pass immediately; new
        // recording remains protected by the metadata writer and partial-file
        // grace rules while that idempotent pass is in progress.
        tracing::info!("recording reconciliation scheduled in the background");
        let (reconciliation_shutdown, reconciliation_shutdown_rx) = watch::channel(false);
        let reconciliation_task = tokio::spawn(reconciliation.run(reconciliation_shutdown_rx));
        reconciliation_owner = Some((reconciliation_shutdown, reconciliation_task));
        let retention_task_owner = RetentionOwner::new(
            repository.clone(),
            SafeRecordingDelete::new(config.layout.clone()),
            config.layout.root().to_path_buf(),
            retention.policy,
            retention.interval,
        )
        .with_metrics(phase4_metrics.clone())
        .with_event_bus(event_bus.clone());
        let (retention_shutdown, retention_shutdown_rx) = watch::channel(false);
        let retention_task = tokio::spawn(retention_task_owner.run(retention_shutdown_rx));
        retention_owner = Some((retention_shutdown, retention_task));
        let (writer, writer_handle) = MetadataWriter::new(
            repository.clone(),
            std::num::NonZeroUsize::new(bootstrap_count(
                &configuration.bootstrap,
                SettingId::MetadataQueueItems,
                256,
            ))
            .unwrap_or(std::num::NonZeroUsize::MIN),
            bootstrap_duration(&configuration.bootstrap, SettingId::MetadataFlushInterval)
                .unwrap_or(std::time::Duration::from_millis(250)),
            bootstrap_duration(&configuration.bootstrap, SettingId::MetadataRetryBackoff)
                .unwrap_or(std::time::Duration::from_secs(30)),
        );
        let (shutdown, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(writer.run(shutdown_rx));
        metadata_owner = Some((shutdown, task));
        recording_repository = Some(repository);
        playback_assembler = Some(PlaybackAssembler::new(
            config.layout.root().to_path_buf(),
            ffmpeg,
            bootstrap_duration(&configuration.bootstrap, SettingId::PlaybackProbeTimeout)
                .unwrap_or(std::time::Duration::from_secs(5)),
        ));
        Some(writer_handle)
    } else {
        tracing::warn!("database is not configured; recordings remain durable but unindexed until reconciliation");
        let discovery =
            discover_reconciliation_candidates(&config.layout, config.reconciliation_limit).await?;
        if !discovery.candidates.is_empty() {
            tracing::warn!(
                candidates = discovery.candidates.len(),
                examined = discovery.examined,
                "recording files require metadata reconciliation"
            );
        }
        None
    };
    let metadata_observer = metadata_handle.clone();

    let (supervisor, handle) = CameraSupervisor::with_event_bus(
        supervision.command_queue_items,
        supervision.maximum_cameras,
        supervision.command_timeout,
        metadata_handle,
        event_bus,
    );
    let supervisor_task = tokio::spawn(supervisor.run());
    handle.add(Arc::clone(&config)).await?;
    handle.start(config.camera_id.clone()).await?;

    let (adapter_shutdown, adapter_shutdown_rx) = watch::channel(false);
    let registry = handle.health_registry();
    let recording_root = config.layout.root().to_path_buf();

    // Persistent configuration. The settings store is optional: without it the
    // server runs on bootstrap layers and refuses changes, rather than
    // accepting changes it cannot make durable.
    let settings_repository = load_database_from_environment()?
        .as_ref()
        .map(PostgresSettingsRepository::connect_lazy);
    let settings_service = Arc::new(SettingsService::new(
        configuration.base,
        configuration.environment,
        settings_repository,
        Cache::new(
            bootstrap_path(&configuration.bootstrap, SettingId::ConfigCachePath)
                .unwrap_or_else(|| PathBuf::from("config/last-known-good.toml")),
        ),
        // The salt only matters for secret audit rows, which cannot occur while
        // secrets are not persistable. It is read here so that enabling secret
        // persistence later does not need a second wiring change.
        bootstrap_secret(&configuration.bootstrap, SettingId::AuditFingerprintSalt)
            .unwrap_or_else(|| Secret::new(String::new())),
        Some(handle.event_bus()),
        StaleRetry::new(
            bootstrap_duration(
                &configuration.bootstrap,
                SettingId::ConfigStaleRetryInterval,
            )
            .unwrap_or(std::time::Duration::from_secs(5)),
            bootstrap_duration(&configuration.bootstrap, SettingId::ConfigStaleRetryBudget)
                .unwrap_or(std::time::Duration::from_secs(300)),
        ),
        MutatingSurface::from_enabled(bootstrap_flag(
            &configuration.bootstrap,
            SettingId::ApiMutatingSurfaceEnabled,
        )),
    ));
    let freshness = settings_service.load().await;
    if freshness.is_stale() {
        tracing::warn!(
            "configuration loaded from the last-known-good cache; the settings store was \
             unavailable. Recording is unaffected and readiness reports degradation."
        );
    }
    let settings_service = Some(settings_service);

    // ADR 009: identity, trust boundary, and the rule that unlocks a routable
    // listener. Built here because it needs the database configuration and the
    // resolved bind address, and enforced before the listener is created.
    let trust_boundary = load_trust_boundary(|key| std::env::var(key).ok())?;
    let (security, has_enabled_principal) = match database.as_ref() {
        Some(database) => {
            let repository = PostgresAuthRepository::connect_lazy(database);
            // A store that cannot be reached reports no principals, which
            // refuses a routable bind. Failing closed is the correct direction:
            // binding anyway would fall back to the development identity.
            let present = repository.has_enabled_principal().await.unwrap_or(false);
            let verifier = Arc::new(Verifier::new(HashParams::default())?);
            let concurrency = VerifyConcurrency::from_budget(
                HashParams::default(),
                DEFAULT_VERIFY_MEMORY_BUDGET_BYTES,
            );
            let authenticator: Arc<dyn RequestAuthenticator> = Arc::new(Authenticator::new(
                repository,
                SystemClock,
                verifier,
                concurrency,
            ));
            (
                SecurityOwner::new(trust_boundary, Some(authenticator)),
                present,
            )
        }
        None => (SecurityOwner::new(trust_boundary, None), false),
    };
    // Same per-unit database as the identity store and the recording index.
    let access_audit = database
        .as_ref()
        .map(PostgresAccessAuditRepository::connect_lazy);
    // A second lazy handle to the same per-unit database. The authenticator
    // owns one for verification; the identity API needs one for administration,
    // and they have different lifetimes.
    let identities = database
        .as_ref()
        .map(|config| Arc::new(PostgresAuthRepository::connect_lazy(config)));
    let health_bind = resolve_bind(health_bind, security.boundary(), has_enabled_principal)?;
    if !security.has_authenticator() {
        tracing::warn!(
            "no identity store is configured; the API grants administrator to every loopback \
             caller as principal:service:loopback-development. This is a development posture \
             and a routable listener is refused while it holds."
        );
    }
    let security = Some(Arc::new(security));

    let mut adapter_task = tokio::spawn(serve_health(
        health_bind,
        CameraDefaults::new(recording_root, livekit_template),
        registry,
        handle.clone(),
        recording_repository,
        playback_assembler,
        metadata_observer,
        Some(phase4_metrics),
        viewer_tokens,
        settings_service,
        security,
        access_audit,
        identities,
        adapter_shutdown_rx,
    ));

    tokio::select! {
        signal = tokio::signal::ctrl_c() => signal?,
        result = &mut adapter_task => {
            match result {
                Ok(Ok(())) => tracing::warn!("health/API adapter stopped; camera pipelines remain active"),
                Ok(Err(error)) => tracing::error!(%error, "health/API adapter failed; camera pipelines remain active"),
                Err(error) => tracing::error!(%error, "health/API adapter task failed; camera pipelines remain active"),
            }
            tokio::signal::ctrl_c().await?;
        }
    }

    tracing::info!("process shutdown requested; joining all camera pipelines");
    adapter_shutdown.send_replace(true);
    handle.shutdown().await?;
    supervisor_task.await?;
    if let Some((shutdown, task)) = metadata_owner {
        shutdown.send_replace(true);
        task.await?;
    }
    if let Some((shutdown, task)) = reconciliation_owner {
        shutdown.send_replace(true);
        task.await?;
    }
    if let Some((shutdown, task)) = retention_owner {
        shutdown.send_replace(true);
        task.await?;
    }
    if !adapter_task.is_finished() {
        adapter_task.await??;
    }
    Ok(())
}
