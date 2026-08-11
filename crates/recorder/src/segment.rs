//! Recording segment creation and finalization.
//!
//! This module models lifecycle ownership but performs no file I/O. The muxer
//! and storage adapter must prove durable finalization before constructing the
//! evidence consumed by [`FinalizingSegment::complete`].

use std::{
    error::Error,
    fmt,
    num::{NonZeroU32, NonZeroU64},
    path::{Path, PathBuf},
    time::SystemTime,
};
use surveillance_storage::filesystem::SegmentPaths;

/// Validated encoded-video dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoDimensions {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl VideoDimensions {
    /// Validates nonzero pixel dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`VideoDimensionsError`] if either dimension is zero.
    pub fn new(width: u32, height: u32) -> Result<Self, VideoDimensionsError> {
        let Some(width) = NonZeroU32::new(width) else {
            return Err(VideoDimensionsError::ZeroWidth);
        };
        let Some(height) = NonZeroU32::new(height) else {
            return Err(VideoDimensionsError::ZeroHeight);
        };
        Ok(Self { width, height })
    }

    /// Returns the encoded width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width.get()
    }

    /// Returns the encoded height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height.get()
    }
}

/// Invalid encoded-video dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoDimensionsError {
    ZeroWidth,
    ZeroHeight,
}

impl fmt::Display for VideoDimensionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroWidth => formatter.write_str("video width must be nonzero"),
            Self::ZeroHeight => formatter.write_str("video height must be nonzero"),
        }
    }
}

impl Error for VideoDimensionsError {}

/// The only recording format qualified for the first MVP slice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingFormat {
    /// H.264 video in fragmented MP4.
    FragmentedMp4H264,
}

/// A segment whose muxer owns an active partial file.
#[derive(Debug, Eq, PartialEq)]
pub struct OpenSegment {
    paths: SegmentPaths,
    started_at: SystemTime,
    format: RecordingFormat,
}

impl OpenSegment {
    /// Begins the lifecycle for a newly opened partial file.
    #[must_use]
    pub const fn new(paths: SegmentPaths, started_at: SystemTime, format: RecordingFormat) -> Self {
        Self {
            paths,
            started_at,
            format,
        }
    }

    /// Moves an open segment into finalization.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentTransitionError::EndNotAfterStart`] if `ended_at` is
    /// not later than the segment start.
    pub fn begin_finalization(
        self,
        ended_at: SystemTime,
    ) -> Result<FinalizingSegment, SegmentTransitionError> {
        if ended_at <= self.started_at {
            return Err(SegmentTransitionError::EndNotAfterStart);
        }

        Ok(FinalizingSegment {
            paths: self.paths,
            started_at: self.started_at,
            ended_at,
            format: self.format,
        })
    }

    /// Marks an open segment failed and retains its partial path for recovery.
    #[must_use]
    pub fn fail(self, failure: SegmentFailure) -> FailedSegment {
        FailedSegment {
            partial_path: self.paths.into_parts().0,
            started_at: self.started_at,
            ended_at: None,
            stage: FailureStage::Open,
            failure,
        }
    }
}

/// A segment whose muxer has stopped accepting media and is being finalized.
#[derive(Debug, Eq, PartialEq)]
pub struct FinalizingSegment {
    paths: SegmentPaths,
    started_at: SystemTime,
    ended_at: SystemTime,
    format: RecordingFormat,
}

impl FinalizingSegment {
    /// Completes the segment only from durable-file validation evidence.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentTransitionError::UnexpectedDurablePath`] if validation
    /// refers to a path other than the atomic-rename target.
    pub fn complete(
        self,
        evidence: DurableFileEvidence,
    ) -> Result<CompletedSegment, SegmentTransitionError> {
        if evidence.path != self.paths.durable() {
            return Err(SegmentTransitionError::UnexpectedDurablePath);
        }

        Ok(CompletedSegment {
            path: evidence.path,
            started_at: self.started_at,
            ended_at: self.ended_at,
            format: self.format,
            dimensions: evidence.dimensions,
            size_bytes: evidence.size_bytes,
        })
    }

