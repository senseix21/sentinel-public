//! Filesystem layout and safe file operations.

use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use std::time::SystemTime;
use surveillance_common::types::{CameraId, RecordingId};

/// A validated fragmented-MP4 partial and durable path pair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentPaths {
    partial: PathBuf,
    durable: PathBuf,
}

impl SegmentPaths {
    /// Validates an active segment path and derives its durable path.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentPathError`] if the path is not a nonempty
    /// `.partial.mp4` file path.
    pub fn from_partial(partial: PathBuf) -> Result<Self, SegmentPathError> {
        let Some(file_name) = partial.file_name().and_then(|name| name.to_str()) else {
            return Err(SegmentPathError::MissingUtf8FileName);
        };
        let Some(durable_name) = file_name.strip_suffix(".partial.mp4") else {
            return Err(SegmentPathError::InvalidPartialSuffix);
        };
        if durable_name.is_empty() {
            return Err(SegmentPathError::EmptyStem);
        }

        let durable = partial.with_file_name(format!("{durable_name}.mp4"));
        Ok(Self { partial, durable })
    }

    #[must_use]
    pub fn partial(&self) -> &Path {
        &self.partial
    }

    #[must_use]
    pub fn durable(&self) -> &Path {
        &self.durable
    }

    #[must_use]
    pub fn into_parts(self) -> (PathBuf, PathBuf) {
        (self.partial, self.durable)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentPathError {
    MissingUtf8FileName,
    InvalidPartialSuffix,
    EmptyStem,
}

impl fmt::Display for SegmentPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingUtf8FileName => {
                formatter.write_str("segment path must have a UTF-8 filename")
            }
            Self::InvalidPartialSuffix => {
                formatter.write_str("active segment path must end in .partial.mp4")
            }
            Self::EmptyStem => formatter.write_str("segment filename stem must not be empty"),
        }
    }
}

impl Error for SegmentPathError {}

/// Validated Gregorian UTC components for deterministic placement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UtcPathTime {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

impl UtcPathTime {
    /// Validates Gregorian UTC components. Leap seconds are not accepted.
    ///
    /// # Errors
    ///
    /// Returns [`UtcPathTimeError`] for an invalid component.
    pub fn new(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<Self, UtcPathTimeError> {
        if !(1970..=9999).contains(&year) {
            return Err(UtcPathTimeError::Year);
        }
        if !(1..=12).contains(&month) {
            return Err(UtcPathTimeError::Month);
        }
        if day == 0 || day > days_in_month(year, month) {
            return Err(UtcPathTimeError::Day);
        }
        if hour > 23 {
            return Err(UtcPathTimeError::Hour);
        }
        if minute > 59 {
            return Err(UtcPathTimeError::Minute);
        }
        if second > 59 {
            return Err(UtcPathTimeError::Second);
        }
        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UtcPathTimeError {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
}

impl fmt::Display for UtcPathTimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid UTC path-time component: {self:?}")
    }
}

impl Error for UtcPathTimeError {}

/// Owns deterministic recording placement beneath one configured root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingLayout {
    root: PathBuf,
}

impl RecordingLayout {
    /// Creates a layout rooted at a nonempty configured path.
    ///
    /// # Errors
    ///
    /// Returns [`RecordingLayoutError::EmptyRoot`] for an empty path.
    pub fn new(root: PathBuf) -> Result<Self, RecordingLayoutError> {
        if root.as_os_str().is_empty() {
            return Err(RecordingLayoutError::EmptyRoot);
        }
        Ok(Self { root })
    }

    /// Constructs a unique, camera-scoped UTC partial path.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentPathError`] if the constructed filename violates the
    /// segment-path invariant.
    pub fn segment_paths(
        &self,
        camera_id: &CameraId,
        recording_id: &RecordingId,
        time: UtcPathTime,
    ) -> Result<SegmentPaths, SegmentPathError> {
        let partial = self
            .root
            .join(camera_id.as_str())
            .join(format!("{:04}", time.year))
            .join(format!("{:02}", time.month))
            .join(format!("{:02}", time.day))
            .join(format!(
                "{:02}-{:02}-{:02}--{}.partial.mp4",
                time.hour,
                time.minute,
                time.second,
                recording_id.as_str()
            ));
        SegmentPaths::from_partial(partial)
    }

