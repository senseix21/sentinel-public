//! Owned detection composition and decoder boundary.

use crate::{
    gate::{GateDecision, GatePolicy, MotionGate},
    inference::{DetectionInference, InferenceDescriptor, InferenceResults},
    sampler::{SampleDecision, SamplerCounters, TimestampSampler},
};
use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    fmt,
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use surveillance_common::types::CameraId;
use surveillance_events::{
    bus::EventBus,
    event::{
        DeliveryClass, DetectorState as EventDetectorState, DomainEvent, EventId, EventPayload,
        FailureCategory, OccurredAtUnixMicros, SourceMediaTime,
    },
};
use surveillance_media::{
    distribution::{MediaSubscriber, QueueMetrics, ReceiveOutcome},
    frame::{DecodedFrame, FrameLimits, PixelFormat},
    packet::H264AccessUnit,
};
use tokio::{sync::watch, task::JoinHandle};

pub trait H264Decoder: Send + 'static {
    fn reset(&mut self);
    /// # Errors
    /// Returns a typed decode failure for malformed, incompatible, unavailable, or oversized input.
    fn decode(
        &mut self,
        camera_id: &CameraId,
        unit: &H264AccessUnit,
        observed_at: SystemTime,
    ) -> Result<Option<DecodedFrame>, DecodeError>;
}

#[cfg(test)]
mod tests {
    use super::{DetectionOwner, DetectionStopReason, FixtureH264Decoder};
    use crate::{
        gate::GatePolicy,
        inference::DeterministicFakeInference,
        sampler::{SampleRateHz, TimestampSampler},
    };
    use std::{sync::Arc, time::Duration};
    use surveillance_common::types::CameraId;
    use surveillance_events::{
        bus::{EventBus, SubscriptionCapacity},
        event::EventPayload,
    };
    use surveillance_media::{
        distribution::{MediaDistributor, QueueCapacity},
        frame::FrameLimits,
        packet::{H264AccessUnit, H264AccessUnitAssembler, H264NalUnit, MediaTimestamp},
    };

    fn capacity(value: usize) -> QueueCapacity {
        QueueCapacity::new(value).unwrap_or_else(|error| panic!("capacity: {error}"))
    }
    fn keyframe(ticks: i64, epoch: u64) -> Arc<H264AccessUnit> {
        let timestamp =
            MediaTimestamp::new(ticks, 90_000).unwrap_or_else(|error| panic!("timestamp: {error}"));
        let mut assembler = H264AccessUnitAssembler::new();
        for (bytes, marker) in [
            (&[0x67, 1][..], false),
            (&[0x68, 1][..], false),
            (&[0x65, 1][..], true),
        ] {
            let nal = H264NalUnit::from_ebsp(Arc::from(bytes))
                .unwrap_or_else(|error| panic!("NAL: {error}"));
            if let Some(unit) = assembler
                .push(timestamp, nal, marker)
                .unwrap_or_else(|error| panic!("assemble: {error}"))
            {
                return Arc::new(
                    unit.with_connection_epoch(epoch)
                        .unwrap_or_else(|error| panic!("epoch: {error}")),
                );
            }
        }
        panic!("keyframe was not completed")
    }

    #[test]
    fn roi_filter_drops_detections_whose_centre_falls_inside_the_rect() {
        use surveillance_events::event::{
            Confidence, DetectionSet, NormalizedBoundingBox, PersonDetection,
        };
        let inside = PersonDetection {
            confidence: Confidence::new(0.9).unwrap(),
            bounding_box: NormalizedBoundingBox::new(0.05, 0.05, 0.1, 0.1).unwrap(),
            track_id: None,
        };
        let outside = PersonDetection {
            confidence: Confidence::new(0.9).unwrap(),
            bounding_box: NormalizedBoundingBox::new(0.6, 0.6, 0.2, 0.2).unwrap(),
            track_id: None,
        };
        let set = DetectionSet::new(vec![inside, outside]).unwrap();
        let filtered = super::filter_detections_outside(&set, [0.0, 0.0, 0.3, 1.0]);
        assert_eq!(
            filtered.as_slice().len(),
            1,
            "the inside box must be dropped"
        );
        let kept = &filtered.as_slice()[0];
        assert!((kept.bounding_box.left() - 0.6).abs() < 1e-6);
    }

    #[test]
    fn roi_filter_keeps_detections_touching_but_centred_outside() {
        use surveillance_events::event::{
            Confidence, DetectionSet, NormalizedBoundingBox, PersonDetection,
        };
        // Box at (0.2, 0.0) 0.2 wide: centre at 0.3 — on the right edge of the
        // 0.0..0.3 exclude region, outside by `>` definition.
        let edge = PersonDetection {
            confidence: Confidence::new(0.9).unwrap(),
            bounding_box: NormalizedBoundingBox::new(0.25, 0.0, 0.2, 0.5).unwrap(),
            track_id: None,
        };
        let set = DetectionSet::new(vec![edge]).unwrap();
        let filtered = super::filter_detections_outside(&set, [0.0, 0.0, 0.3, 1.0]);
        // Centre 0.35 is strictly outside, keep.
        assert_eq!(filtered.as_slice().len(), 1);
    }

