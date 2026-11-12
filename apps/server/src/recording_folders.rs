//! Approved read-only recording roots and one owned bounded indexing worker.
use crate::{playback::PlaybackAssembler, reconciliation_owner::ReconciliationOwner};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use surveillance_common::types::CameraId;
use surveillance_database::{recording_sources::RecordingSource, PostgresRecordingRepository};
use surveillance_storage::{
    filesystem::RecordingLayout, reconciliation::ReconciliationLimit,
    segment_store::SegmentFileStore,
};
use tokio::sync::{mpsc, watch, Mutex, OwnedSemaphorePermit, Semaphore};

const MAX_FOLDERS: usize = 32;
const MAX_DISCOVERY_ENTRIES: usize = 5_000;

#[derive(Clone, Serialize)]
pub struct FolderChoice {
    pub id: String,
    pub label: String,
    #[serde(skip)]
    root: PathBuf,
}

pub struct RecordingFolders {
    approved: Vec<PathBuf>,
    primary: PathBuf,
    repository: PostgresRecordingRepository,
    choices: Mutex<Vec<FolderChoice>>,
    sender: mpsc::Sender<IndexJob>,
    permit: Arc<Semaphore>,
    ffprobe: PathBuf,
    ffmpeg: PathBuf,
    probe_timeout_ms: std::num::NonZeroU64,
    scan_limit: ReconciliationLimit,
    initialized: AtomicBool,
    rejected: AtomicU64,
    completed: AtomicU64,
    failed: AtomicU64,
}
struct IndexJob {
    source: RecordingSource,
    _permit: OwnedSemaphorePermit,
}
pub struct FolderIndexer {
    folders: Arc<RecordingFolders>,
    jobs: mpsc::Receiver<IndexJob>,
}

impl RecordingFolders {
    /// Resolves deployment-approved roots without admitting arbitrary browser paths.
    /// # Errors
    /// Rejects absent/insecure roots and an excessive root list.
    pub async fn new(
        roots: Vec<PathBuf>,
        primary: PathBuf,
        repository: PostgresRecordingRepository,
        ffprobe: PathBuf,
        ffmpeg: PathBuf,
        probe_timeout_ms: std::num::NonZeroU64,
        scan_limit: ReconciliationLimit,
    ) -> io::Result<(Arc<Self>, FolderIndexer)> {
        let mut approved = Vec::new();
        for root in roots {
            if approved.len() >= 8 {
                return Err(io::Error::other("too many recording browse roots"));
            }
            if tokio::fs::symlink_metadata(&root)
                .await?
                .file_type()
                .is_symlink()
            {
                return Err(io::Error::other("recording browse root is a symlink"));
            }
            approved.push(tokio::fs::canonicalize(root).await?);
        }
        let primary = tokio::fs::canonicalize(primary).await?;
        let (sender, jobs) = mpsc::channel(1);
        let folders = Arc::new(Self {
            approved,
            primary,
            repository,
            choices: Mutex::new(Vec::new()),
            sender,
            permit: Arc::new(Semaphore::new(1)),
            ffprobe,
            ffmpeg,
            probe_timeout_ms,
            scan_limit,
            initialized: AtomicBool::new(false),
            rejected: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        });
        Ok((Arc::clone(&folders), FolderIndexer { folders, jobs }))
    }

