//! Epoch-aware mapping from media timestamps to observed wall time.

use std::{
    error::Error,
    fmt,
    num::NonZeroU64,
    time::{Duration, SystemTime},
};

use surveillance_media::packet::MediaTimestamp;

/// Maximum tolerated difference between projected media time and receipt time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockDriftTolerance(Duration);

impl ClockDriftTolerance {
    /// Validates a nonzero tolerance.
    ///
    /// # Errors
    ///
    /// Returns [`ClockConfigError`] when the tolerance is zero.
    pub fn new(value: Duration) -> Result<Self, ClockConfigError> {
        if value.is_zero() {
            return Err(ClockConfigError);
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockConfigError;

impl fmt::Display for ClockConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("media clock drift tolerance must be nonzero")
    }
}

impl Error for ClockConfigError {}

/// Why a new media-to-wall-clock epoch was established.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockEpochReason {
    Initial,
    TimestampRegression,
    TimebaseChanged,
    WallClockDiscontinuity,
}

/// One mapped timestamp and its stable epoch identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappedTimestamp {
    wall_time: SystemTime,
    epoch: NonZeroU64,
    epoch_reason: Option<ClockEpochReason>,
}

impl MappedTimestamp {
    #[must_use]
    pub const fn wall_time(self) -> SystemTime {
        self.wall_time
    }

    #[must_use]
    pub const fn epoch(self) -> u64 {
        self.epoch.get()
    }

    /// Returns a reason only for the first observation in a new epoch.
    #[must_use]
    pub const fn epoch_reason(self) -> Option<ClockEpochReason> {
        self.epoch_reason
    }
}

#[derive(Clone, Copy, Debug)]
struct Anchor {
    media: MediaTimestamp,
    wall: SystemTime,
    last_media: MediaTimestamp,
    epoch: NonZeroU64,
}

/// Maps monotonic timestamps within one source epoch and detects discontinuity.
#[derive(Debug)]
pub struct MediaWallClock {
    tolerance: ClockDriftTolerance,
    anchor: Option<Anchor>,
}

impl MediaWallClock {
    #[must_use]
    pub const fn new(tolerance: ClockDriftTolerance) -> Self {
        Self {
            tolerance,
            anchor: None,
        }
    }

    /// Observes one media timestamp at a wall-clock receipt time.
    ///
    /// # Errors
    ///
    /// Returns [`ClockMapError::EpochExhausted`] only if the process observes
    /// more than `u64::MAX` media epochs.
    pub fn observe(
        &mut self,
        media: MediaTimestamp,
        received_at: SystemTime,
    ) -> Result<MappedTimestamp, ClockMapError> {
        let Some(mut anchor) = self.anchor else {
            return self.reset(media, received_at, ClockEpochReason::Initial);
        };
        let reason = if media.timebase_hz() != anchor.media.timebase_hz() {
            Some(ClockEpochReason::TimebaseChanged)
        } else if media.ticks() <= anchor.last_media.ticks() {
            Some(ClockEpochReason::TimestampRegression)
        } else {
            None
        };
        if let Some(reason) = reason {
            return self.reset(media, received_at, reason);
        }

        let tick_delta = u64::try_from(media.ticks() - anchor.media.ticks())
            .map_err(|_| ClockMapError::TimestampOutOfRange)?;
        let projected = anchor
            .wall
            .checked_add(duration_from_ticks(tick_delta, media.timebase_hz()))
            .ok_or(ClockMapError::TimestampOutOfRange)?;
        if absolute_difference(projected, received_at) > self.tolerance.0 {
            return self.reset(media, received_at, ClockEpochReason::WallClockDiscontinuity);
        }
        anchor.last_media = media;
        self.anchor = Some(anchor);
        Ok(MappedTimestamp {
            wall_time: projected,
            epoch: anchor.epoch,
            epoch_reason: None,
        })
    }

