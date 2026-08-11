//! Durable partial-file persistence, atomic publication, and media probing.

use std::{error::Error, fmt, path::PathBuf, process::ExitStatus, time::Duration};

use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
    process::Command,
};

use crate::filesystem::{RecordingLayout, SegmentPaths};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredVideoCodec {
    H264,
}

/// Metadata observed from the durable file rather than inferred from config.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSegment {
    path: PathBuf,
    size_bytes: u64,
    codec: StoredVideoCodec,
    width: u32,
    height: u32,
    duration: Duration,
}

impl StoredSegment {
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn codec(&self) -> StoredVideoCodec {
        self.codec
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }
}

/// Owns the sole configured recording root and the probe executable boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentFileStore {
    layout: RecordingLayout,
    ffprobe: PathBuf,
}

/// Singular owner of one exclusively created active partial file.
pub struct ActiveSegmentFile {
    paths: SegmentPaths,
    file: File,
    bytes_written: u64,
    max_bytes: u64,
}

impl ActiveSegmentFile {
    /// Appends one complete muxer emission to the active partial.
    ///
    /// # Errors
    ///
    /// Returns a classified write error or rejects an empty chunk.
    pub async fn append(&mut self, bytes: &[u8]) -> Result<(), SegmentStoreError> {
        if bytes.is_empty() {
            return Err(SegmentStoreError::new(
                SegmentStoreStage::ValidateBytes,
                SegmentStoreErrorKind::EmptySegment,
            ));
        }
        let chunk_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let next_bytes = self.bytes_written.checked_add(chunk_bytes).ok_or_else(|| {
            SegmentStoreError::new(
                SegmentStoreStage::ValidateBytes,
                SegmentStoreErrorKind::SegmentTooLarge,
            )
        })?;
        if next_bytes > self.max_bytes {
            return Err(SegmentStoreError::new(
                SegmentStoreStage::ValidateBytes,
                SegmentStoreErrorKind::SegmentTooLarge,
            ));
        }
        self.file
            .write_all(bytes)
            .await
            .map_err(|error| io_error(SegmentStoreStage::WritePartial, error))?;
        self.file
            .sync_data()
            .await
            .map_err(|error| io_error(SegmentStoreStage::SyncPartial, error))?;
        self.bytes_written = next_bytes;
        Ok(())
    }

    #[must_use]
    pub const fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    #[must_use]
    pub fn partial_path(&self) -> &std::path::Path {
        self.paths.partial()
    }
}

impl SegmentFileStore {
    #[must_use]
    pub const fn new(layout: RecordingLayout, ffprobe: PathBuf) -> Self {
        Self { layout, ffprobe }
    }

    /// Writes a new partial file, flushes and syncs it, atomically renames it,
    /// syncs the parent directory, then probes the durable file.
    ///
    /// # Errors
    ///
    /// Returns a stage-classified error. Existing partial or durable files are
    /// never overwritten.
    pub async fn persist(
        &self,
        paths: &SegmentPaths,
        bytes: &[u8],
    ) -> Result<StoredSegment, SegmentStoreError> {
        let mut active = self.begin(paths, bytes.len()).await?;
        active.append(bytes).await?;
        self.finalize(active).await
    }

    /// Exclusively creates an active partial beneath the configured root.
    ///
    /// # Errors
    ///
    /// Returns a classified path, directory, or exclusive-create error.
    pub async fn begin(
        &self,
        paths: &SegmentPaths,
        max_bytes: usize,
    ) -> Result<ActiveSegmentFile, SegmentStoreError> {
        if !self.layout.owns(paths) {
            return Err(SegmentStoreError::new(
                SegmentStoreStage::ValidatePath,
                SegmentStoreErrorKind::OutsideConfiguredRoot,
            ));
        }
        let max_bytes = u64::try_from(max_bytes)
            .ok()
            .filter(|value| *value != 0)
            .ok_or_else(|| {
                SegmentStoreError::new(
                    SegmentStoreStage::ValidateBytes,
                    SegmentStoreErrorKind::EmptySegment,
                )
            })?;
        if fs::try_exists(paths.durable())
            .await
            .map_err(|error| io_error(SegmentStoreStage::ValidatePath, error))?
        {
            return Err(SegmentStoreError::new(
                SegmentStoreStage::ValidatePath,
                SegmentStoreErrorKind::DurableAlreadyExists,
            ));
        }
        let parent = paths.partial().parent().ok_or_else(|| {
            SegmentStoreError::new(
                SegmentStoreStage::ValidatePath,
                SegmentStoreErrorKind::MissingParent,
            )
        })?;
        fs::create_dir_all(parent)
            .await
            .map_err(|error| io_error(SegmentStoreStage::CreateDirectories, error))?;
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(paths.partial())
            .await
            .map_err(|error| io_error(SegmentStoreStage::CreatePartial, error))?;
        Ok(ActiveSegmentFile {
            paths: paths.clone(),
            file,
            bytes_written: 0,
            max_bytes,
        })
    }