    #[tokio::test]
    async fn produces_typed_event_and_joins_on_input_close() {
        let (distributor, subscriptions) =
            MediaDistributor::new_with_detection(capacity(1), capacity(1), capacity(1));
        drop(subscriptions.recorder);
        drop(subscriptions.live);
        let bus = EventBus::new();
        let mut events = bus
            .subscribe(
                SubscriptionCapacity::new(2)
                    .unwrap_or_else(|error| panic!("event capacity: {error}")),
            )
            .unwrap_or_else(|error| panic!("subscribe: {error}"));
        let sampler = TimestampSampler::new(
            SampleRateHz::new(5).unwrap_or_else(|error| panic!("rate: {error}")),
            Duration::from_secs(2),
        )
        .unwrap_or_else(|error| panic!("sampler: {error}"));
        let limits = FrameLimits::new(2, 2, 12).unwrap_or_else(|error| panic!("limits: {error}"));
        let owner = DetectionOwner::start(
            CameraId::new("camera-01").unwrap_or_else(|error| panic!("camera: {error}")),
            subscriptions.detection,
            sampler,
            FixtureH264Decoder::new(limits, 2, 2),
            DeterministicFakeInference::new().unwrap_or_else(|error| panic!("inference: {error}")),
            // A gate that admits everything, so these tests keep measuring the
            // behaviour they were written for rather than the gate's.
            GatePolicy::new(0, Duration::from_secs(0), Duration::from_secs(0))
                .unwrap_or_else(|error| panic!("gate: {error}")),
            bus,
            None,
        );
        let _ = distributor.publish(keyframe(90_000, 1));
        drop(distributor);
        let event = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let event = events
                    .receive()
                    .await
                    .unwrap_or_else(|| panic!("event bus closed"));
                if matches!(event.payload, EventPayload::PersonDetectionObserved { .. }) {
                    break event;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("event timeout"));
        assert!(matches!(
            event.payload,
            EventPayload::PersonDetectionObserved { .. }
        ));
        let exit = owner
            .join()
            .await
            .unwrap_or_else(|error| panic!("join: {error}"));
        assert_eq!(exit.reason, DetectionStopReason::InputClosed);
        assert_eq!(exit.counters.inferred, 1);
    }

    struct FaceFixture {
        model: DeterministicFakeInference,
        calls: u32,
    }
    impl crate::inference::DetectionInference for FaceFixture {
        fn model_name(&self) -> &surveillance_events::event::ModelName {
            crate::inference::PersonInference::model_name(&self.model)
        }
        fn model_version(&self) -> &surveillance_events::event::ModelVersion {
            crate::inference::PersonInference::model_version(&self.model)
        }
        fn mode(&self) -> crate::inference::DetectionMode {
            crate::inference::DetectionMode::Face
        }
        fn is_production_model(&self) -> bool {
            false
        }
        fn infer_detections(
            &mut self,
            _: &surveillance_media::frame::DecodedFrame,
        ) -> Result<crate::inference::InferenceResults, crate::inference::InferenceError> {
            use surveillance_events::event::{
                Confidence, FaceDetection, FaceDetectionSet, NormalizedBoundingBox,
            };
            self.calls += 1;
            let detections = if self.calls == 1 {
                vec![FaceDetection {
                    confidence: Confidence::new(0.9).unwrap(),
                    bounding_box: NormalizedBoundingBox::new(0.2, 0.1, 0.15, 0.2).unwrap(),
                }]
            } else {
                vec![]
            };
            Ok(crate::inference::InferenceResults::Faces(
                FaceDetectionSet::new(detections).unwrap(),
            ))
        }
    }
    #[tokio::test]
    async fn face_owner_emits_faces_then_empty_without_person_tracking() {
        let (distributor, mut subscriptions) =
            MediaDistributor::new_with_detection(capacity(2), capacity(2), capacity(2));
        let bus = EventBus::new();
        let mut events = bus
            .subscribe(surveillance_events::bus::SubscriptionCapacity::new(16).unwrap())
            .unwrap();
        let owner = DetectionOwner::start(
            CameraId::new("face-owner").unwrap(),
            subscriptions.detection,
            TimestampSampler::new(SampleRateHz::new(5).unwrap(), Duration::from_secs(2)).unwrap(),
            FixtureH264Decoder::new(FrameLimits::new(2, 2, 12).unwrap(), 2, 2),
            FaceFixture {
                model: DeterministicFakeInference::new().unwrap(),
                calls: 0,
            },
            GatePolicy::new(0, Duration::ZERO, Duration::ZERO).unwrap(),
            bus,
            None,
        );
        let _ = distributor.publish(keyframe(90000, 1));
        let _ = distributor.publish(keyframe(180_000, 1));
        drop(distributor);
        let exit = owner.join().await.unwrap();
        assert_eq!(exit.counters.inferred, 2);
        assert_eq!(exit.counters.detections_produced, 1);
        assert!(matches!(
            subscriptions.recorder.receive().await,
            surveillance_media::distribution::ReceiveOutcome::Item(_)
        ));
        assert!(matches!(
            subscriptions.recorder.receive().await,
            surveillance_media::distribution::ReceiveOutcome::Item(_)
        ));
        let observations = tokio::time::timeout(Duration::from_secs(1), async {
            let mut counts = Vec::new();
            while counts.len() < 2 {
                let event = events.receive().await.unwrap();
                assert!(!matches!(
                    event.payload,
                    EventPayload::PersonDetectionObserved { .. }
                ));
                if let EventPayload::FaceDetectionObserved {
                    detections,
                    results_are_fixture,
                    ..
                } = &event.payload
                {
                    assert!(*results_are_fixture);
                    counts.push(detections.as_slice().len());
                }
            }
            counts
        })
        .await
        .unwrap();
        assert_eq!(observations, vec![1, 0]);
    }

