//! Bounded, time-based evidence tracking. Predictions never become observations.
use crate::tracker::iou;
use std::time::Duration;
use surveillance_events::event::{
    DetectionSet, NormalizedBoundingBox, PresenceSnapshot, PresenceTrack, TrackEvidence,
};

#[derive(Clone, Copy, Debug)]
pub struct PresencePolicy {
    confirmation: Duration,
    grace: Duration,
    freshness: Duration,
}
impl PresencePolicy {
    #[must_use]
    pub const fn confirmation(self) -> Duration {
        self.confirmation
    }
    #[must_use]
    pub const fn grace(self) -> Duration {
        self.grace
    }
    #[must_use]
    pub const fn freshness(self) -> Duration {
        self.freshness
    }
    /// # Errors
    /// Refuses unbounded, zero or inconsistent evidence durations.
    pub fn new(
        confirmation: Duration,
        grace: Duration,
        freshness: Duration,
    ) -> Result<Self, &'static str> {
        if [confirmation, grace, freshness]
            .iter()
            .any(|d| d.is_zero() || *d > Duration::from_secs(60))
            || freshness > grace
        {
            return Err("invalid presence durations");
        }
        Ok(Self {
            confirmation,
            grace,
            freshness,
        })
    }
}
struct Track {
    id: u32,
    bbox: NormalizedBoundingBox,
    first: Duration,
    last: Duration,
    confirmed: bool,
    observed: bool,
}
pub struct PresenceTracker {
    tracks: Vec<Track>,
    next: u32,
    policy: PresencePolicy,
    saturated: bool,
    last_sample: Option<Duration>,
}
impl PresenceTracker {
    #[must_use]
    pub const fn new(policy: PresencePolicy) -> Self {
        Self {
            tracks: Vec::new(),
            next: 1,
            policy,
            saturated: false,
            last_sample: None,
        }
    }
    pub fn reset(&mut self) {
        self.tracks.clear();
        self.next = 1;
        self.last_sample = None;
        self.saturated = false;
    }
    pub fn expire(&mut self, now: Duration) {
        self.tracks.retain(|t| {
            now.saturating_sub(t.last)
                < if t.confirmed {
                    self.policy.grace
                } else {
                    self.policy.confirmation
                }
        });
    }
    /// # Errors
    /// ID exhaustion requires an explicit new session, never silent key reuse.
    pub fn observe(
        &mut self,
        set: &DetectionSet,
        now: Duration,
        complete: bool,
    ) -> Result<(), &'static str> {
        if self.last_sample.is_some_and(|last| now <= last) {
            return Err("out of order observation");
        }
        self.expire(now);
        self.saturated = !complete;
        for t in &mut self.tracks {
            t.observed = false;
        }
        let mut pairs = Vec::with_capacity(self.tracks.len() * set.as_slice().len());
        for (ti, track) in self.tracks.iter().enumerate() {
            for (di, det) in set.as_slice().iter().enumerate() {
                let overlap = iou(track.bbox, det.bounding_box);
                if overlap >= 0.3 {
                    pairs.push((overlap, ti, di));
                }
            }
        }
        pairs.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        let mut matched = vec![false; self.tracks.len()];
        let mut used = vec![false; set.as_slice().len()];
        for (_, ti, di) in pairs {
            if matched[ti] || used[di] {
                continue;
            }
            let t = &mut self.tracks[ti];
            t.confirmed |= now.saturating_sub(t.first) <= self.policy.confirmation;
            t.last = now;
            t.observed = true;
            t.bbox = set.as_slice()[di].bounding_box;
            matched[ti] = true;
            used[di] = true;
        }
        for (di, det) in set.as_slice().iter().enumerate() {
            if used[di] {
                continue;
            }
            if self.tracks.len() == 64 {
                if let Some(index) = self
                    .tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| !t.observed)
                    .min_by_key(|(_, t)| t.last)
                    .map(|(i, _)| i)
                {
                    self.tracks.swap_remove(index);
                } else {
                    self.saturated = true;
                    continue;
                }
                self.saturated = true;
            }
            let id = self.next;
            self.next = self.next.checked_add(1).ok_or("track ID exhausted")?;
            self.tracks.push(Track {
                id,
                bbox: det.bounding_box,
                first: now,
                last: now,
                confirmed: false,
                observed: true,
            });
        }
        self.last_sample = Some(now);
        Ok(())
    }
    #[must_use]
    pub fn snapshot(&mut self, now: Duration, available: bool) -> PresenceSnapshot {
        self.expire(now);
        let available = available
            && self
                .last_sample
                .is_some_and(|last| now.saturating_sub(last) <= self.policy.freshness);
        let tracks = self
            .tracks
            .iter()
            .map(|t| PresenceTrack {
                id: t.id,
                evidence: if !t.confirmed {
                    TrackEvidence::Tentative
                } else if available
                    && t.observed
                    && now.saturating_sub(t.last) <= self.policy.freshness
                {
                    TrackEvidence::Observed
                } else {
                    TrackEvidence::Coasting
                },
            })
            .collect();
        // Locally proven: internal state <=64, IDs nonzero/distinct, generated enums.
        PresenceSnapshot::new(tracks, available, self.saturated)
            .unwrap_or_else(|_| unreachable!("bounded tracker invariants"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use surveillance_events::event::{Confidence, PersonDetection};
    fn policy() -> PresencePolicy {
        PresencePolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap()
    }
    fn people(n: usize, top: f32) -> DetectionSet {
        DetectionSet::new(
            (0..n)
                .map(|i| PersonDetection {
                    confidence: Confidence::new(0.9).unwrap(),
                    bounding_box: NormalizedBoundingBox::new(
                        f32::from(u16::try_from(i).unwrap()) / 128.0,
                        top,
                        0.005,
                        0.1,
                    )
                    .unwrap(),
                    track_id: None,
                })
                .collect(),
        )
        .unwrap()
    }
    #[test]
    fn confirmation_misses_reacquisition_and_timer_expiry() {
        let mut t = PresenceTracker::new(policy());
        t.observe(&people(1, 0.1), Duration::ZERO, true).unwrap();
        assert_eq!(t.snapshot(Duration::ZERO, true).counts(), (0, 0, 1));
        t.observe(&people(1, 0.1), Duration::from_millis(200), true)
            .unwrap();
        let id = t.snapshot(Duration::from_millis(200), true).tracks()[0].id;
        assert_eq!(
            t.snapshot(Duration::from_millis(200), true).counts(),
            (1, 0, 0)
        );
        t.observe(&people(0, 0.1), Duration::from_millis(400), true)
            .unwrap();
        assert_eq!(
            t.snapshot(Duration::from_millis(400), true).counts(),
            (0, 1, 0)
        );
        t.observe(&people(1, 0.1), Duration::from_millis(600), true)
            .unwrap();
        assert_eq!(
            t.snapshot(Duration::from_millis(600), true).tracks()[0].id,
            id
        );
        assert!(t
            .snapshot(Duration::from_millis(1600), false)
            .tracks()
            .is_empty());
        assert!(!t.snapshot(Duration::from_millis(1600), false).available());
    }
    #[test]
    fn crowded_churn_bounds_internal_state_and_rejects_old_samples() {
        let mut t = PresenceTracker::new(policy());
        for i in 0..30 {
            t.observe(
                &people(64, if i % 2 == 0 { 0.1 } else { 0.7 }),
                Duration::from_millis(i * 10),
                true,
            )
            .unwrap();
            assert!(t.tracks.len() <= 64);
        }
        assert!(t.observe(&people(1, 0.1), Duration::ZERO, true).is_err());
        t.reset();
        assert!(t.snapshot(Duration::ZERO, false).tracks().is_empty());
        t.next = u32::MAX;
        assert!(t.observe(&people(1, 0.1), Duration::ZERO, true).is_err());
    }
}
