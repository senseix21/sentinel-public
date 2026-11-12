//! Process-level ownership of configured camera pipelines.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    num::NonZeroUsize,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};

use surveillance_common::types::CameraId;
use surveillance_config::settings::OneCameraConfig;
use surveillance_events::bus::EventBus;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinSet,
};

use crate::{
    camera_catalog::{CameraCatalog, CameraRegistration},
    metadata_writer::MetadataWriterHandle,
    one_camera::{OneCameraExit, OneCameraHealth, OneCameraHealthSnapshot, PipelineStopReason},
    pipeline::{classify_exit, ActionableFailure, CameraPipeline, CameraPipelineError},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineState {
    Configured,
    Starting,
    Running,
    Stopping,
    Stopped,
    Degraded,
    Terminal,
}

#[derive(Clone, Debug)]
pub struct CameraStatusSnapshot {
    pub camera_id: CameraId,
    pub pipeline_state: PipelineState,
    pub health: Option<OneCameraHealthSnapshot>,
    pub last_failure: Option<ActionableFailure>,
}

#[derive(Clone)]
pub struct CameraHealthRegistry {
    maximum: NonZeroUsize,
    entries: Arc<RwLock<BTreeMap<CameraId, RegistryEntry>>>,
}

#[derive(Clone)]
struct RegistryEntry {
    state: PipelineState,
    health: Option<OneCameraHealth>,
    last_failure: Option<ActionableFailure>,
}

impl CameraHealthRegistry {
    #[must_use]
    pub fn new(maximum: NonZeroUsize) -> Self {
        Self {
            maximum,
            entries: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    #[must_use]
    pub fn snapshots(&self) -> Vec<CameraStatusSnapshot> {
        let entries = self
            .entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries
            .iter()
            .map(|(camera_id, entry)| CameraStatusSnapshot {
                camera_id: camera_id.clone(),
                pipeline_state: entry.state,
                health: entry.health.as_ref().map(OneCameraHealth::snapshot),
                last_failure: entry.last_failure,
            })
            .collect()
    }

    #[must_use]
    pub fn snapshot(&self, camera_id: &CameraId) -> Option<CameraStatusSnapshot> {
        let entries = self
            .entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.get(camera_id).map(|entry| CameraStatusSnapshot {
            camera_id: camera_id.clone(),
            pipeline_state: entry.state,
            health: entry.health.as_ref().map(OneCameraHealth::snapshot),
            last_failure: entry.last_failure,
        })
    }

    fn insert_configured(&self, camera_id: CameraId) -> Result<(), SupervisorOperationError> {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if entries.len() >= self.maximum.get() {
            return Err(SupervisorOperationError::RegistryFull);
        }
        entries.insert(
            camera_id,
            RegistryEntry {
                state: PipelineState::Configured,
                health: None,
                last_failure: None,
            },
        );
        Ok(())
    }

    fn update(
        &self,
        camera_id: &CameraId,
        state: PipelineState,
        health: Option<OneCameraHealth>,
        failure: Option<ActionableFailure>,
    ) {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = entries.get_mut(camera_id) {
            entry.state = state;
            if health.is_some() {
                entry.health = health;
            }
            entry.last_failure = failure.or(entry.last_failure);
        }
    }

    fn remove(&self, camera_id: &CameraId) {
        self.entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(camera_id);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorOperationError {
    DuplicateCamera,
    UnknownCamera,
    RegistryFull,
    PipelineStart,
    PipelineJoin,
    ShuttingDown,
    Persistence,
}

impl fmt::Display for SupervisorOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "camera supervisor operation failed: {self:?}")
    }
}

impl Error for SupervisorOperationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorRequestError {
    Full,
    Closed,
    TimedOut,
    Operation(SupervisorOperationError),
}

impl fmt::Display for SupervisorRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "camera supervisor request failed: {self:?}")
    }
}

impl Error for SupervisorRequestError {}

type Reply = oneshot::Sender<Result<(), SupervisorOperationError>>;

