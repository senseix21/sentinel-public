//! The last-known-good cache, and the freshness state it drives.
//!
//! ADR 008 Decision 2: when the settings store is unavailable at startup the
//! server loads this cache and enters [`Freshness::Stale`]. It never falls back
//! to defaults.
//!
//! That is the load-bearing part. Several operational settings are destructive
//! when they revert: retention age and storage watermarks delete recordings. A
//! unit that silently reverted a deliberately-raised retention window because
//! `PostgreSQL` was slow to start would delete footage that policy says to keep,
//! quietly, at startup, when nobody is looking. Starting with stale-but-real
//! configuration is strictly safer than starting with fresh-but-wrong
//! configuration.
//!
//! `Stale` is transient, not terminal: [`StaleRetry`] retries on a bounded
//! interval and the state clears on the first successful load, without a
//! restart.

use std::{
    error::Error,
    fmt::{self, Write as _},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use surveillance_common::types::CameraId;

use crate::{
    classification::{Origin, Scope, SettingId},
    resolution::{Layer, SettingKey},
    secret::{Fingerprint, Secret},
    value::SettingValue,
};

/// What the cache file was found to be.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheState {
    /// Read, checksum verified.
    Valid,
    /// Present but unreadable or checksum-mismatched. Preserved, not
    /// overwritten: it is evidence about how the unit reached this state.
    Corrupt,
    /// No cache file exists yet.
    Absent,
    /// A successful load could not be written back. Degrades the *next*
    /// startup, not this one.
    Unwritable,
}

impl CacheState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Corrupt => "corrupt",
            Self::Absent => "absent",
            Self::Unwritable => "unwritable",
        }
    }

    pub const ALL: &'static [Self] = &[Self::Valid, Self::Corrupt, Self::Absent, Self::Unwritable];
}

impl fmt::Display for CacheState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Whether the effective configuration came from the store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Freshness {
    /// Loaded from the settings store.
    Fresh { loaded_at: SystemTime },
    /// The store was unavailable. Degrades readiness; never stops recording.
    Stale {
        since: SystemTime,
        cache: CacheState,
        last_success: Option<SystemTime>,
    },
}

impl Freshness {
    /// Metric-friendly value: 0 fresh, 1 stale.
    #[must_use]
    pub const fn gauge(self) -> u8 {
        match self {
            Self::Fresh { .. } => 0,
            Self::Stale { .. } => 1,
        }
    }

    #[must_use]
    pub const fn is_stale(self) -> bool {
        matches!(self, Self::Stale { .. })
    }

    /// Whether readiness should report degradation.
    ///
    /// Liveness is unaffected: a stale unit is running and recording.
    #[must_use]
    pub const fn degrades_readiness(self) -> bool {
        self.is_stale()
    }
}

/// The bounded retry that makes `Stale` transient.
///
/// The budget deliberately outlasts typical service start ordering, because the
/// dominant cause of `Stale` is a local `PostgreSQL` that has not finished
/// starting. It never blocks the server from coming up.
#[derive(Clone, Copy, Debug)]
pub struct StaleRetry {
    interval: Duration,
    budget: Duration,
    spent: Duration,
}

impl StaleRetry {
    #[must_use]
    pub const fn new(interval: Duration, budget: Duration) -> Self {
        Self {
            interval,
            budget,
            spent: Duration::ZERO,
        }
    }

    /// How long to wait before the next attempt, or `None` when the budget is
    /// spent.
    ///
    /// A zero interval would spin, so it is treated as budget-exhausted rather
    /// than as an invitation to retry without pause.
    #[must_use]
    pub fn next_delay(&self) -> Option<Duration> {
        if self.interval.is_zero() || self.spent >= self.budget {
            return None;
        }
        Some(self.interval.min(self.budget.saturating_sub(self.spent)))
    }

    /// Records that an attempt consumed `delay` of the budget.
    pub fn record_attempt(&mut self, delay: Duration) {
        self.spent = self.spent.saturating_add(delay).min(self.budget);
    }

