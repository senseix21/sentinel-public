#![no_main]

use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};
use surveillance_common::types::{CameraId, RecordingId};
use surveillance_storage::{
    metadata::{
        CompletedRecording, MetadataProvenance, RecordingContainer, RecordingLocator, VideoCodec,
    },
    recordings::{PlaybackPlan, RecordingTimeRange},
};

fn word(input: &mut &[u8]) -> u16 {
    if input.len() < 2 {
        return 0;
    }
    let value = u16::from_le_bytes([input[0], input[1]]);
    *input = &input[2..];
    value
}

fn time(seconds: i32) -> SystemTime {
    let duration = Duration::from_secs(u64::from(seconds.unsigned_abs()));
    if seconds < 0 {
        SystemTime::UNIX_EPOCH - duration
    } else {
        SystemTime::UNIX_EPOCH + duration
    }
}

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    // Six header bytes and at most 32 eight-byte segment descriptions. Every
    // timestamp is within thirteen days of the epoch, so SystemTime arithmetic is
    // representable. The oracle uses integer interval intersections instead of
    // reproducing PlaybackPlan's pairwise validation.
    let mut input = data;
    let start = i32::from(word(&mut input)) - 32_768;
    let end = i32::from(word(&mut input)) - 32_768;
    let maximum = u64::from(word(&mut input));
    let range = RecordingTimeRange::new(time(start), time(end), Duration::from_secs(maximum));
    let valid_range = end > start && u64::try_from(end - start).is_ok_and(|n| n <= maximum);
    assert_eq!(range.is_ok(), valid_range);
    let Ok(range) = range else {
        return;
    };
    let mut rows = Vec::new();
    let mut intervals = Vec::new();
    for index in 0..32 {
        if input.len() < 8 {
            break;
        }
        let from = i32::from(word(&mut input)) - 32_768;
        let to = from + i32::from(word(&mut input)) + 1;
        let camera = word(&mut input) % 2;
        let flags = word(&mut input);
        // Also generate contiguous runs anchored at the query, so successful
        // multi-segment plans are reachable as readily as malformed plans.
        let (from, to) = if flags & 1 == 1 {
            let from = intervals.last().map_or(start, |&(_, to, _)| to);
            (from, from + i32::from(flags >> 1) + 1)
        } else {
            (from, to)
        };
        let id = RecordingId::new(format!("segment-{index}")).unwrap();
        let row = CompletedRecording::new(
            id,
            CameraId::new(format!("camera-{camera}")).unwrap(),
            time(from),
            time(to),
            RecordingLocator::new(PathBuf::from(format!(
                "camera-{camera}/segment-{index}.mp4"
            )))
            .unwrap(),
            RecordingContainer::FragmentedMp4,
            VideoCodec::H264,
            1,
            1,
            1,
            MetadataProvenance::Recorder,
        )
        .unwrap();
        // For valid segments, overlap means a nonempty interval intersection.
        assert_eq!(
            range.overlaps(time(from), time(to)),
            from.max(start) < to.min(end)
        );
        intervals.push((from, to, camera));
        rows.push(row);
    }
    let mut coverage_end = start;
    let mut valid_plan = !intervals.is_empty();
    let camera = intervals.first().map(|row| row.2);
    for (index, &(from, to, row_camera)) in intervals.iter().enumerate() {
        valid_plan &= Some(row_camera) == camera;
        valid_plan &= if index == 0 {
            from <= start && start < to
        } else {
            from == coverage_end
        };
        coverage_end = to;
    }
    valid_plan &= coverage_end >= end;
    let plan = PlaybackPlan::new(range, rows);
    assert_eq!(plan.is_ok(), valid_plan);
    if let Ok(plan) = plan {
        assert_eq!(plan.requested(), range);
        assert_eq!(plan.segments().len(), intervals.len());
    }
});