    /// Discovers supported camera/UTC layouts with explicit work and depth bounds.
    /// # Errors
    /// Fails closed on a work-limit overrun; never returns arbitrary files.
    pub async fn discover(&self) -> Result<Vec<FolderChoice>, &'static str> {
        let mut found = Vec::new();
        let mut budget = MAX_DISCOVERY_ENTRIES;
        for root in &self.approved {
            let mut pending = vec![(root.clone(), 0)];
            while let Some((path, depth)) = pending.pop() {
                if path == self.primary || path.starts_with(&self.primary) {
                    continue;
                }
                let entries = children(&path, &mut budget).await?;
                let recording_root = has_recording_layout(&path, &entries, &mut budget).await?;
                if recording_root {
                    let canonical = self.validate_root(&path).await?;
                    if !found
                        .iter()
                        .any(|item: &FolderChoice| item.root == canonical)
                    {
                        let relative = path.strip_prefix(root).map_err(|_| "folder_boundary")?;
                        let suffix = if relative.as_os_str().is_empty() {
                            ".".to_owned()
                        } else {
                            relative.to_string_lossy().into_owned()
                        };
                        let root_name = root
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("recordings");
                        found.push(FolderChoice {
                            id: String::new(),
                            label: format!("{root_name}/{suffix}"),
                            root: canonical,
                        });
                        if found.len() > MAX_FOLDERS {
                            return Err("folder_capacity");
                        }
                    }
                } else if depth < 2 {
                    for child in entries {
                        let name = child.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        if !matches!(
                            name,
                            "pgdata"
                                | "logs"
                                | "socket"
                                | "target"
                                | ".git"
                                | "node_modules"
                                | "onnx"
                        ) {
                            pending.push((child, depth + 1));
                        }
                    }
                }
            }
        }
        found.sort_by(|a, b| a.label.cmp(&b.label).then(a.root.cmp(&b.root)));
        for item in &mut found {
            item.id = format!(
                "folder-{:x}",
                Sha256::digest(item.root.as_os_str().as_encoded_bytes())
            );
        }
        self.choices.lock().await.clone_from(&found);
        Ok(found)
    }

    async fn validate_root(&self, root: &Path) -> Result<PathBuf, &'static str> {
        if tokio::fs::symlink_metadata(root)
            .await
            .map_err(|_| "folder_unavailable")?
            .file_type()
            .is_symlink()
        {
            return Err("folder_symlink");
        }
        let canonical = tokio::fs::canonicalize(root)
            .await
            .map_err(|_| "folder_unavailable")?;
        if canonical != root
            || !self.approved.iter().any(|base| canonical.starts_with(base))
            || canonical == self.primary
            || canonical.starts_with(&self.primary)
            || self.primary.starts_with(&canonical)
        {
            return Err("folder_boundary");
        }
        Ok(canonical)
    }

    /// Enqueues one index operation; another request is rejected while it runs.
    /// # Errors
    /// Returns fixed categories, never absolute paths or raw database errors.
    pub async fn index(&self, choice: &str) -> Result<String, &'static str> {
        if !self.initialized.load(Ordering::Acquire) {
            return Err("index_worker_starting");
        }
        let permit = Arc::clone(&self.permit).try_acquire_owned().map_err(|_| {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            "index_busy"
        })?;
        let folder = self
            .choices
            .lock()
            .await
            .iter()
            .find(|entry| entry.id == choice)
            .cloned()
            .ok_or("folder_not_found")?;
        self.validate_root(&folder.root).await?;
        self.repository
            .migrate()
            .await
            .map_err(|_| "recording_index_unavailable")?;
        let source_id = self
            .repository
            .register_recording_source(&folder.root, &folder.label)
            .await
            .map_err(|error| {
                if error.retryable() {
                    "recording_index_unavailable"
                } else {
                    "source_registration_failed"
                }
            })?;
        self.repository
            .source_index_state(&source_id, "indexing")
            .await
            .map_err(|_| "recording_index_unavailable")?;
        let source = RecordingSource {
            source_id: source_id.clone(),
            root: folder.root,
            label: folder.label,
            index_state: "indexing".to_owned(),
        };
        self.sender
            .try_send(IndexJob {
                source,
                _permit: permit,
            })
            .map_err(|_| "index_worker_unavailable")?;
        Ok(source_id)
    }

    /// Loads a durable source and revalidates its filesystem boundary.
    /// # Errors
    /// Returns fixed categories for missing, escaped or unavailable roots.
    pub async fn resolve(
        &self,
        id: &str,
    ) -> Result<(PostgresRecordingRepository, PlaybackAssembler), &'static str> {
        let source = self
            .repository
            .recording_sources()
            .await
            .map_err(|_| "recording_index_unavailable")?
            .into_iter()
            .find(|source| source.source_id == id)
            .ok_or("source_not_found")?;
        let root = self.validate_root(&source.root).await?;
        let repository = self
            .repository
            .for_archive(id)
            .map_err(|_| "source_not_found")?;
        Ok((
            repository,
            PlaybackAssembler::new(root, self.ffmpeg.clone(), Duration::from_secs(5)),
        ))
    }

    /// Resolves a source only when it contains this authorized camera's footage.
    /// # Errors
    /// Missing source and missing camera coverage have the same fixed response.
    pub async fn resolve_for_camera(
        &self,
        id: &str,
        camera: &CameraId,
    ) -> Result<(PostgresRecordingRepository, PlaybackAssembler), &'static str> {
        let repository = self
            .repository
            .for_archive(id)
            .map_err(|_| "source_not_found")?;
        let cameras = repository
            .source_cameras(Some(vec![camera.as_str().to_owned()]))
            .await
            .map_err(|_| "recording_index_unavailable")?;
        if cameras.is_empty() {
            return Err("source_not_found");
        }
        self.resolve(id).await
    }

    /// Returns API-safe source labels and scoped indexed coverage.
    /// # Errors
    /// Returns fixed errors without exposing source roots.
    pub async fn list(
        &self,
        scope: Option<Vec<String>>,
    ) -> Result<serde_json::Value, &'static str> {
        let mut sources = Vec::new();
        for source in self
            .repository
            .recording_sources()
            .await
            .map_err(|_| "recording_index_unavailable")?
        {
            let repository = self
                .repository
                .for_archive(&source.source_id)
                .map_err(|_| "source_not_found")?;
            let cameras = repository
                .source_cameras(scope.clone())
                .await
                .map_err(|_| "recording_index_unavailable")?;
            if scope.is_some() && cameras.is_empty() {
                continue;
            }
            let cameras: Vec<_> = cameras.into_iter().map(|camera| serde_json::json!({"camera_id": camera.camera_id, "first_recorded_at":camera.first_recorded_at.to_rfc3339(), "last_recorded_at":camera.last_recorded_at.to_rfc3339(), "recordings":camera.recordings})).collect();
            let state = if self.validate_root(&source.root).await.is_err() {
                "unavailable"
            } else {
                &source.index_state
            };
            sources.push(serde_json::json!({"id":source.source_id, "label":source.label, "state":state, "cameras":cameras}));
        }
        Ok(serde_json::json!(sources))
    }

    #[must_use]
    pub fn metrics(&self) -> String {
        format!("# TYPE surveillance_archive_index_busy gauge\nsurveillance_archive_index_busy {}\n# TYPE surveillance_archive_index_rejected_total counter\nsurveillance_archive_index_rejected_total {}\n# TYPE surveillance_archive_index_completed_total counter\nsurveillance_archive_index_completed_total {}\n# TYPE surveillance_archive_index_failed_total counter\nsurveillance_archive_index_failed_total {}\n", u8::from(self.permit.available_permits() == 0), self.rejected.load(Ordering::Relaxed), self.completed.load(Ordering::Relaxed), self.failed.load(Ordering::Relaxed))
    }
}

