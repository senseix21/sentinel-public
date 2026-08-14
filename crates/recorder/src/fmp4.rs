//! Bounded H.264 access-unit to fragmented-MP4 segment adapter.

use std::{collections::VecDeque, error::Error, fmt, num::NonZeroUsize, time::Duration};

use muxide::fragmented::{FragmentConfig, FragmentedError, FragmentedMuxer};
use surveillance_media::packet::{H264AccessUnit, H264NalKind, MediaTimestamp};

use crate::{
    recorder::H264AccessUnitSink,
    segment::{VideoDimensions, VideoDimensionsError},
};

const H264_TIMEBASE_HZ: u32 = 90_000;

/// A production segment duration constrained to the qualified 30–60 second range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentDuration(Duration);

impl SegmentDuration {
    /// Validates the production segment range.
    ///
    /// # Errors
    ///
    /// Rejects durations below 30 seconds or above 60 seconds.
    pub fn new(duration: Duration) -> Result<Self, Fmp4ConfigError> {
        if duration < Duration::from_secs(30) {
            return Err(Fmp4ConfigError::DurationBelowThirtySeconds);
        }
        if duration > Duration::from_secs(60) {
            return Err(Fmp4ConfigError::DurationAboveSixtySeconds);
        }
        Ok(Self(duration))
    }

    #[must_use]
    pub const fn get(self) -> Duration {
        self.0
    }
}

/// Explicit memory and media configuration for one recorder sink.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fmp4SinkConfig {
    dimensions: VideoDimensions,
    segment_duration: SegmentDuration,
    fragment_duration: Duration,
    max_segment_bytes: NonZeroUsize,
    max_fragment_bytes: NonZeroUsize,
    max_samples: NonZeroUsize,
}

impl Fmp4SinkConfig {
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.dimensions.width()
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.dimensions.height()
    }

    /// Creates a bounded sink configuration.
    ///
    /// # Errors
    ///
    /// Rejects zero dimensions, byte capacity, or sample capacity.
    pub fn new(
        width: u32,
        height: u32,
        segment_duration: SegmentDuration,
        max_segment_bytes: usize,
        max_samples: usize,
    ) -> Result<Self, Fmp4ConfigError> {
        Self::new_with_fragment_bounds(
            width,
            height,
            segment_duration,
            Duration::from_secs(1),
            max_segment_bytes,
            max_segment_bytes,
            max_samples,
        )
    }

    /// Creates a sink with separate file and internal-fragment bounds.
    ///
    /// # Errors
    ///
    /// Rejects invalid dimensions, durations, or zero/inconsistent bounds.
    #[allow(
        clippy::too_many_arguments,
        reason = "all bounds are independent media invariants"
    )]
    pub fn new_with_fragment_bounds(
        width: u32,
        height: u32,
        segment_duration: SegmentDuration,
        fragment_duration: Duration,
        max_segment_bytes: usize,
        max_fragment_bytes: usize,
        max_samples: usize,
    ) -> Result<Self, Fmp4ConfigError> {
        let dimensions =
            VideoDimensions::new(width, height).map_err(Fmp4ConfigError::Dimensions)?;
        if fragment_duration.is_zero() {
            return Err(Fmp4ConfigError::ZeroFragmentDuration);
        }
        if fragment_duration >= segment_duration.get() {
            return Err(Fmp4ConfigError::FragmentNotBelowSegment);
        }
        let max_segment_bytes =
            NonZeroUsize::new(max_segment_bytes).ok_or(Fmp4ConfigError::ZeroMaxSegmentBytes)?;
        let max_fragment_bytes =
            NonZeroUsize::new(max_fragment_bytes).ok_or(Fmp4ConfigError::ZeroMaxFragmentBytes)?;
        if max_fragment_bytes > max_segment_bytes {
            return Err(Fmp4ConfigError::FragmentBytesAboveSegment);
        }
        let max_samples = NonZeroUsize::new(max_samples).ok_or(Fmp4ConfigError::ZeroMaxSamples)?;
        Ok(Self {
            dimensions,
            segment_duration,
            fragment_duration,
            max_segment_bytes,
            max_fragment_bytes,
            max_samples,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fmp4ConfigError {
    DurationBelowThirtySeconds,
    DurationAboveSixtySeconds,
    Dimensions(VideoDimensionsError),
    ZeroMaxSegmentBytes,
    ZeroMaxFragmentBytes,
    ZeroMaxSamples,
    ZeroFragmentDuration,
    FragmentNotBelowSegment,
    FragmentBytesAboveSegment,
}

impl fmt::Display for Fmp4ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid fragmented-MP4 sink configuration: {self:?}"
        )
    }
}

