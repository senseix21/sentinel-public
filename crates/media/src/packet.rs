//! Encoded media packet representation.

use std::{error::Error, fmt, sync::Arc};

/// Timestamp attached by the depayloader to every NAL unit in one access unit.
///
/// `ticks` are meaningful only with `timebase_hz`; neither value is converted to
/// wall-clock time at this boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaTimestamp {
    ticks: i64,
    timebase_hz: u32,
}

impl MediaTimestamp {
    /// Creates a timestamp when the timebase is non-zero.
    ///
    /// # Errors
    ///
    /// Returns [`TimestampError::ZeroTimebase`] when `timebase_hz` is zero.
    pub fn new(ticks: i64, timebase_hz: u32) -> Result<Self, TimestampError> {
        if timebase_hz == 0 {
            return Err(TimestampError::ZeroTimebase);
        }

        Ok(Self { ticks, timebase_hz })
    }

    #[must_use]
    pub const fn ticks(self) -> i64 {
        self.ticks
    }

    #[must_use]
    pub const fn timebase_hz(self) -> u32 {
        self.timebase_hz
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimestampError {
    ZeroTimebase,
}

impl fmt::Display for TimestampError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTimebase => formatter.write_str("media timestamp timebase must be non-zero"),
        }
    }
}

impl Error for TimestampError {}

/// H.264 NAL unit classes needed at the distribution/publication boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum H264NalKind {
    NonIdrSlice,
    IdrSlice,
    SupplementalEnhancementInformation,
    SequenceParameterSet,
    PictureParameterSet,
    AccessUnitDelimiter,
    Other(u8),
}

/// One depayloaded H.264 NAL unit in EBSP form, including its one-byte header.
///
/// Annex-B start codes and RTP aggregation/fragmentation headers must already be
/// removed. Payload storage is immutable and reference counted because consumers
/// cross asynchronous ownership boundaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct H264NalUnit {
    bytes: Arc<[u8]>,
    kind: H264NalKind,
}

impl H264NalUnit {
    /// Validates a depayloaded NAL unit in EBSP form.
    ///
    /// # Errors
    ///
    /// Returns [`H264NalError`] for empty input, a set forbidden bit, an
    /// unspecified type, or an RTP payload header that still needs depayloading.
    pub fn from_ebsp(bytes: Arc<[u8]>) -> Result<Self, H264NalError> {
        let Some(header) = bytes.first().copied() else {
            return Err(H264NalError::Empty);
        };

        if header & 0x80 != 0 {
            return Err(H264NalError::ForbiddenBitSet);
        }

        let unit_type = header & 0x1f;
        let kind = match unit_type {
            1 => H264NalKind::NonIdrSlice,
            5 => H264NalKind::IdrSlice,
            6 => H264NalKind::SupplementalEnhancementInformation,
            7 => H264NalKind::SequenceParameterSet,
            8 => H264NalKind::PictureParameterSet,
            9 => H264NalKind::AccessUnitDelimiter,
            0 => return Err(H264NalError::UnspecifiedType),
            24..=31 => return Err(H264NalError::RtpPayloadHeader(unit_type)),
            _ => H264NalKind::Other(unit_type),
        };

        Ok(Self { bytes, kind })
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn kind(&self) -> H264NalKind {
        self.kind
    }

    #[must_use]
    pub const fn is_vcl(&self) -> bool {
        matches!(self.kind, H264NalKind::NonIdrSlice | H264NalKind::IdrSlice)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum H264NalError {
    Empty,
    ForbiddenBitSet,
    UnspecifiedType,
    RtpPayloadHeader(u8),
}

impl fmt::Display for H264NalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("H.264 NAL unit is empty"),
            Self::ForbiddenBitSet => formatter.write_str("H.264 forbidden_zero_bit is set"),
            Self::UnspecifiedType => formatter.write_str("H.264 NAL unit type zero is unspecified"),
            Self::RtpPayloadHeader(unit_type) => write!(
                formatter,
                "RTP H.264 payload header type {unit_type} must be depayloaded first"
            ),
        }
    }
}

impl Error for H264NalError {}