enum Command {
    Add(Arc<OneCameraConfig>, Reply),
    Register(Arc<OneCameraConfig>, CameraRegistration, Reply),
    Start(CameraId, Reply),
    Stop(CameraId, Reply),
    Remove(CameraId, Reply),
    Shutdown(Reply),
}

#[derive(Clone)]
pub struct CameraSupervisorHandle {
    commands: mpsc::Sender<Command>,
    response_timeout: Duration,
    registry: CameraHealthRegistry,
    event_bus: EventBus,
    catalog_available: Arc<AtomicBool>,
}

impl CameraSupervisorHandle {
    /// Whether durable registration storage is available for mutation.
    #[must_use]
    pub fn catalog_available(&self) -> bool {
        self.catalog_available.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn health_registry(&self) -> CameraHealthRegistry {
        self.registry.clone()
    }

    #[must_use]
    pub fn event_bus(&self) -> EventBus {
        self.event_bus.clone()
    }

    /// Adds one validated stopped configuration.
    ///
    /// # Errors
    /// Returns bounded transport or typed operation failure.
    pub async fn add(&self, config: Arc<OneCameraConfig>) -> Result<(), SupervisorRequestError> {
        self.request(|reply| Command::Add(config, reply)).await
    }

    /// Adds a durable stopped camera. Persistence precedes runtime visibility.
    /// # Errors
    /// Returns bounded transport, validation or persistence failure.
    pub async fn register(
        &self,
        config: Arc<OneCameraConfig>,
        camera: CameraRegistration,
    ) -> Result<(), SupervisorRequestError> {
        self.request(|reply| Command::Register(config, camera, reply))
            .await
    }

    /// Starts one configured camera; repeated active starts are idempotent.
    ///
    /// # Errors
    /// Returns bounded transport or typed operation failure.
    pub async fn start(&self, camera_id: CameraId) -> Result<(), SupervisorRequestError> {
        self.request(|reply| Command::Start(camera_id, reply)).await
    }

    /// Stops and joins one camera; repeated stopped requests are idempotent.
    ///
    /// # Errors
    /// Returns bounded transport or typed operation failure.
    pub async fn stop(&self, camera_id: CameraId) -> Result<(), SupervisorRequestError> {
        self.request(|reply| Command::Stop(camera_id, reply)).await
    }

    /// Stops, joins, then removes one camera.
    ///
    /// # Errors
    /// Returns bounded transport or typed operation failure.
    pub async fn remove(&self, camera_id: CameraId) -> Result<(), SupervisorRequestError> {
        self.request(|reply| Command::Remove(camera_id, reply))
            .await
    }

    /// Stops and joins every owned pipeline before returning.
    ///
    /// # Errors
    /// Returns bounded transport or typed operation failure.
    pub async fn shutdown(&self) -> Result<(), SupervisorRequestError> {
        self.request(Command::Shutdown).await
    }

    async fn request(
        &self,
        command: impl FnOnce(Reply) -> Command,
    ) -> Result<(), SupervisorRequestError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.commands
            .try_send(command(reply_tx))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => SupervisorRequestError::Full,
                mpsc::error::TrySendError::Closed(_) => SupervisorRequestError::Closed,
            })?;
        tokio::time::timeout(self.response_timeout, reply_rx)
            .await
            .map_err(|_| SupervisorRequestError::TimedOut)?
            .map_err(|_| SupervisorRequestError::Closed)?
            .map_err(SupervisorRequestError::Operation)
    }
}

pub struct CameraSupervisor {
    commands: mpsc::Receiver<Command>,
    registry: CameraHealthRegistry,
    entries: BTreeMap<CameraId, CameraEntry>,
    catalog: Option<CameraCatalog>,
    catalog_available: Arc<AtomicBool>,
    operations: JoinSet<OperationCompletion>,
    shutting_down: bool,
    shutdown_reply: Option<Reply>,
    observation_interval: tokio::time::Interval,
    metadata_writer: Option<MetadataWriterHandle>,
    event_bus: EventBus,
}

struct CameraEntry {
    config: Arc<OneCameraConfig>,
    state: PipelineState,
    pipeline: Option<CameraPipeline>,
    pending: Vec<Reply>,
    remove_after_stop: bool,
    stop_after_start: bool,
}