impl Error for Fmp4ConfigError {}

/// One completed recording-file boundary. Bytes are emitted separately.
#[derive(Debug)]
pub struct MuxedSegment {
    started_at: MediaTimestamp,
    ended_at: MediaTimestamp,
    samples: usize,
}

impl MuxedSegment {
    #[must_use]
    pub const fn started_at(&self) -> MediaTimestamp {
        self.started_at
    }

    #[must_use]
    pub const fn ended_at(&self) -> MediaTimestamp {
        self.ended_at
    }

    #[must_use]
    pub const fn samples(&self) -> usize {
        self.samples
    }
}

struct ActiveSegment {
    muxer: FragmentedMuxer,
    started_at: MediaTimestamp,
    last_timestamp: MediaTimestamp,
    input_bytes: usize,
    fragment_input_bytes: usize,
    samples: usize,
}

/// Ordered output from the synchronous muxer to the async persistence owner.
#[derive(Debug)]
pub enum Fmp4Output {
    Bytes(Vec<u8>),
    Completed(MuxedSegment),
}

/// Bounded segmenter which emits at most one pending completed segment.
pub struct Fmp4SegmentSink {
    config: Fmp4SinkConfig,
    active: Option<ActiveSegment>,
    output: VecDeque<Fmp4Output>,
    waiting_for_keyframe: u64,
}

impl Fmp4SegmentSink {
    #[must_use]
    pub fn new(config: Fmp4SinkConfig) -> Self {
        Self {
            config,
            active: None,
            output: VecDeque::with_capacity(3),
            waiting_for_keyframe: 0,
        }
    }

    /// Removes the next ordered muxer emission.
    pub fn take_output(&mut self) -> Option<Fmp4Output> {
        self.output.pop_front()
    }

    /// Finalizes the current nonempty segment during orderly shutdown.
    ///
    /// # Errors
    ///
    /// Fails if another completed segment has not been taken or final output
    /// exceeds the configured byte bound.
    pub fn finish(&mut self) -> Result<(), Fmp4SinkError> {
        if !self.output.is_empty() {
            return Err(Fmp4SinkError::OutputPending);
        }
        if let Some(active) = self.active.take() {
            let ended_at = active.last_timestamp;
            enqueue_finalized(
                &mut self.output,
                active,
                ended_at,
                self.config.max_fragment_bytes,
            )?;
        }
        Ok(())
    }

    #[must_use]
    pub const fn waiting_for_keyframe(&self) -> u64 {
        self.waiting_for_keyframe
    }

    /// Returns the media timestamp of the active segment, if one exists.
    #[must_use]
    pub fn active_started_at(&self) -> Option<MediaTimestamp> {
        self.active.as_ref().map(|active| active.started_at)
    }

    #[must_use]
    pub fn max_segment_bytes(&self) -> usize {
        self.config.max_segment_bytes.get()
    }

    fn start(
        &self,
        access_unit: &H264AccessUnit,
    ) -> Result<(ActiveSegment, Vec<u8>), Fmp4SinkError> {
        let (sps, pps) = parameter_sets(access_unit)?;
        let milliseconds = self.config.fragment_duration.as_millis();
        let fragment_duration_ms =
            u32::try_from(milliseconds).map_err(|_| Fmp4SinkError::TimestampOutOfRange)?;
        let config = FragmentConfig {
            width: self.config.dimensions.width(),
            height: self.config.dimensions.height(),
            timescale: H264_TIMEBASE_HZ,
            fragment_duration_ms,
            sps,
            pps,
            vps: None,
            av1_sequence_header: None,
            vp9_config: None,
        };
        let mut muxer = FragmentedMuxer::new(config);
        let init = muxer.init_segment();
        if init.len() > self.config.max_fragment_bytes.get() {
            return Err(Fmp4SinkError::FragmentTooLarge);
        }
        Ok((
            ActiveSegment {
                muxer,
                started_at: access_unit.timestamp(),
                last_timestamp: access_unit.timestamp(),
                input_bytes: 0,
                fragment_input_bytes: 0,
                samples: 0,
            },
            init,
        ))
    }