/// A complete H.264 presentation unit. It always contains at least one VCL NAL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct H264AccessUnit {
    timestamp: MediaTimestamp,
    connection_epoch: u64,
    nal_units: Arc<[H264NalUnit]>,
    keyframe: bool,
}

impl H264AccessUnit {
    #[must_use]
    pub const fn timestamp(&self) -> MediaTimestamp {
        self.timestamp
    }

    /// Returns zero until the camera lifecycle owner stamps this unit.
    #[must_use]
    pub const fn connection_epoch(&self) -> u64 {
        self.connection_epoch
    }

    /// Stamps the access unit at the camera-owned distribution boundary.
    ///
    /// # Errors
    /// Returns an error for epoch zero.
    pub fn with_connection_epoch(mut self, epoch: u64) -> Result<Self, AccessUnitError> {
        if epoch == 0 {
            return Err(AccessUnitError::ZeroConnectionEpoch);
        }
        self.connection_epoch = epoch;
        Ok(self)
    }

    #[must_use]
    pub fn nal_units(&self) -> &[H264NalUnit] {
        &self.nal_units
    }

    #[must_use]
    pub const fn is_keyframe(&self) -> bool {
        self.keyframe
    }

    #[must_use]
    pub fn has_parameter_sets(&self) -> bool {
        let mut found_sequence_parameter_set = false;
        let mut found_picture_parameter_set = false;
        for unit in self.nal_units.iter() {
            found_sequence_parameter_set |= unit.kind() == H264NalKind::SequenceParameterSet;
            found_picture_parameter_set |= unit.kind() == H264NalKind::PictureParameterSet;
        }
        found_sequence_parameter_set && found_picture_parameter_set
    }
}

impl crate::distribution::DecodableRunStart for H264AccessUnit {
    /// A decoder can start here only on an IDR that also carries the parameter
    /// sets describing it. A keyframe without them is not self-contained: a
    /// consumer joining at that point has no sequence or picture parameter set
    /// to decode it against.
    fn starts_decodable_run(&self) -> bool {
        self.is_keyframe() && self.has_parameter_sets()
    }
}

/// Incrementally groups depayloaded NAL units using the transport's explicit
/// end-of-access-unit marker.
#[derive(Debug, Default)]
pub struct H264AccessUnitAssembler {
    pending_timestamp: Option<MediaTimestamp>,
    pending: Vec<H264NalUnit>,
}

impl H264AccessUnitAssembler {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending_timestamp: None,
            pending: Vec::new(),
        }
    }

    /// Adds a depayloaded NAL. `ends_access_unit` must come from a boundary that
    /// understands the source transport (for RTP, normally the marker bit after
    /// FU-A/STAP-A reconstruction).
    ///
    /// # Errors
    ///
    /// Returns [`AccessUnitError::TimestampChangedBeforeBoundary`] if a new
    /// timestamp arrives while a unit is incomplete, or
    /// [`AccessUnitError::MissingVclNal`] if a completed unit contains no picture.
    pub fn push(
        &mut self,
        timestamp: MediaTimestamp,
        nal: H264NalUnit,
        ends_access_unit: bool,
    ) -> Result<Option<H264AccessUnit>, AccessUnitError> {
        if let Some(pending) = self.pending_timestamp {
            if pending != timestamp {
                return Err(AccessUnitError::TimestampChangedBeforeBoundary {
                    pending,
                    received: timestamp,
                });
            }
        } else {
            self.pending_timestamp = Some(timestamp);
        }

        self.pending.push(nal);

        if !ends_access_unit {
            return Ok(None);
        }

        let nal_units = std::mem::take(&mut self.pending);
        self.pending_timestamp = None;
        let has_vcl = nal_units.iter().any(H264NalUnit::is_vcl);
        if !has_vcl {
            return Err(AccessUnitError::MissingVclNal);
        }
        let keyframe = nal_units
            .iter()
            .any(|unit| unit.kind() == H264NalKind::IdrSlice);

        Ok(Some(H264AccessUnit {
            timestamp,
            connection_epoch: 0,
            nal_units: nal_units.into(),
            keyframe,
        }))
    }

    /// Discards an incomplete unit at an explicit stream discontinuity.
    /// Returns the number of discarded NAL units for metrics.
    pub fn reset_for_discontinuity(&mut self) -> usize {
        self.pending_timestamp = None;
        let discarded = self.pending.len();
        self.pending.clear();
        discarded
    }

    #[must_use]
    pub fn pending_nal_count(&self) -> usize {
        self.pending.len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessUnitError {
    TimestampChangedBeforeBoundary {
        pending: MediaTimestamp,
        received: MediaTimestamp,
    },
    MissingVclNal,
    ZeroConnectionEpoch,
}

impl fmt::Display for AccessUnitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimestampChangedBeforeBoundary { pending, received } => write!(
                formatter,
                "H.264 timestamp changed before access-unit boundary: {pending:?} -> {received:?}"
            ),
            Self::MissingVclNal => {
                formatter.write_str("completed H.264 access unit contains no VCL NAL")
            }
            Self::ZeroConnectionEpoch => {
                formatter.write_str("H.264 access unit connection epoch must be nonzero")
            }
        }
    }
}