    /// Marks finalization failed and retains the partial path for reconciliation.
    #[must_use]
    pub fn fail(self, failure: SegmentFailure) -> FailedSegment {
        FailedSegment {
            partial_path: self.paths.into_parts().0,
            started_at: self.started_at,
            ended_at: Some(self.ended_at),
            stage: FailureStage::Finalizing,
            failure,
        }
    }
}

/// Proof returned by the future storage/muxer boundary after durable rename and
/// media probing succeed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableFileEvidence {
    path: PathBuf,
    size_bytes: NonZeroU64,
    dimensions: VideoDimensions,
}

impl DurableFileEvidence {
    /// Creates finalization evidence with a nonzero observed file size.
    ///
    /// # Errors
    ///
    /// Returns [`DurableFileEvidenceError::EmptyFile`] when `size_bytes` is
    /// zero.
    pub fn new(
        path: PathBuf,
        size_bytes: u64,
        dimensions: VideoDimensions,
    ) -> Result<Self, DurableFileEvidenceError> {
        let Some(size_bytes) = NonZeroU64::new(size_bytes) else {
            return Err(DurableFileEvidenceError::EmptyFile);
        };
        Ok(Self {
            path,
            size_bytes,
            dimensions,
        })
    }
}

/// Invalid durable-file validation evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableFileEvidenceError {
    EmptyFile,
}

impl fmt::Display for DurableFileEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("completed recording file must not be empty")
    }
}

impl Error for DurableFileEvidenceError {}

/// A durable, probe-validated recording segment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedSegment {
    path: PathBuf,
    started_at: SystemTime,
    ended_at: SystemTime,
    format: RecordingFormat,
    dimensions: VideoDimensions,
    size_bytes: NonZeroU64,
}

impl CompletedSegment {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
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
    pub const fn format(&self) -> RecordingFormat {
        self.format
    }

    #[must_use]
    pub const fn dimensions(&self) -> VideoDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes.get()
    }
}

/// The lifecycle stage at which a segment failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureStage {
    Open,
    Finalizing,
}

/// Typed, operator-actionable recording failure classes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentFailure {
    Muxer,
    Storage,
    Validation,
    Shutdown,
}

/// A failed segment retained for metrics and partial-file reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailedSegment {
    partial_path: PathBuf,
    started_at: SystemTime,
    ended_at: Option<SystemTime>,
    stage: FailureStage,
    failure: SegmentFailure,
}

impl FailedSegment {
    #[must_use]
    pub fn partial_path(&self) -> &Path {
        &self.partial_path
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
    pub const fn stage(&self) -> FailureStage {
        self.stage
    }

    #[must_use]
    pub const fn failure(&self) -> SegmentFailure {
        self.failure
    }
}

/// Invalid segment lifecycle transitions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentTransitionError {
    EndNotAfterStart,
    UnexpectedDurablePath,
}

impl fmt::Display for SegmentTransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EndNotAfterStart => {
                formatter.write_str("segment end time must be later than start time")
            }
            Self::UnexpectedDurablePath => formatter.write_str(
                "durable-file evidence path does not match the segment finalization target",
            ),
        }
    }
}

impl Error for SegmentTransitionError {}

#[cfg(test)]
mod tests {
    use super::{
        DurableFileEvidence, DurableFileEvidenceError, FailureStage, OpenSegment, RecordingFormat,
        SegmentFailure, SegmentTransitionError, VideoDimensions, VideoDimensionsError,
    };
    use std::{path::PathBuf, time::Duration};
    use surveillance_storage::filesystem::{SegmentPathError, SegmentPaths};

    fn paths() -> SegmentPaths {
        SegmentPaths::from_partial(PathBuf::from(
            "recordings/camera-01/2026/08/10/22-12-00.partial.mp4",
        ))
        .unwrap_or_else(|error| panic!("valid test path: {error}"))
    }

    fn dimensions() -> VideoDimensions {
        VideoDimensions::new(2560, 1440)
            .unwrap_or_else(|error| panic!("valid test dimensions: {error}"))
    }