    fn should_rotate(&self, active: &ActiveSegment, access_unit: &H264AccessUnit) -> bool {
        if !access_unit.is_keyframe() {
            return false;
        }
        let elapsed = access_unit
            .timestamp()
            .ticks()
            .saturating_sub(active.started_at.ticks());
        let target_ticks = i64::try_from(
            self.config
                .segment_duration
                .get()
                .as_secs()
                .saturating_mul(u64::from(H264_TIMEBASE_HZ)),
        )
        .unwrap_or(i64::MAX);
        elapsed >= target_ticks
    }
}

impl H264AccessUnitSink for Fmp4SegmentSink {
    type Error = Fmp4SinkError;

    fn write_access_unit(&mut self, access_unit: &H264AccessUnit) -> Result<(), Self::Error> {
        if !self.output.is_empty() {
            return Err(Fmp4SinkError::OutputPending);
        }
        if access_unit.timestamp().timebase_hz() != H264_TIMEBASE_HZ {
            return Err(Fmp4SinkError::UnexpectedTimebase);
        }

        let timestamp_discontinuity = self
            .active
            .as_ref()
            .is_some_and(|active| access_unit.timestamp().ticks() <= active.last_timestamp.ticks());
        let should_close_active = timestamp_discontinuity
            || self
                .active
                .as_ref()
                .is_some_and(|active| self.should_rotate(active, access_unit));
        if should_close_active {
            if !self.output.is_empty() {
                return Err(Fmp4SinkError::OutputPending);
            }
            if let Some(active) = self.active.take() {
                let ended_at = if timestamp_discontinuity {
                    active.last_timestamp
                } else {
                    access_unit.timestamp()
                };
                enqueue_finalized(
                    &mut self.output,
                    active,
                    ended_at,
                    self.config.max_fragment_bytes,
                )?;
            }
        }

        if self.active.is_none() {
            if !access_unit.is_keyframe() {
                self.waiting_for_keyframe = self.waiting_for_keyframe.saturating_add(1);
                return Ok(());
            }
            let (active, init) = self.start(access_unit)?;
            self.output.push_back(Fmp4Output::Bytes(init));
            self.active = Some(active);
        }

        let sample = length_prefixed_sample(access_unit)?;
        let active = self.active.as_mut().ok_or(Fmp4SinkError::NoActiveSegment)?;
        if active.samples >= self.config.max_samples.get() {
            return Err(Fmp4SinkError::TooManySamples);
        }
        let next_bytes = active
            .input_bytes
            .checked_add(sample.len())
            .ok_or(Fmp4SinkError::SegmentTooLarge)?;
        if next_bytes > self.config.max_segment_bytes.get() {
            return Err(Fmp4SinkError::SegmentTooLarge);
        }
        let next_fragment_bytes = active
            .fragment_input_bytes
            .checked_add(sample.len())
            .ok_or(Fmp4SinkError::FragmentTooLarge)?;
        if next_fragment_bytes > self.config.max_fragment_bytes.get() {
            return Err(Fmp4SinkError::FragmentTooLarge);
        }
        let relative_ticks = access_unit
            .timestamp()
            .ticks()
            .checked_sub(active.started_at.ticks())
            .and_then(|ticks| u64::try_from(ticks).ok())
            .ok_or(Fmp4SinkError::TimestampOutOfRange)?;
        active
            .muxer
            .write_video(
                relative_ticks,
                relative_ticks,
                &sample,
                access_unit.is_keyframe(),
            )
            .map_err(Fmp4SinkError::Muxer)?;
        active.last_timestamp = access_unit.timestamp();
        active.input_bytes = next_bytes;
        active.fragment_input_bytes = next_fragment_bytes;
        active.samples = active.samples.saturating_add(1);
        if active.muxer.ready_to_flush() {
            let fragment = active
                .muxer
                .flush_segment()
                .ok_or(Fmp4SinkError::EmptySegment)?;
            if fragment.len() > self.config.max_fragment_bytes.get() {
                return Err(Fmp4SinkError::FragmentTooLarge);
            }
            active.fragment_input_bytes = 0;
            self.output.push_back(Fmp4Output::Bytes(fragment));
        }
        Ok(())
    }
}

