//! Coded picture dimensions read from an H.264 sequence parameter set.
//!
//! A publisher that has to declare a track resolution must not invent one. The
//! stream already carries its own answer in the SPS that every parameterized
//! keyframe includes, so this reads it there and reports a typed failure when
//! the bytes do not support a definite answer.
//!
//! This models the subset of ITU-T H.264 clause 7.3.2.1.1 that determines
//! picture size. Fields that do not affect the size are parsed only far enough
//! to stay aligned, and anything this parser cannot follow is reported rather
//! than guessed.

use crate::packet::{H264AccessUnit, H264NalKind};

/// The picture size a decoder produces for a stream, in luma samples.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodedDimensions {
    width: u32,
    height: u32,
}

impl CodedDimensions {
    /// Reads the coded size from the access unit's sequence parameter set.
    ///
    /// # Errors
    ///
    /// Returns [`SpsError::Missing`] when the access unit carries no sequence
    /// parameter set, [`SpsError::Truncated`] when one ends mid-field,
    /// [`SpsError::Unsupported`] when it uses a construct this parser does not
    /// model, and [`SpsError::Degenerate`] when the fields decode to a picture
    /// with no samples.
    pub fn from_access_unit(access_unit: &H264AccessUnit) -> Result<Self, SpsError> {
        let sequence_parameter_set = access_unit
            .nal_units()
            .iter()
            .find(|unit| unit.kind() == H264NalKind::SequenceParameterSet)
            .ok_or(SpsError::Missing)?;
        Self::from_ebsp(sequence_parameter_set.bytes())
    }

    /// Reads the coded size from one sequence parameter set NAL unit.
    ///
    /// `ebsp` is the complete NAL unit including its one-byte header, still
    /// carrying emulation prevention bytes.
    ///
    /// # Errors
    ///
    /// As [`CodedDimensions::from_access_unit`].
    pub fn from_ebsp(ebsp: &[u8]) -> Result<Self, SpsError> {
        let payload = ebsp.get(1..).ok_or(SpsError::Truncated)?;
        let rbsp = remove_emulation_prevention(payload);
        Self::from_rbsp(&rbsp)
    }

    /// Reads the coded size from an Annex-B byte stream.
    ///
    /// For callers holding serialised bytes rather than a parsed access unit,
    /// such as a publication adapter that receives what it is about to send.
    ///
    /// # Errors
    ///
    /// As [`CodedDimensions::from_access_unit`].
    pub fn from_annex_b(stream: &[u8]) -> Result<Self, SpsError> {
        for nal in AnnexBNalUnits::new(stream) {
            // The NAL header's low five bits are its type; 7 is the sequence
            // parameter set.
            if nal.first().is_some_and(|header| header & 0x1f == 7) {
                return Self::from_ebsp(nal);
            }
        }
        Err(SpsError::Missing)
    }

    /// The picture width in luma samples.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// The picture height in luma samples.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    fn from_rbsp(rbsp: &[u8]) -> Result<Self, SpsError> {
        let mut bits = BitReader::new(rbsp);
        let profile_idc = bits.read_bits(8)?;
        let _constraint_flags = bits.read_bits(8)?;
        let _level_idc = bits.read_bits(8)?;
        let _seq_parameter_set_id = bits.read_unsigned_exp_golomb()?;

        let chroma = ChromaSampling::read(&mut bits, profile_idc)?;
        let _log2_max_frame_num_minus4 = bits.read_unsigned_exp_golomb()?;
        skip_picture_order_count(&mut bits)?;
        let _max_num_ref_frames = bits.read_unsigned_exp_golomb()?;
        let _gaps_in_frame_num_value_allowed_flag = bits.read_flag()?;
        let pic_width_in_mbs_minus1 = bits.read_unsigned_exp_golomb()?;
        let pic_height_in_map_units_minus1 = bits.read_unsigned_exp_golomb()?;
        let frame_mbs_only = bits.read_flag()?;
        if !frame_mbs_only {
            let _mb_adaptive_frame_field_flag = bits.read_flag()?;
        }
        let _direct_8x8_inference_flag = bits.read_flag()?;

        let (mut crop_left, mut crop_right, mut crop_top, mut crop_bottom) = (0, 0, 0, 0);
        if bits.read_flag()? {
            crop_left = bits.read_unsigned_exp_golomb()?;
            crop_right = bits.read_unsigned_exp_golomb()?;
            crop_top = bits.read_unsigned_exp_golomb()?;
            crop_bottom = bits.read_unsigned_exp_golomb()?;
        }

