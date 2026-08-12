//! Validated recording metadata and lifecycle contracts.

use std::{
    error::Error,
    fmt,
    num::{NonZeroU32, NonZeroU64},
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

use surveillance_common::types::{CameraId, RecordingId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingStatus {
    Open,
    Finalizing,
    Completed,
    Failed,
    Incomplete,
    DeletionPending,
    Deleted,
    Quarantined,
    Inconsistent,
}

impl RecordingStatus {
    #[must_use]
    pub fn may_transition_to(self, next: Self) -> bool {
        self == next
            || matches!(
                (self, next),
                (
                    Self::Open,
                    Self::Finalizing | Self::Failed | Self::Incomplete
                ) | (
                    Self::Finalizing,
                    Self::Completed | Self::Failed | Self::Incomplete
                ) | (
                    Self::Completed,
                    Self::DeletionPending | Self::Quarantined | Self::Inconsistent
                ) | (
                    Self::DeletionPending,
                    Self::Deleted | Self::Completed | Self::Inconsistent
                ) | (Self::Incomplete | Self::Inconsistent, Self::Quarantined)
            )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingContainer {
    FragmentedMp4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoCodec {
    H264,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataProvenance {
    Recorder,
    Reconciliation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingFailureCategory {
    Muxer,
    Storage,
    Validation,
    Shutdown,
    Reconciliation,
}

/// A safe relative locator for partial or durable reconciliation evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingEvidenceLocator(String);

impl RecordingEvidenceLocator {
    /// Validates a relative `.mp4` or `.partial.mp4` evidence path.
    ///
    /// # Errors
    /// Returns [`RecordingLocatorError`] for unsafe or non-UTF-8 paths.
    pub fn new(path: PathBuf) -> Result<Self, RecordingLocatorError> {
        if path.as_os_str().is_empty() {
            return Err(RecordingLocatorError::Empty);
        }
        if path.is_absolute() {
            return Err(RecordingLocatorError::Absolute);
        }
        if path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(RecordingLocatorError::UnsafeComponent);
        }
        if !path.to_string_lossy().bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.' | b'/')
        }) {
            return Err(RecordingLocatorError::InvalidCharacter);
        }
        if path.extension() != Some(std::ffi::OsStr::new("mp4")) {
            return Err(RecordingLocatorError::NotMp4Evidence);
        }
        path.into_os_string()
            .into_string()
            .map(Self)
            .map_err(|_| RecordingLocatorError::NonUtf8)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A database-safe locator interpreted only relative to the configured root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingLocator(String);

impl RecordingLocator {
    /// Validates a durable, relative `.mp4` path with no traversal components.
    ///
    /// # Errors
    /// Returns [`RecordingLocatorError`] for an absolute, partial, empty, or
    /// non-normal path.
    pub fn new(path: PathBuf) -> Result<Self, RecordingLocatorError> {
        if path.as_os_str().is_empty() {
            return Err(RecordingLocatorError::Empty);
        }
        if path.is_absolute() {
            return Err(RecordingLocatorError::Absolute);
        }
        if path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(RecordingLocatorError::UnsafeComponent);
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Err(RecordingLocatorError::InvalidFileName);
        };
        if path.extension() != Some(std::ffi::OsStr::new("mp4")) || name.ends_with(".partial.mp4") {
            return Err(RecordingLocatorError::NotDurableMp4);
        }
        path.into_os_string()
            .into_string()
            .map(Self)
            .map_err(|_| RecordingLocatorError::NonUtf8)
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingLocatorError {
    Empty,
    Absolute,
    NonUtf8,
    UnsafeComponent,
    InvalidCharacter,
    InvalidFileName,
    NotDurableMp4,
    NotMp4Evidence,
}

impl fmt::Display for RecordingLocatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid recording locator: {self:?}")
    }
}

impl Error for RecordingLocatorError {}

/// The single valid representation of playable completed metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedRecording {
    recording_id: RecordingId,
    camera_id: CameraId,
    started_at: SystemTime,
    ended_at: SystemTime,
    locator: RecordingLocator,
    container: RecordingContainer,
    codec: VideoCodec,
    width: NonZeroU32,
    height: NonZeroU32,
    size_bytes: NonZeroU64,
    provenance: MetadataProvenance,
}

impl CompletedRecording {
    /// Validates all evidence required for a completed/playable row.
    ///
    /// # Errors
    /// Returns [`CompletedRecordingError`] for a non-positive interval, zero
    /// dimensions, or an empty file.
    #[allow(
        clippy::too_many_arguments,
        reason = "the constructor audits every durable completion invariant"
    )]
    pub fn new(
        recording_id: RecordingId,
        camera_id: CameraId,
        started_at: SystemTime,
        ended_at: SystemTime,
        locator: RecordingLocator,
        container: RecordingContainer,
        codec: VideoCodec,
        width: u32,
        height: u32,
        size_bytes: u64,
        provenance: MetadataProvenance,
    ) -> Result<Self, CompletedRecordingError> {
        if ended_at <= started_at {
            return Err(CompletedRecordingError::NonPositiveInterval);
        }
        let width = NonZeroU32::new(width).ok_or(CompletedRecordingError::ZeroWidth)?;
        let height = NonZeroU32::new(height).ok_or(CompletedRecordingError::ZeroHeight)?;
        let size_bytes = NonZeroU64::new(size_bytes).ok_or(CompletedRecordingError::EmptyFile)?;
        Ok(Self {
            recording_id,
            camera_id,
            started_at,
            ended_at,
            locator,
            container,
            codec,
            width,
            height,
            size_bytes,
            provenance,
        })
    }

    #[must_use]
    pub fn recording_id(&self) -> &RecordingId {
        &self.recording_id
    }
    #[must_use]
    pub fn camera_id(&self) -> &CameraId {
        &self.camera_id
    }
    #[must_use]
    pub const fn started_at(&self) -> SystemTime {
        self.started_at
    }
    #[must_use]
    pub const fn ended_at(&self) -> SystemTime {
        self.ended_at
    }
    #[must_use]
    pub const fn status(&self) -> RecordingStatus {
        RecordingStatus::Completed
    }
    #[must_use]
    pub fn locator(&self) -> &RecordingLocator {
        &self.locator
    }
    #[must_use]
    pub const fn container(&self) -> RecordingContainer {
        self.container
    }
    #[must_use]
    pub const fn codec(&self) -> VideoCodec {
        self.codec
    }
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width.get()
    }
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height.get()
    }
    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes.get()
    }
    #[must_use]
    pub const fn provenance(&self) -> MetadataProvenance {
        self.provenance
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletedRecordingError {
    NonPositiveInterval,
    ZeroWidth,
    ZeroHeight,
    EmptyFile,
}

impl fmt::Display for CompletedRecordingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid completed recording: {self:?}")
    }
}

