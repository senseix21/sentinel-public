//! Owns effective configuration, applies changes, and reports freshness.
//!
//! Sequencing here is what ADR 008 actually requires:
//!
//! 1. Plan the whole batch against current effective values. Nothing is written
//!    unless the batch validates as a unit.
//! 2. Commit the settings rows and their audit rows in one transaction.
//! 3. Rebuild the effective configuration and rewrite the last-known-good cache.
//! 4. Publish the notification — strictly after the commit, and never as a
//!    reason to roll one back.
//!
//! A cache-write failure or a dropped notification does not fail the change.
//! The change happened; a notification is not evidence.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

use surveillance_common::types::CameraId;
use surveillance_config::{
    cache::{Cache, CacheContents, CacheError, CacheState, Freshness, StaleRetry},
    change::{plan, ChangeOutcome, ChangeReport, ProposedChange, UpdateRejection},
    classification::{ApplicationBoundary, Origin, SettingId},
    repository::RejectedRow,
    resolution::{Layer, ResolvedSettings, SettingKey},
    secret::Secret,
    value::SettingValue,
};
use surveillance_database::{audit_records, PostgresSettingsRepository};
use surveillance_events::{
    bus::EventBus,
    event::{
        ChangeOutcomeName, ChangedSetting, ChangedSettingSet, DeliveryClass, DomainEvent, EventId,
        EventPayload, OccurredAtUnixMicros, SettingKeyName,
    },
};
use tokio::sync::RwLock;

use crate::authorize::{Authorized, MutatingSurface};

/// Counters that make degradation diagnosable.
///
/// Audit-write and notification outcomes are counted separately and
/// deliberately: a change that is durably recorded but whose notification was
/// dropped is correct behavior, and must not read as a failure.
#[derive(Debug, Default)]
pub struct SettingsMetrics {
    pub rejected: AtomicU64,
    pub applied: AtomicU64,
    pub audit_written: AtomicU64,
    pub audit_failed: AtomicU64,
    pub notify_published: AtomicU64,
    pub notify_dropped: AtomicU64,
    pub cache_writes: AtomicU64,
    pub cache_write_failures: AtomicU64,
}

/// Everything `/metrics` needs, gathered under one lock acquisition.
///
/// Label cardinality is bounded by construction: origins, boundaries, and cache
/// states are closed enumerations, and no setting *value* appears anywhere.
pub struct ConfigurationMetricsView {
    pub generation: u64,
    pub freshness: Freshness,
    pub cache: CacheState,
    pub store_available: bool,
    pub mutating_surface_enabled: bool,
    pub last_success_age_seconds: Option<u64>,
    pub unreadable_rows: usize,
    /// Effective settings per origin. Four entries at most.
    pub by_origin: Vec<(Origin, usize)>,
    /// Settings awaiting each boundary. Five entries at most.
    pub pending_by_boundary: Vec<(ApplicationBoundary, usize)>,
    pub applied: u64,
    pub rejected: u64,
    pub audit_written: u64,
    pub audit_failed: u64,
    pub notify_published: u64,
    pub notify_dropped: u64,
    pub cache_writes: u64,
    pub cache_write_failures: u64,
}

/// A point-in-time view of the configuration.
pub struct SettingsSnapshot {
    pub generation: u64,
    pub freshness: Freshness,
    pub cache: CacheState,
    pub rejected_rows: Vec<RejectedRow>,
    pub pending: Vec<(SettingKey, ApplicationBoundary)>,
}

struct State {
    /// Layers below the persisted one, in precedence order. Kept so the
    /// persisted layer can be replaced without re-reading files or the
    /// environment.
    base: Vec<Layer>,
    /// The persisted layer, replaced wholesale on every successful load.
    persisted: Layer,
    /// The environment layer, which outranks everything.
    environment: Layer,
    resolved: ResolvedSettings,
    generation: u64,
    freshness: Freshness,
    cache: CacheState,
    rejected_rows: Vec<RejectedRow>,
    pending: Vec<(SettingKey, ApplicationBoundary)>,
}

impl State {
    fn rebuild(&mut self) {
        let mut layers = self.base.clone();
        layers.push(self.persisted.clone());
        layers.push(self.environment.clone());
        self.resolved = ResolvedSettings::resolve(layers);
    }
}

/// Owns the effective configuration for the process.
pub struct SettingsService {
    state: RwLock<State>,
    repository: Option<PostgresSettingsRepository>,
    cache: Cache,
    salt: Secret<String>,
    events: Option<EventBus>,
    metrics: Arc<SettingsMetrics>,
    retry: std::sync::Mutex<StaleRetry>,
    /// Fixed for the process: `api.mutating_surface_enabled` is a bootstrap
    /// setting, so it cannot be changed by the interface it gates.
    surface: MutatingSurface,
}

