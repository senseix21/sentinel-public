//! Constant-memory operational evidence for Phase 4 owners.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use surveillance_storage::retention::StoragePressure;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Phase4Snapshot {
    pub reconciliation_active: u64,
    pub reconciliation_passes: u64,
    pub reconciliation_failures: u64,
    pub reconciliation_examined: u64,
    pub reconciliation_imported: u64,
    pub reconciliation_incomplete: u64,
    pub reconciliation_inconsistent: u64,
    pub reconciliation_skipped_indexed: u64,
    pub recording_queries: u64,
    pub recording_query_failures: u64,
    pub recording_query_results: u64,
    pub recording_ranges_rejected: u64,
    pub playback_active: u64,
    pub playback_started: u64,
    pub playback_completed: u64,
    pub playback_cancelled: u64,
    pub playback_failures: u64,
    pub playback_buffer_bytes: u64,
    pub retention_passes: u64,
    pub retention_failures: u64,
    pub retention_candidates: u64,
    pub retention_deleted: u64,
    pub retention_delete_failures: u64,
    pub retention_bytes_reclaimed: u64,
    pub storage_total_bytes: u64,
    pub storage_free_bytes: u64,
    pub storage_pressure: StoragePressure,
}

#[derive(Clone, Default)]
pub struct Phase4Metrics(Arc<Counters>);

#[derive(Default)]
struct Counters {
    reconciliation_active: AtomicU64,
    reconciliation_passes: AtomicU64,
    reconciliation_failures: AtomicU64,
    reconciliation_examined: AtomicU64,
    reconciliation_imported: AtomicU64,
    reconciliation_incomplete: AtomicU64,
    reconciliation_inconsistent: AtomicU64,
    reconciliation_skipped_indexed: AtomicU64,
    recording_queries: AtomicU64,
    recording_query_failures: AtomicU64,
    recording_query_results: AtomicU64,
    recording_ranges_rejected: AtomicU64,
    playback_active: AtomicU64,
    playback_started: AtomicU64,
    playback_completed: AtomicU64,
    playback_cancelled: AtomicU64,
    playback_failures: AtomicU64,
    retention_passes: AtomicU64,
    retention_failures: AtomicU64,
    retention_candidates: AtomicU64,
    retention_deleted: AtomicU64,
    retention_delete_failures: AtomicU64,
    retention_bytes_reclaimed: AtomicU64,
    storage_total_bytes: AtomicU64,
    storage_free_bytes: AtomicU64,
    storage_pressure: AtomicU64,
}