impl FolderIndexer {
    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        // A previous owner may have exited mid-index. Its durable partial index
        // remains usable, but never masquerades as a completed scan.
        let mut retry_delay = Duration::from_millis(250);
        loop {
            if *shutdown.borrow() {
                return;
            }
            let recovered = match self.folders.repository.migrate().await {
                Ok(()) => self.folders.repository.interrupt_archive_indexes().await,
                Err(error) => Err(error),
            };
            if recovered.is_ok() {
                break;
            }
            let jitter = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| u64::from(elapsed.subsec_nanos()) % 251);
            let delay = retry_delay
                .saturating_add(Duration::from_millis(jitter))
                .min(Duration::from_secs(30));
            retry_delay = retry_delay.saturating_mul(2).min(Duration::from_secs(30));
            tracing::warn!(?delay, "archive index recovery deferred; will retry");
            tokio::select! {
                changed = shutdown.changed() => { let _ = changed; return; }
                () = tokio::time::sleep(delay) => {}
            }
        }
        self.folders.initialized.store(true, Ordering::Release);
        loop {
            let job = tokio::select! {
                changed = shutdown.changed() => { let _ = changed; return; }
                job = self.jobs.recv() => match job { Some(job) => job, None => return },
            };
            let result = self.index_job(&job, shutdown.clone()).await;
            let state = match result {
                Ok(true) => "ready",
                Ok(false) => "partial",
                Err("cancelled") => "interrupted",
                Err(_) => "failed",
            };
            if state == "ready" || state == "partial" {
                self.folders.completed.fetch_add(1, Ordering::Relaxed);
            } else {
                self.folders.failed.fetch_add(1, Ordering::Relaxed);
            }
            if let Err(error) = self
                .folders
                .repository
                .source_index_state(&job.source.source_id, state)
                .await
            {
                tracing::warn!(%error, "archive index status persistence failed");
            }
            tracing::info!(source_id = %job.source.source_id, state, "archive indexing finished");
            drop(job);
            if *shutdown.borrow() {
                return;
            }
        }
    }
    async fn index_job(
        &self,
        job: &IndexJob,
        shutdown: watch::Receiver<bool>,
    ) -> Result<bool, &'static str> {
        let root = self.folders.validate_root(&job.source.root).await?;
        let layout = RecordingLayout::new(root).map_err(|_| "folder_boundary")?;
        let repository = self
            .folders
            .repository
            .for_archive(&job.source.source_id)
            .map_err(|_| "source_not_found")?;
        let mut owner = ReconciliationOwner::new(
            repository.clone(),
            layout.clone(),
            SegmentFileStore::new(layout, self.folders.ffprobe.clone())
                .with_probe_timeout(self.folders.probe_timeout_ms),
            self.folders.scan_limit,
            256,
            Duration::from_secs(120),
            Duration::from_secs(120),
        )
        .with_cancellation(shutdown);
        let pass = owner.run_pass().await?;
        Ok(repository
            .source_unplayable_count()
            .await
            .map_err(|_| "database_status")?
            == 0
            && pass.inconsistent == 0
            && pass.malformed == 0
            && pass.incomplete == 0
            && pass.skipped_active == 0)
    }
}

