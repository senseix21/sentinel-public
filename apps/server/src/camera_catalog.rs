//! Bounded secret-file persistence. The supervisor is the single mutation owner.

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 1024 * 1024;

/// Write-only registration input. Deliberately has no `Debug` implementation.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraRegistration {
    pub camera_id: String,
    pub rtsp_endpoint: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub width: u32,
    pub height: u32,
    #[serde(default = "enabled")]
    pub detection_enabled: bool,
    pub live_publication_enabled: Option<bool>,
}
const fn enabled() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredCamera {
    pub camera: CameraRegistration,
    pub desired_running: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    cameras: BTreeMap<String, RegisteredCamera>,
}

pub struct CameraCatalog {
    path: PathBuf,
    document: Document,
    maximum: usize,
    healthy: bool,
    // The OS releases this lock even after forced exit. Never delete the lock
    // inode: another process could otherwise lock a replacement concurrently.
    _lock: File,
}

impl CameraCatalog {
    /// Opens and exclusively locks a secret catalog off the async executor.
    /// # Errors
    /// Rejects insecure permissions, concurrent owners, corrupt or oversized data.
    pub async fn open(path: PathBuf, maximum: usize) -> io::Result<Self> {
        tokio::task::spawn_blocking(move || {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            std::fs::create_dir_all(parent)?;
            let lock = secret_options()
                .create(true)
                .truncate(false)
                .open(path.with_extension("lock"))?;
            check_secret(&lock)?;
            fs2::FileExt::try_lock_exclusive(&lock)
                .map_err(|_| io::Error::other("camera catalog already has an owner"))?;
            let document = match File::open(&path) {
                Ok(file) => {
                    check_secret(&file)?;
                    let mut bytes = Vec::new();
                    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
                    if bytes.len() as u64 > MAX_BYTES {
                        return Err(io::Error::other("camera catalog exceeds size limit"));
                    }
                    serde_json::from_slice::<Document>(&bytes)
                        .map_err(|_| io::Error::other("invalid camera catalog"))?
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => Document {
                    version: 1,
                    cameras: BTreeMap::new(),
                },
                Err(error) => return Err(error),
            };
            if document.version != 1
                || document.cameras.len() > maximum
                || document
                    .cameras
                    .iter()
                    .any(|(id, entry)| id != &entry.camera.camera_id)
            {
                return Err(io::Error::other(
                    "unsupported or inconsistent camera catalog",
                ));
            }
            Ok(Self {
                path,
                document,
                maximum,
                healthy: true,
                _lock: lock,
            })
        })
        .await
        .map_err(|_| io::Error::other("camera catalog worker failed"))?
    }

    #[must_use]
    pub const fn available(&self) -> bool {
        self.healthy
    }

    #[must_use]
    pub fn registrations(&self) -> Vec<RegisteredCamera> {
        self.document.cameras.values().cloned().collect()
    }

    /// Atomically persists a new stopped registration.
    /// # Errors
    /// Returns a credential-free error; no caller input is included.
    pub async fn add(&mut self, camera: CameraRegistration) -> io::Result<()> {
        if self.document.cameras.contains_key(&camera.camera_id)
            || self.document.cameras.len() >= self.maximum
        {
            return Err(io::Error::other(
                "camera catalog duplicate or capacity reached",
            ));
        }
        let mut next = self.document.clone();
        next.cameras.insert(
            camera.camera_id.clone(),
            RegisteredCamera {
                camera,
                desired_running: false,
            },
        );
        self.commit(next).await
    }

    /// Records intent only for durable registrations; environment cameras stay ephemeral.
    /// # Errors
    /// Returns a credential-free persistence error before runtime mutation.
    pub async fn intent(&mut self, id: &str, running: Option<bool>) -> io::Result<()> {
        if !self.document.cameras.contains_key(id) {
            return Ok(());
        }
        let mut next = self.document.clone();
        if let Some(running) = running {
            if let Some(camera) = next.cameras.get_mut(id) {
                camera.desired_running = running;
            }
        } else {
            next.cameras.remove(id);
        }
        self.commit(next).await
    }

    async fn commit(&mut self, next: Document) -> io::Result<()> {
        if !self.healthy {
            return Err(io::Error::other(
                "camera catalog requires restart after write failure",
            ));
        }
        let bytes = serde_json::to_vec(&next)
            .map_err(|_| io::Error::other("camera catalog serialization failed"))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(io::Error::other("camera catalog exceeds size limit"));
        }
        let path = self.path.clone();
        let outcome = tokio::task::spawn_blocking(move || atomic_write(&path, &bytes))
            .await
            .map_err(|_| io::Error::other("camera catalog worker failed"))
            .and_then(std::convert::identity);
        if outcome.is_ok() {
            self.document = next;
        } else {
            self.healthy = false;
        }
        outcome
    }
}

