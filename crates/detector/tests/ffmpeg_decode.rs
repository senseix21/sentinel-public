//! Qualifies bounded RGB frame decoding from a real `FFmpeg`-generated H.264 keyframe.

use std::{
    process::Command,
    sync::Arc,
    time::{Duration, SystemTime},
};
use surveillance_common::types::CameraId;
use surveillance_detector::{
    decoder::{FfmpegDecoderConfig, FfmpegH264Decoder},
    detector::H264Decoder,
};
use surveillance_media::{
    frame::FrameLimits,
    packet::{H264AccessUnit, H264AccessUnitAssembler, H264NalUnit, MediaTimestamp},
};

#[test]
#[ignore = "requires ffmpeg with libx264"]
fn real_h264_keyframe_decodes_to_bounded_rgb() {
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=16x16:r=5",
            "-frames:v",
            "1",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-tune",
            "zerolatency",
            "-x264-params",
            "keyint=1:min-keyint=1:scenecut=0",
            "-f",
            "h264",
            "pipe:1",
        ])
        .output()
        .unwrap_or_else(|error| panic!("fixture ffmpeg: {error}"));
    assert!(output.status.success());
    let unit = access_unit(&output.stdout);
    let limits =
        FrameLimits::new(16, 16, 16 * 16 * 3).unwrap_or_else(|error| panic!("limits: {error}"));
    let mut decoder = FfmpegH264Decoder::start(FfmpegDecoderConfig {
        executable: "ffmpeg".into(),
        width: 16,
        height: 16,
        frame_limits: limits,
        response_timeout: Duration::from_secs(3),
        max_access_unit_bytes: 1024 * 1024,
    })
    .unwrap_or_else(|error| panic!("decoder: {error}"));
    let first = decoder
        .decode(
            &CameraId::new("camera-01").unwrap_or_else(|error| panic!("camera: {error}")),
            &unit,
            SystemTime::now(),
        )
        .unwrap_or_else(|error| panic!("decode: {error}"));
    let mut frame = first;
    for _ in 0..20 {
        if frame.is_some() {
            break;
        }
        frame = decoder
            .decode(
                &CameraId::new("camera-01").unwrap_or_else(|error| panic!("camera: {error}")),
                &unit,
                SystemTime::now(),
            )
            .unwrap_or_else(|error| panic!("decode warm-up: {error}"));
    }
    let frame =
        frame.unwrap_or_else(|| panic!("streaming decoder must emit after bounded warm-up"));
    assert_eq!(frame.pixels().len(), 16 * 16 * 3);
}

#[test]
#[ignore = "requires ffmpeg with libx264"]
fn real_inter_frame_sequence_preserves_bounded_timestamp_order() {
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=16x16:rate=5",
            "-frames:v",
            "40",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-tune",
            "zerolatency",
            "-x264-params",
            "aud=1:keyint=10:min-keyint=10:scenecut=0",
            "-f",
            "h264",
            "pipe:1",
        ])
        .output()
        .unwrap_or_else(|error| panic!("fixture ffmpeg: {error}"));
    assert!(output.status.success());
    let units = access_units(&output.stdout);
    assert!(units.len() >= 40);
    let limits =
        FrameLimits::new(16, 16, 16 * 16 * 3).unwrap_or_else(|error| panic!("limits: {error}"));
    let mut decoder = FfmpegH264Decoder::start(FfmpegDecoderConfig {
        executable: "ffmpeg".into(),
        width: 16,
        height: 16,
        frame_limits: limits,
        response_timeout: Duration::from_secs(3),
        max_access_unit_bytes: 1024 * 1024,
    })
    .unwrap_or_else(|error| panic!("decoder: {error}"));
    let camera = CameraId::new("camera-02").unwrap_or_else(|error| panic!("camera: {error}"));
    let mut timestamps = Vec::new();
    for unit in &units {
        if let Some(frame) = decoder
            .decode(&camera, unit, SystemTime::now())
            .unwrap_or_else(|error| panic!("decode: {error}"))
        {
            timestamps.push(frame.media_timestamp().ticks());
        }
    }
    assert!(
        timestamps.len() >= 20,
        "decoded frames: {}",
        timestamps.len()
    );
    assert!(timestamps.windows(2).all(|pair| pair[0] < pair[1]));
}

fn access_unit(bytes: &[u8]) -> H264AccessUnit {
    let nals = split_annex_b(bytes);
    let mut assembler = H264AccessUnitAssembler::new();
    let timestamp =
        MediaTimestamp::new(90_000, 90_000).unwrap_or_else(|error| panic!("timestamp: {error}"));
    let count = nals.len();
    for (index, bytes) in nals.into_iter().enumerate() {
        let nal =
            H264NalUnit::from_ebsp(Arc::from(bytes)).unwrap_or_else(|error| panic!("NAL: {error}"));
        if let Some(unit) = assembler
            .push(timestamp, nal, index + 1 == count)
            .unwrap_or_else(|error| panic!("assemble: {error}"))
        {
            return unit
                .with_connection_epoch(1)
                .unwrap_or_else(|error| panic!("epoch: {error}"));
        }
    }
    panic!("fixture produced no access unit")
}

fn access_units(bytes: &[u8]) -> Vec<H264AccessUnit> {
    let mut groups: Vec<Vec<&[u8]>> = Vec::new();
    let mut current = Vec::new();
    let mut has_vcl = false;
    for bytes in split_annex_b(bytes) {
        let nal_type = bytes.first().map_or(0, |header| header & 0x1f);
        if nal_type == 9 && has_vcl {
            groups.push(std::mem::take(&mut current));
            has_vcl = false;
        }
        has_vcl |= matches!(nal_type, 1 | 5);
        current.push(bytes);
    }
    if has_vcl {
        groups.push(current);
    }
    groups
        .into_iter()
        .enumerate()
        .map(|(index, group)| {
            let mut assembler = H264AccessUnitAssembler::new();
            let timestamp = MediaTimestamp::new(
                90_000 + i64::try_from(index).unwrap_or(i64::MAX) * 18_000,
                90_000,
            )
            .unwrap_or_else(|error| panic!("timestamp: {error}"));
            let count = group.len();
            for (nal_index, bytes) in group.into_iter().enumerate() {
                let nal = H264NalUnit::from_ebsp(Arc::from(bytes))
                    .unwrap_or_else(|error| panic!("NAL: {error}"));
                if let Some(unit) = assembler
                    .push(timestamp, nal, nal_index + 1 == count)
                    .unwrap_or_else(|error| panic!("assemble: {error}"))
                {
                    return unit
                        .with_connection_epoch(1)
                        .unwrap_or_else(|error| panic!("epoch: {error}"));
                }
            }
            panic!("group produced no unit")
        })
        .collect()
}

fn split_annex_b(bytes: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::<(usize, usize)>::new();
    let mut index = 0;
    while index + 3 < bytes.len() {
        let length = if bytes[index..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if bytes[index..].starts_with(&[0, 0, 1]) {
            3
        } else {
            index += 1;
            continue;
        };
        starts.push((index, index + length));
        index += length;
    }
    starts
        .iter()
        .enumerate()
        .filter_map(|(position, (_, start))| {
            let end = starts.get(position + 1).map_or(bytes.len(), |next| next.0);
            (*start < end).then_some(&bytes[*start..end])
        })
        .collect()
}
