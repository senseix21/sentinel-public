use std::{error::Error, sync::Arc};

use surveillance_config::settings::OneCameraConfig;
use surveillance_server::{
    camera_supervisor::CameraSupervisor,
    health_http::serve_health,
    runtime_config::{
        load_from_environment, load_health_bind_from_environment, load_supervision_from_environment,
    },
};
use surveillance_storage::reconciliation::discover_reconciliation_candidates;
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

#[tokio::main]
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
    tracing::info!(
        camera_id = %config.camera_id,
        endpoint = %config.source.endpoint(),
        bounded_memory_bytes = config.bounded_memory_bytes,
        "validated initial camera configuration"
    );
    let recovery =
        discover_reconciliation_candidates(&config.layout, config.reconciliation_limit).await?;
    if !recovery.candidates.is_empty() {
        tracing::warn!(
            candidates = recovery.candidates.len(),
            "recording files require metadata reconciliation"
        );
    }

    let (supervisor, handle) = CameraSupervisor::new(
        supervision.command_queue_items,
        supervision.maximum_cameras,
        supervision.command_timeout,
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
    if !adapter_task.is_finished() {
        adapter_task.await??;
    }
    Ok(())
}