impl SettingsService {
    /// Builds the service from already-resolved bootstrap layers.
    ///
    /// `base` holds compiled defaults and the configuration file, in that
    /// order. `environment` is kept separate because it outranks the persisted
    /// layer, which is what makes a persisted change reportable as shadowed.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "each dependency is independently owned and separately testable; \
                  bundling them into a config struct would hide which are optional"
    )]
    pub fn new(
        base: Vec<Layer>,
        environment: Layer,
        repository: Option<PostgresSettingsRepository>,
        cache: Cache,
        salt: Secret<String>,
        events: Option<EventBus>,
        retry: StaleRetry,
        surface: MutatingSurface,
    ) -> Self {
        let mut state = State {
            base,
            persisted: Layer::new(Origin::Persisted),
            environment,
            resolved: ResolvedSettings::resolve(Vec::new()),
            generation: 0,
            freshness: Freshness::Stale {
                since: SystemTime::UNIX_EPOCH,
                cache: CacheState::Absent,
                last_success: None,
            },
            cache: CacheState::Absent,
            rejected_rows: Vec::new(),
            pending: Vec::new(),
        };
        state.rebuild();
        Self {
            state: RwLock::new(state),
            repository,
            cache,
            salt,
            events,
            metrics: Arc::new(SettingsMetrics::default()),
            retry: std::sync::Mutex::new(retry),
            surface,
        }
    }

    /// Whether this deployment exposes the mutating configuration surface.
    #[must_use]
    pub const fn surface(&self) -> MutatingSurface {
        self.surface
    }

    #[must_use]
    pub fn metrics(&self) -> Arc<SettingsMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Loads persisted settings, falling back to the last-known-good cache.
    ///
    /// Never falls back to defaults: several operational settings are
    /// destructive when they revert, and a silently reverted retention window
    /// deletes footage that policy says to keep. Returns the resulting
    /// freshness so the caller can log the transition.
    pub async fn load(&self) -> Freshness {
        let now = SystemTime::now();
        let loaded = match &self.repository {
            Some(repository) => repository.load_all().await.ok(),
            None => None,
        };

        let mut state = self.state.write().await;
        if let Some(persisted) = loaded {
            state.persisted = persisted.layer;
            state.generation = persisted.generation;
            state.rejected_rows = persisted.rejected;
            state.freshness = Freshness::Fresh { loaded_at: now };
            state.rebuild();
            drop(state);
            self.reset_retry();
            self.write_cache().await;
            return Freshness::Fresh { loaded_at: now };
        }

        // Store unavailable. The cache is the only acceptable source; defaults
        // are not. Reverting retention or watermarks to a default deletes
        // footage that policy says to keep.
        let (layer, generation, cache) = self.cache.load().map_or_else(
            |error| (Layer::new(Origin::Persisted), 0, error.state()),
            |contents| (contents.layer, contents.generation, CacheState::Valid),
        );
        state.persisted = layer;
        state.generation = generation;
        state.cache = cache;
        let freshness = Freshness::Stale {
            since: now,
            cache,
            last_success: None,
        };
        state.freshness = freshness;
        state.rebuild();
        freshness
    }

    /// Whether another load attempt is within the retry budget, and how long to
    /// wait first.
    ///
    /// `Stale` is transient. The budget outlasts typical service start ordering
    /// because the dominant cause is a local database that has not finished
    /// starting.
    #[must_use]
    pub fn next_retry_delay(&self) -> Option<Duration> {
        let mut retry = self
            .retry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let delay = retry.next_delay();
        if let Some(delay) = delay {
            retry.record_attempt(delay);
        }
        delay
    }

    fn reset_retry(&self) {
        self.retry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .reset();
    }

    /// A point-in-time view for the read endpoints and metrics.
    pub async fn snapshot(&self) -> SettingsSnapshot {
        let state = self.state.read().await;
        SettingsSnapshot {
            generation: state.generation,
            freshness: state.freshness,
            cache: state.cache,
            rejected_rows: state.rejected_rows.clone(),
            pending: state.pending.clone(),
        }
    }

    /// The effective value and origin of one setting.
    pub async fn effective(
        &self,
        id: SettingId,
        camera: Option<&CameraId>,
    ) -> Option<(String, Origin)> {
        let state = self.state.read().await;
        state
            .resolved
            .effective(id, camera)
            .map(|resolved| (resolved.value().render(), resolved.origin()))
    }

    /// Gathers everything `/metrics` needs in one pass.
    ///
    /// Fulfils `docs/phase-7/contracts.md` §6. Setting *identity* and outcome
    /// category are exposed; values never are.
    pub async fn metrics_view(&self) -> ConfigurationMetricsView {
        let state = self.state.read().await;
        let mut by_origin: Vec<(Origin, usize)> = Vec::with_capacity(Origin::ALL.len());
        for origin in Origin::ALL.iter().copied() {
            let count = SettingId::ALL
                .iter()
                .filter(|id| state.resolved.origin_of(**id, None) == Some(origin))
                .count();
            by_origin.push((origin, count));
        }

        let mut pending_by_boundary: Vec<(ApplicationBoundary, usize)> =
            Vec::with_capacity(ApplicationBoundary::ALL.len());
        for boundary in ApplicationBoundary::ALL.iter().copied() {
            let count = state
                .pending
                .iter()
                .filter(|(_, pending)| *pending == boundary)
                .count();
            pending_by_boundary.push((boundary, count));
        }

        let last_success_age_seconds = match state.freshness {
            Freshness::Fresh { loaded_at } => SystemTime::now()
                .duration_since(loaded_at)
                .ok()
                .map(|elapsed| elapsed.as_secs()),
            // A unit that has never loaded successfully has no age to report.
            // Reporting zero would read as "just loaded", which is the opposite
            // of the truth.
            Freshness::Stale { last_success, .. } => last_success.and_then(|at| {
                SystemTime::now()
                    .duration_since(at)
                    .ok()
                    .map(|elapsed| elapsed.as_secs())
            }),
        };

        ConfigurationMetricsView {
            generation: state.generation,
            freshness: state.freshness,
            cache: state.cache,
            store_available: self.repository.is_some() && !state.freshness.is_stale(),
            mutating_surface_enabled: self.surface.is_enabled(),
            last_success_age_seconds,
            unreadable_rows: state.rejected_rows.len(),
            by_origin,
            pending_by_boundary,
            applied: self.metrics.applied.load(Ordering::Relaxed),
            rejected: self.metrics.rejected.load(Ordering::Relaxed),
            audit_written: self.metrics.audit_written.load(Ordering::Relaxed),
            audit_failed: self.metrics.audit_failed.load(Ordering::Relaxed),
            notify_published: self.metrics.notify_published.load(Ordering::Relaxed),
            notify_dropped: self.metrics.notify_dropped.load(Ordering::Relaxed),
            cache_writes: self.metrics.cache_writes.load(Ordering::Relaxed),
            cache_write_failures: self.metrics.cache_write_failures.load(Ordering::Relaxed),
        }
    }

    /// The effective value of one setting, typed.
    ///
    /// Used by consumers that re-read an `Immediate` setting at each use.
    pub async fn effective_value(
        &self,
        id: SettingId,
        camera: Option<&CameraId>,
    ) -> Option<SettingValue> {
        let state = self.state.read().await;
        state
            .resolved
            .effective(id, camera)
            .map(|resolved| resolved.value().clone())
    }

    /// The value the store holds, which may differ from the effective value.
    ///
    /// Reported separately so an operator can see a setting whose stored and
    /// effective values disagree — the condition ADR 008 Decision 4 produces.
    pub async fn stored(&self, id: SettingId, camera: Option<&CameraId>) -> Option<String> {
        let state = self.state.read().await;
        state
            .resolved
            .layer_value(Origin::Persisted, id, camera)
            .map(SettingValue::render)
    }

    /// Applies a validated batch.
    ///
    /// # Errors
    /// Returns the rejection that refused the whole batch. Nothing is written
    /// when this returns an error.
    pub async fn update(
        &self,
        authorized: Authorized,
        changes: Vec<ProposedChange>,
    ) -> Result<ChangeReport, UpdateRejection> {
        let planned = {
            let state = self.state.read().await;
            plan(&state.resolved, changes, &self.salt)
        };
        let planned = match planned {
            Ok(planned) => planned,
            Err(rejection) => {
                self.metrics.rejected.fetch_add(1, Ordering::Relaxed);
                return Err(rejection);
            }
        };

        if planned.is_empty() {
            // Nothing to write and nothing to audit. Reporting success here is
            // honest: the requested state is the state.
            return Ok(planned.into_report(self.snapshot().await.generation));
        }

        let Some(repository) = &self.repository else {
            // Without a store there is nowhere durable to put the change, and
            // an in-memory-only change would silently disappear on restart.
            self.metrics.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(UpdateRejection::SurfaceDisabled);
        };

        let audit = audit_records(
            planned.entries(),
            authorized.actor(),
            Origin::Persisted,
            "applied",
            &self.salt,
        );
        let batch = surveillance_config::repository::SettingsBatch {
            entries: planned
                .entries()
                .iter()
                .map(|(key, value, _)| (key.clone(), value.clone()))
                .collect(),
            audit,
        };

        let Ok(generation) = repository.apply(&batch).await else {
            // Settings and audit share one transaction, so a failure means
            // neither committed.
            self.metrics.audit_failed.fetch_add(1, Ordering::Relaxed);
            self.metrics.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(UpdateRejection::ShuttingDown);
        };
        self.metrics
            .audit_written
            .fetch_add(batch.audit.len() as u64, Ordering::Relaxed);
        self.metrics.applied.fetch_add(1, Ordering::Relaxed);

        let report = planned.into_report(generation);
        self.adopt(&report, &batch.entries, generation).await;
        self.write_cache().await;
        self.notify(&report);
        Ok(report)
    }

    /// Folds a committed batch into the in-memory persisted layer.
    async fn adopt(
        &self,
        report: &ChangeReport,
        entries: &[(SettingKey, SettingValue)],
        generation: u64,
    ) {
        let mut state = self.state.write().await;
        for (key, value) in entries {
            let _ = state.persisted.insert(key.clone(), value.clone());
        }
        state.generation = generation;
        for (key, outcome) in &report.outcomes {
            state.pending.retain(|(pending, _)| pending != key);
            match outcome {
                ChangeOutcome::PendingBoundary { boundary } => {
                    state.pending.push((key.clone(), *boundary));
                }
                ChangeOutcome::PendingRestart => {
                    state
                        .pending
                        .push((key.clone(), ApplicationBoundary::ProcessRestart));
                }
                _ => {}
            }
        }
        state.rebuild();
    }

    /// Rewrites the last-known-good cache after a successful load or change.
    ///
    /// A failure here does not fail the caller: the settings are already valid
    /// and effective, and an unwritable cache degrades the next startup rather
    /// than this one.
    async fn write_cache(&self) {
        let (generation, layer) = {
            let state = self.state.read().await;
            (state.generation, state.persisted.clone())
        };
        match self.cache.store(&CacheContents { generation, layer }) {
            Ok(()) => {
                self.metrics.cache_writes.fetch_add(1, Ordering::Relaxed);
                let mut state = self.state.write().await;
                state.cache = CacheState::Valid;
            }
            Err(CacheError::Unwritable | CacheError::Absent | CacheError::Corrupt) => {
                self.metrics
                    .cache_write_failures
                    .fetch_add(1, Ordering::Relaxed);
                let mut state = self.state.write().await;
                state.cache = CacheState::Unwritable;
            }
        }
    }

    /// Publishes the notification, strictly after the commit.
    ///
    /// Overflow or a build failure is counted and ignored. The bus provides no
    /// durability, so a dropped notification means a consumer is briefly
    /// behind, not that the change was lost.
    fn notify(&self, report: &ChangeReport) {
        let Some(events) = &self.events else {
            return;
        };
        let changed: Vec<ChangedSetting> = report
            .outcomes
            .iter()
            .filter_map(|(key, outcome)| {
                Some(ChangedSetting {
                    key: SettingKeyName::new(key.id().key()).ok()?,
                    camera_id: key.camera().cloned(),
                    outcome: ChangeOutcomeName::new(outcome.as_str()).ok()?,
                })
            })
            .collect();

        let payload =
            ChangedSettingSet::new(changed).map(|changed| EventPayload::ConfigurationChanged {
                generation: report.generation,
                changed,
            });
        let Ok(payload) = payload else {
            self.metrics.notify_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };

        let occurred_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .and_then(|since| i64::try_from(since.as_micros()).ok())
            .and_then(|micros| OccurredAtUnixMicros::new(micros).ok());
        let event = EventId::new(format!("configuration-{}", report.generation))
            .ok()
            .zip(occurred_at)
            .and_then(|(event_id, occurred_at)| {
                DomainEvent::new(
                    event_id,
                    occurred_at,
                    None,
                    DeliveryClass::LatestState,
                    payload,
                )
                .ok()
            });

        match event {
            Some(event) => {
                let published = events.publish(Arc::new(event));
                // A coalesced latest-state event is delivered, not lost: the
                // newer generation supersedes the one it replaced.
                if published.accepted > 0 || published.coalesced > 0 {
                    self.metrics
                        .notify_published
                        .fetch_add(1, Ordering::Relaxed);
                } else if published.rejected_full > 0
                    || published.dropped_newest > 0
                    || published.closed > 0
                {
                    self.metrics.notify_dropped.fetch_add(1, Ordering::Relaxed);
                } else {
                    // No subscribers at all. Nothing was dropped; there was
                    // nobody to tell.
                    self.metrics
                        .notify_published
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            None => {
                self.metrics.notify_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}