        // A map unit is a frame's worth of macroblocks only when the stream is
        // frame-coded; field coding halves it, which doubles the frame height.
        let field_factor = if frame_mbs_only { 1 } else { 2 };
        let width_in_mbs = pic_width_in_mbs_minus1
            .checked_add(1)
            .ok_or(SpsError::Unsupported)?;
        let height_in_mbs = pic_height_in_map_units_minus1
            .checked_add(1)
            .and_then(|units| units.checked_mul(field_factor))
            .ok_or(SpsError::Unsupported)?;

        let (crop_unit_x, crop_unit_y) = chroma.crop_units(field_factor);

        let width = width_in_mbs
            .checked_mul(16)
            .and_then(|samples| {
                let cropped = crop_left
                    .checked_add(crop_right)?
                    .checked_mul(crop_unit_x)?;
                samples.checked_sub(cropped)
            })
            .ok_or(SpsError::Degenerate)?;
        let height = height_in_mbs
            .checked_mul(16)
            .and_then(|samples| {
                let cropped = crop_top
                    .checked_add(crop_bottom)?
                    .checked_mul(crop_unit_y)?;
                samples.checked_sub(cropped)
            })
            .ok_or(SpsError::Degenerate)?;

        let width = u32::try_from(width).map_err(|_| SpsError::Degenerate)?;
        let height = u32::try_from(height).map_err(|_| SpsError::Degenerate)?;
        if width == 0 || height == 0 {
            return Err(SpsError::Degenerate);
        }
        Ok(Self { width, height })
    }
}

/// Profiles whose sequence parameter set signals chroma format and scaling
/// matrices, per ITU-T H.264 clause 7.3.2.1.1.
const HIGH_PROFILES: [u64; 13] = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135];

/// How chroma is sampled relative to luma, which is what scales the frame
/// cropping offsets from chroma units back into luma samples.
#[derive(Clone, Copy)]
struct ChromaSampling {
    array_type: u64,
}

impl ChromaSampling {
    /// Reads the chroma signalling and consumes the profile-dependent fields
    /// that follow it, leaving the reader positioned at
    /// `log2_max_frame_num_minus4`.
    fn read(bits: &mut BitReader<'_>, profile_idc: u64) -> Result<Self, SpsError> {
        // Only the profiles that can vary chroma sampling signal it. Every
        // other profile is 4:2:0 by definition, so the absence of the field is
        // itself the answer rather than a reason to guess.
        if !HIGH_PROFILES.contains(&profile_idc) {
            return Ok(Self { array_type: 1 });
        }

        let chroma_format_idc = bits.read_unsigned_exp_golomb()?;
        if chroma_format_idc > 3 {
            return Err(SpsError::Unsupported);
        }
        let separate_colour_plane = chroma_format_idc == 3 && bits.read_flag()?;
        let _bit_depth_luma_minus8 = bits.read_unsigned_exp_golomb()?;
        let _bit_depth_chroma_minus8 = bits.read_unsigned_exp_golomb()?;
        let _qpprime_y_zero_transform_bypass_flag = bits.read_flag()?;
        if bits.read_flag()? {
            let lists = if chroma_format_idc == 3 { 12 } else { 8 };
            for index in 0..lists {
                if bits.read_flag()? {
                    let size = if index < 6 { 16 } else { 64 };
                    skip_scaling_list(bits, size)?;
                }
            }
        }

        // Separately coded colour planes are each monochrome, so cropping is
        // in whole luma samples regardless of the signalled format.
        Ok(Self {
            array_type: if separate_colour_plane {
                0
            } else {
                chroma_format_idc
            },
        })
    }

    const fn crop_units(self, field_factor: u64) -> (u64, u64) {
        match self.array_type {
            1 => (2, 2 * field_factor),
            2 => (2, field_factor),
            // Monochrome and 4:4:4 crop in whole luma samples; anything else
            // was rejected before this point.
            _ => (1, field_factor),
        }
    }
}