    #[test]
    fn derives_durable_path_without_crossing_directories() {
        let paths = paths();
        assert_eq!(
            paths.durable(),
            std::path::Path::new("recordings/camera-01/2026/08/10/22-12-00.mp4")
        );
        assert_eq!(
            SegmentPaths::from_partial(PathBuf::from("recording.mp4")),
            Err(SegmentPathError::InvalidPartialSuffix)
        );
        assert_eq!(
            SegmentPaths::from_partial(PathBuf::from(".partial.mp4")),
            Err(SegmentPathError::EmptyStem)
        );
    }

    #[test]
    fn validates_dimensions_and_durable_file_size() {
        assert_eq!(
            VideoDimensions::new(0, 1080),
            Err(VideoDimensionsError::ZeroWidth)
        );
        assert_eq!(
            VideoDimensions::new(1920, 0),
            Err(VideoDimensionsError::ZeroHeight)
        );
        assert_eq!(
            DurableFileEvidence::new(PathBuf::from("segment.mp4"), 0, dimensions()),
            Err(DurableFileEvidenceError::EmptyFile)
        );
    }

    #[test]
    fn completed_segment_requires_ordered_times_and_matching_durable_path() {
        let start = std::time::UNIX_EPOCH + Duration::from_secs(100);
        let open = OpenSegment::new(paths(), start, RecordingFormat::FragmentedMp4H264);
        let finalizing = open
            .begin_finalization(start + Duration::from_secs(30))
            .unwrap_or_else(|error| panic!("valid transition: {error}"));
        let evidence = DurableFileEvidence::new(
            PathBuf::from("recordings/camera-01/2026/08/10/22-12-00.mp4"),
            1_048_576,
            dimensions(),
        )
        .unwrap_or_else(|error| panic!("valid evidence: {error}"));

        let completed = finalizing
            .complete(evidence)
            .unwrap_or_else(|error| panic!("valid completion: {error}"));
        assert_eq!(completed.started_at(), start);
        assert_eq!(completed.ended_at(), start + Duration::from_secs(30));
        assert_eq!(completed.size_bytes(), 1_048_576);
        assert_eq!(completed.dimensions(), dimensions());
        assert_eq!(completed.format(), RecordingFormat::FragmentedMp4H264);
    }

    #[test]
    fn refuses_nonpositive_interval_and_wrong_final_path() {
        let start = std::time::UNIX_EPOCH + Duration::from_secs(100);
        let open = OpenSegment::new(paths(), start, RecordingFormat::FragmentedMp4H264);
        assert_eq!(
            open.begin_finalization(start),
            Err(SegmentTransitionError::EndNotAfterStart)
        );

        let open = OpenSegment::new(paths(), start, RecordingFormat::FragmentedMp4H264);
        let finalizing = open
            .begin_finalization(start + Duration::from_secs(30))
            .unwrap_or_else(|error| panic!("valid transition: {error}"));
        let evidence =
            DurableFileEvidence::new(PathBuf::from("recordings/other.mp4"), 1, dimensions())
                .unwrap_or_else(|error| panic!("valid evidence: {error}"));
        assert_eq!(
            finalizing.complete(evidence),
            Err(SegmentTransitionError::UnexpectedDurablePath)
        );
    }

    #[test]
    fn failures_retain_partial_path_and_lifecycle_stage() {
        let start = std::time::UNIX_EPOCH + Duration::from_secs(100);
        let open = OpenSegment::new(paths(), start, RecordingFormat::FragmentedMp4H264);
        let failed = open.fail(SegmentFailure::Muxer);
        assert_eq!(failed.stage(), FailureStage::Open);
        assert_eq!(failed.ended_at(), None);
        assert_eq!(failed.failure(), SegmentFailure::Muxer);

        let open = OpenSegment::new(paths(), start, RecordingFormat::FragmentedMp4H264);
        let finalizing = open
            .begin_finalization(start + Duration::from_secs(30))
            .unwrap_or_else(|error| panic!("valid transition: {error}"));
        let failed = finalizing.fail(SegmentFailure::Validation);
        assert_eq!(failed.stage(), FailureStage::Finalizing);
        assert_eq!(failed.ended_at(), Some(start + Duration::from_secs(30)));
        assert_eq!(failed.failure(), SegmentFailure::Validation);
        assert!(failed.partial_path().ends_with("22-12-00.partial.mp4"));
    }
}
