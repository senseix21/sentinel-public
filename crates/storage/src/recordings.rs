//! Bounded half-open recording search contracts.

use std::{
    error::Error,
    fmt,
    num::NonZeroUsize,
    time::{Duration, SystemTime},
};

use crate::metadata::CompletedRecording;
use surveillance_common::types::{CameraId, RecordingId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordingTimeRange {
    start: SystemTime,
    end: SystemTime,
}

impl RecordingTimeRange {
    /// Validates a half-open `[start, end)` query interval.
    ///
    /// # Errors
    /// Returns [`RecordingTimeRangeError`] when the interval is empty/reversed
    /// or exceeds the caller-selected maximum span.
    pub fn new(
        start: SystemTime,
        end: SystemTime,
        maximum_span: Duration,
    ) -> Result<Self, RecordingTimeRangeError> {
        let span = end
            .duration_since(start)
            .map_err(|_| RecordingTimeRangeError::NonPositive)?;
        if span.is_zero() {
            return Err(RecordingTimeRangeError::NonPositive);
        }
        if span > maximum_span {
            return Err(RecordingTimeRangeError::TooLong);
        }
        Ok(Self { start, end })
    }

    #[must_use]
    pub const fn start(self) -> SystemTime {
        self.start
    }
    #[must_use]
    pub const fn end(self) -> SystemTime {
        self.end
    }

    /// Implements the normative SQL predicate: segment start precedes query
    /// end and segment end follows query start.
    #[must_use]
    pub fn overlaps(self, segment_start: SystemTime, segment_end: SystemTime) -> bool {
        segment_start < self.end && segment_end > self.start
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingTimeRangeError {
    NonPositive,
    TooLong,
}

impl fmt::Display for RecordingTimeRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid recording time range: {self:?}")
    }
}

impl Error for RecordingTimeRangeError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingCursor {
    started_at: SystemTime,
    recording_id: RecordingId,
}

impl RecordingCursor {
    #[must_use]
    pub const fn new(started_at: SystemTime, recording_id: RecordingId) -> Self {
        Self {
            started_at,
            recording_id,
        }
    }

    #[must_use]
    pub const fn started_at(&self) -> SystemTime {
        self.started_at
    }
    #[must_use]
    pub fn recording_id(&self) -> &RecordingId {
        &self.recording_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordingResultLimit(NonZeroUsize);

impl RecordingResultLimit {
    pub const MAXIMUM: usize = 1_000;

    /// Validates a nonzero bounded page size.
    ///
    /// # Errors
    /// Returns [`RecordingResultLimitError`] outside `1..=1000`.
    pub fn new(value: usize) -> Result<Self, RecordingResultLimitError> {
        NonZeroUsize::new(value)
            .filter(|value| value.get() <= Self::MAXIMUM)
            .map(Self)
            .ok_or(RecordingResultLimitError)
    }

    #[must_use]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordingResultLimitError;

impl fmt::Display for RecordingResultLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("recording result limit must be between 1 and 1000")
    }
}

impl Error for RecordingResultLimitError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingSearch {
    camera_id: CameraId,
    range: RecordingTimeRange,
    limit: RecordingResultLimit,
    cursor: Option<RecordingCursor>,
}

impl RecordingSearch {
    #[must_use]
    pub const fn new(
        camera_id: CameraId,
        range: RecordingTimeRange,
        limit: RecordingResultLimit,
        cursor: Option<RecordingCursor>,
    ) -> Self {
        Self {
            camera_id,
            range,
            limit,
            cursor,
        }
    }

    #[must_use]
    pub fn camera_id(&self) -> &CameraId {
        &self.camera_id
    }
    #[must_use]
    pub const fn range(&self) -> RecordingTimeRange {
        self.range
    }
    #[must_use]
    pub const fn limit(&self) -> RecordingResultLimit {
        self.limit
    }
    #[must_use]
    pub const fn cursor(&self) -> Option<&RecordingCursor> {
        self.cursor.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackAccuracy {
    SegmentAligned,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaybackPlan {
    requested: RecordingTimeRange,
    segments: Vec<CompletedRecording>,
}

impl PlaybackPlan {
    /// Builds a segment-aligned stream-copy plan and rejects missing, gapped,
    /// overlapping, unordered, or cross-camera evidence.
    ///
    /// # Errors
    /// Returns [`PlaybackPlanError`] unless the segments continuously cover the
    /// requested half-open range.
    pub fn new(
        requested: RecordingTimeRange,
        segments: Vec<CompletedRecording>,
    ) -> Result<Self, PlaybackPlanError> {
        let Some(first) = segments.first() else {
            return Err(PlaybackPlanError::NoCoverage);
        };
        if first.started_at() > requested.start() || first.ended_at() <= requested.start() {
            return Err(PlaybackPlanError::NoCoverage);
        }
        let camera = first.camera_id();
        for pair in segments.windows(2) {
            if pair[1].camera_id() != camera {
                return Err(PlaybackPlanError::CrossCamera);
            }
            if pair[1].started_at() < pair[0].started_at() {
                return Err(PlaybackPlanError::Unordered);
            }
            if pair[1].started_at() > pair[0].ended_at() {
                return Err(PlaybackPlanError::Gap);
            }
            if pair[1].started_at() < pair[0].ended_at() {
                return Err(PlaybackPlanError::Overlap);
            }
        }
        if segments
            .last()
            .is_none_or(|last| last.ended_at() < requested.end())
        {
            return Err(PlaybackPlanError::NoCoverage);
        }
        Ok(Self {
            requested,
            segments,
        })
    }

    #[must_use]
    pub const fn accuracy(&self) -> PlaybackAccuracy {
        PlaybackAccuracy::SegmentAligned
    }
    #[must_use]
    pub const fn requested(&self) -> RecordingTimeRange {
        self.requested
    }
    #[must_use]
    pub fn segments(&self) -> &[CompletedRecording] {
        &self.segments
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackPlanError {
    NoCoverage,
    Gap,
    Overlap,
    Unordered,
    CrossCamera,
}

impl fmt::Display for PlaybackPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid playback plan: {self:?}")
    }
}
impl Error for PlaybackPlanError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{MetadataProvenance, RecordingContainer, RecordingLocator, VideoCodec};
    use std::path::PathBuf;

    fn at(second: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(second)
    }

    #[test]
    fn half_open_boundaries_do_not_overlap() {
        let query = RecordingTimeRange::new(at(20), at(30), Duration::from_secs(60))
            .unwrap_or_else(|error| panic!("valid range: {error}"));
        assert!(!query.overlaps(at(10), at(20)));
        assert!(!query.overlaps(at(30), at(40)));
        assert!(query.overlaps(at(19), at(21)));
        assert!(query.overlaps(at(29), at(31)));
    }

    #[test]
    fn overlap_predicate_matches_nonempty_intersection_for_bounded_domain() {
        for query_start in 0..20 {
            for query_end in (query_start + 1)..=20 {
                let query = RecordingTimeRange::new(
                    at(query_start),
                    at(query_end),
                    Duration::from_secs(20),
                )
                .unwrap_or_else(|error| panic!("valid range: {error}"));
                for segment_start in 0..20 {
                    for segment_end in (segment_start + 1)..=20 {
                        let expected = query_start.max(segment_start) < query_end.min(segment_end);
                        assert_eq!(query.overlaps(at(segment_start), at(segment_end)), expected);
                    }
                }
            }
        }
    }

    #[test]
    fn rejects_nonpositive_and_excessive_ranges() {
        assert_eq!(
            RecordingTimeRange::new(at(2), at(2), Duration::from_secs(1)),
            Err(RecordingTimeRangeError::NonPositive)
        );
        assert_eq!(
            RecordingTimeRange::new(at(2), at(1), Duration::from_secs(1)),
            Err(RecordingTimeRangeError::NonPositive)
        );
        assert_eq!(
            RecordingTimeRange::new(at(1), at(3), Duration::from_secs(1)),
            Err(RecordingTimeRangeError::TooLong)
        );
    }

    fn completed(id: &str, start: u64, end: u64) -> CompletedRecording {
        CompletedRecording::new(
            RecordingId::new(id).unwrap_or_else(|error| panic!("id: {error}")),
            CameraId::new("camera-1").unwrap_or_else(|error| panic!("camera: {error}")),
            at(start),
            at(end),
            RecordingLocator::new(PathBuf::from(format!("camera-1/{id}.mp4")))
                .unwrap_or_else(|error| panic!("locator: {error}")),
            RecordingContainer::FragmentedMp4,
            VideoCodec::H264,
            1,
            1,
            1,
            MetadataProvenance::Recorder,
        )
        .unwrap_or_else(|error| panic!("completed: {error}"))
    }

    #[test]
    fn playback_plan_requires_continuous_three_segment_coverage() {
        let requested = RecordingTimeRange::new(at(15), at(35), Duration::from_secs(60))
            .unwrap_or_else(|error| panic!("range: {error}"));
        let plan = PlaybackPlan::new(
            requested,
            vec![
                completed("segment-1", 10, 20),
                completed("segment-2", 20, 30),
                completed("segment-3", 30, 40),
            ],
        )
        .unwrap_or_else(|error| panic!("plan: {error}"));
        assert_eq!(plan.accuracy(), PlaybackAccuracy::SegmentAligned);
        assert_eq!(plan.segments().len(), 3);
        assert_eq!(
            PlaybackPlan::new(
                requested,
                vec![completed("gap-1", 10, 20), completed("gap-2", 21, 40)]
            ),
            Err(PlaybackPlanError::Gap)
        );
        assert_eq!(
            PlaybackPlan::new(
                requested,
                vec![
                    completed("overlap-1", 10, 25),
                    completed("overlap-2", 20, 40)
                ]
            ),
            Err(PlaybackPlanError::Overlap)
        );
    }
}
