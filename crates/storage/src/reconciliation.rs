//! Bounded startup discovery of recording files requiring reconciliation.

use crate::filesystem::RecordingLayout;
use std::{error::Error, fmt, path::PathBuf};
use tokio::fs;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconciliationLimit(usize);

impl ReconciliationLimit {
    /// Creates a nonzero maximum number of examined filesystem entries.
    ///
    /// # Errors
    ///
    /// Returns [`ReconciliationConfigError`] when `value` is zero.
    pub fn new(value: usize) -> Result<Self, ReconciliationConfigError> {
        (value != 0)
            .then_some(Self(value))
            .ok_or(ReconciliationConfigError)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconciliationConfigError;

impl fmt::Display for ReconciliationConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("reconciliation scan limit must be greater than zero")
    }
}
impl Error for ReconciliationConfigError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconciliationCandidate {
    IncompletePartial(PathBuf),
    DurableUnindexed(PathBuf),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReconciliationReport {
    pub examined: usize,
    pub candidates: Vec<ReconciliationCandidate>,
    pub skipped_symlinks: usize,
}

/// Walks only beneath the configured root, never follows symlinks, and fails
/// closed instead of exceeding its explicit work bound.
///
/// # Errors
///
/// Returns an I/O error or [`ReconciliationError::LimitExceeded`].
pub async fn discover_reconciliation_candidates(
    layout: &RecordingLayout,
    limit: ReconciliationLimit,
) -> Result<ReconciliationReport, ReconciliationError> {
    let mut report = ReconciliationReport::default();
    let mut directories = vec![layout.root().to_path_buf()];
    while let Some(directory) = directories.pop() {
        let mut entries = match fs::read_dir(directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(ReconciliationError::Io(error)),
        };
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(ReconciliationError::Io)?
        {
            report.examined = report.examined.saturating_add(1);
            if report.examined > limit.0 {
                return Err(ReconciliationError::LimitExceeded { limit: limit.0 });
            }
            let kind = entry.file_type().await.map_err(ReconciliationError::Io)?;
            if kind.is_symlink() {
                report.skipped_symlinks = report.skipped_symlinks.saturating_add(1);
            } else if kind.is_dir() {
                directories.push(entry.path());
            } else if kind.is_file() {
                let path = entry.path();
                let name = path.file_name().and_then(|name| name.to_str());
                #[allow(
                    clippy::case_sensitive_file_extension_comparisons,
                    reason = "the lowercase suffix is the exact lifecycle contract, not a generic extension"
                )]
                if name.is_some_and(|name| name.ends_with(".partial.mp4")) {
                    report
                        .candidates
                        .push(ReconciliationCandidate::IncompletePartial(path));
                } else if name.is_some_and(|name| name.ends_with(".mp4")) {
                    report
                        .candidates
                        .push(ReconciliationCandidate::DurableUnindexed(path));
                }
            }
        }
    }
    report
        .candidates
        .sort_by(|a, b| candidate_path(a).cmp(candidate_path(b)));
    tracing::info!(
        root = %layout.root().display(),
        examined = report.examined,
        candidates = report.candidates.len(),
        skipped_symlinks = report.skipped_symlinks,
        "recording reconciliation discovery completed"
    );
    Ok(report)
}

fn candidate_path(candidate: &ReconciliationCandidate) -> &PathBuf {
    match candidate {
        ReconciliationCandidate::IncompletePartial(path)
        | ReconciliationCandidate::DurableUnindexed(path) => path,
    }
}

#[derive(Debug)]
pub enum ReconciliationError {
    Io(std::io::Error),
    LimitExceeded { limit: usize },
}

impl fmt::Display for ReconciliationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => f.write_str("recording reconciliation filesystem error"),
            Self::LimitExceeded { limit } => {
                write!(f, "recording reconciliation exceeded limit {limit}")
            }
        }
    }
}
impl Error for ReconciliationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::LimitExceeded { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "surveillance-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[tokio::test]
    async fn classifies_partial_and_durable_without_promoting_partial() {
        let root = root("reconcile");
        let directory = root.join("camera-01/2026/08/11");
        fs::create_dir_all(&directory)
            .await
            .unwrap_or_else(|e| panic!("fixture: {e}"));
        fs::write(directory.join("a.partial.mp4"), b"partial")
            .await
            .unwrap_or_else(|e| panic!("partial: {e}"));
        fs::write(directory.join("b.mp4"), b"durable")
            .await
            .unwrap_or_else(|e| panic!("durable: {e}"));
        fs::write(directory.join("ignored.txt"), b"other")
            .await
            .unwrap_or_else(|e| panic!("other: {e}"));
        let layout = RecordingLayout::new(root.clone()).unwrap_or_else(|e| panic!("root: {e}"));
        let report = discover_reconciliation_candidates(
            &layout,
            ReconciliationLimit::new(20).unwrap_or_else(|e| panic!("limit: {e}")),
        )
        .await
        .unwrap_or_else(|e| panic!("scan: {e}"));
        assert_eq!(report.candidates.len(), 2);
        assert!(
            matches!(&report.candidates[0], ReconciliationCandidate::IncompletePartial(path) if path.ends_with("a.partial.mp4"))
        );
        assert!(
            matches!(&report.candidates[1], ReconciliationCandidate::DurableUnindexed(path) if path.ends_with("b.mp4"))
        );
        fs::remove_dir_all(root)
            .await
            .unwrap_or_else(|e| panic!("cleanup: {e}"));
    }

    #[tokio::test]
    async fn fails_closed_at_work_bound() {
        let root = root("limit");
        fs::create_dir_all(&root)
            .await
            .unwrap_or_else(|e| panic!("fixture: {e}"));
        fs::write(root.join("one.mp4"), b"1")
            .await
            .unwrap_or_else(|e| panic!("one: {e}"));
        fs::write(root.join("two.mp4"), b"2")
            .await
            .unwrap_or_else(|e| panic!("two: {e}"));
        let layout = RecordingLayout::new(root.clone()).unwrap_or_else(|e| panic!("root: {e}"));
        let result = discover_reconciliation_candidates(
            &layout,
            ReconciliationLimit::new(1).unwrap_or_else(|e| panic!("limit: {e}")),
        )
        .await;
        assert!(matches!(
            result,
            Err(ReconciliationError::LimitExceeded { .. })
        ));
        fs::remove_dir_all(root)
            .await
            .unwrap_or_else(|e| panic!("cleanup: {e}"));
    }
}