fn secret_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}
fn check_secret(file: &File) -> io::Result<()> {
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("camera catalog must be a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file.metadata()?.permissions().mode() & 0o777 != 0o600 {
            return Err(io::Error::other("camera catalog requires mode 0600"));
        }
    }
    Ok(())
}
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = path.with_extension("pending");
    // A leftover pending file is never authoritative. The process lock makes
    // cleanup safe against other cooperating server instances.
    match std::fs::remove_file(&temp) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let result = (|| {
        let mut file = secret_options().create_new(true).open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        File::open(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?
        .sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera_supervisor::{CameraSupervisor, PipelineState};
    use std::num::NonZeroUsize;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };
    use std::time::Duration;
    use surveillance_config::settings::{OneCameraConfig, OneCameraSettings};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "sentinel-catalog-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            )))
        }
        fn path(&self) -> PathBuf {
            self.0.join("cameras.json")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn registration() -> CameraRegistration {
        CameraRegistration {
            camera_id: "front-door".into(),
            rtsp_endpoint: "rtsp://127.0.0.1:9/front".into(),
            username: Some("operator".into()),
            password: Some("secret-test-value".into()),
            width: 640,
            height: 360,
            detection_enabled: false,
            live_publication_enabled: Some(false),
        }
    }
    fn config(
        camera: &CameraRegistration,
    ) -> Result<Arc<OneCameraConfig>, Box<dyn std::error::Error>> {
        let mut settings = OneCameraSettings::h264_mvp(
            camera.camera_id.clone(),
            camera.rtsp_endpoint.clone(),
            None,
            camera.width,
            camera.height,
            PathBuf::from("target/catalog-tests"),
        );
        settings.detection_enabled = false;
        Ok(Arc::new(OneCameraConfig::validate(settings)?))
    }
    fn supervisor() -> (
        CameraSupervisor,
        crate::camera_supervisor::CameraSupervisorHandle,
    ) {
        CameraSupervisor::new(NonZeroUsize::MIN, NonZeroUsize::MIN, Duration::from_secs(5))
    }

    #[tokio::test]
    async fn durable_registration_intent_survives_shutdown_and_remove(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = Directory::new();
        let catalog = CameraCatalog::open(dir.path(), 1).await?;
        assert!(catalog.registrations().is_empty());
        assert!(
            CameraCatalog::open(dir.path(), 1).await.is_err(),
            "one process owner"
        );
        let (owner, handle) = supervisor();
        let task = tokio::spawn(owner.with_catalog(catalog).run());
        let camera = registration();
        let config = config(&camera)?;
        let id = config.camera_id.clone();
        handle.register(config.clone(), camera.clone()).await?;
        assert_eq!(
            handle
                .health_registry()
                .snapshot(&id)
                .map(|s| s.pipeline_state),
            Some(PipelineState::Configured)
        );
        assert!(handle.register(config.clone(), camera).await.is_err());
        handle.start(id.clone()).await?;
        handle.shutdown().await?;
        task.await?;
        let catalog = CameraCatalog::open(dir.path(), 1).await?;
        let saved = catalog.registrations();
        assert_eq!(saved.len(), 1);
        assert!(
            saved[0].desired_running,
            "shutdown must preserve running intent"
        );
        assert_eq!(
            saved[0].camera.password.as_deref(),
            Some("secret-test-value")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.path())?.permissions().mode() & 0o777,
                0o600
            );
        }
        let (owner, handle) = supervisor();
        let task = tokio::spawn(owner.with_catalog(catalog).run());
        handle.add(config.clone()).await?;
        handle.stop(id.clone()).await?;
        handle.shutdown().await?;
        task.await?;
        let catalog = CameraCatalog::open(dir.path(), 1).await?;
        assert!(!catalog.registrations()[0].desired_running);
        let (owner, handle) = supervisor();
        let task = tokio::spawn(owner.with_catalog(catalog).run());
        handle.add(config).await?;
        handle.remove(id).await?;
        handle.shutdown().await?;
        task.await?;
        assert!(CameraCatalog::open(dir.path(), 1)
            .await?
            .registrations()
            .is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn catalog_refuses_insecure_corrupt_and_over_capacity_data() -> io::Result<()> {
        let dir = Directory::new();
        let mut catalog = CameraCatalog::open(dir.path(), 1).await?;
        catalog.add(registration()).await?;
        let mut second = registration();
        second.camera_id = "back-door".into();
        assert!(catalog.add(second).await.is_err());
        drop(catalog);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o644))?;
            assert!(CameraCatalog::open(dir.path(), 1).await.is_err());
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::write(dir.path(), b"not-json-with-secret-test-value")?;
        let error = CameraCatalog::open(dir.path(), 1)
            .await
            .err()
            .ok_or_else(|| io::Error::other("expected rejection"))?;
        assert!(!error.to_string().contains("secret-test-value"));
        Ok(())
    }

    #[tokio::test]
    async fn write_failure_rejects_runtime_registration() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = Directory::new();
        let catalog = CameraCatalog::open(dir.path(), 1).await?;
        let moved = Directory(dir.0.with_extension("moved"));
        std::fs::rename(&dir.0, &moved.0)?;
        let (owner, handle) = supervisor();
        let task = tokio::spawn(owner.with_catalog(catalog).run());
        let camera = registration();
        assert!(handle.register(config(&camera)?, camera).await.is_err());
        assert!(handle.health_registry().snapshots().is_empty());
        assert!(!handle.catalog_available());
        handle.shutdown().await?;
        task.await?;
        Ok(())
    }
}