impl Error for CompletedRecordingError {}

/// Failed or incomplete evidence that must never be returned as playable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NonPlayableRecording {
    recording_id: RecordingId,
    camera_id: CameraId,
    started_at: SystemTime,
    ended_at: Option<SystemTime>,
    locator: RecordingEvidenceLocator,
    status: RecordingStatus,
    failure: RecordingFailureCategory,
    provenance: MetadataProvenance,
}

impl NonPlayableRecording {
    /// Validates failed/incomplete evidence.
    ///
    /// # Errors
    /// Returns [`NonPlayableRecordingError`] for a playable/terminal status or
    /// a non-positive optional interval.
    #[allow(
        clippy::too_many_arguments,
        reason = "the constructor audits persisted failure evidence"
    )]
    pub fn new(
        recording_id: RecordingId,
        camera_id: CameraId,
        started_at: SystemTime,
        ended_at: Option<SystemTime>,
        locator: RecordingEvidenceLocator,
        status: RecordingStatus,
        failure: RecordingFailureCategory,
        provenance: MetadataProvenance,
    ) -> Result<Self, NonPlayableRecordingError> {
        if !matches!(
            status,
            RecordingStatus::Failed | RecordingStatus::Incomplete | RecordingStatus::Inconsistent
        ) {
            return Err(NonPlayableRecordingError::InvalidStatus);
        }
        if ended_at.is_some_and(|end| end <= started_at) {
            return Err(NonPlayableRecordingError::NonPositiveInterval);
        }
        Ok(Self {
            recording_id,
            camera_id,
            started_at,
            ended_at,
            locator,
            status,
            failure,
            provenance,
        })
    }

    #[must_use]
    pub fn recording_id(&self) -> &RecordingId {
        &self.recording_id
    }
    #[must_use]
    pub fn camera_id(&self) -> &CameraId {
        &self.camera_id
    }
    #[must_use]
    pub const fn started_at(&self) -> SystemTime {
        self.started_at
    }
    #[must_use]
    pub const fn ended_at(&self) -> Option<SystemTime> {
        self.ended_at
    }
    #[must_use]
    pub fn locator(&self) -> &RecordingEvidenceLocator {
        &self.locator
    }
    #[must_use]
    pub const fn status(&self) -> RecordingStatus {
        self.status
    }
    #[must_use]
    pub const fn failure(&self) -> RecordingFailureCategory {
        self.failure
    }
    #[must_use]
    pub const fn provenance(&self) -> MetadataProvenance {
        self.provenance
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NonPlayableRecordingError {
    InvalidStatus,
    NonPositiveInterval,
}

impl fmt::Display for NonPlayableRecordingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid non-playable recording evidence: {self:?}"
        )
    }
}