impl Error for AccessUnitError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp(ticks: i64) -> MediaTimestamp {
        MediaTimestamp::new(ticks, 90_000).expect("test timestamp is valid")
    }

    fn nal(header: u8) -> H264NalUnit {
        H264NalUnit::from_ebsp(Arc::from([header, 0xaa])).expect("test NAL is valid")
    }

    #[test]
    fn rejects_zero_timebase() {
        assert_eq!(MediaTimestamp::new(1, 0), Err(TimestampError::ZeroTimebase));
    }

    #[test]
    fn rejects_malformed_or_not_yet_depayloaded_nals() {
        assert_eq!(
            H264NalUnit::from_ebsp(Arc::from([])),
            Err(H264NalError::Empty)
        );
        assert_eq!(
            H264NalUnit::from_ebsp(Arc::from([0x81])),
            Err(H264NalError::ForbiddenBitSet)
        );
        assert_eq!(
            H264NalUnit::from_ebsp(Arc::from([24])),
            Err(H264NalError::RtpPayloadHeader(24))
        );
    }

    #[test]
    fn assembles_parameter_sets_and_multiple_idr_slices_once() {
        let mut assembler = H264AccessUnitAssembler::new();
        let ts = timestamp(180_000);

        assert_eq!(assembler.push(ts, nal(7), false), Ok(None));
        assert_eq!(assembler.push(ts, nal(8), false), Ok(None));
        assert_eq!(assembler.push(ts, nal(5), false), Ok(None));
        let unit = assembler
            .push(ts, nal(5), true)
            .expect("assembly succeeds")
            .expect("marker completes access unit");

        assert_eq!(unit.timestamp(), ts);
        assert!(unit.is_keyframe());
        assert!(unit.has_parameter_sets());
        assert_eq!(unit.nal_units().len(), 4);
        assert_eq!(assembler.pending_nal_count(), 0);
    }

    #[test]
    fn classifies_non_idr_access_unit() {
        let mut assembler = H264AccessUnitAssembler::new();
        let unit = assembler
            .push(timestamp(90_000), nal(1), true)
            .expect("assembly succeeds")
            .expect("marker completes access unit");

        assert!(!unit.is_keyframe());
    }

    #[test]
    fn timestamp_change_requires_explicit_discontinuity() {
        let mut assembler = H264AccessUnitAssembler::new();
        let first = timestamp(90_000);
        let second = timestamp(180_000);

        assert_eq!(assembler.push(first, nal(1), false), Ok(None));
        assert_eq!(
            assembler.push(second, nal(1), true),
            Err(AccessUnitError::TimestampChangedBeforeBoundary {
                pending: first,
                received: second,
            })
        );
        assert_eq!(assembler.pending_nal_count(), 1);
        assert_eq!(assembler.reset_for_discontinuity(), 1);
        assert_eq!(assembler.pending_nal_count(), 0);
        assert!(assembler
            .push(second, nal(1), true)
            .expect("assembly succeeds after reset")
            .is_some());
    }

    #[test]
    fn completed_parameter_sets_without_picture_are_rejected_and_cleared() {
        let mut assembler = H264AccessUnitAssembler::new();
        let result = assembler.push(timestamp(90_000), nal(7), true);

        assert_eq!(result, Err(AccessUnitError::MissingVclNal));
        assert_eq!(assembler.pending_nal_count(), 0);
    }
}
