//! Maps H.264 access units into bounded RTP timestamps and payloads for publication.

#![allow(
    clippy::missing_errors_doc,
    reason = "adaptation methods share the typed AdaptationError contract"
)]
#![allow(
    clippy::struct_field_names,
    reason = "max prefixes make independent safety bounds explicit"
)]
use std::{error::Error, fmt, sync::Arc};

use surveillance_media::packet::{H264AccessUnit, H264NalKind, MediaTimestamp};

pub const H264_RTP_CLOCK_HZ: u32 = 90_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicationEpoch(u64);

impl PublicationEpoch {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Default)]
pub struct RtpTimestampMapper {
    epoch: Option<PublicationEpoch>,
    first: Option<MediaTimestamp>,
    last_unwrapped: Option<u64>,
}

impl RtpTimestampMapper {
    pub fn map(
        &mut self,
        epoch: PublicationEpoch,
        timestamp: MediaTimestamp,
    ) -> Result<u32, AdaptationError> {
        if self.epoch != Some(epoch) {
            self.epoch = Some(epoch);
            self.first = Some(timestamp);
            self.last_unwrapped = Some(0);
            return Ok(0);
        }
        let first = self.first.ok_or(AdaptationError::TimestampDiscontinuity)?;
        if timestamp.timebase_hz() != first.timebase_hz() || timestamp.ticks() < first.ticks() {
            return Err(AdaptationError::TimestampDiscontinuity);
        }
        let delta = u128::try_from(timestamp.ticks() - first.ticks())
            .map_err(|_| AdaptationError::TimestampOverflow)?;
        let scaled = delta
            .checked_mul(u128::from(H264_RTP_CLOCK_HZ))
            .ok_or(AdaptationError::TimestampOverflow)?;
        let rounded = scaled
            .checked_add(u128::from(timestamp.timebase_hz() / 2))
            .ok_or(AdaptationError::TimestampOverflow)?
            / u128::from(timestamp.timebase_hz());
        let unwrapped = u64::try_from(rounded).map_err(|_| AdaptationError::TimestampOverflow)?;
        if self.last_unwrapped.is_some_and(|last| unwrapped < last) {
            return Err(AdaptationError::TimestampDiscontinuity);
        }
        self.last_unwrapped = Some(unwrapped);
        Ok(u32::try_from(unwrapped & u64::from(u32::MAX))
            .unwrap_or_else(|_| unreachable!("value is masked to the RTP timestamp width")))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RtpPayload {
    pub timestamp: u32,
    pub marker: bool,
    pub bytes: Arc<[u8]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketizationConfig {
    max_payload_bytes: usize,
    max_access_unit_bytes: usize,
    max_packets_per_access_unit: usize,
}

impl PacketizationConfig {
    pub fn new(
        max_payload_bytes: usize,
        max_access_unit_bytes: usize,
        max_packets_per_access_unit: usize,
    ) -> Result<Self, AdaptationError> {
        if max_payload_bytes < 3 || max_access_unit_bytes == 0 || max_packets_per_access_unit == 0 {
            return Err(AdaptationError::InvalidBounds);
        }
        Ok(Self {
            max_payload_bytes,
            max_access_unit_bytes,
            max_packets_per_access_unit,
        })
    }
}

#[derive(Debug)]
pub struct H264Adapter {
    config: PacketizationConfig,
    mapper: RtpTimestampMapper,
    sps: Option<Arc<[u8]>>,
    pps: Option<Arc<[u8]>>,
}

impl H264Adapter {
    #[must_use]
    pub fn new(config: PacketizationConfig) -> Self {
        Self {
            config,
            mapper: RtpTimestampMapper::default(),
            sps: None,
            pps: None,
        }
    }

    pub fn packetize(
        &mut self,
        epoch: PublicationEpoch,
        access_unit: &H264AccessUnit,
    ) -> Result<Vec<RtpPayload>, AdaptationError> {
        let total_bytes = access_unit
            .nal_units()
            .iter()
            .try_fold(0_usize, |total, nal| total.checked_add(nal.bytes().len()))
            .ok_or(AdaptationError::OversizedAccessUnit)?;
        if total_bytes > self.config.max_access_unit_bytes {
            return Err(AdaptationError::OversizedAccessUnit);
        }
        for nal in access_unit.nal_units() {
            match nal.kind() {
                H264NalKind::SequenceParameterSet => self.sps = Some(Arc::from(nal.bytes())),
                H264NalKind::PictureParameterSet => self.pps = Some(Arc::from(nal.bytes())),
                _ => {}
            }
        }
        if access_unit.is_keyframe() && (self.sps.is_none() || self.pps.is_none()) {
            return Err(AdaptationError::MissingParameterSets);
        }
        let timestamp = self.mapper.map(epoch, access_unit.timestamp())?;
        let mut packets = Vec::new();
        if access_unit.is_keyframe() {
            for parameter_set in [&self.sps, &self.pps].into_iter().flatten() {
                packetize_nal(parameter_set, timestamp, self.config, &mut packets)?;
            }
        }
        for nal in access_unit.nal_units() {
            if !matches!(
                nal.kind(),
                H264NalKind::SequenceParameterSet | H264NalKind::PictureParameterSet
            ) {
                packetize_nal(nal.bytes(), timestamp, self.config, &mut packets)?;
            }
        }
        let Some(last) = packets.last_mut() else {
            return Err(AdaptationError::MalformedAccessUnit);
        };
        last.marker = true;
        Ok(packets)
    }

    pub fn annex_b(&mut self, access_unit: &H264AccessUnit) -> Result<Vec<u8>, AdaptationError> {
        for nal in access_unit.nal_units() {
            match nal.kind() {
                H264NalKind::SequenceParameterSet => self.sps = Some(Arc::from(nal.bytes())),
                H264NalKind::PictureParameterSet => self.pps = Some(Arc::from(nal.bytes())),
                _ => {}
            }
        }
        if access_unit.is_keyframe() && (self.sps.is_none() || self.pps.is_none()) {
            return Err(AdaptationError::MissingParameterSets);
        }
        let mut output = Vec::new();
        if access_unit.is_keyframe() {
            for parameter_set in [&self.sps, &self.pps].into_iter().flatten() {
                append_annex_b(
                    &mut output,
                    parameter_set,
                    self.config.max_access_unit_bytes,
                )?;
            }
        }
        for nal in access_unit.nal_units() {
            if !matches!(
                nal.kind(),
                H264NalKind::SequenceParameterSet | H264NalKind::PictureParameterSet
            ) {
                append_annex_b(&mut output, nal.bytes(), self.config.max_access_unit_bytes)?;
            }
        }
        if output.is_empty() {
            return Err(AdaptationError::MalformedAccessUnit);
        }
        Ok(output)
    }

    pub fn timestamp(
        &mut self,
        epoch: PublicationEpoch,
        access_unit: &H264AccessUnit,
    ) -> Result<u32, AdaptationError> {
        self.mapper.map(epoch, access_unit.timestamp())
    }
}

fn append_annex_b(output: &mut Vec<u8>, nal: &[u8], maximum: usize) -> Result<(), AdaptationError> {
    let required = output
        .len()
        .checked_add(4)
        .and_then(|value| value.checked_add(nal.len()))
        .ok_or(AdaptationError::OversizedAccessUnit)?;
    if required > maximum {
        return Err(AdaptationError::OversizedAccessUnit);
    }
    output.extend_from_slice(&[0, 0, 0, 1]);
    output.extend_from_slice(nal);
    Ok(())
}

fn packetize_nal(
    nal: &[u8],
    timestamp: u32,
    config: PacketizationConfig,
    output: &mut Vec<RtpPayload>,
) -> Result<(), AdaptationError> {
    let Some(header) = nal.first().copied() else {
        return Err(AdaptationError::MalformedAccessUnit);
    };
    if nal.len() <= config.max_payload_bytes {
        push_packet(
            output,
            RtpPayload {
                timestamp,
                marker: false,
                bytes: Arc::from(nal),
            },
            config,
        )?;
        return Ok(());
    }
    let fragment_size = config.max_payload_bytes - 2;
    let nal_type = header & 0x1f;
    let fu_indicator = (header & 0xe0) | 28;
    let body = &nal[1..];
    for (index, chunk) in body.chunks(fragment_size).enumerate() {
        let mut bytes = Vec::with_capacity(chunk.len() + 2);
        bytes.push(fu_indicator);
        let mut fu_header = nal_type;
        if index == 0 {
            fu_header |= 0x80;
        }
        if (index + 1) * fragment_size >= body.len() {
            fu_header |= 0x40;
        }
        bytes.push(fu_header);
        bytes.extend_from_slice(chunk);
        push_packet(
            output,
            RtpPayload {
                timestamp,
                marker: false,
                bytes: bytes.into(),
            },
            config,
        )?;
    }
    Ok(())
}

fn push_packet(
    output: &mut Vec<RtpPayload>,
    packet: RtpPayload,
    config: PacketizationConfig,
) -> Result<(), AdaptationError> {
    if output.len() >= config.max_packets_per_access_unit {
        return Err(AdaptationError::TooManyPackets);
    }
    output.push(packet);
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdaptationError {
    InvalidBounds,
    MissingParameterSets,
    TimestampDiscontinuity,
    TimestampOverflow,
    OversizedAccessUnit,
    TooManyPackets,
    MalformedAccessUnit,
}

impl fmt::Display for AdaptationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "H.264 publication adaptation failed: {self:?}")
    }
}
impl Error for AdaptationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use surveillance_media::packet::{H264AccessUnitAssembler, H264NalUnit};