async fn has_recording_layout(
    root: &Path,
    cameras: &[PathBuf],
    budget: &mut usize,
) -> Result<bool, &'static str> {
    let layout = RecordingLayout::new(root.to_path_buf()).map_err(|_| "folder_boundary")?;
    for camera in cameras {
        let Some(name) = camera.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if CameraId::new(name).is_err()
            || matches!(
                name,
                "pgdata" | "logs" | "socket" | "target" | "node_modules" | "onnx"
            )
        {
            continue;
        }
        for year in children(camera, budget).await? {
            if !numeric_component(&year, 4, 1, 9999) {
                continue;
            }
            for month in children(&year, budget).await? {
                if !numeric_component(&month, 2, 1, 12) {
                    continue;
                }
                for day in children(&month, budget).await? {
                    if !numeric_component(&day, 2, 1, 31) {
                        continue;
                    }
                    let mut entries = tokio::fs::read_dir(&day)
                        .await
                        .map_err(|_| "folder_unavailable")?;
                    while let Some(entry) = entries
                        .next_entry()
                        .await
                        .map_err(|_| "folder_unavailable")?
                    {
                        if *budget == 0 {
                            return Err("folder_scan_limit");
                        }
                        *budget -= 1;
                        if entry
                            .file_type()
                            .await
                            .map_err(|_| "folder_unavailable")?
                            .is_file()
                            && surveillance_storage::reconciliation::ReconciliationIdentity::parse(
                                &layout,
                                &entry.path(),
                            )
                            .is_ok()
                        {
                            return Ok(true);
                        }
                    }
                }
            }
        }
    }
    Ok(false)
}