    /// Syncs and atomically publishes a nonempty active partial, then probes it.
    ///
    /// # Errors
    ///
    /// Returns a classified validation, durability, rename, or probe error.
    pub async fn finalize(
        &self,
        mut active: ActiveSegmentFile,
    ) -> Result<StoredSegment, SegmentStoreError> {
        if !self.layout.owns(&active.paths) {
            return Err(SegmentStoreError::new(
                SegmentStoreStage::ValidatePath,
                SegmentStoreErrorKind::OutsideConfiguredRoot,
            ));
        }
        if active.bytes_written == 0 {
            return Err(SegmentStoreError::new(
                SegmentStoreStage::ValidateBytes,
                SegmentStoreErrorKind::EmptySegment,
            ));
        }
        active
            .file
            .flush()
            .await
            .map_err(|error| io_error(SegmentStoreStage::FlushPartial, error))?;
        active
            .file
            .sync_all()
            .await
            .map_err(|error| io_error(SegmentStoreStage::SyncPartial, error))?;
        drop(active.file);
        fs::rename(active.paths.partial(), active.paths.durable())
            .await
            .map_err(|error| io_error(SegmentStoreStage::RenameDurable, error))?;
        let parent = active.paths.durable().parent().ok_or_else(|| {
            SegmentStoreError::new(
                SegmentStoreStage::ValidatePath,
                SegmentStoreErrorKind::MissingParent,
            )
        })?;
        sync_directory(parent.to_path_buf()).await?;
        self.probe(active.paths.durable().to_path_buf()).await
    }

    async fn probe(&self, path: PathBuf) -> Result<StoredSegment, SegmentStoreError> {
        let output = Command::new(&self.ffprobe)
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,width,height:format=duration",
                "-of",
                "default=noprint_wrappers=1",
            ])
            .arg(&path)
            .output()
            .await
            .map_err(|error| io_error(SegmentStoreStage::Probe, error))?;
        if !output.status.success() {
            return Err(SegmentStoreError::new(
                SegmentStoreStage::Probe,
                SegmentStoreErrorKind::ProbeFailed(output.status),
            ));
        }
        let stdout = String::from_utf8(output.stdout).map_err(|_| {
            SegmentStoreError::new(
                SegmentStoreStage::Probe,
                SegmentStoreErrorKind::InvalidProbeOutput,
            )
        })?;
        let codec = parse_value(&stdout, "codec_name")?;
        if codec != "h264" {
            return Err(SegmentStoreError::new(
                SegmentStoreStage::Probe,
                SegmentStoreErrorKind::UnsupportedCodec,
            ));
        }
        let width = parse_u32(&stdout, "width")?;
        let height = parse_u32(&stdout, "height")?;
        if width == 0 || height == 0 {
            return Err(invalid_probe_output());
        }
        let duration = parse_duration(parse_value(&stdout, "duration")?)?;
        let size_bytes = fs::metadata(&path)
            .await
            .map_err(|error| io_error(SegmentStoreStage::Probe, error))?
            .len();
        if size_bytes == 0 {
            return Err(invalid_probe_output());
        }
        Ok(StoredSegment {
            path,
            size_bytes,
            codec: StoredVideoCodec::H264,
            width,
            height,
            duration,
        })
    }
}

async fn sync_directory(path: PathBuf) -> Result<(), SegmentStoreError> {
    tokio::task::spawn_blocking(move || std::fs::File::open(path)?.sync_all())
        .await
        .map_err(|error| {
            SegmentStoreError::new(
                SegmentStoreStage::SyncDirectory,
                SegmentStoreErrorKind::Join(error),
            )
        })?
        .map_err(|error| io_error(SegmentStoreStage::SyncDirectory, error))
}

fn parse_value<'a>(output: &'a str, key: &str) -> Result<&'a str, SegmentStoreError> {
    output
        .lines()
        .find_map(|line| {
            line.strip_prefix(key)
                .and_then(|value| value.strip_prefix('='))
        })
        .ok_or_else(invalid_probe_output)
}

fn parse_u32(output: &str, key: &str) -> Result<u32, SegmentStoreError> {
    parse_value(output, key)?
        .parse()
        .map_err(|_| invalid_probe_output())
}

fn parse_duration(value: &str) -> Result<Duration, SegmentStoreError> {
    let (seconds, fraction) = value.split_once('.').unwrap_or((value, ""));
    let seconds = seconds.parse::<u64>().map_err(|_| invalid_probe_output())?;
    let fraction = fraction.trim_end_matches('0');
    if fraction.len() > 9 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_probe_output());
    }
    let fraction_value = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u32>()
            .map_err(|_| invalid_probe_output())?
    };
    let scale = 10_u32.pow(u32::try_from(9_usize.saturating_sub(fraction.len())).unwrap_or(0));
    let duration = Duration::new(seconds, fraction_value.saturating_mul(scale));
    if duration.is_zero() {
        return Err(invalid_probe_output());
    }
    Ok(duration)
}