    fn unit(timestamp: i64, nals: &[&[u8]]) -> H264AccessUnit {
        let mut assembler = H264AccessUnitAssembler::new();
        let timestamp =
            MediaTimestamp::new(timestamp, 1_000).unwrap_or_else(|error| panic!("{error}"));
        let mut result = None;
        for (index, bytes) in nals.iter().enumerate() {
            let nal =
                H264NalUnit::from_ebsp(Arc::from(*bytes)).unwrap_or_else(|error| panic!("{error}"));
            result = assembler
                .push(timestamp, nal, index + 1 == nals.len())
                .unwrap_or_else(|error| panic!("{error}"));
        }
        result.unwrap_or_else(|| panic!("complete access unit"))
    }

    #[test]
    fn multi_slice_access_unit_has_one_timestamp_and_one_marker() {
        let config =
            PacketizationConfig::new(5, 1024, 64).unwrap_or_else(|error| panic!("{error}"));
        let mut adapter = H264Adapter::new(config);
        let access_unit = unit(
            500,
            &[
                &[0x67, 100, 0],
                &[0x68, 0],
                &[0x65, 1, 2, 3, 4, 5],
                &[0x65, 6, 7],
            ],
        );
        let packets = adapter
            .packetize(PublicationEpoch::new(1), &access_unit)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(packets.iter().all(|packet| packet.timestamp == 0));
        assert_eq!(packets.iter().filter(|packet| packet.marker).count(), 1);
        assert!(packets.len() > access_unit.nal_units().len());
    }