fn parameter_sets(access_unit: &H264AccessUnit) -> Result<(Vec<u8>, Vec<u8>), Fmp4SinkError> {
    let sps = access_unit
        .nal_units()
        .iter()
        .find(|nal| nal.kind() == H264NalKind::SequenceParameterSet)
        .map(|nal| nal.bytes().to_vec())
        .ok_or(Fmp4SinkError::MissingSequenceParameterSet)?;
    let pps = access_unit
        .nal_units()
        .iter()
        .find(|nal| nal.kind() == H264NalKind::PictureParameterSet)
        .map(|nal| nal.bytes().to_vec())
        .ok_or(Fmp4SinkError::MissingPictureParameterSet)?;
    Ok((sps, pps))
}

fn length_prefixed_sample(access_unit: &H264AccessUnit) -> Result<Vec<u8>, Fmp4SinkError> {
    let capacity = access_unit
        .nal_units()
        .iter()
        .try_fold(0_usize, |total, nal| {
            total
                .checked_add(4)
                .and_then(|value| value.checked_add(nal.bytes().len()))
        });
    let mut sample = Vec::with_capacity(capacity.ok_or(Fmp4SinkError::SegmentTooLarge)?);
    for nal in access_unit.nal_units() {
        let length = u32::try_from(nal.bytes().len()).map_err(|_| Fmp4SinkError::NalTooLarge)?;
        sample.extend_from_slice(&length.to_be_bytes());
        sample.extend_from_slice(nal.bytes());
    }
    Ok(sample)
}

fn finalize(
    mut active: ActiveSegment,
    ended_at: MediaTimestamp,
) -> (Option<Vec<u8>>, MuxedSegment) {
    let media = active.muxer.flush_segment();
    (
        media,
        MuxedSegment {
            started_at: active.started_at,
            ended_at,
            samples: active.samples,
        },
    )
}

fn enqueue_finalized(
    output: &mut VecDeque<Fmp4Output>,
    active: ActiveSegment,
    ended_at: MediaTimestamp,
    max_fragment_bytes: NonZeroUsize,
) -> Result<(), Fmp4SinkError> {
    let (fragment, completed) = finalize(active, ended_at);
    if let Some(fragment) = fragment {
        if fragment.len() > max_fragment_bytes.get() {
            return Err(Fmp4SinkError::FragmentTooLarge);
        }
        output.push_back(Fmp4Output::Bytes(fragment));
    }
    output.push_back(Fmp4Output::Completed(completed));
    Ok(())
}

#[derive(Debug)]
pub enum Fmp4SinkError {
    UnexpectedTimebase,
    MissingSequenceParameterSet,
    MissingPictureParameterSet,
    OutputPending,
    SegmentTooLarge,
    FragmentTooLarge,
    TooManySamples,
    NalTooLarge,
    TimestampOutOfRange,
    NoActiveSegment,
    EmptySegment,
    Muxer(FragmentedError),
}

impl fmt::Display for Fmp4SinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "fragmented-MP4 sink failure: {self:?}")
    }
}

