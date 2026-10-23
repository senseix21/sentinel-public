//! One owned decoder, two independently owned bounded inference lanes.
use crate::{
    detector::{
        DetectionExit, DetectionHealth, DetectionOwner, DetectionState, DetectionStopReason,
        H264Decoder, LaneCounters,
    },
    inference::{DetectionInference, InferenceDescriptor, InferenceResults},
    presence::{PresencePolicy, PresenceTracker},
    sampler::{SampleDecision, TimestampSampler},
};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime},
};
use surveillance_common::types::CameraId;
use surveillance_events::{
    bus::EventBus,
    event::{
        DeliveryClass, DomainEvent, EventId, EventPayload, OccurredAtUnixMicros, SourceMediaTime,
    },
};
use surveillance_media::{
    distribution::{MediaSubscriber, ReceiveOutcome},
    frame::{DecodedFrame, FrameLimits, PixelFormat},
    packet::H264AccessUnit,
};
use tokio::sync::{watch, Notify, Semaphore};

static EMISSION: AtomicU64 = AtomicU64::new(1);
static SESSION: AtomicU64 = AtomicU64::new(1);
static PERMITS: OnceLock<(usize, Arc<Semaphore>, Arc<Semaphore>)> = OnceLock::new();
/// Separate global lane budgets prevent hung face work consuming person slots.
/// # Errors
/// Refuses inconsistent process-wide limits rather than silently ignoring configuration.
pub fn admission(limit: usize) -> Result<(Arc<Semaphore>, Arc<Semaphore>), &'static str> {
    if limit == 0 || limit > 64 {
        return Err("invalid inference concurrency");
    }
    let held = PERMITS.get_or_init(|| {
        (
            limit,
            Arc::new(Semaphore::new(limit)),
            Arc::new(Semaphore::new(limit)),
        )
    });
    if held.0 != limit {
        return Err("inference concurrency requires process restart");
    }
    Ok((held.1.clone(), held.2.clone()))
}
#[derive(Clone)]
struct Job {
    frame: DecodedFrame,
    admitted: Instant,
    session: u64,
}
#[derive(Default)]
struct Latest {
    pending: Mutex<Option<Job>>,
    ready: Notify,
    dropped: AtomicU64,
    session: AtomicU64,
}
impl Latest {
    fn push(&self, job: Job) {
        if self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(job)
            .is_some()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        self.ready.notify_one();
    }
    fn take(&self) -> Option<Job> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
    fn clear(&self) {
        self.take();
    }
}
/// Resize canonical RGB to one model's independently validated input geometry.
/// # Errors
/// Rejects geometry/budget mismatches before allocating a tensor input frame.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "geometry bounded to 4096 and bilinear convex RGB sums remain 0..255"
)]
pub fn resize(
    frame: &DecodedFrame,
    width: u32,
    height: u32,
) -> Result<DecodedFrame, crate::inference::InferenceError> {
    use crate::inference::{InferenceError, InferenceFailureKind};
    let err = || InferenceError::new(InferenceFailureKind::MalformedOutput);
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || frame.width() > 4096
        || frame.height() > 4096
        || frame.pixel_format() != PixelFormat::Rgb24
    {
        return Err(err());
    }
    if width == frame.width() && height == frame.height() {
        return Ok(frame.clone());
    }
    let stride = width as usize * 3;
    let mut pixels = vec![0; stride * height as usize];
    // Bilinear RGB resize includes existing letterbox padding. Models own color conversion.
    for y in 0..height as usize {
        for x in 0..width as usize {
            let fx =
                ((x as f64 + 0.5) * f64::from(frame.width()) / f64::from(width) - 0.5).max(0.0);
            let fy =
                ((y as f64 + 0.5) * f64::from(frame.height()) / f64::from(height) - 0.5).max(0.0);
            let x0 = (fx as usize).min(frame.width() as usize - 1);
            let x1 = (x0 + 1).min(frame.width() as usize - 1);
            let y0 = (fy as usize).min(frame.height() as usize - 1);
            let y1 = (y0 + 1).min(frame.height() as usize - 1);
            let ax = (fx - x0 as f64).min(1.0);
            let ay = (fy - y0 as f64).min(1.0);
            for c in 0..3 {
                let at = |xx, yy| f64::from(frame.pixels()[yy * frame.stride() + xx * 3 + c]);
                pixels[y * stride + x * 3 + c] = ((at(x0, y0) * (1.0 - ax) + at(x1, y0) * ax)
                    * (1.0 - ay)
                    + (at(x0, y1) * (1.0 - ax) + at(x1, y1) * ax) * ay)
                    .round() as u8;
            }
        }
    }
    DecodedFrame::new(
        frame.camera_id().clone(),
        frame.connection_epoch(),
        frame.media_timestamp(),
        frame.observed_at(),
        width,
        height,
        stride,
        PixelFormat::Rgb24,
        pixels.into(),
        FrameLimits::new(width, height, stride * height as usize).map_err(|_| err())?,
    )
    .map_err(|_| err())
}
struct Lane {
    camera: CameraId,
    bus: EventBus,
    health: DetectionHealth,
    pending: Arc<Latest>,
    stop: watch::Receiver<bool>,
    model: Option<Box<dyn DetectionInference>>,
    geometry: (u32, u32),
    person: bool,
    policy: PresencePolicy,
    permits: Arc<Semaphore>,
    roi: Option<[f32; 4]>,
}
impl Lane {
    #[allow(
        clippy::too_many_lines,
        reason = "one linear owned lane keeps permits, model transfer and publication visible"
    )]
    async fn run(mut self) {
        let mut tracker = PresenceTracker::new(self.policy);
        let mut identity = None;
        let mut retry_at = None;
        let mut failure_streak = 0u32;
        let origin = Instant::now();
        let mut last_evidence: Option<(u64, SourceMediaTime, bool)> = None;
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let notified = self.pending.ready.notified();
            tokio::select! {
                biased;
                _=self.stop.changed()=>break,
                _=tick.tick()=> {
                    if self.person { if let Some((session,time,fixture))=&last_evidence { self.publish_presence_time(*session,*time,&mut tracker,origin.elapsed(),self.model.is_some() && self.is_running(),*fixture); } }
                    continue;
                },
                ()=notified=>{},
            }
            let Some(job) = self.pending.take() else {
                continue;
            };
            if job.session != self.pending.session.load(Ordering::Relaxed) {
                continue;
            }
            if retry_at.is_some_and(|at: Instant| Instant::now() < at) {
                self.pending.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let Some(mut model) = self.model.take() else {
                if self.person {
                    self.publish_presence(&job, &mut tracker, origin.elapsed(), false);
                }
                continue;
            };
            let permit = tokio::select! { _=self.stop.changed()=>break, permit=self.permits.clone().acquire_owned()=>match permit {Ok(p)=>p,Err(_)=>break} };
            if job.session != self.pending.session.load(Ordering::Relaxed)
                || job.admitted.elapsed() > self.policy.freshness()
            {
                self.model = Some(model);
                self.with_counters(|c| c.stale += 1);
                continue;
            }
            let frame = job.frame.clone();
            let (width, height) = self.geometry;
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let preprocess = Instant::now();
                let resized = resize(&frame, width, height);
                let preprocessing = preprocess.elapsed();
                let infer = Instant::now();
                let output = resized.and_then(|f| model.infer_detections(&f));
                let inference = infer.elapsed();
                (model, output, preprocessing, inference)
            })
            .await;
            let Ok((model, result, preprocessing, inference)) = result else {
                self.set_state(DetectionState::Terminal);
                continue;
            };
            self.with_counters(|c| {
                c.preprocessing_micros =
                    c.preprocessing_micros.saturating_add(micros(preprocessing));
                c.inference_micros = c.inference_micros.saturating_add(micros(inference));
                let elapsed = micros(inference);
                for (i, bound) in [
                    10_000,
                    50_000,
                    100_000,
                    250_000,
                    500_000,
                    1_000_000,
                    u64::MAX,
                ]
                .iter()
                .enumerate()
                {
                    if elapsed <= *bound {
                        c.latency_buckets[i] += 1;
                    }
                }
            });
            let fixture = !model.is_production_model();
            let dropped = model.output_capacity_dropped();
            self.model = Some(model);
            {
                let mut h = self
                    .health
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                h.counters.inference_attempts += 1;
                h.counters.output_capacity_dropped =
                    h.counters.output_capacity_dropped.saturating_add(dropped);
            }
            if job.session != self.pending.session.load(Ordering::Relaxed) {
                continue;
            }
            if job.admitted.elapsed() > self.policy.freshness() {
                self.pending.dropped.fetch_add(1, Ordering::Relaxed);
                self.with_counters(|c| c.stale += 1);
                continue;
            }
            match result {
                Err(error) => {
                    use crate::inference::InferenceFailureKind;
                    self.set_state(DetectionState::Degraded);
                    self.with_counters(|c| c.failed += 1);
                    failure_streak = failure_streak.saturating_add(1);
                    let backoff = Duration::from_millis(
                        100u64.saturating_mul(1u64 << failure_streak.min(5)) + job.session % 97,
                    );
                    retry_at = Some(Instant::now() + backoff);
                    self.health
                        .0
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .counters
                        .inference_failed += 1;
                    if matches!(
                        error.kind(),
                        InferenceFailureKind::Terminal | InferenceFailureKind::MalformedOutput
                    ) {
                        self.model = None;
                        self.set_state(DetectionState::Terminal);
                    }
                    if self.person {
                        self.publish_presence(&job, &mut tracker, origin.elapsed(), false);
                    }
                    // Bound retry work: at most one attempt per admitted sample; no spawned retry queue.
                }
                Ok(output) => {
                    self.set_state(DetectionState::Running);
                    self.with_counters(|c| c.completed += 1);
                    failure_streak = 0;
                    retry_at = None;
                    if matches!(&output, InferenceResults::Persons(_)) != self.person {
                        self.model = None;
                        self.set_state(DetectionState::Terminal);
                        continue;
                    }
                    self.health
                        .0
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .counters
                        .inferred += 1;
                    let current = (job.session, job.frame.connection_epoch());
                    if identity != Some(current) {
                        tracker.reset();
                        identity = Some(current);
                    }
                    match output {
                        InferenceResults::Persons(set) => {
                            let set = match self.roi {
                                Some(rect) => {
                                    crate::detector::filter_detections_outside(&set, rect)
                                }
                                None => set,
                            };
                            if tracker
                                .observe(
                                    &set,
                                    job.admitted.saturating_duration_since(origin),
                                    dropped == 0,
                                )
                                .is_err()
                            {
                                self.model = None;
                                self.set_state(DetectionState::Terminal);
                                continue;
                            }
                            self.publish_presence_with_fixture(
                                &job,
                                &mut tracker,
                                origin.elapsed(),
                                true,
                                fixture,
                            );
                            last_evidence =
                                source_time(&job).map(|time| (job.session, time, fixture));
                        }
                        InferenceResults::Faces(set) => {
                            let set = match self.roi {
                                Some([left, top, width, height]) => set.retain(|d| {
                                    let center_x =
                                        d.bounding_box.left() + d.bounding_box.width() / 2.0;
                                    let center_y =
                                        d.bounding_box.top() + d.bounding_box.height() / 2.0;
                                    !(center_x >= left
                                        && center_x <= left + width
                                        && center_y >= top
                                        && center_y <= top + height)
                                }),
                                None => set,
                            };
                            if let Some(time) = source_time(&job) {
                                self.publish(
                                    job.session,
                                    EventPayload::FaceDetectionObserved {
                                        session: std::num::NonZeroU64::new(job.session),
                                        media_time: time,
                                        model: self.model.as_ref().map_or_else(
                                            || unreachable!("model returned"),
                                            |m| m.model_name().clone(),
                                        ),
                                        model_version: self.model.as_ref().map_or_else(
                                            || unreachable!("model returned"),
                                            |m| m.model_version().clone(),
                                        ),
                                        results_are_fixture: fixture,
                                        detections: set,
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
        self.pending.clear();
        self.set_state(DetectionState::Stopped);
    }
    fn is_running(&self) -> bool {
        let h = self
            .health
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (if self.person {
            h.person_lane
        } else {
            h.face_lane
        }) == Some(DetectionState::Running)
    }
    fn with_counters(&self, update: impl FnOnce(&mut LaneCounters)) {
        let mut h = self
            .health
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        update(if self.person {
            &mut h.person_counters
        } else {
            &mut h.face_counters
        });
    }
    fn set_state(&self, state: DetectionState) {
        let mut h = self
            .health
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.person {
            h.person_lane = Some(state);
        } else {
            h.face_lane = Some(state);
        }
    }
    fn publish_presence(
        &self,
        job: &Job,
        tracker: &mut PresenceTracker,
        now: Duration,
        available: bool,
    ) {
        self.publish_presence_with_fixture(
            job,
            tracker,
            now,
            available,
            self.model.as_ref().is_none_or(|m| !m.is_production_model()),
        );
    }
    fn publish_presence_with_fixture(
        &self,
        job: &Job,
        tracker: &mut PresenceTracker,
        now: Duration,
        available: bool,
        fixture: bool,
    ) {
        if let Some(time) = source_time(job) {
            self.publish_presence_time(job.session, time, tracker, now, available, fixture);
        }
    }
    fn publish_presence_time(
        &self,
        session: u64,
        time: SourceMediaTime,
        tracker: &mut PresenceTracker,
        now: Duration,
        available: bool,
        fixture: bool,
    ) {
        if session != self.pending.session.load(Ordering::Relaxed) {
            return;
        }
        self.publish(
            session,
            EventPayload::PersonPresenceObserved {
                media_time: time,
                session: std::num::NonZeroU64::new(session)
                    .unwrap_or_else(|| unreachable!("session IDs start at one")),
                results_are_fixture: fixture,
                presence: tracker.snapshot(now, available),
            },
        );
    }
    fn publish(&self, session: u64, payload: EventPayload) {
        let micros = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_micros()).ok())
            .and_then(|m| OccurredAtUnixMicros::new(m).ok());
        let id = EventId::new(format!(
            "presence-{}-{}",
            session,
            EMISSION.fetch_add(1, Ordering::Relaxed)
        ));
        if let (Some(at), Ok(id)) = (micros, id) {
            if let Ok(event) = DomainEvent::new(
                id,
                at,
                Some(self.camera.clone()),
                DeliveryClass::Detection,
                payload,
            ) {
                let report = self.bus.publish(Arc::new(event));
                let mut h = self
                    .health
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                h.counters.events_published += 1;
                h.counters.event_delivery_losses = h.counters.event_delivery_losses.saturating_add(
                    report.dropped_newest + report.rejected_full + report.replaced_oldest,
                );
            }
        }
    }
}
fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}
fn source_time(job: &Job) -> Option<SourceMediaTime> {
    Some(SourceMediaTime {
        epoch: std::num::NonZeroU64::new(job.frame.connection_epoch())?,
        ticks: job.frame.media_timestamp().ticks(),
        timebase_hz: std::num::NonZeroU64::new(u64::from(
            job.frame.media_timestamp().timebase_hz(),
        ))?,
    })
}
/// Complete combined pipeline input, owned at composition.
pub struct CombinedConfig {
    pub camera: CameraId,
    pub person: Option<Box<dyn DetectionInference>>,
    pub face: Option<Box<dyn DetectionInference>>,
    pub person_geometry: (u32, u32),
    pub face_geometry: (u32, u32),
    pub policy: PresencePolicy,
    pub permits: (Arc<Semaphore>, Arc<Semaphore>),
    pub face_interval: Duration,
    pub roi: Option<[f32; 4]>,
}
impl DetectionOwner {
    #[allow(
        clippy::too_many_lines,
        reason = "owned decoder and lane shutdown are reviewed together"
    )]
    pub fn start_combined<D: H264Decoder>(
        config: CombinedConfig,
        mut input: MediaSubscriber<H264AccessUnit>,
        mut sampler: TimestampSampler,
        mut decoder: D,
        bus: EventBus,
    ) -> (Self, DetectionHealth) {
        let health = DetectionHealth::disabled();
        {
            let mut h = health
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            h.state = DetectionState::Starting;
            h.backend = Some(InferenceDescriptor::combined(
                config
                    .person
                    .as_ref()
                    .is_some_and(|m| m.is_production_model())
                    && config
                        .face
                        .as_ref()
                        .is_some_and(|m| m.is_production_model()),
            ));
            h.person_lane = Some(if config.person.is_some() {
                DetectionState::Starting
            } else {
                DetectionState::Terminal
            });
            h.face_lane = Some(if config.face.is_some() {
                DetectionState::Starting
            } else {
                DetectionState::Terminal
            });
        }
        let (shutdown, stop) = watch::channel(false);
        let person = Arc::new(Latest::default());
        let face = Arc::new(Latest::default());
        let person_task = tokio::spawn(
            Lane {
                camera: config.camera.clone(),
                bus: bus.clone(),
                health: health.clone(),
                pending: person.clone(),
                stop: stop.clone(),
                model: config.person,
                geometry: config.person_geometry,
                person: true,
                policy: config.policy,
                permits: config.permits.0,
                roi: config.roi,
            }
            .run(),
        );
        let face_task = tokio::spawn(
            Lane {
                camera: config.camera.clone(),
                bus,
                health: health.clone(),
                pending: face.clone(),
                stop: stop.clone(),
                model: config.face,
                geometry: config.face_geometry,
                person: false,
                policy: config.policy,
                permits: config.permits.1,
                roi: config.roi,
            }
            .run(),
        );
        let task_health = health.clone();
        let task_shutdown = shutdown.clone();
        let mut stop = stop;
        let task = tokio::spawn(async move {
            let mut session = SESSION.fetch_add(1, Ordering::Relaxed);
            let mut epoch = None;
            let mut sequence = 0u64;
            let mut face_at = None;
            let reason = loop {
                let unit = tokio::select! {_=stop.changed()=>break DetectionStopReason::Requested,item=input.receive()=>match item {ReceiveOutcome::Item(u)=>u,ReceiveOutcome::Closed=>break DetectionStopReason::InputClosed,ReceiveOutcome::Empty=>continue}};
                if epoch != Some(unit.connection_epoch()) {
                    task_health
                        .0
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .state = DetectionState::Degraded;
                    decoder.reset();
                    epoch = Some(unit.connection_epoch());
                    session = SESSION.fetch_add(1, Ordering::Relaxed);
                    person.clear();
                    face.clear();
                    person.session.store(session, Ordering::Relaxed);
                    face.session.store(session, Ordering::Relaxed);
                    face_at = None;
                }
                task_health
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .counters
                    .decode_attempts += 1;
                let cam = config.camera.clone();
                let observed = SystemTime::now();
                let started = Instant::now();
                let decode_task = tokio::task::spawn_blocking(move || {
                    let result = decoder.decode(&cam, &unit, observed);
                    (decoder, result)
                })
                .await;
                let Ok((returned, result)) = decode_task else {
                    break DetectionStopReason::TerminalDecode;
                };
                decoder = returned;
                {
                    let mut h = task_health
                        .0
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    h.counters.decode_duration_micros =
                        h.counters.decode_duration_micros.saturating_add(
                            u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
                        );
                    h.input = input.metrics();
                    h.sampler = sampler.counters();
                }
                let frame = match result {
                    Ok(Some(f)) => {
                        task_health
                            .0
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .state = DetectionState::Running;
                        f
                    }
                    Ok(None) => continue,
                    Err(_) => {
                        task_health
                            .0
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .state = DetectionState::Degraded;
                        decoder.reset();
                        session = SESSION.fetch_add(1, Ordering::Relaxed);
                        person.clear();
                        face.clear();
                        person.session.store(session, Ordering::Relaxed);
                        face.session.store(session, Ordering::Relaxed);
                        task_health
                            .0
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .counters
                            .decode_failed += 1;
                        continue;
                    }
                };
                match sampler.consider(frame.connection_epoch(), frame.media_timestamp()) {
                    Ok(SampleDecision::Sample) => {}
                    Ok(SampleDecision::ResetAndSample) => {
                        session = SESSION.fetch_add(1, Ordering::Relaxed);
                        person.clear();
                        face.clear();
                        person.session.store(session, Ordering::Relaxed);
                        face.session.store(session, Ordering::Relaxed);
                        face_at = None;
                    }
                    _ => continue,
                }
                sequence = match sequence.checked_add(1) {
                    Some(s) => s,
                    None => break DetectionStopReason::TerminalInference,
                };
                let job = Job {
                    frame,
                    admitted: Instant::now(),
                    session,
                };
                person.push(job.clone());
                task_health
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .person_counters
                    .offered += 1;
                if face_at.is_none_or(|at: Instant| {
                    job.admitted.duration_since(at) >= config.face_interval
                }) {
                    face_at = Some(job.admitted);
                    face.push(job);
                    task_health
                        .0
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .face_counters
                        .offered += 1;
                }
                let mut h = task_health
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                h.counters.decoded += 1;
                h.person_counters.replaced = person.dropped.load(Ordering::Relaxed);
                h.face_counters.replaced = face.dropped.load(Ordering::Relaxed);
            };
            task_shutdown.send_replace(true);
            let _joined = tokio::join!(person_task, face_task);
            let mut h = task_health
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            h.state = DetectionState::Stopped;
            DetectionExit {
                reason,
                sampler: sampler.counters(),
                counters: h.counters,
                input: input.metrics(),
            }
        });
        (Self { shutdown, task }, health)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        detector::FixtureH264Decoder,
        inference::{
            DetectionMode, DeterministicFakeInference, InferenceError, InferenceFailureKind,
        },
        sampler::SampleRateHz,
    };
    use surveillance_events::{
        bus::SubscriptionCapacity,
        event::{FaceDetectionSet, ModelName, ModelVersion},
    };
    use surveillance_media::{
        distribution::{MediaDistributor, QueueCapacity},
        packet::{H264AccessUnitAssembler, H264NalUnit, MediaTimestamp},
    };
    struct Faces {
        name: ModelName,
        version: ModelVersion,
        fail: bool,
        delay: Duration,
    }
    impl DetectionInference for Faces {
        fn model_name(&self) -> &ModelName {
            &self.name
        }
        fn model_version(&self) -> &ModelVersion {
            &self.version
        }
        fn mode(&self) -> DetectionMode {
            DetectionMode::Face
        }
        fn is_production_model(&self) -> bool {
            false
        }
        fn infer_detections(
            &mut self,
            _: &DecodedFrame,
        ) -> Result<InferenceResults, InferenceError> {
            std::thread::sleep(self.delay);
            if self.fail {
                Err(InferenceError::new(InferenceFailureKind::Terminal))
            } else {
                Ok(InferenceResults::Faces(
                    FaceDetectionSet::new(vec![]).unwrap(),
                ))
            }
        }
    }
    fn keyframe(ticks: i64) -> Arc<H264AccessUnit> {
        let time = MediaTimestamp::new(ticks, 90000).unwrap();
        let mut assembler = H264AccessUnitAssembler::new();
        for (bytes, marker) in [
            (&[0x67, 1][..], false),
            (&[0x68, 1][..], false),
            (&[0x65, 1][..], true),
        ] {
            if let Some(unit) = assembler
                .push(
                    time,
                    H264NalUnit::from_ebsp(Arc::from(bytes)).unwrap(),
                    marker,
                )
                .unwrap()
            {
                return Arc::new(unit.with_connection_epoch(1).unwrap());
            }
        }
        panic!("fixture assembler");
    }
    fn policy() -> PresencePolicy {
        PresencePolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap()
    }
    async fn run_isolation(face: Option<Box<dyn DetectionInference>>) {
        let capacity = QueueCapacity::new(8).unwrap();
        let (distributor, subscriptions) =
            MediaDistributor::new_with_detection(capacity, capacity, capacity);
        let bus = EventBus::new();
        let mut events = bus
            .subscribe(SubscriptionCapacity::new(64).unwrap())
            .unwrap();
        let config = CombinedConfig {
            camera: CameraId::new("test-camera").unwrap(),
            person: Some(Box::new(DeterministicFakeInference::new().unwrap())),
            face,
            person_geometry: (2, 2),
            face_geometry: (2, 2),
            policy: policy(),
            permits: (Arc::new(Semaphore::new(1)), Arc::new(Semaphore::new(1))),
            face_interval: Duration::from_millis(200),
            roi: None,
        };
        let (owner, health) = DetectionOwner::start_combined(
            config,
            subscriptions.detection,
            TimestampSampler::new(SampleRateHz::new(5).unwrap(), Duration::from_secs(2)).unwrap(),
            FixtureH264Decoder::new(FrameLimits::new(2, 2, 12).unwrap(), 2, 2),
            bus,
        );
        for i in 0..3 {
            let report = distributor.publish(keyframe(i * 18000));
            assert_eq!(
                report.recorder,
                surveillance_media::distribution::PublishOutcome::Accepted
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let observed = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = events.receive().await.unwrap();
                if let EventPayload::PersonPresenceObserved { presence, .. } = &event.payload {
                    if presence.counts().0 == 1 {
                        break presence.clone();
                    }
                }
            }
        })
        .await
        .unwrap();
        assert!(observed.available());
        assert_eq!(observed.counts(), (1, 0, 0));
        for _ in 0..3 {
            assert!(matches!(
                subscriptions.recorder.try_receive(),
                ReceiveOutcome::Item(_)
            ));
        }
        let exit = owner.stop_and_join().await.unwrap();
        assert_eq!(exit.reason, DetectionStopReason::Requested);
        assert_eq!(health.snapshot().person_lane, Some(DetectionState::Stopped));
    }
    #[tokio::test]
    async fn absent_face_lane_does_not_gate_back_facing_or_stationary_person_evidence() {
        run_isolation(None).await;
    }
    #[tokio::test]
    async fn slow_terminal_face_lane_does_not_stop_presence_recording_delivery_or_shutdown() {
        run_isolation(Some(Box::new(Faces {
            name: ModelName::new("test-face").unwrap(),
            version: ModelVersion::new("1").unwrap(),
            fail: true,
            delay: Duration::from_millis(250),
        })))
        .await;
    }
    struct FailingDecoder {
        inner: FixtureH264Decoder,
        fail: Arc<std::sync::atomic::AtomicBool>,
    }
    impl H264Decoder for FailingDecoder {
        fn reset(&mut self) {
            self.inner.reset();
        }
        fn decode(
            &mut self,
            camera: &CameraId,
            unit: &H264AccessUnit,
            at: SystemTime,
        ) -> Result<Option<DecodedFrame>, crate::detector::DecodeError> {
            if self.fail.swap(false, Ordering::Relaxed) {
                return Err(crate::detector::DecodeError::Unavailable);
            }
            self.inner.decode(camera, unit, at)
        }
    }
    #[tokio::test]
    async fn decode_failure_is_degraded_until_new_epoch_frames_resume() {
        let capacity = QueueCapacity::new(8).unwrap();
        let (distributor, subscriptions) =
            MediaDistributor::new_with_detection(capacity, capacity, capacity);
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (owner, health) = DetectionOwner::start_combined(
            CombinedConfig {
                camera: CameraId::new("test-camera").unwrap(),
                person: Some(Box::new(DeterministicFakeInference::new().unwrap())),
                face: None,
                person_geometry: (2, 2),
                face_geometry: (2, 2),
                policy: policy(),
                permits: (Arc::new(Semaphore::new(1)), Arc::new(Semaphore::new(1))),
                face_interval: Duration::from_millis(200),
                roi: None,
            },
            subscriptions.detection,
            TimestampSampler::new(SampleRateHz::new(5).unwrap(), Duration::from_secs(2)).unwrap(),
            FailingDecoder {
                inner: FixtureH264Decoder::new(FrameLimits::new(2, 2, 12).unwrap(), 2, 2),
                fail,
            },
            EventBus::new(),
        );
        distributor.publish(keyframe(0));
        tokio::time::timeout(Duration::from_secs(2), async {
            while health.snapshot().counters.decode_failed == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(health.snapshot().state, DetectionState::Degraded);
        let next = (*keyframe(0)).clone().with_connection_epoch(2).unwrap();
        distributor.publish(Arc::new(next));
        tokio::time::timeout(Duration::from_secs(2), async {
            while health.snapshot().counters.inferred == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(health.snapshot().state, DetectionState::Running);
        owner.stop_and_join().await.unwrap();
    }

    #[test]
    fn pending_slot_replaces_oldest_and_resize_preserves_rgb_channels() {
        let frame = DecodedFrame::new(
            CameraId::new("cam").unwrap(),
            1,
            MediaTimestamp::new(0, 90000).unwrap(),
            SystemTime::UNIX_EPOCH,
            2,
            2,
            6,
            PixelFormat::Rgb24,
            Arc::from([255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]),
            FrameLimits::new(2, 2, 12).unwrap(),
        )
        .unwrap();
        let slot = Latest::default();
        for session in 1..100 {
            slot.push(Job {
                frame: frame.clone(),
                admitted: Instant::now(),
                session,
            });
        }
        assert_eq!(slot.dropped.load(Ordering::Relaxed), 98);
        assert_eq!(slot.take().unwrap().session, 99);
        assert!(slot.take().is_none());
        let result = resize(&frame, 1, 1).unwrap();
        assert_eq!(result.pixels(), [128, 128, 128]);
        assert_eq!(result.connection_epoch(), 1);
    }
}
