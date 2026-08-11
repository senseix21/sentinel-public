//! Bounded RTP/H.264 depayloading for RFC 6184 non-interleaved mode.

use std::{error::Error, fmt, sync::Arc};

use surveillance_media::packet::{H264NalError, H264NalUnit, MediaTimestamp};

const STAP_A_TYPE: u8 = 24;
const FU_A_TYPE: u8 = 28;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct H264DepayloaderLimits {
    max_nal_size: usize,
    max_nals_per_packet: usize,
}

impl H264DepayloaderLimits {
    /// Creates explicit allocation limits for untrusted RTP payloads.
    ///
    /// # Errors
    ///
    /// Returns [`LimitsError`] when either limit is zero.
    pub const fn new(max_nal_size: usize, max_nals_per_packet: usize) -> Result<Self, LimitsError> {
        if max_nal_size == 0 {
            return Err(LimitsError::ZeroMaxNalSize);
        }
        if max_nals_per_packet == 0 {
            return Err(LimitsError::ZeroMaxNalsPerPacket);
        }

        Ok(Self {
            max_nal_size,
            max_nals_per_packet,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitsError {
    ZeroMaxNalSize,
    ZeroMaxNalsPerPacket,
}

impl fmt::Display for LimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroMaxNalSize => formatter.write_str("maximum H.264 NAL size must be non-zero"),
            Self::ZeroMaxNalsPerPacket => {
                formatter.write_str("maximum H.264 NAL count per packet must be non-zero")
            }
        }
    }
}

impl Error for LimitsError {}

#[derive(Clone, Copy, Debug)]
pub struct RtpH264Payload<'a> {
    pub sequence_number: u16,
    pub timestamp: MediaTimestamp,
    pub marker: bool,
    pub payload: &'a [u8],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DepayloadedNal {
    pub timestamp: MediaTimestamp,
    pub nal: H264NalUnit,
    pub ends_access_unit: bool,
}

#[derive(Debug)]
struct FragmentedNal {
    timestamp: MediaTimestamp,
    next_sequence_number: u16,
    reconstructed_header: u8,
    bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct H264Depayloader {
    limits: H264DepayloaderLimits,
    fragment: Option<FragmentedNal>,
}

impl H264Depayloader {
    #[must_use]
    pub const fn new(limits: H264DepayloaderLimits) -> Self {
        Self {
            limits,
            fragment: None,
        }
    }

    /// Depayloads one RTP payload in single-NAL or non-interleaved mode.
    ///
    /// # Errors
    ///
    /// Returns [`DepayloadError`] for malformed, unsupported, oversized, or
    /// discontinuous input. An error involving FU-A clears partial fragment state.
    pub fn push(
        &mut self,
        packet: RtpH264Payload<'_>,
    ) -> Result<Vec<DepayloadedNal>, DepayloadError> {
        let Some(indicator) = packet.payload.first().copied() else {
            return Err(DepayloadError::EmptyPayload);
        };
        if indicator & 0x80 != 0 {
            return Err(DepayloadError::ForbiddenBitSet);
        }

        let unit_type = indicator & 0x1f;
        if self.fragment.is_some() && unit_type != FU_A_TYPE {
            self.fragment = None;
            return Err(DepayloadError::FragmentInterruptedByPacketType(unit_type));
        }
        match unit_type {
            1..=23 => self.single_nal(packet),
            STAP_A_TYPE => self.stap_a(packet),
            FU_A_TYPE => self.fu_a(packet),
            0 => Err(DepayloadError::UnspecifiedType),
            other => Err(DepayloadError::UnsupportedPacketType(other)),
        }
    }

    pub fn reset_for_discontinuity(&mut self) -> usize {
        self.fragment
            .take()
            .map_or(0, |fragment| fragment.bytes.len())
    }