    fn reset(
        &mut self,
        media: MediaTimestamp,
        wall: SystemTime,
        reason: ClockEpochReason,
    ) -> Result<MappedTimestamp, ClockMapError> {
        let epoch = match self.anchor {
            Some(anchor) => anchor
                .epoch
                .get()
                .checked_add(1)
                .and_then(NonZeroU64::new)
                .ok_or(ClockMapError::EpochExhausted)?,
            None => NonZeroU64::MIN,
        };
        self.anchor = Some(Anchor {
            media,
            wall,
            last_media: media,
            epoch,
        });
        Ok(MappedTimestamp {
            wall_time: wall,
            epoch,
            epoch_reason: Some(reason),
        })
    }
}

fn duration_from_ticks(ticks: u64, timebase_hz: u32) -> Duration {
    let seconds = ticks / u64::from(timebase_hz);
    let remainder = ticks % u64::from(timebase_hz);
    let nanos = remainder.saturating_mul(1_000_000_000) / u64::from(timebase_hz);
    Duration::new(seconds, u32::try_from(nanos).unwrap_or(999_999_999))
}

fn absolute_difference(left: SystemTime, right: SystemTime) -> Duration {
    left.duration_since(right)
        .or_else(|_| right.duration_since(left))
        .unwrap_or(Duration::ZERO)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockMapError {
    TimestampOutOfRange,
    EpochExhausted,
}

impl fmt::Display for ClockMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "media clock mapping failed: {self:?}")
    }
}

impl Error for ClockMapError {}

#[cfg(test)]
mod tests {
    use super::{ClockDriftTolerance, ClockEpochReason, MediaWallClock};
    use std::time::{Duration, SystemTime};
    use surveillance_media::packet::MediaTimestamp;

    fn timestamp(ticks: i64, timebase: u32) -> MediaTimestamp {
        MediaTimestamp::new(ticks, timebase)
            .unwrap_or_else(|error| panic!("valid test timestamp: {error}"))
    }

    fn mapper() -> MediaWallClock {
        MediaWallClock::new(
            ClockDriftTolerance::new(Duration::from_secs(2))
                .unwrap_or_else(|error| panic!("valid tolerance: {error}")),
        )
    }

    #[test]
    fn projects_monotonic_media_time_within_one_epoch() {
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let mut mapper = mapper();
        let first = mapper
            .observe(timestamp(90_000, 90_000), wall)
            .unwrap_or_else(|error| panic!("first mapping: {error}"));
        assert_eq!(first.epoch(), 1);
        assert_eq!(first.epoch_reason(), Some(ClockEpochReason::Initial));
        let second = mapper
            .observe(
                timestamp(135_000, 90_000),
                wall + Duration::from_millis(510),
            )
            .unwrap_or_else(|error| panic!("second mapping: {error}"));
        assert_eq!(second.epoch(), 1);
        assert_eq!(second.wall_time(), wall + Duration::from_millis(500));
        assert_eq!(second.epoch_reason(), None);
    }

    #[test]
    fn resets_on_timestamp_timebase_and_wall_clock_discontinuities() {
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let mut mapper = mapper();
        let _ = mapper.observe(timestamp(90_000, 90_000), wall);
        let regression = mapper
            .observe(timestamp(0, 90_000), wall + Duration::from_secs(1))
            .unwrap_or_else(|error| panic!("regression mapping: {error}"));
        assert_eq!(
            regression.epoch_reason(),
            Some(ClockEpochReason::TimestampRegression)
        );
        let timebase = mapper
            .observe(timestamp(1, 1_000), wall + Duration::from_secs(2))
            .unwrap_or_else(|error| panic!("timebase mapping: {error}"));
        assert_eq!(
            timebase.epoch_reason(),
            Some(ClockEpochReason::TimebaseChanged)
        );
        let jump = mapper
            .observe(timestamp(2, 1_000), wall + Duration::from_secs(20))
            .unwrap_or_else(|error| panic!("clock jump mapping: {error}"));
        assert_eq!(
            jump.epoch_reason(),
            Some(ClockEpochReason::WallClockDiscontinuity)
        );
        assert_eq!(jump.epoch(), 4);
    }
}