enum OperationCompletion {
    Started {
        camera_id: CameraId,
        result: Result<CameraPipeline, CameraPipelineError>,
    },
    Stopped {
        camera_id: CameraId,
        result: Box<Result<Option<OneCameraExit>, CameraPipelineError>>,
    },
}

impl CameraSupervisor {
    #[must_use]
    pub fn new(
        command_capacity: NonZeroUsize,
        maximum_cameras: NonZeroUsize,
        response_timeout: Duration,
    ) -> (Self, CameraSupervisorHandle) {
        Self::with_metadata_writer(command_capacity, maximum_cameras, response_timeout, None)
    }

    #[must_use]
    pub fn with_metadata_writer(
        command_capacity: NonZeroUsize,
        maximum_cameras: NonZeroUsize,
        response_timeout: Duration,
        metadata_writer: Option<MetadataWriterHandle>,
    ) -> (Self, CameraSupervisorHandle) {
        Self::with_event_bus(
            command_capacity,
            maximum_cameras,
            response_timeout,
            metadata_writer,
            EventBus::new(),
        )
    }

    #[must_use]
    pub fn with_event_bus(
        command_capacity: NonZeroUsize,
        maximum_cameras: NonZeroUsize,
        response_timeout: Duration,
        metadata_writer: Option<MetadataWriterHandle>,
        event_bus: EventBus,
    ) -> (Self, CameraSupervisorHandle) {
        let (tx, rx) = mpsc::channel(command_capacity.get());
        let registry = CameraHealthRegistry::new(maximum_cameras);
        let catalog_available = Arc::new(AtomicBool::new(true));
        (
            Self {
                commands: rx,
                registry: registry.clone(),
                entries: BTreeMap::new(),
                catalog: None,
                catalog_available: Arc::clone(&catalog_available),
                operations: JoinSet::new(),
                shutting_down: false,
                shutdown_reply: None,
                observation_interval: tokio::time::interval(Duration::from_millis(25)),
                metadata_writer,
                event_bus: event_bus.clone(),
            },
            CameraSupervisorHandle {
                commands: tx,
                response_timeout,
                registry,
                event_bus,
                catalog_available,
            },
        )
    }

    /// Installs the single persistence owner before its command loop starts.
    #[must_use]
    pub fn with_catalog(mut self, catalog: CameraCatalog) -> Self {
        self.catalog = Some(catalog);
        self
    }

    pub async fn run(mut self) {
        loop {
            if self.shutting_down && self.operations.is_empty() && self.all_pipelines_joined() {
                if let Some(reply) = self.shutdown_reply.take() {
                    let _ = reply.send(Ok(()));
                }
                return;
            }
            tokio::select! {
                command = self.commands.recv(), if !self.shutting_down => {
                    match command {
                        Some(command) => self.handle_command(command).await,
                        None => self.begin_shutdown(None),
                    }
                }
                completion = self.operations.join_next(), if !self.operations.is_empty() => {
                    if let Some(completion) = completion {
                        if let Ok(completion) = completion {
                            self.handle_completion(completion);
                        } else {
                            tracing::error!("supervisor-owned pipeline operation panicked");
                        }
                    }
                }
                _ = self.observation_interval.tick() => self.observe_finished_pipelines(),
            }
        }
    }