    #[tokio::test]
    async fn requested_shutdown_joins_without_waiting_for_media() {
        let (_distributor, subscriptions) = MediaDistributor::<H264AccessUnit>::new_with_detection(
            capacity(1),
            capacity(1),
            capacity(1),
        );
        let owner = DetectionOwner::start(
            CameraId::new("camera-02").unwrap_or_else(|error| panic!("camera: {error}")),
            subscriptions.detection,
            TimestampSampler::new(
                SampleRateHz::new(5).unwrap_or_else(|error| panic!("rate: {error}")),
                Duration::from_secs(2),
            )
            .unwrap_or_else(|error| panic!("sampler: {error}")),
            FixtureH264Decoder::new(
                FrameLimits::new(2, 2, 12).unwrap_or_else(|error| panic!("limits: {error}")),
                2,
                2,
            ),
            DeterministicFakeInference::new().unwrap_or_else(|error| panic!("inference: {error}")),
            // A gate that admits everything, so these tests keep measuring the
            // behaviour they were written for rather than the gate's.
            GatePolicy::new(0, Duration::from_secs(0), Duration::from_secs(0))
                .unwrap_or_else(|error| panic!("gate: {error}")),
            EventBus::new(),
            None,
        );
        let exit = tokio::time::timeout(Duration::from_secs(1), owner.stop_and_join())
            .await
            .unwrap_or_else(|_| panic!("shutdown timeout"))
            .unwrap_or_else(|error| panic!("join: {error}"));
        assert_eq!(exit.reason, DetectionStopReason::Requested);
    }

    struct TerminalDecoder;
    impl super::H264Decoder for TerminalDecoder {
        fn reset(&mut self) {}
        fn decode(
            &mut self,
            _: &CameraId,
            _: &H264AccessUnit,
            _: std::time::SystemTime,
        ) -> Result<Option<surveillance_media::frame::DecodedFrame>, super::DecodeError> {
            Err(super::DecodeError::Incompatible)
        }
    }

    #[tokio::test]
    async fn terminal_detector_does_not_close_recorder_or_live_consumers() {
        let (distributor, subscriptions) =
            MediaDistributor::new_with_detection(capacity(2), capacity(2), capacity(1));
        let bus = EventBus::new();
        let owner = DetectionOwner::start(
            CameraId::new("camera-03").unwrap_or_else(|error| panic!("camera: {error}")),
            subscriptions.detection,
            TimestampSampler::new(
                SampleRateHz::new(5).unwrap_or_else(|error| panic!("rate: {error}")),
                Duration::from_secs(2),
            )
            .unwrap_or_else(|error| panic!("sampler: {error}")),
            TerminalDecoder,
            DeterministicFakeInference::new().unwrap_or_else(|error| panic!("inference: {error}")),
            // A gate that admits everything, so these tests keep measuring the
            // behaviour they were written for rather than the gate's.
            GatePolicy::new(0, Duration::from_secs(0), Duration::from_secs(0))
                .unwrap_or_else(|error| panic!("gate: {error}")),
            bus,
            None,
        );
        let _ = distributor.publish(keyframe(90_000, 1));
        let exit = tokio::time::timeout(Duration::from_secs(1), owner.join())
            .await
            .unwrap_or_else(|_| panic!("terminal timeout"))
            .unwrap_or_else(|error| panic!("join: {error}"));
        assert_eq!(exit.reason, DetectionStopReason::TerminalDecode);
        let report = distributor.publish(keyframe(108_000, 1));
        assert_eq!(
            report.recorder,
            surveillance_media::distribution::PublishOutcome::Accepted
        );
        assert_eq!(
            report.live,
            surveillance_media::distribution::PublishOutcome::Accepted
        );
        assert!(matches!(
            subscriptions.recorder.try_receive(),
            surveillance_media::distribution::ReceiveOutcome::Item(_)
        ));
        assert!(matches!(
            subscriptions.live.try_receive(),
            surveillance_media::distribution::ReceiveOutcome::Item(_)
        ));
    }

    struct SlowInference {
        name: surveillance_events::event::ModelName,
        version: surveillance_events::event::ModelVersion,
    }
    impl crate::inference::PersonInference for SlowInference {
        fn model_name(&self) -> &surveillance_events::event::ModelName {
            &self.name
        }
        fn model_version(&self) -> &surveillance_events::event::ModelVersion {
            &self.version
        }
        fn infer(
            &mut self,
            _: &surveillance_media::frame::DecodedFrame,
        ) -> Result<surveillance_events::event::DetectionSet, crate::inference::InferenceError>
        {
            std::thread::sleep(Duration::from_millis(100));
            surveillance_events::event::DetectionSet::new(Vec::new()).map_err(|_| {
                crate::inference::InferenceError::new(
                    crate::inference::InferenceFailureKind::MalformedOutput,
                )
            })
        }
    }