/// Consumes the picture order count syntax, whose length depends on its type
/// and which contributes nothing to picture size.
fn skip_picture_order_count(bits: &mut BitReader<'_>) -> Result<(), SpsError> {
    match bits.read_unsigned_exp_golomb()? {
        0 => {
            let _log2_max_pic_order_cnt_lsb_minus4 = bits.read_unsigned_exp_golomb()?;
        }
        1 => {
            let _delta_pic_order_always_zero_flag = bits.read_flag()?;
            let _offset_for_non_ref_pic = bits.read_signed_exp_golomb()?;
            let _offset_for_top_to_bottom_field = bits.read_signed_exp_golomb()?;
            let cycle_length = bits.read_unsigned_exp_golomb()?;
            for _ in 0..cycle_length {
                let _offset_for_ref_frame = bits.read_signed_exp_golomb()?;
            }
        }
        2 => {}
        _ => return Err(SpsError::Unsupported),
    }
    Ok(())
}

/// A sequence parameter set this parser cannot turn into a picture size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpsError {
    /// The access unit carried no sequence parameter set.
    Missing,
    /// The sequence parameter set ended before a required field.
    Truncated,
    /// The sequence parameter set uses a construct this parser does not model.
    Unsupported,
    /// The fields decoded to a picture with no samples.
    Degenerate,
}

impl core::fmt::Display for SpsError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "access unit carries no sequence parameter set",
            Self::Truncated => "sequence parameter set ended mid-field",
            Self::Unsupported => "sequence parameter set uses an unmodelled construct",
            Self::Degenerate => "sequence parameter set describes an empty picture",
        })
    }
}

impl core::error::Error for SpsError {}

/// Walks the NAL units of an Annex-B byte stream, accepting both the three-
/// and four-byte start codes and tolerating trailing zero padding.
struct AnnexBNalUnits<'a> {
    stream: &'a [u8],
    cursor: usize,
}

impl<'a> AnnexBNalUnits<'a> {
    const fn new(stream: &'a [u8]) -> Self {
        Self { stream, cursor: 0 }
    }

    /// The offset just past the next start code at or after `from`, or `None`.
    fn next_start(&self, from: usize) -> Option<usize> {
        self.stream
            .get(from..)?
            .windows(3)
            .position(|window| window == [0, 0, 1])
            .map(|offset| from + offset + 3)
    }
}

impl<'a> Iterator for AnnexBNalUnits<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.next_start(self.cursor)?;
        let end = self.next_start(start).map_or(self.stream.len(), |next| {
            // Step back over the start code and any zero byte that belonged to
            // a four-byte prefix rather than to this NAL unit.
            let mut end = next.saturating_sub(3);
            while end > start && self.stream.get(end - 1) == Some(&0) {
                end -= 1;
            }
            end
        });
        self.cursor = end;
        self.stream.get(start..end)
    }
}

/// Undoes the start-code emulation prevention the transport applies, so the
/// bit reader sees the raw byte sequence payload the syntax is defined over.
fn remove_emulation_prevention(payload: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(payload.len());
    let mut zeros = 0_usize;
    for &byte in payload {
        if zeros >= 2 && byte == 0x03 {
            zeros = 0;
            continue;
        }
        if byte == 0 {
            zeros = zeros.saturating_add(1);
        } else {
            zeros = 0;
        }
        rbsp.push(byte);
    }
    rbsp
}

/// Consumes a scaling list without retaining it; only its length matters here.
fn skip_scaling_list(bits: &mut BitReader<'_>, size: usize) -> Result<(), SpsError> {
    let mut last_scale = 8_i64;
    let mut next_scale = 8_i64;
    for _ in 0..size {
        if next_scale != 0 {
            let delta_scale = bits.read_signed_exp_golomb()?;
            next_scale = (last_scale + delta_scale + 256).rem_euclid(256);
        }
        if next_scale != 0 {
            last_scale = next_scale;
        }
    }
    Ok(())
}