    async fn handle_command(&mut self, command: Command) {
        // All mutations pass this single owner. No HTTP task can race catalog
        // replacement against another command or outlive a canceled request.
        let intent = match &command {
            Command::Start(id, _) => Some((id.clone(), Some(true))),
            Command::Stop(id, _) => Some((id.clone(), Some(false))),
            Command::Remove(id, _) => Some((id.clone(), None)),
            _ => None,
        };
        if let Some((id, running)) = intent {
            if self.entries.contains_key(&id) {
                if let Some(catalog) = self.catalog.as_mut() {
                    if catalog.intent(id.as_str(), running).await.is_err() {
                        self.catalog_available
                            .store(catalog.available(), Ordering::Relaxed);
                        tracing::error!(camera_id = %id, "camera intent persistence failed");
                        match command {
                            Command::Start(_, reply)
                            | Command::Stop(_, reply)
                            | Command::Remove(_, reply) => {
                                let _ = reply.send(Err(SupervisorOperationError::Persistence));
                            }
                            _ => {}
                        }
                        return;
                    }
                }
            }
        }
        match command {
            Command::Register(config, camera, reply) => {
                if self.entries.contains_key(&config.camera_id) {
                    let _ = reply.send(Err(SupervisorOperationError::DuplicateCamera));
                } else if self.entries.len() >= self.registry.maximum.get() {
                    let _ = reply.send(Err(SupervisorOperationError::RegistryFull));
                } else {
                    let saved = match self.catalog.as_mut() {
                        Some(catalog) => {
                            let saved = catalog.add(camera).await.is_ok();
                            self.catalog_available
                                .store(catalog.available(), Ordering::Relaxed);
                            saved
                        }
                        None => false,
                    };
                    if saved {
                        self.add(config, reply);
                    } else {
                        let _ = reply.send(Err(SupervisorOperationError::Persistence));
                    }
                }
            }
            Command::Add(config, reply) => self.add(config, reply),
            Command::Start(camera_id, reply) => self.start(camera_id, reply),
            Command::Stop(camera_id, reply) => self.stop(camera_id, reply, false),
            Command::Remove(camera_id, reply) => self.stop(camera_id, reply, true),
            Command::Shutdown(reply) => self.begin_shutdown(Some(reply)),
        }
    }

    fn add(&mut self, config: Arc<OneCameraConfig>, reply: Reply) {
        let camera_id = config.camera_id.clone();
        let result = if self.entries.contains_key(&camera_id) {
            Err(SupervisorOperationError::DuplicateCamera)
        } else {
            self.registry
                .insert_configured(camera_id.clone())
                .map(|()| {
                    self.entries.insert(
                        camera_id,
                        CameraEntry {
                            config,
                            state: PipelineState::Configured,
                            pipeline: None,
                            pending: Vec::new(),
                            remove_after_stop: false,
                            stop_after_start: false,
                        },
                    );
                })
        };
        let _ = reply.send(result);
    }

    fn start(&mut self, camera_id: CameraId, reply: Reply) {
        let Some(entry) = self.entries.get_mut(&camera_id) else {
            let _ = reply.send(Err(SupervisorOperationError::UnknownCamera));
            return;
        };
        match entry.state {
            PipelineState::Running => {
                let _ = reply.send(Ok(()));
            }
            PipelineState::Starting => entry.pending.push(reply),
            PipelineState::Stopping => {
                let _ = reply.send(Err(SupervisorOperationError::ShuttingDown));
            }
            PipelineState::Configured
            | PipelineState::Stopped
            | PipelineState::Degraded
            | PipelineState::Terminal => {
                entry.state = PipelineState::Starting;
                entry.pending.push(reply);
                self.registry
                    .update(&camera_id, PipelineState::Starting, None, None);
                let config = Arc::clone(&entry.config);
                let metadata_writer = self.metadata_writer.clone();
                let event_bus = self.event_bus.clone();
                self.operations.spawn(async move {
                    OperationCompletion::Started {
                        camera_id,
                        result: CameraPipeline::start(config, metadata_writer, event_bus).await,
                    }
                });
            }
        }
    }

    fn stop(&mut self, camera_id: CameraId, reply: Reply, remove: bool) {
        let Some(entry) = self.entries.get_mut(&camera_id) else {
            let _ = reply.send(Err(SupervisorOperationError::UnknownCamera));
            return;
        };
        if remove {
            entry.remove_after_stop = true;
        }
        match entry.state {
            PipelineState::Starting => {
                entry.stop_after_start = true;
                entry.pending.push(reply);
            }
            PipelineState::Stopping => entry.pending.push(reply),
            PipelineState::Running => {
                entry.pending.push(reply);
                entry.state = PipelineState::Stopping;
                self.registry
                    .update(&camera_id, PipelineState::Stopping, None, None);
                self.spawn_stop(camera_id);
            }
            PipelineState::Configured
            | PipelineState::Stopped
            | PipelineState::Degraded
            | PipelineState::Terminal => {
                if remove {
                    self.entries.remove(&camera_id);
                    self.registry.remove(&camera_id);
                } else {
                    entry.state = PipelineState::Stopped;
                    self.registry
                        .update(&camera_id, PipelineState::Stopped, None, None);
                }
                let _ = reply.send(Ok(()));
            }
        }
    }