    #[tokio::test]
    async fn shutdown_during_blocking_inference_is_bounded_and_joined() {
        let (distributor, subscriptions) =
            MediaDistributor::new_with_detection(capacity(1), capacity(1), capacity(1));
        let owner = DetectionOwner::start(
            CameraId::new("camera-04").unwrap_or_else(|error| panic!("camera: {error}")),
            subscriptions.detection,
            TimestampSampler::new(
                SampleRateHz::new(5).unwrap_or_else(|error| panic!("rate: {error}")),
                Duration::from_secs(2),
            )
            .unwrap_or_else(|error| panic!("sampler: {error}")),
            FixtureH264Decoder::new(
                FrameLimits::new(2, 2, 12).unwrap_or_else(|error| panic!("limits: {error}")),
                2,
                2,
            ),
            SlowInference {
                name: surveillance_events::event::ModelName::new("slow-fixture")
                    .unwrap_or_else(|error| panic!("model: {error}")),
                version: surveillance_events::event::ModelVersion::new("1.0")
                    .unwrap_or_else(|error| panic!("version: {error}")),
            },
            GatePolicy::new(0, Duration::from_secs(0), Duration::from_secs(0))
                .unwrap_or_else(|error| panic!("gate: {error}")),
            EventBus::new(),
            None,
        );
        let _ = distributor.publish(keyframe(90_000, 1));
        tokio::time::sleep(Duration::from_millis(20)).await;
        let exit = tokio::time::timeout(Duration::from_secs(1), owner.stop_and_join())
            .await
            .unwrap_or_else(|_| panic!("shutdown timeout"))
            .unwrap_or_else(|error| panic!("join: {error}"));
        assert_eq!(exit.reason, DetectionStopReason::Requested);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    WaitingForKeyframe,
    Incompatible,
    Malformed,
    Oversized,
    Unavailable,
}
impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "decode failed: {self:?}")
    }
}
impl Error for DecodeError {}

/// Deterministic local adapter that validates random-access warm-up and derives
/// a bounded RGB fixture from encoded bytes. It is not a production pixel decoder.
pub struct FixtureH264Decoder {
    limits: FrameLimits,
    width: u32,
    height: u32,
    ready_epoch: Option<u64>,
}
impl FixtureH264Decoder {
    #[must_use]
    pub const fn new(limits: FrameLimits, width: u32, height: u32) -> Self {
        Self {
            limits,
            width,
            height,
            ready_epoch: None,
        }
    }
}
impl H264Decoder for FixtureH264Decoder {
    fn reset(&mut self) {
        self.ready_epoch = None;
    }
    fn decode(
        &mut self,
        camera_id: &CameraId,
        unit: &H264AccessUnit,
        observed_at: SystemTime,
    ) -> Result<Option<DecodedFrame>, DecodeError> {
        let epoch = unit.connection_epoch();
        if epoch == 0 {
            return Err(DecodeError::Malformed);
        }
        if self.ready_epoch != Some(epoch) {
            self.ready_epoch = None;
            if !unit.is_keyframe() || !unit.has_parameter_sets() {
                return Ok(None);
            }
            self.ready_epoch = Some(epoch);
        }
        let stride = usize::try_from(self.width)
            .ok()
            .and_then(|value| value.checked_mul(3))
            .ok_or(DecodeError::Oversized)?;
        let len = stride
            .checked_mul(usize::try_from(self.height).map_err(|_| DecodeError::Oversized)?)
            .ok_or(DecodeError::Oversized)?;
        if len > self.limits.max_bytes {
            return Err(DecodeError::Oversized);
        }
        let parity = unit
            .nal_units()
            .iter()
            .flat_map(surveillance_media::packet::H264NalUnit::bytes)
            .fold(0_u8, |acc, byte| acc ^ byte)
            & 1;
        let pixels: Arc<[u8]> = vec![parity; len].into();
        DecodedFrame::new(
            camera_id.clone(),
            epoch,
            unit.timestamp(),
            observed_at,
            self.width,
            self.height,
            stride,
            PixelFormat::Rgb24,
            pixels,
            self.limits,
        )
        .map(Some)
        .map_err(|_| DecodeError::Oversized)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DetectionCounters {
    /// Sampled frames the gate declined to run inference on.
    pub gate_skipped: u64,
    /// Time spent turning frames into tensors, separate from inference.
    ///
    /// Kept apart because they move independently: preprocessing scales with
    /// frame size and inference with model size, and one number hides which of
    /// the two a change actually affected.
    pub preprocess_duration_micros: u64,
    /// Time spent turning output tensors back into detections.
    pub output_decode_duration_micros: u64,
    /// Frames admitted because the heartbeat elapsed rather than because
    /// anything moved. A number that approaches the admitted total means the
    /// motion threshold is set too high to be doing any work.
    pub gate_heartbeat: u64,
    pub decode_attempts: u64,
    pub decoded: u64,
    pub decode_failed: u64,
    pub inference_attempts: u64,
    pub inferred: u64,
    pub detections_produced: u64,
    pub inference_failed: u64,
    pub output_capacity_dropped: u64,
    pub events_published: u64,
    pub event_delivery_losses: u64,
    pub decode_duration_micros: u64,
    pub inference_duration_micros: u64,
    pub end_to_event_duration_micros: u64,
    pub latency_le_10ms: u64,
    pub latency_le_50ms: u64,
    pub latency_le_100ms: u64,
    pub latency_le_250ms: u64,
    pub latency_le_500ms: u64,
    pub latency_le_1000ms: u64,
    pub latency_over_1000ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetectionStopReason {
    Requested,
    InputClosed,
    TerminalDecode,
    TerminalInference,
}
pub struct DetectionExit {
    pub reason: DetectionStopReason,
    pub sampler: SamplerCounters,
    pub counters: DetectionCounters,
    pub input: QueueMetrics,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetectionState {
    Disabled,
    Starting,
    Running,
    Degraded,
    Terminal,
    Stopping,
    Stopped,
}

/// Per-capability counters, bounded by camera and lane labels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LaneCounters {
    pub offered: u64,
    pub replaced: u64,
    pub stale: u64,
    pub completed: u64,
    pub failed: u64,
    pub preprocessing_micros: u64,
    pub inference_micros: u64,
    pub latency_buckets: [u64; 7],
}
impl LaneCounters {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            offered: 0,
            replaced: 0,
            stale: 0,
            completed: 0,
            failed: 0,
            preprocessing_micros: 0,
            inference_micros: 0,
            latency_buckets: [0; 7],
        }
    }
}

// Not Copy: the snapshot now carries the live backend's identity, which is
// textual. Callers clone it, which they already did through Copy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetectionHealthSnapshot {
    pub state: DetectionState,
    pub counters: DetectionCounters,
    pub sampler: SamplerCounters,
    pub input: QueueMetrics,
    /// What this detector is actually running, taken from the object it was
    /// handed rather than from anything the build knows about itself.
    ///
    /// `None` means no detector is running for this camera, which is not the
    /// same as running a fixture and must not be reported as though it were.
    pub backend: Option<InferenceDescriptor>,
    pub overlay_max_age: Duration,
    pub person_lane: Option<DetectionState>,
    pub face_lane: Option<DetectionState>,
    pub person_counters: LaneCounters,
    pub face_counters: LaneCounters,
}