    fn single_nal(
        &self,
        packet: RtpH264Payload<'_>,
    ) -> Result<Vec<DepayloadedNal>, DepayloadError> {
        self.check_nal_size(packet.payload.len())?;
        let nal = H264NalUnit::from_ebsp(Arc::from(packet.payload)).map_err(DepayloadError::Nal)?;
        Ok(vec![DepayloadedNal {
            timestamp: packet.timestamp,
            nal,
            ends_access_unit: packet.marker,
        }])
    }

    fn stap_a(&self, packet: RtpH264Payload<'_>) -> Result<Vec<DepayloadedNal>, DepayloadError> {
        let mut cursor = 1;
        let mut units = Vec::new();
        while cursor < packet.payload.len() {
            if units.len() == self.limits.max_nals_per_packet {
                return Err(DepayloadError::TooManyAggregatedNals {
                    limit: self.limits.max_nals_per_packet,
                });
            }
            let length_bytes = packet
                .payload
                .get(cursor..cursor + 2)
                .ok_or(DepayloadError::TruncatedStapLength)?;
            let nal_length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
            cursor += 2;
            if nal_length == 0 {
                return Err(DepayloadError::ZeroLengthStapNal);
            }
            self.check_nal_size(nal_length)?;
            let end = cursor
                .checked_add(nal_length)
                .ok_or(DepayloadError::TruncatedStapNal)?;
            let bytes = packet
                .payload
                .get(cursor..end)
                .ok_or(DepayloadError::TruncatedStapNal)?;
            let nal = H264NalUnit::from_ebsp(Arc::from(bytes)).map_err(DepayloadError::Nal)?;
            units.push(DepayloadedNal {
                timestamp: packet.timestamp,
                nal,
                ends_access_unit: false,
            });
            cursor = end;
        }

        let Some(last) = units.last_mut() else {
            return Err(DepayloadError::EmptyStapA);
        };
        last.ends_access_unit = packet.marker;
        Ok(units)
    }

    fn fu_a(&mut self, packet: RtpH264Payload<'_>) -> Result<Vec<DepayloadedNal>, DepayloadError> {
        let result = self.fu_a_inner(packet);
        if result.is_err() {
            self.fragment = None;
        }
        result
    }

    fn fu_a_inner(
        &mut self,
        packet: RtpH264Payload<'_>,
    ) -> Result<Vec<DepayloadedNal>, DepayloadError> {
        let fu_header = packet
            .payload
            .get(1)
            .copied()
            .ok_or(DepayloadError::TruncatedFuA)?;
        let fragment_payload = packet
            .payload
            .get(2..)
            .filter(|payload| !payload.is_empty())
            .ok_or(DepayloadError::EmptyFuPayload)?;
        let start = fu_header & 0x80 != 0;
        let end = fu_header & 0x40 != 0;
        if start && end {
            return Err(DepayloadError::FuStartAndEndBothSet);
        }
        if packet.marker && !end {
            return Err(DepayloadError::MarkerBeforeFuEnd);
        }
        if fu_header & 0x20 != 0 {
            return Err(DepayloadError::FuReservedBitSet);
        }

        let original_type = fu_header & 0x1f;
        if !(1..=23).contains(&original_type) {
            return Err(DepayloadError::InvalidFuNalType(original_type));
        }
        let reconstructed_header = (packet.payload[0] & 0xe0) | original_type;

        if start {
            if self.fragment.is_some() {
                return Err(DepayloadError::FragmentAlreadyInProgress);
            }
            let length =
                fragment_payload
                    .len()
                    .checked_add(1)
                    .ok_or(DepayloadError::NalTooLarge {
                        size: usize::MAX,
                        limit: self.limits.max_nal_size,
                    })?;
            self.check_nal_size(length)?;
            let mut bytes = Vec::with_capacity(length);
            bytes.push(reconstructed_header);
            bytes.extend_from_slice(fragment_payload);
            self.fragment = Some(FragmentedNal {
                timestamp: packet.timestamp,
                next_sequence_number: packet.sequence_number.wrapping_add(1),
                reconstructed_header,
                bytes,
            });
            return Ok(Vec::new());
        }

        let fragment = self
            .fragment
            .as_mut()
            .ok_or(DepayloadError::FragmentContinuationWithoutStart)?;
        if packet.sequence_number != fragment.next_sequence_number {
            return Err(DepayloadError::FragmentSequenceGap {
                expected: fragment.next_sequence_number,
                received: packet.sequence_number,
            });
        }
        if packet.timestamp != fragment.timestamp {
            return Err(DepayloadError::FragmentTimestampChanged);
        }
        if reconstructed_header != fragment.reconstructed_header {
            return Err(DepayloadError::FragmentHeaderChanged);
        }
        let new_length = fragment
            .bytes
            .len()
            .checked_add(fragment_payload.len())
            .ok_or(DepayloadError::NalTooLarge {
                size: usize::MAX,
                limit: self.limits.max_nal_size,
            })?;
        if new_length > self.limits.max_nal_size {
            return Err(DepayloadError::NalTooLarge {
                size: new_length,
                limit: self.limits.max_nal_size,
            });
        }
        fragment.bytes.extend_from_slice(fragment_payload);
        fragment.next_sequence_number = packet.sequence_number.wrapping_add(1);

        if !end {
            return Ok(Vec::new());
        }

        let completed = self
            .fragment
            .take()
            .ok_or(DepayloadError::FragmentContinuationWithoutStart)?;
        let nal =
            H264NalUnit::from_ebsp(Arc::from(completed.bytes)).map_err(DepayloadError::Nal)?;
        Ok(vec![DepayloadedNal {
            timestamp: packet.timestamp,
            nal,
            ends_access_unit: packet.marker,
        }])
    }

