//! Detection relay: forwards `PersonDetectionObserved` events as JSON
//! `DetectionPacket`s on the room's data channel.
//!
//! The relay is owned by a `Session` (see `transcode.rs`): it is started when
//! the session connects and awaited when the session stops publishing. It uses
//! the room's unreliable lossy transport (`DataPacketKind::Lossy`), because
//! lossy delivery prevents TCP-style backpressure accumulating latency when a
//! viewer's link is slow — the client's staleness window absorbs the occasional
//! drop.
//!
//! The subscriber side filters by topic `"det/v1"`; the version string is the
//! wire contract.

use livekit::participant::LocalParticipant;
use livekit::DataPacket;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use surveillance_events::{
    bus::EventSubscription,
    event::{DetectionSet, DomainEvent, EventPayload, SourceMediaTime},
};
use tokio::sync::watch;
use tracing::{debug, warn};

/// Topic string the client uses to recognise our packets.
pub const DETECTION_TOPIC: &str = "det/v1";

/// One detection event, sent on every `PersonDetectionObserved`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionPacket {
    /// Camera this detection belongs to (empty when the event carries none).
    pub camera_id: String,
    /// Connection epoch from the source media time.
    pub epoch: u64,
    /// RTP clock ticks of the inferred frame. Carried for diagnostics; the
    /// client does not use them for alignment (see spec §2 C-1).
    pub media_ticks: i64,
    /// Timebase of `media_ticks` in Hz (typically `90_000` for H.264 RTP).
    pub timebase_hz: u64,
    /// `true` when the inference backend was a fixture. The client must
    /// not draw boxes when this is set.
    pub results_are_fixture: bool,
    /// Detections for this frame. May be empty.
    pub detections: Vec<DetectionEntry>,
}

/// One tracked person in a `DetectionPacket`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionEntry {
    /// Stable cross-frame identity, or null if the tracker is not running.
    pub track_id: Option<u32>,
    /// Normalised bounding box in `[0.0, 1.0]` of the camera frame.
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    /// Model confidence in `[0.0, 1.0]`.
    pub confidence: f32,
}

impl DetectionPacket {
    /// Builds a packet from the parts of a `PersonDetectionObserved` payload.
    #[must_use]
    pub fn from_detection_payload(
        camera_id: &str,
        media_time: &SourceMediaTime,
        detections: &DetectionSet,
        results_are_fixture: bool,
    ) -> Self {
        Self {
            camera_id: camera_id.to_owned(),
            epoch: media_time.epoch.get(),
            media_ticks: media_time.ticks,
            timebase_hz: media_time.timebase_hz.get(),
            results_are_fixture,
            detections: detections
                .as_slice()
                .iter()
                .map(|d| DetectionEntry {
                    track_id: d.track_id.map(std::num::NonZeroU32::get),
                    left: d.bounding_box.left(),
                    top: d.bounding_box.top(),
                    width: d.bounding_box.width(),
                    height: d.bounding_box.height(),
                    confidence: d.confidence.get(),
                })
                .collect(),
        }
    }
}

/// Pumps domain events to the room's data channel for the lifetime of a session.
pub struct DetectionRelay {
    sub: EventSubscription,
    participant: LocalParticipant,
    namespace: String,
    room: crate::domain::RoomName,
    sequence: u64,
    counters: Arc<crate::transcode::TranscodeCounters>,
}

impl DetectionRelay {
    #[must_use]
    pub const fn new(
        sub: EventSubscription,
        participant: LocalParticipant,
        namespace: String,
        room: crate::domain::RoomName,
        counters: Arc<crate::transcode::TranscodeCounters>,
    ) -> Self {
        Self {
            sub,
            participant,
            namespace,
            room,
            sequence: 0,
            counters,
        }
    }

