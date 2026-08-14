//! Timestamp-aware encoded-input sampling.

use std::{error::Error, fmt, time::Duration};
use surveillance_media::packet::MediaTimestamp;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SampleRateHz(u16);
impl SampleRateHz {
    /// # Errors
    /// Returns an error unless the rate is within `1..=60` FPS.
    pub fn new(value: u16) -> Result<Self, SamplerError> {
        if value == 0 || value > 60 {
            Err(SamplerError::InvalidRate)
        } else {
            Ok(Self(value))
        }
    }
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SamplerCounters {
    pub considered: u64,
    pub sampled: u64,
    pub skipped_by_rate: u64,
    pub duplicate: u64,
    pub resets: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SampleDecision {
    Sample,
    SkipRate,
    Duplicate,
    ResetAndSample,
}

pub struct TimestampSampler {
    rate: SampleRateHz,
    maximum_gap: Duration,
    last: Option<(u64, MediaTimestamp)>,
    counters: SamplerCounters,
}
impl TimestampSampler {
    /// # Errors
    /// Returns an error when the maximum reset gap is zero.
    pub fn new(rate: SampleRateHz, maximum_gap: Duration) -> Result<Self, SamplerError> {
        if maximum_gap.is_zero() {
            return Err(SamplerError::ZeroMaximumGap);
        }
        Ok(Self {
            rate,
            maximum_gap,
            last: None,
            counters: SamplerCounters::default(),
        })
    }
    /// # Errors
    /// Returns an error for epoch zero.
    pub fn consider(
        &mut self,
        epoch: u64,
        timestamp: MediaTimestamp,
    ) -> Result<SampleDecision, SamplerError> {
        if epoch == 0 {
            return Err(SamplerError::ZeroEpoch);
        }
        self.counters.considered = self.counters.considered.saturating_add(1);
        let Some((last_epoch, last)) = self.last else {
            self.last = Some((epoch, timestamp));
            self.counters.sampled += 1;
            return Ok(SampleDecision::Sample);
        };
        if epoch != last_epoch
            || timestamp.timebase_hz() != last.timebase_hz()
            || timestamp.ticks() < last.ticks()
        {
            return Ok(self.reset_and_sample(epoch, timestamp));
        }
        if timestamp.ticks() == last.ticks() {
            self.counters.duplicate += 1;
            return Ok(SampleDecision::Duplicate);
        }
        let delta_ticks = i128::from(timestamp.ticks() - last.ticks());
        let timebase = i128::from(timestamp.timebase_hz());
        let gap_nanos = delta_ticks.saturating_mul(1_000_000_000) / timebase;
        let max_gap_nanos = i128::try_from(self.maximum_gap.as_nanos()).unwrap_or(i128::MAX);
        if gap_nanos > max_gap_nanos {
            return Ok(self.reset_and_sample(epoch, timestamp));
        }
        if delta_ticks.saturating_mul(i128::from(self.rate.get())) >= timebase {
            self.last = Some((epoch, timestamp));
            self.counters.sampled += 1;
            Ok(SampleDecision::Sample)
        } else {
            self.counters.skipped_by_rate += 1;
            Ok(SampleDecision::SkipRate)
        }
    }
    fn reset_and_sample(&mut self, epoch: u64, timestamp: MediaTimestamp) -> SampleDecision {
        self.last = Some((epoch, timestamp));
        self.counters.resets += 1;
        self.counters.sampled += 1;
        SampleDecision::ResetAndSample
    }
    #[must_use]
    pub const fn counters(&self) -> SamplerCounters {
        self.counters
    }
    pub fn reset(&mut self) {
        self.last = None;
        self.counters.resets = self.counters.resets.saturating_add(1);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SamplerError {
    InvalidRate,
    ZeroMaximumGap,
    ZeroEpoch,
}
impl fmt::Display for SamplerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid timestamp sampler: {self:?}")
    }
}
impl Error for SamplerError {}

#[cfg(test)]
mod tests {
    use super::{SampleDecision, SampleRateHz, TimestampSampler};
    use std::time::Duration;
    use surveillance_media::packet::MediaTimestamp;
    fn ts(ticks: i64, hz: u32) -> MediaTimestamp {
        MediaTimestamp::new(ticks, hz).unwrap_or_else(|error| panic!("timestamp: {error}"))
    }
    #[test]
    fn samples_variable_fps_at_no_more_than_requested_rate() {
        let mut sampler = TimestampSampler::new(
            SampleRateHz::new(5).unwrap_or_else(|error| panic!("rate: {error}")),
            Duration::from_secs(2),
        )
        .unwrap_or_else(|error| panic!("sampler: {error}"));
        let decisions: Vec<_> = [0, 3_000, 9_000, 18_000, 27_000, 36_000]
            .into_iter()
            .map(|ticks| {
                sampler
                    .consider(1, ts(ticks, 90_000))
                    .unwrap_or_else(|error| panic!("sample: {error}"))
            })
            .collect();
        assert_eq!(
            decisions,
            vec![
                SampleDecision::Sample,
                SampleDecision::SkipRate,
                SampleDecision::SkipRate,
                SampleDecision::Sample,
                SampleDecision::SkipRate,
                SampleDecision::Sample
            ]
        );
    }
    #[test]
    fn resets_on_epoch_backward_timebase_and_large_gap() {
        let mut sampler = TimestampSampler::new(
            SampleRateHz::new(5).unwrap_or_else(|error| panic!("rate: {error}")),
            Duration::from_secs(1),
        )
        .unwrap_or_else(|error| panic!("sampler: {error}"));
        assert_eq!(sampler.consider(1, ts(10, 100)), Ok(SampleDecision::Sample));
        assert_eq!(
            sampler.consider(2, ts(10, 100)),
            Ok(SampleDecision::ResetAndSample)
        );
        assert_eq!(
            sampler.consider(2, ts(9, 100)),
            Ok(SampleDecision::ResetAndSample)
        );
        assert_eq!(
            sampler.consider(2, ts(9, 90)),
            Ok(SampleDecision::ResetAndSample)
        );
        assert_eq!(
            sampler.consider(2, ts(200, 90)),
            Ok(SampleDecision::ResetAndSample)
        );
        assert_eq!(sampler.counters().resets, 4);
    }
    #[test]
    fn duplicate_and_nonzero_origin_are_explicit() {
        let mut sampler = TimestampSampler::new(
            SampleRateHz::new(5).unwrap_or_else(|error| panic!("rate: {error}")),
            Duration::from_secs(1),
        )
        .unwrap_or_else(|error| panic!("sampler: {error}"));
        assert_eq!(
            sampler.consider(1, ts(50_000, 90_000)),
            Ok(SampleDecision::Sample)
        );
        assert_eq!(
            sampler.consider(1, ts(50_000, 90_000)),
            Ok(SampleDecision::Duplicate)
        );
    }
}