    fn check_nal_size(&self, size: usize) -> Result<(), DepayloadError> {
        if size > self.limits.max_nal_size {
            return Err(DepayloadError::NalTooLarge {
                size,
                limit: self.limits.max_nal_size,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DepayloadError {
    EmptyPayload,
    ForbiddenBitSet,
    UnspecifiedType,
    UnsupportedPacketType(u8),
    Nal(H264NalError),
    NalTooLarge { size: usize, limit: usize },
    EmptyStapA,
    TruncatedStapLength,
    ZeroLengthStapNal,
    TruncatedStapNal,
    TooManyAggregatedNals { limit: usize },
    TruncatedFuA,
    EmptyFuPayload,
    FuStartAndEndBothSet,
    FuReservedBitSet,
    InvalidFuNalType(u8),
    FragmentAlreadyInProgress,
    FragmentInterruptedByPacketType(u8),
    FragmentContinuationWithoutStart,
    FragmentSequenceGap { expected: u16, received: u16 },
    FragmentTimestampChanged,
    FragmentHeaderChanged,
    MarkerBeforeFuEnd,
}

impl fmt::Display for DepayloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid RTP/H.264 payload: {self:?}")
    }
}

impl Error for DepayloadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Nal(source) => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use surveillance_media::packet::{H264AccessUnitAssembler, H264NalKind};

    use super::*;

    fn timestamp(ticks: i64) -> MediaTimestamp {
        MediaTimestamp::new(ticks, 90_000).expect("test timestamp is valid")
    }

    fn depayloader() -> H264Depayloader {
        H264Depayloader::new(H264DepayloaderLimits::new(64, 4).expect("test limits are valid"))
    }

    fn packet(sequence_number: u16, marker: bool, payload: &[u8]) -> RtpH264Payload<'_> {
        RtpH264Payload {
            sequence_number,
            timestamp: timestamp(90_000),
            marker,
            payload,
        }
    }

    #[test]
    fn depayloads_single_nal_and_propagates_marker() {
        let units = depayloader()
            .push(packet(1, true, &[0x65, 1, 2]))
            .expect("single NAL is valid");

        assert_eq!(units.len(), 1);
        assert_eq!(units[0].nal.kind(), H264NalKind::IdrSlice);
        assert!(units[0].ends_access_unit);
    }

    #[test]
    fn depayloads_stap_a_and_marks_only_last_nal() {
        let payload = [24, 0, 2, 0x67, 1, 0, 2, 0x68, 2];
        let units = depayloader()
            .push(packet(2, true, &payload))
            .expect("STAP-A is valid");

        assert_eq!(units.len(), 2);
        assert_eq!(units[0].nal.kind(), H264NalKind::SequenceParameterSet);
        assert!(!units[0].ends_access_unit);
        assert_eq!(units[1].nal.kind(), H264NalKind::PictureParameterSet);
        assert!(units[1].ends_access_unit);
    }

    #[test]
    fn reconstructs_fu_a_and_wraps_sequence_number() {
        let mut depayloader = depayloader();
        assert!(depayloader
            .push(packet(u16::MAX, false, &[0x7c, 0x85, 1, 2]))
            .expect("FU-A start is valid")
            .is_empty());
        let units = depayloader
            .push(packet(0, true, &[0x7c, 0x45, 3, 4]))
            .expect("FU-A end is valid");

        assert_eq!(units[0].nal.bytes(), &[0x65, 1, 2, 3, 4]);
        assert!(units[0].ends_access_unit);
    }

    #[test]
    fn sequence_gap_discards_fragment_state() {
        let mut depayloader = depayloader();
        depayloader
            .push(packet(10, false, &[0x7c, 0x81, 1]))
            .expect("FU-A start is valid");

        assert_eq!(
            depayloader.push(packet(12, true, &[0x7c, 0x41, 2])),
            Err(DepayloadError::FragmentSequenceGap {
                expected: 11,
                received: 12,
            })
        );
        assert_eq!(
            depayloader.push(packet(13, true, &[0x7c, 0x41, 3])),
            Err(DepayloadError::FragmentContinuationWithoutStart)
        );
    }

    #[test]
    fn unrelated_packet_and_early_marker_discard_fragment_state() {
        let mut depayloader = depayloader();
        depayloader
            .push(packet(30, false, &[0x7c, 0x81, 1]))
            .expect("FU-A start is valid");
        assert_eq!(
            depayloader.push(packet(31, true, &[0x61, 2])),
            Err(DepayloadError::FragmentInterruptedByPacketType(1))
        );

        assert_eq!(
            depayloader.push(packet(40, true, &[0x7c, 0x81, 1])),
            Err(DepayloadError::MarkerBeforeFuEnd)
        );
        assert_eq!(depayloader.reset_for_discontinuity(), 0);
    }

    #[test]
    fn rejects_truncated_and_oversized_payloads() {
        let mut depayloader =
            H264Depayloader::new(H264DepayloaderLimits::new(3, 2).expect("test limits are valid"));

        assert_eq!(
            depayloader.push(packet(1, false, &[24, 0])),
            Err(DepayloadError::TruncatedStapLength)
        );
        assert_eq!(
            depayloader.push(packet(2, false, &[0x61, 1, 2, 3])),
            Err(DepayloadError::NalTooLarge { size: 4, limit: 3 })
        );
        assert_eq!(
            depayloader.push(packet(3, false, &[0x7c, 0xc1, 1])),
            Err(DepayloadError::FuStartAndEndBothSet)
        );
    }

    #[test]
    fn feeds_stap_a_and_multi_packet_idr_into_access_unit_assembler() {
        let mut depayloader = depayloader();
        let mut assembler = H264AccessUnitAssembler::new();
        let stap = [24, 0, 2, 0x67, 1, 0, 2, 0x68, 2];
        let packets = [
            packet(20, false, &stap),
            packet(21, false, &[0x7c, 0x85, 1, 2]),
            packet(22, true, &[0x7c, 0x45, 3, 4]),
        ];
        let mut completed = None;

        for rtp in packets {
            for unit in depayloader.push(rtp).expect("RTP payload is valid") {
                completed = assembler
                    .push(unit.timestamp, unit.nal, unit.ends_access_unit)
                    .expect("access-unit assembly succeeds")
                    .or(completed);
            }
        }

        let access_unit = completed.expect("RTP marker completes access unit");
        assert!(access_unit.is_keyframe());
        assert!(access_unit.has_parameter_sets());
        assert_eq!(access_unit.nal_units().len(), 3);
    }
}