    fn spawn_stop(&mut self, camera_id: CameraId) {
        let Some(entry) = self.entries.get_mut(&camera_id) else {
            return;
        };
        let Some(mut pipeline) = entry.pipeline.take() else {
            return;
        };
        self.operations.spawn(async move {
            OperationCompletion::Stopped {
                camera_id,
                result: Box::new(pipeline.stop_and_join().await),
            }
        });
    }

    fn spawn_join_finished(&mut self, camera_id: CameraId) {
        let Some(entry) = self.entries.get_mut(&camera_id) else {
            return;
        };
        let Some(mut pipeline) = entry.pipeline.take() else {
            return;
        };
        self.operations.spawn(async move {
            OperationCompletion::Stopped {
                camera_id,
                result: Box::new(pipeline.join_finished().await),
            }
        });
    }

    fn observe_finished_pipelines(&mut self) {
        let finished: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(camera_id, entry)| {
                entry
                    .pipeline
                    .as_ref()
                    .filter(|pipeline| pipeline.is_finished())
                    .map(|_| camera_id.clone())
            })
            .collect();
        for camera_id in finished {
            if let Some(entry) = self.entries.get_mut(&camera_id) {
                entry.state = PipelineState::Stopping;
            }
            self.registry
                .update(&camera_id, PipelineState::Stopping, None, None);
            self.spawn_join_finished(camera_id);
        }
    }

    fn handle_completion(&mut self, completion: OperationCompletion) {
        match completion {
            OperationCompletion::Started { camera_id, result } => {
                let Some(entry) = self.entries.get_mut(&camera_id) else {
                    return;
                };
                match result {
                    Ok(pipeline)
                        if entry.remove_after_stop
                            || entry.stop_after_start
                            || self.shutting_down =>
                    {
                        entry.pipeline = Some(pipeline);
                        entry.state = PipelineState::Stopping;
                        self.registry
                            .update(&camera_id, PipelineState::Stopping, None, None);
                        self.spawn_stop(camera_id);
                    }
                    Ok(pipeline) => {
                        let health = pipeline.health();
                        entry.pipeline = Some(pipeline);
                        entry.state = PipelineState::Running;
                        self.registry.update(
                            &camera_id,
                            PipelineState::Running,
                            Some(health),
                            None,
                        );
                        reply_all(&mut entry.pending, Ok(()));
                    }
                    Err(_) => {
                        entry.state = PipelineState::Degraded;
                        self.registry.update(
                            &camera_id,
                            PipelineState::Degraded,
                            None,
                            Some(ActionableFailure::RecorderFailure),
                        );
                        reply_all(
                            &mut entry.pending,
                            Err(SupervisorOperationError::PipelineStart),
                        );
                    }
                }
            }
            OperationCompletion::Stopped { camera_id, result } => {
                let Some(mut entry) = self.entries.remove(&camera_id) else {
                    return;
                };
                let operation_result = if let Ok(exit) = *result {
                    let (state, failure) = terminal_state(exit.as_ref());
                    entry.state = state;
                    self.registry.update(&camera_id, state, None, failure);
                    Ok(())
                } else {
                    entry.state = PipelineState::Degraded;
                    self.registry.update(
                        &camera_id,
                        PipelineState::Degraded,
                        None,
                        Some(ActionableFailure::PipelineTaskPanicked),
                    );
                    Err(SupervisorOperationError::PipelineJoin)
                };
                reply_all(&mut entry.pending, operation_result);
                if entry.remove_after_stop || self.shutting_down {
                    self.registry.remove(&camera_id);
                } else {
                    self.entries.insert(camera_id, entry);
                }
            }
        }
    }

    fn begin_shutdown(&mut self, reply: Option<Reply>) {
        self.shutting_down = true;
        self.shutdown_reply = reply;
        let running: Vec<_> = self
            .entries
            .iter_mut()
            .filter_map(|(camera_id, entry)| {
                entry.remove_after_stop = true;
                entry.stop_after_start = true;
                if entry.state == PipelineState::Running {
                    entry.state = PipelineState::Stopping;
                    Some(camera_id.clone())
                } else {
                    None
                }
            })
            .collect();
        for camera_id in running {
            self.registry
                .update(&camera_id, PipelineState::Stopping, None, None);
            self.spawn_stop(camera_id);
        }
        self.entries.retain(|camera_id, entry| {
            let retain = matches!(
                entry.state,
                PipelineState::Starting | PipelineState::Stopping
            );
            if !retain {
                self.registry.remove(camera_id);
                reply_all(
                    &mut entry.pending,
                    Err(SupervisorOperationError::ShuttingDown),
                );
            }
            retain
        });
    }

    fn all_pipelines_joined(&self) -> bool {
        self.entries.values().all(|entry| entry.pipeline.is_none())
    }
}