    /// Clears the budget after a successful load.
    pub fn reset(&mut self) {
        self.spent = Duration::ZERO;
    }

    #[must_use]
    pub const fn exhausted(&self) -> bool {
        self.spent.as_nanos() >= self.budget.as_nanos()
    }
}

/// The cache file's contents.
pub struct CacheContents {
    pub generation: u64,
    pub layer: Layer,
}

impl fmt::Debug for CacheContents {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CacheContents")
            .field("generation", &self.generation)
            .field("entries", &self.layer.len())
            .finish()
    }
}

/// Reads and writes the last-known-good cache.
pub struct Cache {
    path: PathBuf,
}

impl Cache {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the cache.
    ///
    /// # Errors
    /// Returns [`CacheError::Absent`] when no file exists and
    /// [`CacheError::Corrupt`] when the file is unreadable, unparseable, or
    /// fails its checksum. A corrupt file is left in place.
    pub fn load(&self) -> Result<CacheContents, CacheError> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(CacheError::Absent)
            }
            Err(_) => return Err(CacheError::Corrupt),
        };
        parse(&text)
    }

    /// Writes the cache atomically.
    ///
    /// Writes a sibling temporary file and renames it, so a torn write cannot
    /// produce a half-written cache. Rename within a directory is atomic on the
    /// platforms this targets.
    ///
    /// # Errors
    /// Returns [`CacheError::Unwritable`]. The caller must not fail the load
    /// that produced these settings: they are already valid and effective, and
    /// an unwritable cache degrades the next startup, not this one.
    pub fn store(&self, contents: &CacheContents) -> Result<(), CacheError> {
        let rendered = render(contents);
        let temporary = self.path.with_extension("tmp");
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|_| CacheError::Unwritable)?;
            }
        }
        std::fs::write(&temporary, rendered).map_err(|_| CacheError::Unwritable)?;
        std::fs::rename(&temporary, &self.path).map_err(|_| {
            // Leave no debris behind if the rename fails.
            let _ = std::fs::remove_file(&temporary);
            CacheError::Unwritable
        })
    }
}

/// A cache that could not be used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheError {
    Absent,
    Corrupt,
    Unwritable,
}

impl CacheError {
    /// The state this error corresponds to.
    #[must_use]
    pub const fn state(self) -> CacheState {
        match self {
            Self::Absent => CacheState::Absent,
            Self::Corrupt => CacheState::Corrupt,
            Self::Unwritable => CacheState::Unwritable,
        }
    }
}

impl fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => formatter.write_str("no last-known-good cache exists"),
            Self::Corrupt => formatter.write_str("last-known-good cache is unreadable"),
            Self::Unwritable => formatter.write_str("last-known-good cache is not writable"),
        }
    }
}

impl Error for CacheError {}

/// The unsalted digest used for integrity, not secrecy.
///
/// The cache holds no secrets — it is written from the persisted layer, which
/// refuses them — so a deployment salt would add nothing here and would make the
/// cache unreadable after a salt rotation.
fn checksum(canonical: &str) -> String {
    Fingerprint::of(&Secret::new(String::new()), canonical.as_bytes()).to_hex()
}

/// A deterministic rendering of the entries, used as the checksum input.
///
/// Derived from parsed entries rather than from the file bytes, so that
/// comments and formatting are not part of the integrity claim while every
/// stored key, scope, and value is.
fn canonical(entries: &[(String, Option<String>, String)]) -> String {
    let mut lines: Vec<String> = entries
        .iter()
        .map(|(key, camera, value)| {
            format!(
                "{key}\u{1f}{}\u{1f}{value}",
                camera.as_deref().unwrap_or("")
            )
        })
        .collect();
    lines.sort();
    lines.join("\n")
}

fn entries_of(layer: &Layer) -> Vec<(String, Option<String>, String)> {
    layer
        .iter()
        .filter_map(|(key, value)| {
            // Secrets are not cached, for the same reason they are not
            // persisted. The persisted layer refuses them, so this is defence
            // in depth rather than an expected branch.
            value.persisted_text().map(|text| {
                (
                    key.id().key().to_owned(),
                    key.camera().map(|camera| camera.as_str().to_owned()),
                    text,
                )
            })
        })
        .collect()
}