    #[test]
    fn timestamp_rounding_wraparound_and_epoch_reset_are_deterministic() {
        let mut mapper = RtpTimestampMapper::default();
        assert_eq!(
            mapper.map(
                PublicationEpoch::new(1),
                MediaTimestamp::new(10, 1_000).unwrap_or_else(|error| panic!("{error}"))
            ),
            Ok(0)
        );
        assert_eq!(
            mapper.map(
                PublicationEpoch::new(1),
                MediaTimestamp::new(510, 1_000).unwrap_or_else(|error| panic!("{error}"))
            ),
            Ok(45_000)
        );
        assert_eq!(
            mapper.map(
                PublicationEpoch::new(2),
                MediaTimestamp::new(-100, 90_000).unwrap_or_else(|error| panic!("{error}"))
            ),
            Ok(0)
        );
    }

    #[test]
    fn keyframe_without_parameter_sets_is_rejected() {
        let config =
            PacketizationConfig::new(1200, 1024, 64).unwrap_or_else(|error| panic!("{error}"));
        let mut adapter = H264Adapter::new(config);
        assert_eq!(
            adapter
                .packetize(PublicationEpoch::new(1), &unit(0, &[&[0x65, 1]]))
                .err(),
            Some(AdaptationError::MissingParameterSets)
        );
    }
}
