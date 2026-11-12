//! Bounded database-aware filesystem reconciliation owner.

use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use crate::phase4_metrics::Phase4Metrics;
use surveillance_database::PostgresRecordingRepository;
use surveillance_storage::{
    filesystem::RecordingLayout,
    metadata::{
        CompletedRecording, MetadataProvenance, NonPlayableRecording, RecordingContainer,
        RecordingEvidenceLocator, RecordingFailureCategory, RecordingLocator, RecordingStatus,
        VideoCodec,
    },
    reconciliation::{
        discover_reconciliation_candidates, ReconciliationCandidate, ReconciliationIdentity,
        ReconciliationLimit,
    },
    segment_store::SegmentFileStore,
};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReconciliationPass {
    pub examined: usize,
    pub imported: u64,
    pub incomplete: u64,
    pub inconsistent: u64,
    pub skipped_indexed: u64,
    pub skipped_active: u64,
    pub malformed: u64,
}

pub struct ReconciliationOwner {
    repository: PostgresRecordingRepository,
    layout: RecordingLayout,
    store: SegmentFileStore,
    scan_limit: ReconciliationLimit,
    database_page: usize,
    active_partial_grace: Duration,
    interval: Duration,
    metrics: Phase4Metrics,
    cancellation: Option<watch::Receiver<bool>>,
}

impl ReconciliationOwner {
    #[must_use]
    pub fn new(
        repository: PostgresRecordingRepository,
        layout: RecordingLayout,
        store: SegmentFileStore,
        scan_limit: ReconciliationLimit,
        database_page: usize,
        active_partial_grace: Duration,
        interval: Duration,
    ) -> Self {
        Self {
            repository,
            layout,
            store,
            scan_limit,
            database_page,
            active_partial_grace,
            interval,
            metrics: Phase4Metrics::default(),
            cancellation: None,
        }
    }

    #[must_use]
    pub fn with_metrics(mut self, metrics: Phase4Metrics) -> Self {
        self.metrics = metrics;
        self
    }