fn invalid_probe_output() -> SegmentStoreError {
    SegmentStoreError::new(
        SegmentStoreStage::Probe,
        SegmentStoreErrorKind::InvalidProbeOutput,
    )
}

fn io_error(stage: SegmentStoreStage, error: std::io::Error) -> SegmentStoreError {
    SegmentStoreError::new(stage, SegmentStoreErrorKind::Io(error))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentStoreStage {
    ValidatePath,
    ValidateBytes,
    CreateDirectories,
    CreatePartial,
    WritePartial,
    FlushPartial,
    SyncPartial,
    RenameDurable,
    SyncDirectory,
    Probe,
}

#[derive(Debug)]
pub enum SegmentStoreErrorKind {
    OutsideConfiguredRoot,
    EmptySegment,
    SegmentTooLarge,
    DurableAlreadyExists,
    MissingParent,
    UnsupportedCodec,
    InvalidProbeOutput,
    ProbeFailed(ExitStatus),
    Io(std::io::Error),
    Join(tokio::task::JoinError),
}

#[derive(Debug)]
pub struct SegmentStoreError {
    stage: SegmentStoreStage,
    kind: SegmentStoreErrorKind,
}

impl SegmentStoreError {
    const fn new(stage: SegmentStoreStage, kind: SegmentStoreErrorKind) -> Self {
        Self { stage, kind }
    }

    #[must_use]
    pub const fn stage(&self) -> SegmentStoreStage {
        self.stage
    }

    #[must_use]
    pub const fn kind(&self) -> &SegmentStoreErrorKind {
        &self.kind
    }
}

impl fmt::Display for SegmentStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "segment storage failed during {:?}: {:?}",
            self.stage, self.kind
        )
    }
}

impl Error for SegmentStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            SegmentStoreErrorKind::Io(error) => Some(error),
            SegmentStoreErrorKind::Join(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_duration, SegmentFileStore, SegmentStoreErrorKind, SegmentStoreStage};
    use crate::filesystem::{RecordingLayout, SegmentPaths};
    use std::{
        path::PathBuf,
        time::{Duration, SystemTime},
    };

    #[test]
    fn parses_probe_decimal_duration_without_float_rounding() {
        let duration = parse_duration("30.033333")
            .unwrap_or_else(|error| panic!("valid probe duration: {error}"));
        assert_eq!(duration, Duration::new(30, 33_333_000));
        assert!(parse_duration("0.000000").is_err());
        assert!(parse_duration("N/A").is_err());
    }

    #[tokio::test]
    async fn rejects_paths_outside_the_configured_root_before_io() {
        let layout = RecordingLayout::new(PathBuf::from("recordings"))
            .unwrap_or_else(|error| panic!("valid test layout: {error}"));
        let store = SegmentFileStore::new(layout, PathBuf::from("ffprobe"));
        let paths = SegmentPaths::from_partial(PathBuf::from("elsewhere/test.partial.mp4"))
            .unwrap_or_else(|error| panic!("valid segment suffix: {error}"));
        let error = store
            .persist(&paths, b"bytes")
            .await
            .err()
            .unwrap_or_else(|| panic!("outside path must fail"));
        assert_eq!(error.stage(), SegmentStoreStage::ValidatePath);
        assert!(matches!(
            error.kind(),
            SegmentStoreErrorKind::OutsideConfiguredRoot
        ));
    }

    #[tokio::test]
    async fn dropped_active_writer_leaves_real_partial_bytes_for_reconciliation() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let root = std::env::temp_dir().join(format!(
            "surveillance-active-partial-{}-{nonce}",
            std::process::id()
        ));
        let layout = RecordingLayout::new(root.clone())
            .unwrap_or_else(|error| panic!("valid root: {error}"));
        let store = SegmentFileStore::new(layout, PathBuf::from("ffprobe"));
        let paths = SegmentPaths::from_partial(root.join("camera/test.partial.mp4"))
            .unwrap_or_else(|error| panic!("valid paths: {error}"));
        let mut active = store
            .begin(&paths, 1_024)
            .await
            .unwrap_or_else(|error| panic!("begin active file: {error}"));
        active
            .append(b"real-muxer-emission")
            .await
            .unwrap_or_else(|error| panic!("append bytes: {error}"));
        assert_eq!(active.bytes_written(), 19);
        drop(active);

        let bytes = tokio::fs::read(paths.partial())
            .await
            .unwrap_or_else(|error| panic!("partial remains readable: {error}"));
        assert_eq!(bytes, b"real-muxer-emission");
        assert!(!tokio::fs::try_exists(paths.durable())
            .await
            .unwrap_or_else(|error| panic!("query durable path: {error}")));
        tokio::fs::remove_dir_all(root)
            .await
            .unwrap_or_else(|error| panic!("cleanup fixture: {error}"));
    }
}