    /// Constructs paths from an observed wall time converted explicitly to UTC.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentPathAtError`] if the system time is outside Jiff's
    /// supported timestamp range or a component/path invariant fails.
    pub fn segment_paths_at(
        &self,
        camera_id: &CameraId,
        recording_id: &RecordingId,
        started_at: SystemTime,
    ) -> Result<SegmentPaths, SegmentPathAtError> {
        let timestamp = jiff::Timestamp::try_from(started_at)
            .map_err(|_| SegmentPathAtError::TimestampOutOfRange)?;
        let datetime = jiff::tz::TimeZone::UTC.to_datetime(timestamp);
        let year =
            u16::try_from(datetime.year()).map_err(|_| SegmentPathAtError::TimestampOutOfRange)?;
        let month =
            u8::try_from(datetime.month()).map_err(|_| SegmentPathAtError::TimestampOutOfRange)?;
        let day =
            u8::try_from(datetime.day()).map_err(|_| SegmentPathAtError::TimestampOutOfRange)?;
        let hour =
            u8::try_from(datetime.hour()).map_err(|_| SegmentPathAtError::TimestampOutOfRange)?;
        let minute =
            u8::try_from(datetime.minute()).map_err(|_| SegmentPathAtError::TimestampOutOfRange)?;
        let second =
            u8::try_from(datetime.second()).map_err(|_| SegmentPathAtError::TimestampOutOfRange)?;
        let time = UtcPathTime::new(year, month, day, hour, minute, second)
            .map_err(SegmentPathAtError::Utc)?;
        self.segment_paths(camera_id, recording_id, time)
            .map_err(SegmentPathAtError::Path)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Confirms both paths remain beneath this configured recording root.
    #[must_use]
    pub fn owns(&self, paths: &SegmentPaths) -> bool {
        paths.partial().starts_with(&self.root) && paths.durable().starts_with(&self.root)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentPathAtError {
    TimestampOutOfRange,
    Utc(UtcPathTimeError),
    Path(SegmentPathError),
}

impl fmt::Display for SegmentPathAtError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "cannot construct UTC recording path: {self:?}")
    }
}

impl Error for SegmentPathAtError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingLayoutError {
    EmptyRoot,
}

impl fmt::Display for RecordingLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("recording root must not be empty")
    }
}

impl Error for RecordingLayoutError {}

const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

const fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

#[cfg(test)]
mod tests {
    use super::{RecordingLayout, RecordingLayoutError, UtcPathTime, UtcPathTimeError};
    use std::{
        path::{Path, PathBuf},
        time::{Duration, SystemTime},
    };
    use surveillance_common::types::{CameraId, RecordingId};

    fn camera_id() -> CameraId {
        CameraId::new("camera-01").unwrap_or_else(|error| panic!("valid camera ID: {error}"))
    }

    fn recording_id() -> RecordingId {
        RecordingId::new("01k2a7f4m2t8")
            .unwrap_or_else(|error| panic!("valid recording ID: {error}"))
    }

    #[test]
    fn builds_camera_scoped_utc_partial_and_durable_paths() {
        let layout = RecordingLayout::new(PathBuf::from("/srv/surveillance/recordings"))
            .unwrap_or_else(|error| panic!("valid root: {error}"));
        let time = UtcPathTime::new(2026, 8, 10, 22, 58, 3)
            .unwrap_or_else(|error| panic!("valid UTC time: {error}"));
        let paths = layout
            .segment_paths(&camera_id(), &recording_id(), time)
            .unwrap_or_else(|error| panic!("valid segment path: {error}"));

        assert_eq!(
            paths.partial(),
            Path::new(
                "/srv/surveillance/recordings/camera-01/2026/08/10/22-58-03--01k2a7f4m2t8.partial.mp4"
            )
        );
        assert_eq!(
            paths.durable(),
            Path::new(
                "/srv/surveillance/recordings/camera-01/2026/08/10/22-58-03--01k2a7f4m2t8.mp4"
            )
        );
        assert!(paths.partial().starts_with(layout.root()));
    }

    #[test]
    fn validates_calendar_boundaries() {
        assert!(UtcPathTime::new(2024, 2, 29, 23, 59, 59).is_ok());
        assert_eq!(
            UtcPathTime::new(2026, 2, 29, 0, 0, 0),
            Err(UtcPathTimeError::Day)
        );
        assert_eq!(
            UtcPathTime::new(2026, 1, 1, 0, 0, 60),
            Err(UtcPathTimeError::Second)
        );
    }

    #[test]
    fn rejects_empty_root() {
        assert_eq!(
            RecordingLayout::new(PathBuf::new()),
            Err(RecordingLayoutError::EmptyRoot)
        );
    }

    #[test]
    fn maps_system_time_to_utc_path_without_local_timezone() {
        let layout = RecordingLayout::new(PathBuf::from("recordings"))
            .unwrap_or_else(|error| panic!("valid root: {error}"));
        let started_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_786_403_083);
        let paths = layout
            .segment_paths_at(&camera_id(), &recording_id(), started_at)
            .unwrap_or_else(|error| panic!("valid timestamp: {error}"));
        assert!(paths.partial().starts_with("recordings/camera-01/2026/08"));
    }
}
