//! Whether a sampled frame is worth running inference on.
//!
//! The gate reduces inference and nothing else. H.264 inter-frames cannot be
//! decoded in isolation, so the decoder runs continuously however much this
//! skips; anyone reading a low inference count as a low decode cost is reading
//! it wrong.
//!
//! It is deliberately not just a motion threshold. A person who stops moving
//! would vanish from a pure motion gate, and a threshold set too high would
//! silently stop detecting anything at all. The cooldown answers the first and
//! the heartbeat bounds the second.

use std::time::Duration;

use surveillance_media::frame::DecodedFrame;

/// How the gate is tuned. All three settings require a pipeline restart.
#[derive(Clone, Copy, Debug)]
pub struct GatePolicy {
    motion_threshold_percent: u8,
    cooldown: Duration,
    heartbeat: Duration,
}

impl GatePolicy {
    /// # Errors
    /// Returns an error when the threshold exceeds 100 percent.
    pub const fn new(
        motion_threshold_percent: u8,
        cooldown: Duration,
        heartbeat: Duration,
    ) -> Result<Self, GatePolicyError> {
        if motion_threshold_percent > 100 {
            return Err(GatePolicyError::ThresholdOutOfRange);
        }
        Ok(Self {
            motion_threshold_percent,
            cooldown,
            heartbeat,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatePolicyError {
    ThresholdOutOfRange,
}

impl core::fmt::Display for GatePolicyError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("motion threshold must be a percentage within 0..=100")
    }
}

impl core::error::Error for GatePolicyError {}

/// Why a frame was passed to inference, for counters and for reading a trace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateDecision {
    /// Enough of the frame changed.
    Motion,
    /// Within the cooldown of a recent detection, so a person who has stopped
    /// moving keeps being reported.
    Cooldown,
    /// The heartbeat elapsed. Inference runs regardless of the threshold, which
    /// is what stops a mistuned gate from going quiet forever.
    Heartbeat,
    /// Nothing changed, nothing was recently detected, and the heartbeat has
    /// not elapsed.
    Skip,
}

/// Per-camera gate state.
///
/// Holds one reference frame, which is the gate's entire memory footprint and
/// is charged to the camera's budget alongside the model input tensor.
pub struct MotionGate {
    policy: GatePolicy,
    reference: Option<Vec<u8>>,
    epoch: u64,
    last_inference: Option<Duration>,
    last_detection: Option<Duration>,
}

impl MotionGate {
    #[must_use]
    pub const fn new(policy: GatePolicy) -> Self {
        Self {
            policy,
            reference: None,
            epoch: 0,
            last_inference: None,
            last_detection: None,
        }
    }

    /// Decides whether `frame` should reach inference.
    ///
    /// `now` is the camera's monotonic position rather than wall time, so the
    /// decision does not move when the clock does.
    pub fn admit(&mut self, frame: &DecodedFrame, now: Duration) -> GateDecision {
        if frame.connection_epoch() != self.epoch {
            // A new connection is a new scene. Carrying a reference frame
            // across it would compare two unrelated images and report motion
            // that never happened.
            self.epoch = frame.connection_epoch();
            self.reference = None;
            self.last_inference = None;
            self.last_detection = None;
        }
        let changed = self.changed_fraction(frame);
        let decision = self.decide(changed, now);
        if decision != GateDecision::Skip {
            self.last_inference = Some(now);
            self.reference = Some(frame.pixels().to_vec());
        }
        decision
    }

    /// Records that the last admitted frame produced detections.
    ///
    /// Drives the cooldown: a scene that just contained a person keeps being
    /// examined even once it goes still.
    pub fn observed_detections(&mut self, now: Duration) {
        self.last_detection = Some(now);
    }

    /// Clears media-time deadlines after the sampler detects a timeline reset.
    pub fn reset_timing(&mut self) {
        self.last_inference = None;
        self.last_detection = None;
    }

    fn decide(&self, changed_percent: u8, now: Duration) -> GateDecision {
        if self.reference.is_none() {
            // The first frame of a connection has nothing to compare against.
            return GateDecision::Heartbeat;
        }
        if changed_percent >= self.policy.motion_threshold_percent {
            return GateDecision::Motion;
        }
        if self
            .last_detection
            .is_some_and(|last| now.saturating_sub(last) < self.policy.cooldown)
        {
            return GateDecision::Cooldown;
        }
        if self
            .last_inference
            .is_none_or(|last| now.saturating_sub(last) >= self.policy.heartbeat)
        {
            return GateDecision::Heartbeat;
        }
        GateDecision::Skip
    }

    /// The share of sampled bytes that differ from the reference, as a whole
    /// percentage.
    ///
    /// Sampled rather than exhaustive: every 64th byte is enough to separate a
    /// still scene from a moving one, and comparing a whole frame per sample
    /// would spend more than the inference it is trying to avoid.
    fn changed_fraction(&self, frame: &DecodedFrame) -> u8 {
        const STEP: usize = 64;
        const DIFFERENCE: u8 = 12;
        let Some(reference) = self.reference.as_deref() else {
            return 100;
        };
        let pixels = frame.pixels();
        if reference.len() != pixels.len() {
            return 100;
        }
        let mut examined = 0_u32;
        let mut differing = 0_u32;
        for index in (0..pixels.len()).step_by(STEP) {
            examined += 1;
            if pixels[index].abs_diff(reference[index]) > DIFFERENCE {
                differing += 1;
            }
        }

        if examined == 0 {
            return 0;
        }
        u8::try_from(differing.saturating_mul(100) / examined).unwrap_or(100)
    }
}

#[cfg(test)]
mod tests {
    use super::{GateDecision, GatePolicy, MotionGate};
    use std::{
        sync::Arc,
        time::{Duration, SystemTime},
    };
    use surveillance_common::types::CameraId;
    use surveillance_media::{
        frame::{DecodedFrame, FrameLimits, PixelFormat},
        packet::MediaTimestamp,
    };

    fn frame() -> DecodedFrame {
        DecodedFrame::new(
            CameraId::new("camera-01").unwrap_or_else(|error| panic!("camera: {error}")),
            1,
            MediaTimestamp::new(0, 90_000).unwrap_or_else(|error| panic!("timestamp: {error}")),
            SystemTime::UNIX_EPOCH,
            2,
            2,
            6,
            PixelFormat::Rgb24,
            Arc::from([0_u8; 12]),
            FrameLimits::new(2, 2, 12).unwrap_or_else(|error| panic!("limits: {error}")),
        )
        .unwrap_or_else(|error| panic!("frame: {error}"))
    }

    #[test]
    fn timing_reset_restarts_heartbeat_after_backward_media_time() {
        let policy = GatePolicy::new(100, Duration::from_secs(5), Duration::from_secs(10))
            .unwrap_or_else(|error| panic!("policy: {error}"));
        let mut gate = MotionGate::new(policy);
        let frame = frame();

        assert_eq!(
            gate.admit(&frame, Duration::from_secs(120)),
            GateDecision::Heartbeat
        );
        assert_eq!(gate.admit(&frame, Duration::ZERO), GateDecision::Skip);

        gate.reset_timing();

        assert_eq!(gate.admit(&frame, Duration::ZERO), GateDecision::Heartbeat);
    }
}
