use std::{error::Error, sync::Arc};

use surveillance_config::settings::OneCameraConfig;
use surveillance_database::PostgresRecordingRepository;
use surveillance_events::bus::EventBus;
use surveillance_livekit::token::DevelopmentTokenService;
use surveillance_server::{
    camera_supervisor::CameraSupervisor,
    health_http::serve_health,
    metadata_writer::MetadataWriter,
    phase4_metrics::Phase4Metrics,
    playback::PlaybackAssembler,
    reconciliation_owner::ReconciliationOwner,
    retention_owner::RetentionOwner,
    runtime_config::{
        load_database_from_environment, load_ffmpeg_from_environment, load_from_environment,
        load_health_bind_from_environment, load_retention_from_environment,
        load_supervision_from_environment,
    },
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

    let config = Arc::new(OneCameraConfig::validate(load_from_environment()?)?);
    let health_bind = load_health_bind_from_environment()?;
    let supervision = load_supervision_from_environment()?;
    let database = load_database_from_environment()?;
    let retention = load_retention_from_environment()?;
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
                std::time::Duration::from_secs(5 * 60),
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
    let metadata_handle = if let Some(database) = database {
        let repository = PostgresRecordingRepository::connect_lazy(&database);
        let mut reconciliation = ReconciliationOwner::new(
            repository.clone(),
            config.layout.clone(),
            SegmentFileStore::new(config.layout.clone(), config.ffprobe.clone()),
            config.reconciliation_limit,
            256,
            std::time::Duration::from_secs(120),
            std::time::Duration::from_secs(30),
        )
        .with_metrics(phase4_metrics.clone());
        if let Err(category) = reconciliation.run_pass().await {
            tracing::warn!(
                category,
                "startup reconciliation deferred; recording will start degraded"
            );
        }
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
            std::num::NonZeroUsize::new(256).unwrap_or(std::num::NonZeroUsize::MIN),
            std::time::Duration::from_millis(250),
            std::time::Duration::from_secs(30),
        );
        let (shutdown, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(writer.run(shutdown_rx));
        metadata_owner = Some((shutdown, task));
        recording_repository = Some(repository);
        playback_assembler = Some(PlaybackAssembler::new(
            config.layout.root().to_path_buf(),
            ffmpeg,
            std::time::Duration::from_secs(5),
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
    let mut adapter_task = tokio::spawn(serve_health(
        health_bind,
        recording_root,
        registry,
        handle.clone(),
        recording_repository,
        playback_assembler,
        metadata_observer,
        Some(phase4_metrics),
        viewer_tokens,
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