    /// Runs until the shutdown signal fires or the event bus closes.
    ///
    /// Not cancel-safe to drop mid-call: hold the returned `JoinHandle` and
    /// signal `shutdown` instead of aborting the task.
    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    debug!("DetectionRelay: shutdown signalled");
                    break;
                }
                event = self.sub.receive() => match event {
                    None => {
                        debug!("DetectionRelay: event bus closed");
                        break;
                    }
                    Some(event) => self.handle(&event).await,
                },
            }
        }
    }

    async fn handle(&mut self, event: &Arc<DomainEvent>) {
        let Some(camera) = event.camera_id.as_ref() else {
            return;
        };
        if !event_matches_room(camera, &self.namespace, &self.room) {
            return;
        }
        if let EventPayload::PersonPresenceObserved {
            media_time,
            session,
            results_are_fixture,
            presence,
        } = &event.payload
        {
            self.sequence = self.sequence.saturating_add(1);
            let (observed, uncertain, tentative) = presence.counts();
            let packet = PresencePacket {
                camera_id: camera.as_str().to_owned(),
                epoch: media_time.epoch.to_string(),
                sequence: self.sequence.to_string(),
                session: session.to_string(),
                media_ticks: media_time.ticks.to_string(),
                timebase_hz: media_time.timebase_hz.get(),
                results_are_fixture: *results_are_fixture,
                available: presence.available(),
                saturated: presence.saturated(),
                observed,
                uncertain,
                tentative,
            };
            self.send(&packet, PRESENCE_TOPIC).await;
            return;
        }
        if let EventPayload::FaceDetectionObserved {
            session,
            media_time,
            detections,
            results_are_fixture,
            ..
        } = &event.payload
        {
            self.sequence = self.sequence.saturating_add(1);
            let mut packet = FacePacket::new(
                camera.as_str(),
                media_time,
                detections,
                *results_are_fixture,
                self.sequence,
            );
            packet.session = session.map(|s| s.to_string());
            self.send(&packet, FACE_TOPIC).await;
            return;
        }
        let EventPayload::PersonDetectionObserved {
            media_time,
            detections,
            results_are_fixture,
            ..
        } = &event.payload
        else {
            return;
        };
        let camera_id = event
            .camera_id
            .as_ref()
            .map(|id| id.as_str().to_owned())
            .unwrap_or_default();
        let packet = DetectionPacket::from_detection_payload(
            &camera_id,
            media_time,
            detections,
            *results_are_fixture,
        );
        self.send(&packet, DETECTION_TOPIC).await;
    }
    async fn send<T: Serialize>(&self, packet: &T, topic: &str) {
        let payload = match serde_json::to_vec(packet) {
            Ok(bytes) if bytes.len() <= MAX_DETECTION_PACKET_BYTES => bytes,
            _ => {
                self.counters
                    .relay_dropped
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                warn!("DetectionRelay: oversized or unserializable packet dropped");
                return;
            }
        };
        let data = DataPacket {
            payload,
            topic: Some(topic.to_owned()),
            reliable: false,
            destination_identities: vec![],
        };
        if let Err(error) = self.participant.publish_data(data).await {
            self.counters
                .relay_dropped
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            warn!(?error, "DetectionRelay: publish_data failed");
        }
    }
}

pub const PRESENCE_TOPIC: &str = "presence/v1";
#[derive(Debug, Serialize, Deserialize)]
pub struct PresencePacket {
    pub camera_id: String,
    pub epoch: String,
    pub sequence: String,
    pub session: String,
    pub media_ticks: String,
    pub timebase_hz: u64,
    pub results_are_fixture: bool,
    pub available: bool,
    pub saturated: bool,
    pub observed: usize,
    pub uncertain: usize,
    pub tentative: usize,
}