fn numeric_component(path: &Path, width: usize, minimum: u16, maximum: u16) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.len() == width
                && name.bytes().all(|byte| byte.is_ascii_digit())
                && name
                    .parse::<u16>()
                    .is_ok_and(|value| (minimum..=maximum).contains(&value))
        })
}

async fn children(path: &Path, budget: &mut usize) -> Result<Vec<PathBuf>, &'static str> {
    let mut result = Vec::new();
    let mut entries = tokio::fs::read_dir(path)
        .await
        .map_err(|_| "folder_unavailable")?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|_| "folder_unavailable")?
    {
        if *budget == 0 {
            return Err("folder_scan_limit");
        }
        *budget -= 1;
        if entry
            .file_type()
            .await
            .map_err(|_| "folder_unavailable")?
            .is_dir()
        {
            result.push(entry.path());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{num::NonZeroU32, sync::atomic::AtomicU64};
    use surveillance_database::DatabaseConfig;
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "sentinel-folders-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn repository() -> Result<PostgresRecordingRepository, Box<dyn std::error::Error>> {
        let config = DatabaseConfig::new(
            "postgres://127.0.0.1:9/test",
            NonZeroU32::MIN,
            Duration::from_millis(10),
        )?;
        Ok(PostgresRecordingRepository::connect_lazy(&config))
    }
    #[tokio::test]
    async fn discovery_lists_only_supported_roots_and_uses_stable_opaque_choices(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let fixture = Fixture::new();
        std::fs::create_dir_all(fixture.0.join("primary"))?;
        std::fs::create_dir_all(fixture.0.join("saved/camera-01/2026/10/29"))?;
        std::fs::write(
            fixture
                .0
                .join("saved/camera-01/2026/10/29/17-46-23--fixture-one.mp4"),
            b"fixture",
        )?;
        std::fs::create_dir_all(fixture.0.join("other/logs"))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink("/", fixture.0.join("outside"))?;
        let (folders, _owner) = RecordingFolders::new(
            vec![fixture.0.clone()],
            fixture.0.join("primary"),
            repository()?,
            "ffprobe".into(),
            "ffmpeg".into(),
            std::num::NonZeroU64::MIN,
            ReconciliationLimit::new(100_000)?,
        )
        .await?;
        folders.initialized.store(true, Ordering::Release);
        let choices = folders.discover().await.map_err(io::Error::other)?;
        assert_eq!(choices.len(), 1);
        assert!(choices[0].label.ends_with("/saved"));
        assert!(choices[0].id.starts_with("folder-"));
        let json = serde_json::to_string(&choices)?;
        assert!(!json.contains(&fixture.0.to_string_lossy().to_string()));
        assert!(!json.contains("outside"));
        std::fs::create_dir_all(fixture.0.join("earlier/camera-02/2025/01/01"))?;
        std::fs::write(
            fixture
                .0
                .join("earlier/camera-02/2025/01/01/00-00-00--fixture-two.mp4"),
            b"fixture",
        )?;
        let refreshed = folders.discover().await.map_err(io::Error::other)?;
        assert!(refreshed
            .iter()
            .any(|choice| choice.id == choices[0].id && choice.root == choices[0].root));
        assert!(folders.validate_root(Path::new("/etc")).await.is_err());
        assert!(folders
            .validate_root(&fixture.0.join("primary"))
            .await
            .is_err());
        assert_eq!(
            folders.index("../../etc").await.err(),
            Some("folder_not_found")
        );
        let permit = Arc::clone(&folders.permit).try_acquire_owned()?;
        assert_eq!(
            folders.index(&choices[0].id).await.err(),
            Some("index_busy")
        );
        assert!(folders
            .metrics()
            .contains("surveillance_archive_index_rejected_total 1"));
        drop(permit);
        let mut budget = 0;
        assert_eq!(
            children(&fixture.0, &mut budget).await.err(),
            Some("folder_scan_limit")
        );
        Ok(())
    }
}