#[derive(Clone)]
pub struct DetectionHealth(pub(crate) Arc<Mutex<DetectionHealthSnapshot>>);
impl DetectionHealth {
    #[must_use]
    pub const fn disabled_snapshot() -> DetectionHealthSnapshot {
        DetectionHealthSnapshot {
            state: DetectionState::Disabled,
            backend: None,
            overlay_max_age: Duration::from_secs(3),
            person_lane: None,
            face_lane: None,
            person_counters: LaneCounters::empty(),
            face_counters: LaneCounters::empty(),
            counters: DetectionCounters {
                gate_skipped: 0,
                preprocess_duration_micros: 0,
                output_decode_duration_micros: 0,
                gate_heartbeat: 0,
                decode_attempts: 0,
                decoded: 0,
                decode_failed: 0,
                inference_attempts: 0,
                inferred: 0,
                detections_produced: 0,
                inference_failed: 0,
                output_capacity_dropped: 0,
                events_published: 0,
                event_delivery_losses: 0,
                decode_duration_micros: 0,
                inference_duration_micros: 0,
                end_to_event_duration_micros: 0,
                latency_le_10ms: 0,
                latency_le_50ms: 0,
                latency_le_100ms: 0,
                latency_le_250ms: 0,
                latency_le_500ms: 0,
                latency_le_1000ms: 0,
                latency_over_1000ms: 0,
            },
            sampler: SamplerCounters {
                considered: 0,
                sampled: 0,
                skipped_by_rate: 0,
                duplicate: 0,
                resets: 0,
            },
            input: QueueMetrics {
                capacity: 0,
                depth: 0,
                accepted: 0,
                received: 0,
                rejected_full: 0,
                dropped_oldest: 0,
                dropped_undecodable: 0,
                dropped_on_close: 0,
                consumer_open: false,
                producer_open: false,
            },
        }
    }
    #[must_use]
    pub fn disabled() -> Self {
        Self(Arc::new(Mutex::new(Self::disabled_snapshot())))
    }
    /// Publishes terminal detection state without taking down the camera owner.
    #[must_use]
    pub fn unavailable_observed(camera: &CameraId, bus: &EventBus, category: &'static str) -> Self {
        publish_detector_state(camera, bus, EventDetectorState::Terminal, Some(category), 1);
        Self::unavailable()
    }
    /// A configured model failed to start; media preservation remains independent.
    #[must_use]
    pub fn unavailable() -> Self {
        let mut snapshot = Self::disabled_snapshot();
        snapshot.state = DetectionState::Terminal;
        snapshot.counters.inference_failed = 1;
        Self(Arc::new(Mutex::new(snapshot)))
    }
    pub fn set_overlay_max_age(&self, age: Duration) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .overlay_max_age = age;
    }
    #[must_use]
    pub fn snapshot(&self) -> DetectionHealthSnapshot {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    fn update(
        &self,
        state: DetectionState,
        counters: DetectionCounters,
        sampler: SamplerCounters,
        input: QueueMetrics,
    ) {
        // The backend is established once, when the owner is handed its
        // inference, and does not change for the life of this detector. Each
        // update preserves it rather than restating it, so there is no path by
        // which a counter refresh could quietly change what this camera claims
        // to be running.
        let mut held = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        held.state = state;
        held.counters = counters;
        held.sampler = sampler;
        held.input = input;
    }
}