fn render(contents: &CacheContents) -> String {
    let entries = entries_of(&contents.layer);
    let mut rendered = String::new();
    rendered.push_str(
        "# Last-known-good configuration cache. Written after every successful\n\
         # load from the settings store, and read only when that store is\n\
         # unavailable. Do not edit: the checksum will reject it.\n",
    );
    // Writing into a String is infallible; the results are discarded rather
    // than unwrapped so that no formatting path can panic.
    let _ = writeln!(rendered, "generation = {}", contents.generation);
    let _ = writeln!(
        rendered,
        "checksum = \"{}\"",
        checksum(&canonical(&entries))
    );
    for (key, camera, value) in &entries {
        rendered.push_str("\n[[entry]]\n");
        let _ = writeln!(rendered, "key = {}", toml_string(key));
        if let Some(camera) = camera {
            let _ = writeln!(rendered, "camera = {}", toml_string(camera));
        }
        let _ = writeln!(rendered, "value = {}", toml_string(value));
    }
    rendered
}

/// Renders a TOML basic string with the escapes the format requires.
fn toml_string(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len() + 2);
    escaped.push('"');
    for character in raw.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

fn parse(text: &str) -> Result<CacheContents, CacheError> {
    let document: toml::Value = toml::from_str(text).map_err(|_| CacheError::Corrupt)?;
    let table = document.as_table().ok_or(CacheError::Corrupt)?;
    let generation = table
        .get("generation")
        .and_then(toml::Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(CacheError::Corrupt)?;
    let stored_checksum = table
        .get("checksum")
        .and_then(toml::Value::as_str)
        .ok_or(CacheError::Corrupt)?;

    let mut raw = Vec::new();
    if let Some(array) = table.get("entry").and_then(toml::Value::as_array) {
        for element in array {
            let entry = element.as_table().ok_or(CacheError::Corrupt)?;
            let key = entry
                .get("key")
                .and_then(toml::Value::as_str)
                .ok_or(CacheError::Corrupt)?;
            let value = entry
                .get("value")
                .and_then(toml::Value::as_str)
                .ok_or(CacheError::Corrupt)?;
            let camera = entry
                .get("camera")
                .map(|camera| camera.as_str().ok_or(CacheError::Corrupt))
                .transpose()?;
            raw.push((key.to_owned(), camera.map(str::to_owned), value.to_owned()));
        }
    }

    if checksum(&canonical(&raw)) != stored_checksum {
        return Err(CacheError::Corrupt);
    }

    let mut layer = Layer::new(Origin::Persisted);
    for (key, camera, value) in raw {
        let id = SettingId::from_key(&key).ok_or(CacheError::Corrupt)?;
        let setting_key = match camera {
            Some(camera) => {
                if id.scope() == Scope::Global {
                    return Err(CacheError::Corrupt);
                }
                let camera = CameraId::new(camera).map_err(|_| CacheError::Corrupt)?;
                SettingKey::scoped(id, camera).map_err(|_| CacheError::Corrupt)?
            }
            None => SettingKey::unscoped(id),
        };
        let parsed = SettingValue::parse(id, &value).map_err(|_| CacheError::Corrupt)?;
        layer
            .insert(setting_key, parsed)
            .map_err(|_| CacheError::Corrupt)?;
    }

    Ok(CacheContents { generation, layer })
}

#[cfg(test)]
mod tests {
    use super::{Cache, CacheContents, CacheError, CacheState, Freshness, StaleRetry};
    use crate::{
        classification::{Origin, SettingId},
        resolution::{Layer, ResolvedSettings, SettingKey},
        value::SettingValue,
    };
    use std::time::{Duration, SystemTime};
    use surveillance_common::types::CameraId;

    fn temporary(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("surveillance-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn sample_layer() -> Layer {
        let mut layer = Layer::new(Origin::Persisted);
        layer
            .insert(
                SettingKey::unscoped(SettingId::RetentionMaxAge),
                SettingValue::Duration(Duration::from_secs(14 * 24 * 60 * 60)),
            )
            .expect("valid entry");
        layer
            .insert(
                SettingKey::unscoped(SettingId::StorageWatermarkWarning),
                SettingValue::Unsigned(85),
            )
            .expect("valid entry");
        layer
            .insert(
                SettingKey::scoped(
                    SettingId::CameraDetectionSampleFps,
                    CameraId::new("camera-01".to_owned()).expect("valid identifier"),
                )
                .expect("camera scoped"),
                SettingValue::Unsigned(12),
            )
            .expect("valid entry");
        layer
    }

    #[test]
    fn a_written_cache_reads_back_identically() {
        let path = temporary("round-trip");
        let cache = Cache::new(path.clone());
        cache
            .store(&CacheContents {
                generation: 42,
                layer: sample_layer(),
            })
            .expect("cache is writable");

        let loaded = cache.load().expect("cache reads back");
        assert_eq!(loaded.generation, 42);
        let resolved = ResolvedSettings::resolve(vec![loaded.layer]);
        assert_eq!(
            resolved
                .effective(SettingId::RetentionMaxAge, None)
                .map(|value| value.value().render()),
            Some("14d".to_owned())
        );
        assert_eq!(
            resolved
                .effective(
                    SettingId::CameraDetectionSampleFps,
                    Some(&CameraId::new("camera-01".to_owned()).expect("valid identifier"))
                )
                .map(|value| value.value().render()),
            Some("12".to_owned())
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_absent_cache_is_distinguished_from_a_corrupt_one() {
        let path = temporary("absent");
        let cache = Cache::new(path);
        assert_eq!(cache.load().err(), Some(CacheError::Absent));
    }

    #[test]
    fn a_tampered_value_fails_the_checksum() {
        let path = temporary("tampered");
        let cache = Cache::new(path.clone());
        cache
            .store(&CacheContents {
                generation: 1,
                layer: sample_layer(),
            })
            .expect("cache is writable");

        let text = std::fs::read_to_string(&path).expect("readable");
        let tampered = text.replace("\"85\"", "\"99\"");
        assert_ne!(tampered, text, "the test must actually change a value");
        std::fs::write(&path, tampered).expect("writable");

        assert_eq!(cache.load().err(), Some(CacheError::Corrupt));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_truncated_cache_is_corrupt() {
        let path = temporary("truncated");
        let cache = Cache::new(path.clone());
        cache
            .store(&CacheContents {
                generation: 1,
                layer: sample_layer(),
            })
            .expect("cache is writable");
        let text = std::fs::read_to_string(&path).expect("readable");
        std::fs::write(&path, &text[..text.len() / 2]).expect("writable");
        assert_eq!(cache.load().err(), Some(CacheError::Corrupt));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_corrupt_cache_is_left_in_place_rather_than_overwritten() {
        let path = temporary("preserved");
        let cache = Cache::new(path.clone());
        std::fs::write(&path, "this is not a cache").expect("writable");
        assert_eq!(cache.load().err(), Some(CacheError::Corrupt));
        assert_eq!(
            std::fs::read_to_string(&path).expect("readable"),
            "this is not a cache",
            "a failed load must not destroy the evidence"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn comments_are_not_part_of_the_integrity_claim() {
        let path = temporary("comments");
        let cache = Cache::new(path.clone());
        cache
            .store(&CacheContents {
                generation: 3,
                layer: sample_layer(),
            })
            .expect("cache is writable");
        let text = std::fs::read_to_string(&path).expect("readable");
        std::fs::write(&path, format!("# an added comment\n{text}")).expect("writable");
        assert!(
            cache.load().is_ok(),
            "a comment change must not read as corruption"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_empty_cache_round_trips() {
        let path = temporary("empty");
        let cache = Cache::new(path.clone());
        cache
            .store(&CacheContents {
                generation: 1,
                layer: Layer::new(Origin::Persisted),
            })
            .expect("cache is writable");
        let loaded = cache.load().expect("an empty cache is valid");
        assert_eq!(loaded.generation, 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_rewrite_replaces_the_previous_contents() {
        let path = temporary("rewrite");
        let cache = Cache::new(path.clone());
        cache
            .store(&CacheContents {
                generation: 1,
                layer: sample_layer(),
            })
            .expect("writable");
        let mut second = Layer::new(Origin::Persisted);
        second
            .insert(
                SettingKey::unscoped(SettingId::RetentionMaxAge),
                SettingValue::Duration(Duration::from_secs(60 * 60)),
            )
            .expect("valid entry");
        cache
            .store(&CacheContents {
                generation: 2,
                layer: second,
            })
            .expect("writable");

        let loaded = cache.load().expect("reads back");
        assert_eq!(loaded.generation, 2);
        let resolved = ResolvedSettings::resolve(vec![loaded.layer]);
        assert_eq!(
            resolved
                .effective(SettingId::StorageWatermarkWarning, None)
                .map(|value| value.value().render()),
            None,
            "the rewrite must replace, not merge"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_unwritable_path_reports_unwritable() {
        // A path whose parent is an existing file cannot be created.
        let blocker = temporary("blocker");
        std::fs::write(&blocker, "not a directory").expect("writable");
        let cache = Cache::new(blocker.join("cache.toml"));
        assert_eq!(
            cache.store(&CacheContents {
                generation: 1,
                layer: sample_layer(),
            }),
            Err(CacheError::Unwritable)
        );
        let _ = std::fs::remove_file(&blocker);
    }

    #[test]
    fn cache_error_states_map_one_to_one() {
        assert_eq!(CacheError::Absent.state(), CacheState::Absent);
        assert_eq!(CacheError::Corrupt.state(), CacheState::Corrupt);
        assert_eq!(CacheError::Unwritable.state(), CacheState::Unwritable);
    }

    #[test]
    fn stale_degrades_readiness_but_is_not_a_liveness_failure() {
        let stale = Freshness::Stale {
            since: SystemTime::UNIX_EPOCH,
            cache: CacheState::Valid,
            last_success: None,
        };
        assert!(stale.degrades_readiness());
        assert_eq!(stale.gauge(), 1);
        let fresh = Freshness::Fresh {
            loaded_at: SystemTime::UNIX_EPOCH,
        };
        assert!(!fresh.degrades_readiness());
        assert_eq!(fresh.gauge(), 0);
    }

    #[test]
    fn the_retry_budget_outlasts_several_intervals_then_stops() {
        let mut retry = StaleRetry::new(Duration::from_secs(5), Duration::from_secs(300));
        let mut attempts = 0;
        while let Some(delay) = retry.next_delay() {
            retry.record_attempt(delay);
            attempts += 1;
            assert!(attempts <= 100, "the budget must terminate");
        }
        assert_eq!(attempts, 60, "300s of budget at 5s intervals");
        assert!(retry.exhausted());
    }

    #[test]
    fn a_successful_load_resets_the_budget() {
        let mut retry = StaleRetry::new(Duration::from_secs(5), Duration::from_secs(10));
        while let Some(delay) = retry.next_delay() {
            retry.record_attempt(delay);
        }
        assert!(retry.exhausted());
        retry.reset();
        assert!(!retry.exhausted());
        assert_eq!(retry.next_delay(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn a_zero_interval_does_not_spin() {
        let retry = StaleRetry::new(Duration::ZERO, Duration::from_secs(300));
        assert_eq!(retry.next_delay(), None);
    }

    #[test]
    fn the_final_delay_does_not_overshoot_the_budget() {
        let mut retry = StaleRetry::new(Duration::from_secs(7), Duration::from_secs(10));
        let first = retry.next_delay().expect("first attempt");
        retry.record_attempt(first);
        let second = retry.next_delay().expect("second attempt");
        assert_eq!(second, Duration::from_secs(3));
        retry.record_attempt(second);
        assert_eq!(retry.next_delay(), None);
    }
}
