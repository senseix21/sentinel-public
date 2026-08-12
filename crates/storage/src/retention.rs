//! Validated retention, pressure, and safe unlink contracts.

use std::{
    error::Error,
    fmt,
    num::{NonZeroU64, NonZeroUsize},
    time::Duration,
};

use crate::{filesystem::RecordingLayout, metadata::RecordingLocator};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub enum StoragePressure {
    #[default]
    Normal,
    Warning,
    Critical,
    Emergency,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageWatermarks {
    warning_threshold: u8,
    critical_threshold: u8,
    emergency_threshold: u8,
    hysteresis: u8,
}

impl StorageWatermarks {
    /// Validates strictly increasing percentages below 100 and nonzero
    /// hysteresis smaller than the first transition gap.
    ///
    /// # Errors
    /// Returns [`RetentionConfigError::InvalidWatermarks`] for unsafe ordering.
    pub fn new(
        warning: u8,
        critical: u8,
        emergency: u8,
        hysteresis: u8,
    ) -> Result<Self, RetentionConfigError> {
        if warning == 0
            || warning >= critical
            || critical >= emergency
            || emergency >= 100
            || hysteresis == 0
            || hysteresis >= critical - warning
            || hysteresis >= emergency - critical
        {
            return Err(RetentionConfigError::InvalidWatermarks);
        }
        Ok(Self {
            warning_threshold: warning,
            critical_threshold: critical,
            emergency_threshold: emergency,
            hysteresis,
        })
    }

    #[must_use]
    pub fn classify(self, previous: StoragePressure, used_percent: u8) -> StoragePressure {
        let raw = if used_percent >= self.emergency_threshold {
            StoragePressure::Emergency
        } else if used_percent >= self.critical_threshold {
            StoragePressure::Critical
        } else if used_percent >= self.warning_threshold {
            StoragePressure::Warning
        } else {
            StoragePressure::Normal
        };
        if raw >= previous {
            return raw;
        }
        let release = match previous {
            StoragePressure::Emergency => self.emergency_threshold.saturating_sub(self.hysteresis),
            StoragePressure::Critical => self.critical_threshold.saturating_sub(self.hysteresis),
            StoragePressure::Warning => self.warning_threshold.saturating_sub(self.hysteresis),
            StoragePressure::Normal => 0,
        };
        if used_percent >= release {
            previous
        } else {
            raw
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionPolicy {
    retention: Duration,
    protected_recent: Duration,
    maximum_files_per_pass: NonZeroUsize,
    maximum_bytes_per_pass: NonZeroU64,
    watermarks: StorageWatermarks,
}

impl RetentionPolicy {
    /// Validates age/protection and per-pass work bounds.
    ///
    /// # Errors
    /// Returns [`RetentionConfigError::InvalidDurations`] for an unsafe policy.
    pub fn new(
        retention: Duration,
        protected_recent: Duration,
        maximum_files_per_pass: NonZeroUsize,
        maximum_bytes_per_pass: NonZeroU64,
        watermarks: StorageWatermarks,
    ) -> Result<Self, RetentionConfigError> {
        if retention.is_zero() || protected_recent.is_zero() || protected_recent >= retention {
            return Err(RetentionConfigError::InvalidDurations);
        }
        Ok(Self {
            retention,
            protected_recent,
            maximum_files_per_pass,
            maximum_bytes_per_pass,
            watermarks,
        })
    }

    #[must_use]
    pub const fn retention(self) -> Duration {
        self.retention
    }
    #[must_use]
    pub const fn protected_recent(self) -> Duration {
        self.protected_recent
    }
    #[must_use]
    pub const fn maximum_files(self) -> usize {
        self.maximum_files_per_pass.get()
    }
    #[must_use]
    pub const fn maximum_bytes(self) -> u64 {
        self.maximum_bytes_per_pass.get()
    }
    #[must_use]
    pub const fn watermarks(self) -> StorageWatermarks {
        self.watermarks
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetentionConfigError {
    InvalidDurations,
    InvalidWatermarks,
}

impl fmt::Display for RetentionConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid retention configuration: {self:?}")
    }
}
impl Error for RetentionConfigError {}

#[derive(Clone, Debug)]
pub struct SafeRecordingDelete {
    layout: RecordingLayout,
}

impl SafeRecordingDelete {
    #[must_use]
    pub const fn new(layout: RecordingLayout) -> Self {
        Self { layout }
    }

    /// Removes one claimed regular file beneath the canonical configured root
    /// and syncs its parent. A missing file is an idempotent success.
    ///
    /// # Errors
    /// Returns [`SafeDeleteError`] for path escape, symlinks, non-files, or I/O.
    pub async fn unlink(
        &self,
        locator: &RecordingLocator,
    ) -> Result<DeleteOutcome, SafeDeleteError> {
        let path = self.layout.root().join(locator.as_path());
        if !path.starts_with(self.layout.root()) {
            return Err(SafeDeleteError::OutsideRoot);
        }
        let metadata = match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DeleteOutcome::AlreadyMissing)
            }
            Err(error) => return Err(SafeDeleteError::Io(error)),
        };
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(SafeDeleteError::NotRegularFile);
        }
        let canonical_root = tokio::fs::canonicalize(self.layout.root())
            .await
            .map_err(SafeDeleteError::Io)?;
        let canonical_path = tokio::fs::canonicalize(&path)
            .await
            .map_err(SafeDeleteError::Io)?;
        if !canonical_path.starts_with(&canonical_root) {
            return Err(SafeDeleteError::OutsideRoot);
        }
        tokio::fs::remove_file(&path)
            .await
            .map_err(SafeDeleteError::Io)?;
        let parent = path
            .parent()
            .ok_or(SafeDeleteError::OutsideRoot)?
            .to_path_buf();
        tokio::task::spawn_blocking(move || std::fs::File::open(parent)?.sync_all())
            .await
            .map_err(SafeDeleteError::Join)?
            .map_err(SafeDeleteError::Io)?;
        Ok(DeleteOutcome::Deleted)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteOutcome {
    Deleted,
    AlreadyMissing,
}

#[derive(Debug)]
pub enum SafeDeleteError {
    OutsideRoot,
    NotRegularFile,
    Io(std::io::Error),
    Join(tokio::task::JoinError),
}

impl fmt::Display for SafeDeleteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("safe recording deletion failed")
    }
}
impl Error for SafeDeleteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Join(error) => Some(error),
            Self::OutsideRoot | Self::NotRegularFile => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn pressure_hysteresis_prevents_threshold_flapping() {
        let marks =
            StorageWatermarks::new(80, 90, 97, 2).unwrap_or_else(|error| panic!("valid: {error}"));
        assert_eq!(
            marks.classify(StoragePressure::Normal, 91),
            StoragePressure::Critical
        );
        assert_eq!(
            marks.classify(StoragePressure::Critical, 89),
            StoragePressure::Critical
        );
        assert_eq!(
            marks.classify(StoragePressure::Critical, 87),
            StoragePressure::Warning
        );
    }

    #[tokio::test]
    async fn safe_delete_rejects_traversal_before_io() {
        let root = std::env::temp_dir().join(format!("retention-{}", std::process::id()));
        let layout = RecordingLayout::new(root).unwrap_or_else(|error| panic!("root: {error}"));
        assert!(RecordingLocator::new(PathBuf::from("../outside.mp4")).is_err());
        let delete = SafeRecordingDelete::new(layout);
        let locator = RecordingLocator::new(PathBuf::from("missing.mp4"))
            .unwrap_or_else(|error| panic!("locator: {error}"));
        assert_eq!(
            delete
                .unlink(&locator)
                .await
                .unwrap_or_else(|error| panic!("missing is idempotent: {error}")),
            DeleteOutcome::AlreadyMissing
        );
    }
}