fn terminal_state(exit: Option<&OneCameraExit>) -> (PipelineState, Option<ActionableFailure>) {
    let Some(exit) = exit else {
        return (PipelineState::Stopped, None);
    };
    let failure = classify_exit(exit);
    let state = match (exit.reason, failure) {
        (_, Some(ActionableFailure::TerminalCameraConfiguration)) => PipelineState::Terminal,
        (_, Some(_)) | (PipelineStopReason::RecorderStopped, None) => PipelineState::Degraded,
        (PipelineStopReason::Requested | PipelineStopReason::CameraStopped, None) => {
            PipelineState::Stopped
        }
    };
    (state, failure)
}

fn reply_all(replies: &mut Vec<Reply>, result: Result<(), SupervisorOperationError>) {
    for reply in replies.drain(..) {
        let _ = reply.send(result);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CameraSupervisor, PipelineState, SupervisorOperationError, SupervisorRequestError,
    };
    use std::{num::NonZeroUsize, path::PathBuf, sync::Arc, time::Duration};
    use surveillance_common::types::CameraId;
    use surveillance_config::settings::{OneCameraConfig, OneCameraSettings};

    fn camera_id(value: &str) -> CameraId {
        CameraId::new(value).unwrap_or_else(|error| panic!("valid test camera ID: {error}"))
    }

    fn config(value: &str, port: u16) -> Arc<OneCameraConfig> {
        let mut settings = OneCameraSettings::h264_mvp(
            value.to_owned(),
            format!("rtsp://127.0.0.1:{port}/{value}"),
            None,
            640,
            360,
            PathBuf::from(format!("target/phase3-tests/{value}")),
        );
        settings.connect_timeout = Duration::from_millis(50);
        Arc::new(
            OneCameraConfig::validate(settings)
                .unwrap_or_else(|error| panic!("valid test configuration: {error}")),
        )
    }

    fn supervisor() -> (CameraSupervisor, super::CameraSupervisorHandle) {
        CameraSupervisor::new(
            NonZeroUsize::new(8).unwrap_or_else(|| panic!("nonzero command bound")),
            NonZeroUsize::new(10).unwrap_or_else(|| panic!("nonzero registry bound")),
            Duration::from_secs(2),
        )
    }

    #[tokio::test]
    async fn supervisor_controls_two_entries_independently_and_joins_shutdown() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let first = config("camera-01", 19_101);
        let second = config("camera-02", 19_102);
        handle
            .add(Arc::clone(&first))
            .await
            .unwrap_or_else(|error| panic!("add first: {error}"));
        handle
            .add(Arc::clone(&second))
            .await
            .unwrap_or_else(|error| panic!("add second: {error}"));
        let (first_start, second_start) = tokio::join!(
            handle.start(first.camera_id.clone()),
            handle.start(second.camera_id.clone())
        );
        first_start.unwrap_or_else(|error| panic!("start first: {error}"));
        second_start.unwrap_or_else(|error| panic!("start second: {error}"));

        handle
            .stop(first.camera_id.clone())
            .await
            .unwrap_or_else(|error| panic!("stop first: {error}"));
        let registry = handle.health_registry();
        assert_eq!(
            registry
                .snapshot(&first.camera_id)
                .map(|snapshot| snapshot.pipeline_state),
            Some(PipelineState::Stopped)
        );
        assert_eq!(
            registry
                .snapshot(&second.camera_id)
                .map(|snapshot| snapshot.pipeline_state),
            Some(PipelineState::Running)
        );

        handle
            .remove(first.camera_id.clone())
            .await
            .unwrap_or_else(|error| panic!("remove first: {error}"));
        assert!(registry.snapshot(&first.camera_id).is_none());
        assert!(registry.snapshot(&second.camera_id).is_some());
        handle
            .shutdown()
            .await
            .unwrap_or_else(|error| panic!("shutdown: {error}"));
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap_or_else(|_| panic!("supervisor must join promptly"))
            .unwrap_or_else(|error| panic!("supervisor task: {error}"));
    }

    #[tokio::test]
    async fn rejects_duplicates_unknown_operations_and_serializes_stop_remove() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let config = config("camera-03", 19_103);
        handle
            .add(Arc::clone(&config))
            .await
            .unwrap_or_else(|error| panic!("add: {error}"));
        assert_eq!(
            handle.add(Arc::clone(&config)).await,
            Err(SupervisorRequestError::Operation(
                SupervisorOperationError::DuplicateCamera
            ))
        );
        assert_eq!(
            handle.stop(camera_id("unknown-camera")).await,
            Err(SupervisorRequestError::Operation(
                SupervisorOperationError::UnknownCamera
            ))
        );
        handle
            .start(config.camera_id.clone())
            .await
            .unwrap_or_else(|error| panic!("start: {error}"));
        let (stop, remove) = tokio::join!(
            handle.stop(config.camera_id.clone()),
            handle.remove(config.camera_id.clone())
        );
        stop.unwrap_or_else(|error| panic!("concurrent stop: {error}"));
        remove.unwrap_or_else(|error| panic!("concurrent remove: {error}"));
        assert!(handle
            .health_registry()
            .snapshot(&config.camera_id)
            .is_none());
        handle
            .shutdown()
            .await
            .unwrap_or_else(|error| panic!("shutdown: {error}"));
        task.await
            .unwrap_or_else(|error| panic!("supervisor task: {error}"));
    }

    #[tokio::test]
    async fn shutdown_during_connection_backoff_joins_without_detached_pipeline() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let config = config("camera-04", 19_104);
        handle
            .add(Arc::clone(&config))
            .await
            .unwrap_or_else(|error| panic!("add: {error}"));
        handle
            .start(config.camera_id.clone())
            .await
            .unwrap_or_else(|error| panic!("start: {error}"));
        tokio::time::sleep(Duration::from_millis(75)).await;
        handle
            .shutdown()
            .await
            .unwrap_or_else(|error| panic!("shutdown: {error}"));
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap_or_else(|_| panic!("backoff shutdown must be cancellation-aware"))
            .unwrap_or_else(|error| panic!("supervisor task: {error}"));
    }

    #[test]
    fn health_registry_reads_and_removes_concurrently() {
        let registry = super::CameraHealthRegistry::new(
            NonZeroUsize::new(2).unwrap_or_else(|| panic!("nonzero registry bound")),
        );
        let camera_id = camera_id("camera-05");
        registry
            .insert_configured(camera_id.clone())
            .unwrap_or_else(|error| panic!("insert: {error}"));
        let reader = registry.clone();
        let reader_camera = camera_id.clone();
        let thread = std::thread::spawn(move || reader.snapshot(&reader_camera));
        registry.remove(&camera_id);
        let observed = thread
            .join()
            .unwrap_or_else(|_| panic!("registry reader must not panic"));
        assert!(observed.is_some() || registry.snapshot(&camera_id).is_none());
        assert!(registry.snapshot(&camera_id).is_none());
    }
}