pub struct DetectionOwner {
    pub(crate) shutdown: watch::Sender<bool>,
    pub(crate) task: JoinHandle<DetectionExit>,
}
impl DetectionOwner {
    #[allow(
        clippy::too_many_arguments,
        reason = "assembling the detection pipeline is a flat parameter list by design"
    )]
    pub fn start<D: H264Decoder, I: DetectionInference>(
        camera_id: CameraId,
        input: MediaSubscriber<H264AccessUnit>,
        sampler: TimestampSampler,
        decoder: D,
        inference: I,
        gate_policy: GatePolicy,
        event_bus: EventBus,
        roi_exclude: Option<[f32; 4]>,
    ) -> Self {
        Self::start_observed(
            camera_id,
            input,
            sampler,
            decoder,
            inference,
            gate_policy,
            event_bus,
            roi_exclude,
        )
        .0
    }

    #[allow(
        clippy::too_many_lines,
        reason = "linear ownership loop keeps cancellation, decode, inference, and publication auditable"
    )]
    #[allow(
        clippy::too_many_arguments,
        reason = "assembling the detection pipeline is a flat parameter list by design"
    )]
    pub fn start_observed<D: H264Decoder, I: DetectionInference>(
        camera_id: CameraId,
        mut input: MediaSubscriber<H264AccessUnit>,
        mut sampler: TimestampSampler,
        mut decoder: D,
        mut inference: I,
        gate_policy: GatePolicy,
        event_bus: EventBus,
        roi_exclude: Option<[f32; 4]>,
    ) -> (Self, DetectionHealth) {
        let mut gate = MotionGate::new(gate_policy);
        let (shutdown, mut stop) = watch::channel(false);
        // Described here, from the object this owner was handed. Asking the
        // live backend is the only form of this claim that cannot drift from
        // what is running.
        let backend = Some(InferenceDescriptor::of_detection(&inference));
        let health = DetectionHealth(Arc::new(Mutex::new(DetectionHealthSnapshot {
            state: DetectionState::Starting,
            counters: DetectionCounters::default(),
            sampler: sampler.counters(),
            input: input.metrics(),
            backend,
            overlay_max_age: Duration::from_secs(3),
            person_lane: None,
            face_lane: None,
            person_counters: LaneCounters::empty(),
            face_counters: LaneCounters::empty(),
        })));
        let task_health = health.clone();
        let task = tokio::spawn(async move {
            let mut counters = DetectionCounters::default();
            let mut state_sequence = 1_u64;
            let mut failure_streak = 0_u32;
            publish_detector_state(
                &camera_id,
                &event_bus,
                EventDetectorState::Starting,
                None,
                state_sequence,
            );
            let mut last_media: Option<(u64, surveillance_media::packet::MediaTimestamp)> = None;
            // One tracker instance per detection session. Ids reset when the
            // owner restarts (camera reconnect, inference failure); that is
            // acceptable because the visual session resets too.
            let mut tracker =
                crate::tracker::PersonTracker::new(crate::tracker::TrackerConfig::default());
            task_health.update(
                DetectionState::Running,
                counters,
                sampler.counters(),
                input.metrics(),
            );
            state_sequence += 1;
            publish_detector_state(
                &camera_id,
                &event_bus,
                EventDetectorState::Running,
                None,
                state_sequence,
            );
            let reason = loop {
                let unit = tokio::select! { changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break DetectionStopReason::Requested; } continue; }, item = input.receive() => match item { ReceiveOutcome::Item(unit) => unit, ReceiveOutcome::Closed => break DetectionStopReason::InputClosed, ReceiveOutcome::Empty => continue } };
                let observed_at = SystemTime::now();
                let current = (unit.connection_epoch(), unit.timestamp());
                if last_media.is_some_and(|(epoch, timestamp)| {
                    epoch != current.0
                        || timestamp.timebase_hz() != current.1.timebase_hz()
                        || current.1.ticks() < timestamp.ticks()
                }) {
                    decoder.reset();
                }
                last_media = Some(current);
                counters.decode_attempts += 1;
                let decode_started = Instant::now();
                task_health.update(
                    DetectionState::Running,
                    counters,
                    sampler.counters(),
                    input.metrics(),
                );
                let decode_camera = camera_id.clone();
                let decode_unit = Arc::clone(&unit);
                let decode_task = tokio::task::spawn_blocking(move || {
                    let result = decoder.decode(&decode_camera, &decode_unit, observed_at);
                    (decoder, result)
                })
                .await;
                let decode_result = match decode_task {
                    Ok((returned_decoder, result)) => {
                        decoder = returned_decoder;
                        result
                    }
                    Err(_) => break DetectionStopReason::TerminalDecode,
                };
                counters.decode_duration_micros = counters
                    .decode_duration_micros
                    .saturating_add(duration_micros(decode_started.elapsed()));
                let frame = match decode_result {
                    Ok(Some(frame)) => {
                        counters.decoded += 1;
                        failure_streak = 0;
                        frame
                    }
                    Ok(None) => continue,
                    Err(
                        DecodeError::Unavailable
                        | DecodeError::Malformed
                        | DecodeError::WaitingForKeyframe,
                    ) => {
                        counters.decode_failed += 1;
                        decoder.reset();
                        task_health.update(
                            DetectionState::Degraded,
                            counters,
                            sampler.counters(),
                            input.metrics(),
                        );
                        state_sequence = state_sequence.saturating_add(1);
                        publish_detector_state(
                            &camera_id,
                            &event_bus,
                            EventDetectorState::Degraded,
                            Some("decode"),
                            state_sequence,
                        );
                        failure_streak = failure_streak.saturating_add(1);
                        if wait_retry_backoff(failure_streak, &mut stop).await {
                            break DetectionStopReason::Requested;
                        }
                        continue;
                    }
                    Err(_) => {
                        counters.decode_failed += 1;
                        break DetectionStopReason::TerminalDecode;
                    }
                };
                match sampler.consider(frame.connection_epoch(), frame.media_timestamp()) {
                    Ok(SampleDecision::Sample) => {}
                    Ok(SampleDecision::ResetAndSample) => gate.reset_timing(),
                    Ok(SampleDecision::SkipRate | SampleDecision::Duplicate) => continue,
                    Err(_) => {
                        decoder.reset();
                        continue;
                    }
                }
                // The gate decides after sampling and before inference. It
                // saves inference only: the decoder above already ran, because
                // H.264 inter-frames cannot be decoded in isolation.
                // The camera's own position, from media ticks rather than wall
                // time, so the gate's windows do not move when the clock does.
                let stamp = frame.media_timestamp();
                let position = Duration::from_micros(
                    u64::try_from(stamp.ticks().max(0))
                        .unwrap_or(0)
                        .saturating_mul(1_000_000)
                        / u64::from(stamp.timebase_hz().max(1)),
                );
                match gate.admit(&frame, position) {
                    GateDecision::Skip => {
                        counters.gate_skipped += 1;
                        task_health.update(
                            DetectionState::Running,
                            counters,
                            sampler.counters(),
                            input.metrics(),
                        );
                        continue;
                    }
                    GateDecision::Heartbeat => counters.gate_heartbeat += 1,
                    GateDecision::Motion | GateDecision::Cooldown => {}
                }
                counters.inference_attempts += 1;
                let inference_started = Instant::now();
                let inference_frame = frame.clone();
                let inferred = tokio::task::spawn_blocking(move || {
                    let result = inference.infer_detections(&inference_frame);
                    (inference, result)
                })
                .await;
                let inference_result = match inferred {
                    Ok((returned_inference, result)) => {
                        inference = returned_inference;
                        result
                    }
                    Err(_) => break DetectionStopReason::TerminalInference,
                };
                counters.inference_duration_micros = counters
                    .inference_duration_micros
                    .saturating_add(duration_micros(inference_started.elapsed()));
                let detections = match inference_result {
                    Ok(detections) => {
                        counters.inferred += 1;
                        counters.output_capacity_dropped = counters
                            .output_capacity_dropped
                            .saturating_add(inference.output_capacity_dropped());
                        failure_streak = 0;
                        if !detections.is_empty() {
                            // Starts the cooldown, so a person who stops moving
                            // keeps being examined instead of disappearing the
                            // moment the scene goes still.
                            gate.observed_detections(position);
                        }
                        detections
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            crate::inference::InferenceFailureKind::Terminal
                                | crate::inference::InferenceFailureKind::MalformedOutput
                        ) =>
                    {
                        counters.inference_failed += 1;
                        break DetectionStopReason::TerminalInference;
                    }
                    Err(_) => {
                        counters.inference_failed += 1;
                        task_health.update(
                            DetectionState::Degraded,
                            counters,
                            sampler.counters(),
                            input.metrics(),
                        );
                        state_sequence = state_sequence.saturating_add(1);
                        publish_detector_state(
                            &camera_id,
                            &event_bus,
                            EventDetectorState::Degraded,
                            Some("inference"),
                            state_sequence,
                        );
                        failure_streak = failure_streak.saturating_add(1);
                        if wait_retry_backoff(failure_streak, &mut stop).await {
                            break DetectionStopReason::Requested;
                        }
                        continue;
                    }
                };
                // Drop detections whose box centre falls inside the ROI
                // exclude rectangle, before they reach the tracker. Done
                // here (not post-tracker) so the tracker never allocates an
                // ID for a known-bad region in the first place.
                let detections = match detections {
                    InferenceResults::Persons(set) => {
                        let set = match roi_exclude {
                            Some(rect) => filter_detections_outside(&set, rect),
                            None => set,
                        };
                        InferenceResults::Persons(tracker.update_detections(&set))
                    }
                    // Faces are observed boxes, never coasted or assigned identities.
                    InferenceResults::Faces(set) => InferenceResults::Faces(match roi_exclude {
                        None => set,
                        Some([left, top, width, height]) => set.retain(|face| {
                            let center_x =
                                face.bounding_box.left() + face.bounding_box.width() * 0.5;
                            let center_y =
                                face.bounding_box.top() + face.bounding_box.height() * 0.5;
                            !(center_x >= left
                                && center_x <= left + width
                                && center_y >= top
                                && center_y <= top + height)
                        }),
                    }),
                };
                counters.detections_produced = counters
                    .detections_produced
                    .saturating_add(u64::try_from(detections.len()).unwrap_or(u64::MAX));
                let micros = observed_at
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .ok()
                    .and_then(|duration| i64::try_from(duration.as_micros()).ok())
                    .and_then(|value| OccurredAtUnixMicros::new(value).ok());
                let (Some(occurred_at), Some(epoch), Some(timebase_hz)) = (
                    micros,
                    std::num::NonZeroU64::new(frame.connection_epoch()),
                    std::num::NonZeroU64::new(u64::from(frame.media_timestamp().timebase_hz())),
                ) else {
                    continue;
                };
                let mut camera_hash = DefaultHasher::new();
                camera_id.hash(&mut camera_hash);
                let id = format!(
                    "det-{:016x}-{}-{}",
                    camera_hash.finish(),
                    frame.connection_epoch(),
                    frame.media_timestamp().ticks().unsigned_abs()
                );
                let Ok(event_id) = EventId::new(id) else {
                    continue;
                };
                let media_time = SourceMediaTime {
                    epoch,
                    ticks: frame.media_timestamp().ticks(),
                    timebase_hz,
                };
                let model = inference.model_name().clone();
                let model_version = inference.model_version().clone();
                let results_are_fixture = !inference.is_production_model();
                let payload = match detections {
                    InferenceResults::Persons(detections) => {
                        EventPayload::PersonDetectionObserved {
                            media_time,
                            model,
                            model_version,
                            results_are_fixture,
                            detections,
                        }
                    }
                    InferenceResults::Faces(detections) => EventPayload::FaceDetectionObserved {
                        session: None,
                        media_time,
                        model,
                        model_version,
                        results_are_fixture,
                        detections,
                    },
                };
                let Ok(event) = DomainEvent::new(
                    event_id,
                    occurred_at,
                    Some(camera_id.clone()),
                    DeliveryClass::Detection,
                    payload,
                ) else {
                    continue;
                };
                let report = event_bus.publish(Arc::new(event));
                counters.events_published = counters.events_published.saturating_add(1);
                counters.event_delivery_losses = counters.event_delivery_losses.saturating_add(
                    report
                        .rejected_full
                        .saturating_add(report.dropped_newest)
                        .saturating_add(report.replaced_oldest)
                        .saturating_add(report.closed),
                );
                if let Ok(elapsed) = SystemTime::now().duration_since(observed_at) {
                    record_latency(&mut counters, elapsed);
                }
                task_health.update(
                    DetectionState::Running,
                    counters,
                    sampler.counters(),
                    input.metrics(),
                );
            };
            let exit = DetectionExit {
                reason,
                sampler: sampler.counters(),
                counters,
                input: input.metrics(),
            };
            task_health.update(
                DetectionState::Stopping,
                counters,
                sampler.counters(),
                input.metrics(),
            );
            let state = if matches!(
                reason,
                DetectionStopReason::TerminalDecode | DetectionStopReason::TerminalInference
            ) {
                DetectionState::Terminal
            } else {
                DetectionState::Stopped
            };
            task_health.update(state, counters, sampler.counters(), input.metrics());
            state_sequence = state_sequence.saturating_add(1);
            let event_state = if state == DetectionState::Terminal {
                EventDetectorState::Terminal
            } else {
                EventDetectorState::Stopped
            };
            publish_detector_state(
                &camera_id,
                &event_bus,
                event_state,
                (state == DetectionState::Terminal).then_some("terminal"),
                state_sequence,
            );
            exit
        });
        (Self { shutdown, task }, health)
    }
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
    /// # Errors
    /// Returns a join error if the owned task was cancelled or panicked.
    pub async fn stop_and_join(self) -> Result<DetectionExit, tokio::task::JoinError> {
        let _ = self.shutdown.send(true);
        self.task.await
    }
    /// # Errors
    /// Returns a join error if the owned task was cancelled or panicked.
    pub async fn join(self) -> Result<DetectionExit, tokio::task::JoinError> {
        self.task.await
    }
}

