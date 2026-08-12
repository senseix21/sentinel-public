//! Owned indexed retention and storage-pressure loop.

use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use crate::phase4_metrics::Phase4Metrics;
use surveillance_database::{PostgresRecordingRepository, RetentionCandidate};
use surveillance_storage::retention::{RetentionPolicy, SafeRecordingDelete, StoragePressure};
use tokio::{process::Command, sync::watch};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetentionPass {
    pub candidates: u64,
    pub deleted: u64,
    pub already_missing: u64,
    pub failures: u64,
    pub bytes_reclaimed: u64,
    pub pressure: StoragePressure,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

pub struct RetentionOwner {
    repository: PostgresRecordingRepository,
    delete: SafeRecordingDelete,
    root: PathBuf,
    policy: RetentionPolicy,
    pressure: StoragePressure,
    interval: Duration,
    metrics: Phase4Metrics,
}

impl RetentionOwner {
    #[must_use]
    pub fn new(
        repository: PostgresRecordingRepository,
        delete: SafeRecordingDelete,
        root: PathBuf,
        policy: RetentionPolicy,
        interval: Duration,
    ) -> Self {
        Self {
            repository,
            delete,
            root,
            policy,
            pressure: StoragePressure::Normal,
            interval,
            metrics: Phase4Metrics::default(),
        }
    }

    #[must_use]
    pub fn with_metrics(mut self, metrics: Phase4Metrics) -> Self {
        self.metrics = metrics;
        self
    }

    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        loop {
            match self.run_pass(SystemTime::now()).await {
                Ok(pass) => {
                    self.metrics.retention(pass);
                    tracing::info!(?pass, "bounded indexed retention pass completed");
                }
                Err(category) => {
                    self.metrics.retention_failed();
                    tracing::warn!(category, "indexed retention pass degraded");
                }
            }
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow_and_update() { return; }
                }
                () = tokio::time::sleep(self.interval) => {}
            }
        }
    }

    /// Runs one recoverable bounded deletion pass.
    ///
    /// # Errors
    /// Returns a fixed credential/path-free boundary category.
    pub async fn run_pass(&mut self, now: SystemTime) -> Result<RetentionPass, &'static str> {
        let capacity = observe_capacity(&self.root).await?;
        let used_percent = capacity.used_percent();
        self.pressure = self
            .policy
            .watermarks()
            .classify(self.pressure, used_percent);
        let mut pass = RetentionPass {
            pressure: self.pressure,
            total_bytes: capacity.total_bytes,
            free_bytes: capacity.free_bytes,
            ..RetentionPass::default()
        };
        let mut candidates = self
            .repository
            .deletion_pending_page(self.policy.maximum_files())
            .await
            .map_err(|_| "database_pending")?;
        if candidates.len() < self.policy.maximum_files() {
            let protected_cutoff = now
                .checked_sub(self.policy.protected_recent())
                .ok_or("clock")?;
            let age_cutoff = if self.pressure >= StoragePressure::Critical {
                protected_cutoff
            } else {
                now.checked_sub(self.policy.retention()).ok_or("clock")?
            };
            let remaining_files = self.policy.maximum_files() - candidates.len();
            let claimed = self
                .repository
                .claim_retention_candidates(
                    age_cutoff,
                    protected_cutoff,
                    remaining_files,
                    self.policy.maximum_bytes(),
                )
                .await
                .map_err(|_| "database_claim")?;
            candidates.extend(claimed);
        }
        pass.candidates = u64::try_from(candidates.len()).unwrap_or(u64::MAX);
        for candidate in candidates {
            self.delete_candidate(candidate, &mut pass).await;
        }
        Ok(pass)
    }

    async fn delete_candidate(&self, candidate: RetentionCandidate, pass: &mut RetentionPass) {
        match self.delete.unlink(&candidate.locator).await {
            Ok(outcome) => match self
                .repository
                .finalize_deletion(&candidate.recording_id)
                .await
            {
                Ok(true) => {
                    pass.deleted = pass.deleted.saturating_add(1);
                    pass.bytes_reclaimed =
                        pass.bytes_reclaimed.saturating_add(candidate.size_bytes);
                    if matches!(
                        outcome,
                        surveillance_storage::retention::DeleteOutcome::AlreadyMissing
                    ) {
                        pass.already_missing = pass.already_missing.saturating_add(1);
                    }
                }
                Ok(false) | Err(_) => pass.failures = pass.failures.saturating_add(1),
            },
            Err(_) => pass.failures = pass.failures.saturating_add(1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileSystemCapacity {
    total_bytes: u64,
    free_bytes: u64,
}

impl FileSystemCapacity {
    fn used_percent(self) -> u8 {
        if self.total_bytes == 0 {
            return 100;
        }
        let used = self.total_bytes.saturating_sub(self.free_bytes);
        u8::try_from(used.saturating_mul(100).saturating_div(self.total_bytes)).unwrap_or(100)
    }
}

async fn observe_capacity(root: &std::path::Path) -> Result<FileSystemCapacity, &'static str> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new("df")
            .arg("-Pk")
            .arg(root)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "capacity_timeout")?
    .map_err(|_| "capacity_process")?;
    if !output.status.success() {
        return Err("capacity_status");
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| "capacity_output")?;
    let line = text.lines().last().ok_or("capacity_output")?;
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.len() < 6 {
        return Err("capacity_output");
    }
    let total_kib = fields[1].parse::<u64>().map_err(|_| "capacity_output")?;
    let free_kib = fields[3].parse::<u64>().map_err(|_| "capacity_output")?;
    Ok(FileSystemCapacity {
        total_bytes: total_kib.saturating_mul(1_024),
        free_bytes: free_kib.saturating_mul(1_024),
    })
}

#[cfg(test)]
mod tests {
    use super::FileSystemCapacity;

    #[test]
    fn capacity_percentage_is_bounded() {
        assert_eq!(
            FileSystemCapacity {
                total_bytes: 100,
                free_bytes: 9
            }
            .used_percent(),
            91
        );
        assert_eq!(
            FileSystemCapacity {
                total_bytes: 0,
                free_bytes: 0
            }
            .used_percent(),
            100
        );
    }
}