    #[must_use]
    pub fn with_cancellation(mut self, cancellation: watch::Receiver<bool>) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        loop {
            self.metrics.reconciliation_started();
            match self.run_pass().await {
                Ok(report) => {
                    self.metrics.reconciliation(
                        report.examined,
                        report.imported,
                        report.incomplete,
                        report.inconsistent,
                        report.skipped_indexed,
                    );
                    tracing::info!(?report, "bounded recording reconciliation pass completed");
                }
                Err(category) => {
                    self.metrics.reconciliation_failed();
                    tracing::warn!(category, "recording reconciliation pass degraded");
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

    /// Runs one bounded idempotent comparison pass.
    ///
    /// # Errors
    /// Returns a fixed credential-free category for the failed boundary.
    #[allow(
        clippy::too_many_lines,
        reason = "the linear pass keeps filesystem/database repair ordering auditable"
    )]
    pub async fn run_pass(&mut self) -> Result<ReconciliationPass, &'static str> {
        self.repository
            .migrate()
            .await
            .map_err(|_| "database_migration")?;
        let discovery = discover_reconciliation_candidates(&self.layout, self.scan_limit)
            .await
            .map_err(|_| "filesystem_scan")?;
        let mut pass = ReconciliationPass {
            examined: discovery.examined,
            ..ReconciliationPass::default()
        };
        for candidate in discovery.candidates {
            if self
                .cancellation
                .as_ref()
                .is_some_and(|signal| *signal.borrow())
            {
                return Err("cancelled");
            }
            let path = candidate_path(&candidate);
            let Ok(identity) = ReconciliationIdentity::parse(&self.layout, path) else {
                pass.malformed = pass.malformed.saturating_add(1);
                continue;
            };
            match candidate {
                ReconciliationCandidate::IncompletePartial(path) => {
                    if partial_is_active(&path, self.active_partial_grace).await {
                        pass.skipped_active = pass.skipped_active.saturating_add(1);
                        continue;
                    }
                    if self
                        .repository
                        .recording_status(identity.recording_id())
                        .await
                        .map_err(|_| "database_status")?
                        .is_none()
                    {
                        let relative = path
                            .strip_prefix(self.layout.root())
                            .map_err(|_| "partial_path")?;
                        let evidence = NonPlayableRecording::new(
                            identity.recording_id().clone(),
                            identity.camera_id().clone(),
                            identity.started_at(),
                            None,
                            RecordingEvidenceLocator::new(relative.to_path_buf())
                                .map_err(|_| "partial_locator")?,
                            RecordingStatus::Incomplete,
                            RecordingFailureCategory::Reconciliation,
                            MetadataProvenance::Reconciliation,
                        )
                        .map_err(|_| "partial_evidence")?;
                        self.repository
                            .insert_or_confirm_non_playable(&evidence)
                            .await
                            .map_err(|_| "database_incomplete")?;
                        pass.incomplete = pass.incomplete.saturating_add(1);
                    }
                }
                ReconciliationCandidate::DurableUnindexed(path) => {
                    let existing = self
                        .repository
                        .recording_status(identity.recording_id())
                        .await
                        .map_err(|_| "database_status")?;
                    if existing == Some(RecordingStatus::Completed) {
                        let indexed = self
                            .repository
                            .find_completed(identity.camera_id(), identity.recording_id())
                            .await
                            .map_err(|_| "database_completed")?
                            .ok_or("database_completed_invariant")?;
                        let relative = path
                            .strip_prefix(self.layout.root())
                            .map_err(|_| "durable_path")?;
                        let observed_size = tokio::fs::metadata(&path)
                            .await
                            .map_err(|_| "durable_metadata")?
                            .len();
                        if indexed.locator().as_path() != relative
                            || indexed.size_bytes() != observed_size
                        {
                            self.repository
                                .mark_inconsistent(identity.recording_id())
                                .await
                                .map_err(|_| "database_inconsistent")?;
                            pass.inconsistent = pass.inconsistent.saturating_add(1);
                        } else {
                            // Recorder completion already probed this immutable
                            // file. Re-running ffprobe for every indexed segment
                            // on every reconciliation pass turned repair into an
                            // archive scrub and made startup scale with history.
                            // Path and size still catch replacement/truncation;
                            // content scrubbing is a separate bounded concern.
                            pass.skipped_indexed = pass.skipped_indexed.saturating_add(1);
                        }
                        continue;
                    }
                    if existing.is_some() {
                        // The old path probed these rows and discarded either
                        // result. Preserve the lifecycle state without paying
                        // for an observation no transition consumes.
                        pass.skipped_indexed = pass.skipped_indexed.saturating_add(1);
                        continue;
                    }
                    if let Ok(stored) = self.store.probe_durable(path.clone()).await {
                        let ended_at = identity
                            .started_at()
                            .checked_add(stored.duration())
                            .ok_or("duration_overflow")?;
                        let relative = path
                            .strip_prefix(self.layout.root())
                            .map_err(|_| "durable_path")?;
                        let completed = CompletedRecording::new(
                            identity.recording_id().clone(),
                            identity.camera_id().clone(),
                            identity.started_at(),
                            ended_at,
                            RecordingLocator::new(relative.to_path_buf())
                                .map_err(|_| "durable_locator")?,
                            RecordingContainer::FragmentedMp4,
                            VideoCodec::H264,
                            stored.width(),
                            stored.height(),
                            stored.size_bytes(),
                            MetadataProvenance::Reconciliation,
                        )
                        .map_err(|_| "durable_evidence")?;
                        self.repository
                            .insert_or_confirm_completed(&completed)
                            .await
                            .map_err(|_| "database_import")?;
                        pass.imported = pass.imported.saturating_add(1);
                    } else {
                        let relative = path
                            .strip_prefix(self.layout.root())
                            .map_err(|_| "durable_path")?;
                        let evidence = NonPlayableRecording::new(
                            identity.recording_id().clone(),
                            identity.camera_id().clone(),
                            identity.started_at(),
                            None,
                            RecordingEvidenceLocator::new(relative.to_path_buf())
                                .map_err(|_| "durable_locator")?,
                            RecordingStatus::Inconsistent,
                            RecordingFailureCategory::Reconciliation,
                            MetadataProvenance::Reconciliation,
                        )
                        .map_err(|_| "durable_evidence")?;
                        self.repository
                            .insert_or_confirm_non_playable(&evidence)
                            .await
                            .map_err(|_| "database_inconsistent")?;
                        pass.inconsistent = pass.inconsistent.saturating_add(1);
                    }
                }
            }
        }

        let mut cursor = None;
        loop {
            if self
                .cancellation
                .as_ref()
                .is_some_and(|signal| *signal.borrow())
            {
                return Err("cancelled");
            }
            let page = self
                .repository
                .completed_evidence_page(cursor.as_ref(), self.database_page)
                .await
                .map_err(|_| "database_page")?;
            if page.is_empty() {
                break;
            }
            for evidence in &page {
                let path = self.layout.root().join(evidence.locator.as_path());
                let missing = !tokio::fs::try_exists(&path)
                    .await
                    .map_err(|_| "file_exists")?;
                if missing {
                    self.repository
                        .mark_inconsistent(&evidence.recording_id)
                        .await
                        .map_err(|_| "database_inconsistent")?;
                    pass.inconsistent = pass.inconsistent.saturating_add(1);
                }
            }
            cursor = page.last().map(|item| item.recording_id.clone());
            if page.len() < self.database_page {
                break;
            }
        }
        Ok(pass)
    }
}

fn candidate_path(candidate: &ReconciliationCandidate) -> &PathBuf {
    match candidate {
        ReconciliationCandidate::IncompletePartial(path)
        | ReconciliationCandidate::DurableUnindexed(path) => path,
    }
}

async fn partial_is_active(path: &std::path::Path, grace: Duration) -> bool {
    let Ok(metadata) = tokio::fs::metadata(path).await else {
        return true;
    };
    let Ok(modified) = metadata.modified() else {
        return true;
    };
    SystemTime::now()
        .duration_since(modified)
        .map_or(true, |age| age < grace)
}