/// Drops every detection whose box centre falls inside `rect`.
/// `rect` is `[x, y, w, h]` normalized to the camera frame.
///
/// The box centre is the right discriminator for an exclusion zone: a box
/// that *touches* the rectangle but has its centre outside it is a detection
/// that mostly reaches into real coverage, and dropping it would blind the
/// system to a person leaning past the mask. The centre is also what the
/// tracker uses to match frames.
pub(crate) fn filter_detections_outside(
    detections: &surveillance_events::event::DetectionSet,
    rect: [f32; 4],
) -> surveillance_events::event::DetectionSet {
    let [rx, ry, rw, rh] = rect;
    let kept: Vec<_> = detections
        .as_slice()
        .iter()
        .filter(|det| {
            let cx = det.bounding_box.left() + det.bounding_box.width() * 0.5;
            let cy = det.bounding_box.top() + det.bounding_box.height() * 0.5;
            !(cx >= rx && cx <= rx + rw && cy >= ry && cy <= ry + rh)
        })
        .cloned()
        .collect();
    surveillance_events::event::DetectionSet::new(kept)
        .unwrap_or_else(|_| unreachable!("filtering only removes; it cannot grow"))
}

fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

async fn wait_retry_backoff(failures: u32, shutdown: &mut watch::Receiver<bool>) -> bool {
    let exponent = failures.saturating_sub(1).min(6);
    let delay = Duration::from_millis(100_u64.saturating_mul(1_u64 << exponent))
        .min(Duration::from_secs(5));
    tokio::select! {
        () = tokio::time::sleep(delay) => false,
        changed = shutdown.changed() => changed.is_err() || *shutdown.borrow(),
    }
}