impl Error for NonPlayableRecordingError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn id<T>(result: Result<T, surveillance_common::types::IdentifierError>) -> T {
        result.unwrap_or_else(|error| panic!("valid test identifier: {error}"))
    }

    #[test]
    fn completed_metadata_rejects_invalid_evidence() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let locator = RecordingLocator::new(PathBuf::from("camera-1/segment.mp4"))
            .unwrap_or_else(|error| panic!("valid locator: {error}"));
        let make = |end, width, height, size| {
            CompletedRecording::new(
                id(RecordingId::new("recording-1")),
                id(CameraId::new("camera-1")),
                start,
                end,
                locator.clone(),
                RecordingContainer::FragmentedMp4,
                VideoCodec::H264,
                width,
                height,
                size,
                MetadataProvenance::Recorder,
            )
        };
        assert_eq!(
            make(start, 1, 1, 1),
            Err(CompletedRecordingError::NonPositiveInterval)
        );
        assert_eq!(
            make(start + Duration::from_secs(1), 0, 1, 1),
            Err(CompletedRecordingError::ZeroWidth)
        );
        assert_eq!(
            make(start + Duration::from_secs(1), 1, 0, 1),
            Err(CompletedRecordingError::ZeroHeight)
        );
        assert_eq!(
            make(start + Duration::from_secs(1), 1, 1, 0),
            Err(CompletedRecordingError::EmptyFile)
        );
    }

    #[test]
    fn locator_never_accepts_partial_absolute_or_traversing_paths() {
        assert_eq!(
            RecordingLocator::new(PathBuf::from("camera/a.partial.mp4")),
            Err(RecordingLocatorError::NotDurableMp4)
        );
        assert_eq!(
            RecordingLocator::new(PathBuf::from("../a.mp4")),
            Err(RecordingLocatorError::UnsafeComponent)
        );
        assert_eq!(
            RecordingLocator::new(PathBuf::from("/camera/a.mp4")),
            Err(RecordingLocatorError::Absolute)
        );
    }

    #[test]
    fn lifecycle_transitions_are_idempotent_and_never_regress_completed() {
        assert!(RecordingStatus::Open.may_transition_to(RecordingStatus::Finalizing));
        assert!(RecordingStatus::Completed.may_transition_to(RecordingStatus::Completed));
        assert!(RecordingStatus::Completed.may_transition_to(RecordingStatus::DeletionPending));
        assert!(!RecordingStatus::Completed.may_transition_to(RecordingStatus::Open));
        assert!(!RecordingStatus::Deleted.may_transition_to(RecordingStatus::Completed));
    }
}