pub const FACE_TOPIC: &str = "face/v1";
pub const MAX_DETECTION_PACKET_BYTES: usize = 16 * 1024;
fn event_matches_room(
    camera: &surveillance_common::types::CameraId,
    namespace: &str,
    room: &crate::domain::RoomName,
) -> bool {
    crate::domain::PublicationIdentity::for_camera(namespace, camera)
        .is_ok_and(|identity| identity.room == *room)
}
#[derive(Debug, Serialize, Deserialize)]
pub struct FaceEntry {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub confidence: f32,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct FacePacket {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub camera_id: String,
    pub epoch: String,
    pub sequence: String,
    pub media_ticks: String,
    pub timebase_hz: u64,
    pub results_are_fixture: bool,
    pub detections: Vec<FaceEntry>,
}
impl FacePacket {
    #[must_use]
    pub fn new(
        camera: &str,
        time: &SourceMediaTime,
        set: &surveillance_events::event::FaceDetectionSet,
        fixture: bool,
        sequence: u64,
    ) -> Self {
        Self {
            session: None,
            camera_id: camera.to_owned(),
            epoch: time.epoch.to_string(),
            sequence: sequence.to_string(),
            media_ticks: time.ticks.to_string(),
            timebase_hz: time.timebase_hz.get(),
            results_are_fixture: fixture,
            detections: set
                .as_slice()
                .iter()
                .map(|face| FaceEntry {
                    left: face.bounding_box.left(),
                    top: face.bounding_box.top(),
                    width: face.bounding_box.width(),
                    height: face.bounding_box.height(),
                    confidence: face.confidence.get(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::{NonZeroU32, NonZeroU64};
    use surveillance_events::event::{
        Confidence, DetectionSet, NormalizedBoundingBox, PersonDetection,
    };

    fn source_time(epoch: u64, ticks: i64) -> SourceMediaTime {
        SourceMediaTime {
            epoch: NonZeroU64::new(epoch).unwrap(),
            ticks,
            timebase_hz: NonZeroU64::new(90_000).unwrap(),
        }
    }

    #[test]
    fn face_packets_are_distinct_scoped_and_within_wire_bound() {
        use surveillance_events::event::{FaceDetection, FaceDetectionSet};
        let camera = surveillance_common::types::CameraId::new("camera-01").unwrap();
        let room = crate::domain::PublicationIdentity::for_camera("dev", &camera)
            .unwrap()
            .room;
        assert!(event_matches_room(&camera, "dev", &room));
        assert!(!event_matches_room(
            &surveillance_common::types::CameraId::new("other").unwrap(),
            "dev",
            &room
        ));
        let face = FaceDetection {
            confidence: Confidence::new(0.7).unwrap(),
            bounding_box: NormalizedBoundingBox::new(0.1, 0.1, 0.2, 0.2).unwrap(),
        };
        let packet = FacePacket::new(
            camera.as_str(),
            &source_time(u64::MAX, 100),
            &FaceDetectionSet::new(vec![face; 64]).unwrap(),
            false,
            u64::MAX,
        );
        let bytes = serde_json::to_vec(&packet).unwrap();
        assert!(bytes.len() < MAX_DETECTION_PACKET_BYTES);
        let decoded: FacePacket = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.epoch, u64::MAX.to_string());
        assert_eq!(decoded.sequence, u64::MAX.to_string());
        assert!(!String::from_utf8(bytes).unwrap().contains("track_id"));
    }

    #[test]
    fn detection_packet_round_trips_json() {
        let det = PersonDetection {
            confidence: Confidence::new(0.85).unwrap(),
            bounding_box: NormalizedBoundingBox::new(0.1, 0.2, 0.3, 0.4).unwrap(),
            track_id: Some(NonZeroU32::new(7).unwrap()),
        };
        let ds = DetectionSet::new(vec![det]).unwrap();
        let packet = DetectionPacket::from_detection_payload(
            "cam-abc",
            &source_time(3, 900_000),
            &ds,
            false,
        );
        let json = serde_json::to_string(&packet).unwrap();
        let back: DetectionPacket = serde_json::from_str(&json).unwrap();
        assert_eq!(back.camera_id, "cam-abc");
        assert_eq!(back.epoch, 3);
        assert_eq!(back.media_ticks, 900_000);
        assert_eq!(back.timebase_hz, 90_000);
        assert!(!back.results_are_fixture);
        assert_eq!(back.detections.len(), 1);
        assert_eq!(back.detections[0].track_id, Some(7));
        assert!((back.detections[0].left - 0.1).abs() < 1e-5);
        assert!((back.detections[0].confidence - 0.85).abs() < 1e-5);
    }

    #[test]
    fn null_track_id_serialises_as_json_null() {
        let entry = DetectionEntry {
            track_id: None,
            left: 0.0,
            top: 0.0,
            width: 0.5,
            height: 0.5,
            confidence: 0.7,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(
            json.contains("\"track_id\":null"),
            "expected JSON null for track_id, got {json}"
        );
    }

    #[test]
    fn fixture_flag_is_preserved_in_json() {
        let packet = DetectionPacket {
            camera_id: "cam-1".to_owned(),
            epoch: 1,
            media_ticks: 0,
            timebase_hz: 90_000,
            results_are_fixture: true,
            detections: vec![],
        };
        let json = serde_json::to_string(&packet).unwrap();
        assert!(json.contains("\"results_are_fixture\":true"));
    }

    #[test]
    fn builds_packet_with_tracker_assigned_ids() {
        let det = PersonDetection {
            confidence: Confidence::new(0.75).unwrap(),
            bounding_box: NormalizedBoundingBox::new(0.1, 0.2, 0.3, 0.4).unwrap(),
            track_id: Some(NonZeroU32::new(5).unwrap()),
        };
        let ds = DetectionSet::new(vec![det]).unwrap();
        let packet =
            DetectionPacket::from_detection_payload("cam-1", &source_time(2, 180_000), &ds, false);
        assert_eq!(packet.detections.len(), 1);
        assert_eq!(packet.detections[0].track_id, Some(5));
    }
}