impl Error for Fmp4SinkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Muxer(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Fmp4Output, Fmp4SegmentSink, Fmp4SinkConfig, Fmp4SinkError, SegmentDuration};
    use crate::recorder::H264AccessUnitSink;
    use std::{sync::Arc, time::Duration};
    use surveillance_media::packet::{
        H264AccessUnit, H264AccessUnitAssembler, H264NalUnit, MediaTimestamp,
    };

    fn config() -> Fmp4SinkConfig {
        Fmp4SinkConfig::new(
            640,
            360,
            SegmentDuration::new(Duration::from_secs(30))
                .unwrap_or_else(|error| panic!("valid duration: {error}")),
            1_048_576,
            1_000,
        )
        .unwrap_or_else(|error| panic!("valid config: {error}"))
    }

    fn access_unit(ticks: i64, keyframe: bool) -> H264AccessUnit {
        let timestamp = MediaTimestamp::new(ticks, 90_000)
            .unwrap_or_else(|error| panic!("valid timestamp: {error}"));
        let mut assembler = H264AccessUnitAssembler::new();
        let nals: &[&[u8]] = if keyframe {
            &[
                &[0x67, 0x42, 0x00, 0x1e, 0xaa],
                &[0x68, 0xce, 0x06, 0xe2],
                &[0x65, 0x88, 0x84],
            ]
        } else {
            &[&[0x41, 0x9a, 0x22]]
        };
        let last = nals.len().saturating_sub(1);
        let mut completed = None;
        for (index, bytes) in nals.iter().enumerate() {
            let nal = H264NalUnit::from_ebsp(Arc::from(*bytes))
                .unwrap_or_else(|error| panic!("valid NAL: {error}"));
            completed = assembler
                .push(timestamp, nal, index == last)
                .unwrap_or_else(|error| panic!("valid access unit: {error}"));
        }
        completed.unwrap_or_else(|| panic!("access unit must complete"))
    }

    #[test]
    fn rotates_only_on_keyframe_after_target_and_bounds_pending_output() {
        let mut sink = Fmp4SegmentSink::new(config());
        sink.write_access_unit(&access_unit(0, true))
            .unwrap_or_else(|error| panic!("first unit accepted: {error}"));
        let init = match sink.take_output() {
            Some(Fmp4Output::Bytes(bytes)) => bytes,
            output => panic!("expected init bytes, got {output:?}"),
        };
        sink.write_access_unit(&access_unit(2_700_000, false))
            .unwrap_or_else(|error| panic!("non-key boundary accepted: {error}"));
        let fragment = match sink.take_output() {
            Some(Fmp4Output::Bytes(bytes)) => bytes,
            output => panic!("expected media fragment, got {output:?}"),
        };
        sink.write_access_unit(&access_unit(2_706_000, true))
            .unwrap_or_else(|error| panic!("key boundary accepted: {error}"));
        let completed = match sink.take_output() {
            Some(Fmp4Output::Completed(completed)) => completed,
            output => panic!("expected completed boundary, got {output:?}"),
        };
        assert_eq!(completed.samples(), 2);
        assert!(init.windows(4).any(|bytes| bytes == b"ftyp"));
        assert!(init.windows(4).any(|bytes| bytes == b"moov"));
        assert!(fragment.windows(4).any(|bytes| bytes == b"moof"));
        assert!(fragment.windows(4).any(|bytes| bytes == b"mdat"));
        assert!(matches!(sink.take_output(), Some(Fmp4Output::Bytes(_))));
    }

    #[test]
    fn waits_for_parameterized_keyframe_and_rejects_unbounded_growth() {
        let mut sink = Fmp4SegmentSink::new(config());
        sink.write_access_unit(&access_unit(0, false))
            .unwrap_or_else(|error| panic!("non-keyframe is skipped: {error}"));
        assert_eq!(sink.waiting_for_keyframe(), 1);

        let tiny = Fmp4SinkConfig::new(
            640,
            360,
            SegmentDuration::new(Duration::from_secs(30))
                .unwrap_or_else(|error| panic!("valid duration: {error}")),
            4,
            1,
        )
        .unwrap_or_else(|error| panic!("valid bounded config: {error}"));
        let mut sink = Fmp4SegmentSink::new(tiny);
        assert!(matches!(
            sink.write_access_unit(&access_unit(0, true)),
            Err(Fmp4SinkError::FragmentTooLarge)
        ));
    }
}