fn record_latency(counters: &mut DetectionCounters, duration: Duration) {
    let micros = duration_micros(duration);
    counters.end_to_event_duration_micros =
        counters.end_to_event_duration_micros.saturating_add(micros);
    match micros {
        0..=10_000 => counters.latency_le_10ms = counters.latency_le_10ms.saturating_add(1),
        10_001..=50_000 => counters.latency_le_50ms = counters.latency_le_50ms.saturating_add(1),
        50_001..=100_000 => counters.latency_le_100ms = counters.latency_le_100ms.saturating_add(1),
        100_001..=250_000 => {
            counters.latency_le_250ms = counters.latency_le_250ms.saturating_add(1);
        }
        250_001..=500_000 => {
            counters.latency_le_500ms = counters.latency_le_500ms.saturating_add(1);
        }
        500_001..=1_000_000 => {
            counters.latency_le_1000ms = counters.latency_le_1000ms.saturating_add(1);
        }
        _ => counters.latency_over_1000ms = counters.latency_over_1000ms.saturating_add(1),
    }
}

fn publish_detector_state(
    camera_id: &CameraId,
    bus: &EventBus,
    state: EventDetectorState,
    failure: Option<&str>,
    sequence: u64,
) {
    let Some(occurred_at) = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_micros()).ok())
        .and_then(|value| OccurredAtUnixMicros::new(value).ok())
    else {
        return;
    };
    let mut camera_hash = DefaultHasher::new();
    camera_id.hash(&mut camera_hash);
    let Ok(event_id) = EventId::new(format!("detector-{:016x}-{sequence}", camera_hash.finish()))
    else {
        return;
    };
    let failure_category = failure.and_then(|value| FailureCategory::new(value).ok());
    let Ok(event) = DomainEvent::new(
        event_id,
        occurred_at,
        Some(camera_id.clone()),
        DeliveryClass::LatestState,
        EventPayload::DetectorStateChanged {
            state,
            failure_category,
        },
    ) else {
        return;
    };
    let _ = bus.publish(Arc::new(event));
}