struct BitReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> BitReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read_bit(&mut self) -> Result<u64, SpsError> {
        let byte = self
            .bytes
            .get(self.position / 8)
            .copied()
            .ok_or(SpsError::Truncated)?;
        let shift = 7 - (self.position % 8);
        self.position = self.position.saturating_add(1);
        Ok(u64::from((byte >> shift) & 1))
    }

    fn read_flag(&mut self) -> Result<bool, SpsError> {
        Ok(self.read_bit()? == 1)
    }

    fn read_bits(&mut self, count: usize) -> Result<u64, SpsError> {
        let mut value = 0_u64;
        for _ in 0..count {
            value = (value << 1) | self.read_bit()?;
        }
        Ok(value)
    }

    /// Reads `ue(v)`. The leading-zero count is bounded so that a corrupt or
    /// adversarial stream cannot drive an unbounded read.
    fn read_unsigned_exp_golomb(&mut self) -> Result<u64, SpsError> {
        let mut leading_zeros = 0_usize;
        while self.read_bit()? == 0 {
            leading_zeros = leading_zeros.saturating_add(1);
            if leading_zeros > 31 {
                return Err(SpsError::Unsupported);
            }
        }
        if leading_zeros == 0 {
            return Ok(0);
        }
        let suffix = self.read_bits(leading_zeros)?;
        Ok((1_u64 << leading_zeros) - 1 + suffix)
    }

    /// Reads `se(v)`, which maps the unsigned code onto alternating signs.
    fn read_signed_exp_golomb(&mut self) -> Result<i64, SpsError> {
        let coded = self.read_unsigned_exp_golomb()?;
        let magnitude = i64::try_from(coded.div_ceil(2)).map_err(|_| SpsError::Unsupported)?;
        Ok(if coded % 2 == 0 {
            -magnitude
        } else {
            magnitude
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{CodedDimensions, SpsError};
    use crate::packet::{H264AccessUnitAssembler, H264NalUnit, MediaTimestamp};
    use std::sync::Arc;

    // Every fixture below is the sequence parameter set of an actual H.264
    // elementary stream on this machine, and every asserted size is the one
    // `ffprobe -show_entries stream=coded_width,coded_height` reports for that
    // same stream. A parser checked only against bytes written from memory
    // would prove nothing about real media.

    /// From `recordings/mac-webcam/2026/08/16/05-21-44--r-6a8148e8-a9a1907-2.mp4`,
    /// a previously qualified Mac webcam RTSP bridge. Constrained Baseline.
    const REAL_WEBCAM_1280X720: &[u8] = &[
        0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16, 0xe8, 0x40, 0x00, 0x00, 0x03, 0x03, 0x00,
        0x00, 0x03, 0x00, 0x5a, 0xa3, 0xc6, 0x0c, 0xa8,
    ];

    /// From `recordings/camera-01/2026/08/15/14-16-02--r-6a8074a2-d9b90f8-2.mp4`.
    const REAL_CAMERA_640X360: &[u8] = &[
        0x67, 0x42, 0xc0, 0x16, 0xda, 0x02, 0x80, 0xbf, 0xe5, 0xc0, 0x44, 0x00, 0x00, 0x03, 0x00,
        0x04, 0x00, 0x00, 0x03, 0x00, 0x7a, 0x3c, 0x58, 0xba, 0x80,
    ];

    /// High Profile 1920x1080 from libx264. 1080 is not a multiple of 16, so
    /// this only decodes to the right height once frame cropping is applied.
    const HIGH_1920X1080: &[u8] = &[
        0x67, 0x64, 0x00, 0x28, 0xac, 0xd9, 0x40, 0x78, 0x02, 0x27, 0xe5, 0xc0, 0x44, 0x00, 0x00,
        0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xf0, 0x3c, 0x60, 0xc6, 0x58,
    ];

    /// High Profile 1280x720, which carries the chroma format and scaling
    /// matrix fields that Baseline and Main omit.
    const HIGH_1280X720: &[u8] = &[
        0x67, 0x64, 0x00, 0x1f, 0xac, 0xd9, 0x40, 0x50, 0x05, 0xbb, 0x01, 0x10, 0x00, 0x00, 0x03,
        0x00, 0x10, 0x00, 0x00, 0x03, 0x03, 0xc0, 0xf1, 0x83, 0x19, 0x60,
    ];

    /// Main Profile 640x360.
    const MAIN_640X360: &[u8] = &[
        0x67, 0x4d, 0x40, 0x1e, 0xec, 0xa0, 0x50, 0x17, 0xfc, 0xb8, 0x08, 0x80, 0x00, 0x00, 0x03,
        0x00, 0x80, 0x00, 0x00, 0x1e, 0x07, 0x8b, 0x16, 0xcb,
    ];

    /// Constrained Baseline 320x240, which carries no chroma format field.
    const BASELINE_320X240: &[u8] = &[
        0x67, 0x42, 0xc0, 0x0d, 0xd9, 0x01, 0x41, 0xfb, 0x01, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10,
        0x00, 0x00, 0x03, 0x03, 0xc0, 0xf1, 0x42, 0xa4, 0x80,
    ];

    fn dimensions(sps: &[u8]) -> (u32, u32) {
        let parsed = CodedDimensions::from_ebsp(sps)
            .unwrap_or_else(|error| panic!("valid sequence parameter set: {error}"));
        (parsed.width(), parsed.height())
    }

    #[test]
    fn reads_the_dimensions_of_actually_recorded_media() {
        assert_eq!(dimensions(REAL_WEBCAM_1280X720), (1280, 720));
        assert_eq!(dimensions(REAL_CAMERA_640X360), (640, 360));
    }

    #[test]
    fn applies_frame_cropping_for_a_height_that_is_not_a_multiple_of_sixteen() {
        // 1080 luma rows occupy 68 macroblock rows, which code 1088 samples.
        // Without the bottom crop this reports 1088 and the published track
        // would not match the stream.
        assert_eq!(dimensions(HIGH_1920X1080), (1920, 1080));
    }

    #[test]
    fn reads_high_profile_chroma_format_and_scaling_matrix_fields() {
        assert_eq!(dimensions(HIGH_1280X720), (1280, 720));
    }

    #[test]
    fn reads_main_profile() {
        assert_eq!(dimensions(MAIN_640X360), (640, 360));
    }

    #[test]
    fn reads_a_baseline_sequence_parameter_set_without_a_chroma_format_field() {
        assert_eq!(dimensions(BASELINE_320X240), (320, 240));
    }

    #[test]
    fn only_the_escape_byte_is_removed_not_the_data_it_protects() {
        // The escape byte is the 0x03 that follows two zeros; a 0x03 that is
        // itself the escaped payload must survive, which is why the check
        // cannot simply be "no 0x00 0x00 0x03 remains".
        assert_eq!(
            super::remove_emulation_prevention(&[0x00, 0x00, 0x03, 0x03, 0x00, 0x00, 0x03, 0x01]),
            vec![0x00, 0x00, 0x03, 0x00, 0x00, 0x01]
        );
        // Two escape bytes in the recorded webcam parameter set, and no more.
        let payload = &REAL_WEBCAM_1280X720[1..];
        assert_eq!(
            super::remove_emulation_prevention(payload).len(),
            payload.len() - 2
        );
    }

    #[test]
    fn an_escape_byte_before_the_size_fields_shifts_the_result() {
        // In every fixture here the escapes fall in the video usability
        // information, after the fields that determine size, so the recorded
        // dimensions come out right either way and cannot demonstrate that the
        // removal matters. A stream whose slice header or cropping offsets
        // happen to encode two zero bytes does carry an earlier escape, and
        // there the removal decides the answer. Splice one into the smallest
        // fixture to show the difference is real rather than assumed.
        let mut escaped = BASELINE_320X240.to_vec();
        escaped.splice(4..4, [0x00, 0x00, 0x03]);
        let with_removal = CodedDimensions::from_ebsp(&escaped);
        let without_removal = CodedDimensions::from_rbsp(&escaped[1..]);
        assert_ne!(
            with_removal.map(|size| (size.width(), size.height())),
            without_removal.map(|size| (size.width(), size.height()))
        );
    }

    /// Checks the parser against every segment the recorder has written on
    /// this machine, rather than against the handful of fixtures above.
    ///
    /// Ignored because it needs a populated recordings root plus ffmpeg and
    /// ffprobe. `scripts/phase5/sps-corpus-fixture.sh` builds the corpus and
    /// runs it.
    #[test]
    #[ignore = "needs a recordings corpus built by scripts/phase5/sps-corpus-fixture.sh"]
    fn parses_every_recorded_segment_the_way_ffprobe_does() {
        let Ok(path) = std::env::var("SPS_CORPUS") else {
            panic!("set SPS_CORPUS to a corpus produced by scripts/phase5/sps-corpus-fixture.sh");
        };
        let corpus = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read corpus {path}: {error}"));

        let mut checked = 0_usize;
        let mut disagreements = Vec::new();
        for line in corpus.lines().filter(|line| !line.trim().is_empty()) {
            let mut fields = line.splitn(4, ' ');
            let (Some(hex), Some(width), Some(height), Some(source)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                panic!("malformed corpus line: {line}");
            };
            let bytes: Vec<u8> = (0..hex.len() / 2)
                .map(|index| {
                    u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
                        .unwrap_or_else(|error| panic!("corpus hex for {source}: {error}"))
                })
                .collect();
            let expected = (
                width
                    .parse::<u32>()
                    .unwrap_or_else(|error| panic!("corpus width for {source}: {error}")),
                height
                    .parse::<u32>()
                    .unwrap_or_else(|error| panic!("corpus height for {source}: {error}")),
            );
            match CodedDimensions::from_ebsp(&bytes) {
                Ok(parsed) if (parsed.width(), parsed.height()) == expected => checked += 1,
                Ok(parsed) => disagreements.push(format!(
                    "{source}: parsed {}x{}, ffprobe {}x{}",
                    parsed.width(),
                    parsed.height(),
                    expected.0,
                    expected.1
                )),
                Err(error) => disagreements.push(format!("{source}: {error}")),
            }
        }

        assert!(
            disagreements.is_empty(),
            "{} of {} segments disagreed with ffprobe:\n{}",
            disagreements.len(),
            checked + disagreements.len(),
            disagreements.join("\n")
        );
        assert!(checked > 0, "corpus {path} contained no segments to check");
        println!("agreed with ffprobe on {checked} recorded segments");
    }

    #[test]
    fn finds_the_sequence_parameter_set_in_an_annex_b_stream() {
        // Four-byte prefix on the first NAL, three-byte on the rest, which is
        // what the serialiser emits, plus trailing padding.
        let mut stream = vec![0x00, 0x00, 0x00, 0x01];
        stream.extend_from_slice(REAL_WEBCAM_1280X720);
        stream.extend_from_slice(&[0x00, 0x00, 0x01, 0x68, 0xce, 0x0f, 0xc8]);
        stream.extend_from_slice(&[0x00, 0x00, 0x01, 0x65, 0x88, 0x82, 0x00, 0x00]);
        let parsed = CodedDimensions::from_annex_b(&stream)
            .unwrap_or_else(|error| panic!("dimensions from stream: {error}"));
        assert_eq!((parsed.width(), parsed.height()), (1280, 720));
    }

    #[test]
    fn an_annex_b_stream_without_a_sequence_parameter_set_is_reported() {
        assert_eq!(
            CodedDimensions::from_annex_b(&[0x00, 0x00, 0x01, 0x41, 0x02]),
            Err(SpsError::Missing)
        );
    }

    #[test]
    fn a_truncated_sequence_parameter_set_is_reported_not_guessed() {
        assert_eq!(
            CodedDimensions::from_ebsp(&REAL_WEBCAM_1280X720[..4]),
            Err(SpsError::Truncated)
        );
    }

    #[test]
    fn an_access_unit_without_a_sequence_parameter_set_is_reported() {
        let mut assembler = H264AccessUnitAssembler::new();
        let timestamp = MediaTimestamp::new(0, 90_000)
            .unwrap_or_else(|error| panic!("valid timebase: {error}"));
        let nal = H264NalUnit::from_ebsp(Arc::from([0x41_u8, 0x02].as_slice()))
            .unwrap_or_else(|error| panic!("valid NAL: {error}"));
        let access_unit = assembler
            .push(timestamp, nal, true)
            .unwrap_or_else(|error| panic!("valid access unit: {error}"))
            .unwrap_or_else(|| panic!("completed access unit"));
        assert_eq!(
            CodedDimensions::from_access_unit(&access_unit),
            Err(SpsError::Missing)
        );
    }

    #[test]
    fn reads_dimensions_from_a_parameterized_keyframe() {
        let mut assembler = H264AccessUnitAssembler::new();
        let timestamp = MediaTimestamp::new(0, 90_000)
            .unwrap_or_else(|error| panic!("valid timebase: {error}"));
        // The parameter sets and the leading bytes of the IDR slice as they
        // appear in the recorded webcam stream.
        let nals: [&[u8]; 3] = [
            REAL_WEBCAM_1280X720,
            &[0x68, 0xce, 0x0f, 0xc8],
            &[0x65, 0x88, 0x82, 0x02, 0x28, 0x31, 0x80],
        ];
        let mut completed = None;
        for (index, bytes) in nals.iter().enumerate() {
            completed = assembler
                .push(
                    timestamp,
                    H264NalUnit::from_ebsp(Arc::from(*bytes))
                        .unwrap_or_else(|error| panic!("valid NAL: {error}")),
                    index == 2,
                )
                .unwrap_or_else(|error| panic!("valid access unit: {error}"));
        }
        let access_unit = completed.unwrap_or_else(|| panic!("completed access unit"));
        let parsed = CodedDimensions::from_access_unit(&access_unit)
            .unwrap_or_else(|error| panic!("dimensions from keyframe: {error}"));
        assert_eq!((parsed.width(), parsed.height()), (1280, 720));
    }
}
