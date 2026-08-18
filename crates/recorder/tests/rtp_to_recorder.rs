use std::sync::Arc;

use surveillance_camera::h264_depay::{H264Depayloader, H264DepayloaderLimits, RtpH264Payload};
use surveillance_media::{
    distribution::{MediaDistributor, PublishOutcome, QueueCapacity},
    packet::{H264AccessUnit, H264AccessUnitAssembler, MediaTimestamp},
};
use surveillance_recorder::recorder::{
    H264AccessUnitSink, RecorderConsumeOutcome, RecorderConsumer,
};

#[derive(Default)]
struct ObservingSink {
    timestamps: Vec<i64>,
    keyframes: Vec<bool>,
}

impl H264AccessUnitSink for ObservingSink {
    type Error = std::convert::Infallible;

    fn write_access_unit(&mut self, access_unit: &H264AccessUnit) -> Result<(), Self::Error> {
        self.timestamps.push(access_unit.timestamp().ticks());
        self.keyframes.push(access_unit.is_keyframe());
        Ok(())
    }
}

fn timestamp(ticks: i64) -> MediaTimestamp {
    MediaTimestamp::new(ticks, 90_000)
        .unwrap_or_else(|error| panic!("valid RTP timestamp: {error}"))
}

fn capacity(value: usize) -> QueueCapacity {
    QueueCapacity::new(value).unwrap_or_else(|error| panic!("valid queue capacity: {error}"))
}

#[test]
fn rtp_payloads_cross_one_ingest_distribution_and_recorder_path() {
    let limits = H264DepayloaderLimits::new(1_024, 8)
        .unwrap_or_else(|error| panic!("valid depayloader limits: {error}"));
    let mut depayloader = H264Depayloader::new(limits);
    let mut assembler = H264AccessUnitAssembler::new();
    let (distributor, subscriptions) = MediaDistributor::new(capacity(1), capacity(1));
    let mut recorder = RecorderConsumer::new(subscriptions.recorder, ObservingSink::default());

    // One STAP-A carries SPS, PPS, and an IDR slice. The marker closes one
    // complete access unit; consumers never observe its individual NAL units.
    let keyframe_payload = [
        0x78, 0x00, 0x02, 0x67, 0x01, 0x00, 0x02, 0x68, 0x01, 0x00, 0x02, 0x65, 0x01,
    ];
    let packets = [
        RtpH264Payload {
            sequence_number: 10,
            timestamp: timestamp(90_000),
            marker: true,
            payload: &keyframe_payload,
        },
        RtpH264Payload {
            sequence_number: 11,
            timestamp: timestamp(96_000),
            marker: true,
            payload: &[0x41, 0x02],
        },
    ];

    for packet in packets {
        let nals = depayloader
            .push(packet)
            .unwrap_or_else(|error| panic!("valid RTP/H.264 payload: {error}"));
        for nal in nals {
            if let Some(access_unit) = assembler
                .push(nal.timestamp, nal.nal, nal.ends_access_unit)
                .unwrap_or_else(|error| panic!("valid access-unit boundary: {error}"))
            {
                let report = distributor.publish(Arc::new(access_unit));
                assert_eq!(report.recorder, PublishOutcome::Accepted);
                assert!(matches!(
                    recorder.try_consume(),
                    RecorderConsumeOutcome::Written
                ));
            }
        }
    }

    drop(distributor);
    assert!(matches!(
        recorder.try_consume(),
        RecorderConsumeOutcome::InputClosed
    ));
    assert_eq!(recorder.sink().timestamps, [90_000, 96_000]);
    assert_eq!(recorder.sink().keyframes, [true, false]);
    assert_eq!(recorder.metrics().written, 2);

    // Live was intentionally never drained. Its private loss did not prevent
    // recorder delivery: the recorder still wrote both access units above.
    //
    // With a live capacity of one, the second access unit is a non-IDR slice
    // that only decodes against the keyframe already queued. Evicting the
    // keyframe for it would leave the live consumer holding a picture it cannot
    // decode, so the queue keeps the keyframe and withholds the delta until the
    // next run start.
    let live_metrics = subscriptions.live.metrics();
    assert_eq!(live_metrics.accepted, 1);
    assert_eq!(live_metrics.dropped_oldest, 0);
    assert_eq!(live_metrics.dropped_undecodable, 1);
}