impl Phase4Metrics {
    pub fn reconciliation_started(&self) {
        self.0.reconciliation_active.store(1, Ordering::Relaxed);
    }
    pub fn reconciliation(
        &self,
        examined: usize,
        imported: u64,
        incomplete: u64,
        inconsistent: u64,
        skipped_indexed: u64,
    ) {
        self.0.reconciliation_active.store(0, Ordering::Relaxed);
        self.0.reconciliation_passes.fetch_add(1, Ordering::Relaxed);
        self.0.reconciliation_examined.fetch_add(
            u64::try_from(examined).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        self.0
            .reconciliation_imported
            .fetch_add(imported, Ordering::Relaxed);
        self.0
            .reconciliation_incomplete
            .fetch_add(incomplete, Ordering::Relaxed);
        self.0
            .reconciliation_inconsistent
            .fetch_add(inconsistent, Ordering::Relaxed);
        self.0
            .reconciliation_skipped_indexed
            .fetch_add(skipped_indexed, Ordering::Relaxed);
    }
    pub fn reconciliation_failed(&self) {
        self.0.reconciliation_active.store(0, Ordering::Relaxed);
        self.0
            .reconciliation_failures
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn query(&self, results: usize) {
        self.0.recording_queries.fetch_add(1, Ordering::Relaxed);
        self.0.recording_query_results.fetch_add(
            u64::try_from(results).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }
    pub fn query_failed(&self) {
        self.0
            .recording_query_failures
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn range_rejected(&self) {
        self.0
            .recording_ranges_rejected
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn playback_started(&self) {
        self.0.playback_active.fetch_add(1, Ordering::Relaxed);
        self.0.playback_started.fetch_add(1, Ordering::Relaxed);
    }
    pub fn playback_completed(&self) {
        self.0.playback_completed.fetch_add(1, Ordering::Relaxed);
    }
    pub fn playback_cancelled(&self) {
        self.0.playback_cancelled.fetch_add(1, Ordering::Relaxed);
    }
    pub fn playback_failed(&self) {
        self.0.playback_failures.fetch_add(1, Ordering::Relaxed);
    }
    pub fn playback_closed(&self) {
        self.0.playback_active.fetch_sub(1, Ordering::Relaxed);
    }
    pub fn retention(&self, pass: crate::retention_owner::RetentionPass) {
        self.0.retention_passes.fetch_add(1, Ordering::Relaxed);
        self.0
            .retention_candidates
            .fetch_add(pass.candidates, Ordering::Relaxed);
        self.0
            .retention_deleted
            .fetch_add(pass.deleted, Ordering::Relaxed);
        self.0
            .retention_delete_failures
            .fetch_add(pass.failures, Ordering::Relaxed);
        self.0
            .retention_bytes_reclaimed
            .fetch_add(pass.bytes_reclaimed, Ordering::Relaxed);
        self.0
            .storage_total_bytes
            .store(pass.total_bytes, Ordering::Relaxed);
        self.0
            .storage_free_bytes
            .store(pass.free_bytes, Ordering::Relaxed);
        self.0
            .storage_pressure
            .store(pressure_value(pass.pressure), Ordering::Relaxed);
    }
    pub fn retention_failed(&self) {
        self.0.retention_failures.fetch_add(1, Ordering::Relaxed);
    }
    #[must_use]
    pub fn snapshot(&self) -> Phase4Snapshot {
        macro_rules! load {
            ($field:ident) => {
                self.0.$field.load(Ordering::Relaxed)
            };
        }
        Phase4Snapshot {
            reconciliation_active: load!(reconciliation_active),
            reconciliation_passes: load!(reconciliation_passes),
            reconciliation_failures: load!(reconciliation_failures),
            reconciliation_examined: load!(reconciliation_examined),
            reconciliation_imported: load!(reconciliation_imported),
            reconciliation_incomplete: load!(reconciliation_incomplete),
            reconciliation_inconsistent: load!(reconciliation_inconsistent),
            reconciliation_skipped_indexed: load!(reconciliation_skipped_indexed),
            recording_queries: load!(recording_queries),
            recording_query_failures: load!(recording_query_failures),
            recording_query_results: load!(recording_query_results),
            recording_ranges_rejected: load!(recording_ranges_rejected),
            playback_active: load!(playback_active),
            playback_started: load!(playback_started),
            playback_completed: load!(playback_completed),
            playback_cancelled: load!(playback_cancelled),
            playback_failures: load!(playback_failures),
            playback_buffer_bytes: 64 * 1024,
            retention_passes: load!(retention_passes),
            retention_failures: load!(retention_failures),
            retention_candidates: load!(retention_candidates),
            retention_deleted: load!(retention_deleted),
            retention_delete_failures: load!(retention_delete_failures),
            retention_bytes_reclaimed: load!(retention_bytes_reclaimed),
            storage_total_bytes: load!(storage_total_bytes),
            storage_free_bytes: load!(storage_free_bytes),
            storage_pressure: pressure_from_value(load!(storage_pressure)),
        }
    }
}

const fn pressure_value(value: StoragePressure) -> u64 {
    match value {
        StoragePressure::Normal => 0,
        StoragePressure::Warning => 1,
        StoragePressure::Critical => 2,
        StoragePressure::Emergency => 3,
    }
}
const fn pressure_from_value(value: u64) -> StoragePressure {
    match value {
        1 => StoragePressure::Warning,
        2 => StoragePressure::Critical,
        3 => StoragePressure::Emergency,
        _ => StoragePressure::Normal,
    }
}
