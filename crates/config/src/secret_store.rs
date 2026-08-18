//! Secret storage: a permission-enforced file, with per-key environment
//! override.
//!
//! ADR 009 Decision 4, closing the question ADR 003 left open and ADR 008
//! deferred. Secrets do **not** live in `PostgreSQL`: a database outage would
//! otherwise become a camera-authentication outage, and recording preservation
//! outranks configuration convenience.
//!
//! Two properties this module exists to hold:
//!
//! 1. **A too-readable file is refused, with the observed mode in the error.**
//!    Discovering that credentials were world-readable is worth an operator's
//!    attention; silently reading them anyway is not a control.
//! 2. **A missing secret is a named failure, never a silent fallback.** The
//!    Phase 7 rule applies unchanged: degrade visibly, never substitute.
//!
//! ## What this does not enforce
//!
//! ADR 009 also calls for the file to be *owned* by the running user. That is
//! not checked here: reading the process's effective uid needs either a `libc`
//! call, which `unsafe_code = "forbid"` rules out, or a new dependency whose
//! only purpose is one syscall. The mode check is the part that stops another
//! account reading the file; ownership belongs with the packaging that creates
//! the service user, which is roadmap item 8 and currently blocked on the
//! deployment target. Recorded in `docs/phase-8/SPEC.md` as a residual rather
//! than quietly dropped.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use surveillance_common::secret::Secret;

/// The only file mode a secrets file may have: owner read/write, nothing else.
pub const REQUIRED_MODE: u32 = 0o600;

/// Prefix for the per-key environment override.
pub const ENVIRONMENT_PREFIX: &str = "SURVEILLANCE_SECRET_";

/// The largest secrets file this loader will read, in bytes.
///
/// A secrets file is a handful of short strings. A bound stops a mistaken path
/// — a log, a recording — from being slurped into memory and, worse, from
/// having its contents treated as credentials.
pub const MAX_FILE_BYTES: u64 = 64 * 1024;

/// A validated, namespaced secret key.
///
/// The namespace is meaningful: `camera.<id>.rtsp_password` is what makes
/// per-camera credentials expressible at all. The Phase 7 environment layer had
/// no per-camera syntax, which is why cameras could not have different
/// passwords.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SecretKey(String);

impl SecretKey {
    /// Accepts dot-separated segments of ASCII alphanumerics, `-`, and `_`.
    ///
    /// # Errors
    /// Returns [`SecretKeyError`] naming the violation.
    pub fn new(value: &str) -> Result<Self, SecretKeyError> {
        if value.is_empty() {
            return Err(SecretKeyError::Empty);
        }
        if value.len() > 128 {
            return Err(SecretKeyError::TooLong);
        }
        let mut segments = 0;
        for segment in value.split('.') {
            if segment.is_empty() {
                return Err(SecretKeyError::EmptySegment);
            }
            if !segment.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            }) {
                return Err(SecretKeyError::IllegalCharacter);
            }
            segments += 1;
        }
        if segments < 2 {
            return Err(SecretKeyError::NotNamespaced);
        }
        Ok(Self(value.to_owned()))
    }

    /// The per-camera RTSP password key for `camera`.
    ///
    /// # Errors
    /// Returns [`SecretKeyError`] if the camera identifier cannot appear in a
    /// key. `CameraId` validation is stricter than this, so in practice this
    /// cannot fail — but returning the error keeps the two validators
    /// independent rather than coupling one to the other's current rules.
    pub fn camera_rtsp_password(camera: &str) -> Result<Self, SecretKeyError> {
        Self::new(&format!("camera.{camera}.rtsp_password"))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The environment variable that overrides this key.
    ///
    /// `camera.front-door.rtsp_password` becomes
    /// `SURVEILLANCE_SECRET_CAMERA_FRONT_DOOR_RTSP_PASSWORD`.
    #[must_use]
    pub fn environment_variable(&self) -> String {
        let mut rendered = String::with_capacity(ENVIRONMENT_PREFIX.len() + self.0.len());
        rendered.push_str(ENVIRONMENT_PREFIX);
        for character in self.0.chars() {
            rendered.push(match character {
                '.' | '-' => '_',
                other => other.to_ascii_uppercase(),
            });
        }
        rendered
    }
}

impl fmt::Display for SecretKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A key that cannot be represented.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretKeyError {
    Empty,
    TooLong,
    EmptySegment,
    IllegalCharacter,
    NotNamespaced,
}

impl fmt::Display for SecretKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("secret key is empty"),
            Self::TooLong => formatter.write_str("secret key exceeds 128 bytes"),
            Self::EmptySegment => formatter.write_str("secret key has an empty segment"),
            Self::IllegalCharacter => {
                formatter.write_str("secret key accepts only alphanumerics, '-', and '_'")
            }
            Self::NotNamespaced => {
                formatter.write_str("secret key must have at least two dot-separated segments")
            }
        }
    }
}

impl Error for SecretKeyError {}

/// Where a resolved secret came from.
///
/// Reported through the settings API so an operator can see *where* a
/// credential came from without seeing what it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretSource {
    File,
    Environment,
}

impl SecretSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Environment => "environment",
        }
    }
}

/// A resolved secret and its provenance.
pub struct ResolvedSecret {
    value: Secret<String>,
    source: SecretSource,
}

impl ResolvedSecret {
    #[must_use]
    pub const fn value(&self) -> &Secret<String> {
        &self.value
    }

    #[must_use]
    pub const fn source(&self) -> SecretSource {
        self.source
    }
}

impl fmt::Debug for ResolvedSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The source is diagnostic; the value is not.
        formatter
            .debug_struct("ResolvedSecret")
            .field("source", &self.source)
            .field("value", &self.value)
            .finish()
    }
}

/// Resolved secrets for one deployment.
#[derive(Debug, Default)]
pub struct SecretStore {
    entries: BTreeMap<SecretKey, ResolvedSecret>,
    path: Option<PathBuf>,
}

impl SecretStore {
    /// An empty store, for deployments that inject everything by environment.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Loads from `path` if present, then applies environment overrides.
    ///
    /// An absent file is not an error: a container may inject every secret by
    /// environment. A *present but unreadable* file is an error, because that
    /// is a deployment that meant to provide secrets and failed to.
    ///
    /// # Errors
    /// Returns [`SecretLoadError`], including
    /// [`SecretLoadError::PermissionsTooOpen`] with the observed mode.
    pub fn load(
        path: Option<&Path>,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, SecretLoadError> {
        let mut store = Self::empty();
        if let Some(path) = path {
            store.path = Some(path.to_path_buf());
            if path.exists() {
                store.load_file(path)?;
            }
        }
        store.apply_environment(&lookup);
        Ok(store)
    }

    fn load_file(&mut self, path: &Path) -> Result<(), SecretLoadError> {
        let metadata = std::fs::metadata(path).map_err(|_| SecretLoadError::Unreadable)?;
        if !metadata.is_file() {
            return Err(SecretLoadError::NotAFile);
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err(SecretLoadError::TooLarge);
        }
        let observed = file_mode(&metadata);
        if observed != REQUIRED_MODE {
            return Err(SecretLoadError::PermissionsTooOpen { observed });
        }

        let text = std::fs::read_to_string(path).map_err(|_| SecretLoadError::Unreadable)?;
        let document: toml::Value = text.parse().map_err(|_| SecretLoadError::Malformed)?;
        let table = document.as_table().ok_or(SecretLoadError::Malformed)?;

        for (key, value) in table {
            let key = SecretKey::new(key).map_err(SecretLoadError::Key)?;
            let value = value.as_str().ok_or(SecretLoadError::NotAString)?;
            self.entries.insert(
                key,
                ResolvedSecret {
                    value: Secret::new(value.to_owned()),
                    source: SecretSource::File,
                },
            );
        }
        Ok(())
    }

    fn apply_environment(&mut self, lookup: &impl Fn(&str) -> Option<String>) {
        // Overrides apply only to keys the file already declares, plus the
        // well-known keys below. An environment variable cannot introduce an
        // arbitrary key, because a typo would then create a silently unused
        // secret rather than failing.
        let known: Vec<SecretKey> = self.entries.keys().cloned().collect();
        for key in known {
            if let Some(value) = lookup(&key.environment_variable()) {
                self.entries.insert(
                    key,
                    ResolvedSecret {
                        value: Secret::new(value),
                        source: SecretSource::Environment,
                    },
                );
            }
        }
    }

    /// Adds a key that the environment may supply even when no file declares
    /// it.
    ///
    /// Used for the fixed deployment secrets — the proxy proof, the audit salt,
    /// the `LiveKit` key — which a container injects without a file.
    pub fn declare(&mut self, key: SecretKey, lookup: &impl Fn(&str) -> Option<String>) {
        if let Some(value) = lookup(&key.environment_variable()) {
            self.entries.insert(
                key,
                ResolvedSecret {
                    value: Secret::new(value),
                    source: SecretSource::Environment,
                },
            );
        }
    }

    /// Looks up a secret.
    #[must_use]
    pub fn get(&self, key: &SecretKey) -> Option<&ResolvedSecret> {
        self.entries.get(key)
    }

    /// Looks up a secret, or reports which key was missing.
    ///
    /// # Errors
    /// Returns [`SecretMissing`] naming the key. Callers must surface this
    /// rather than substituting an empty or default credential.
    pub fn require(&self, key: &SecretKey) -> Result<&ResolvedSecret, SecretMissing> {
        self.get(key).ok_or_else(|| SecretMissing {
            key: key.clone(),
            variable: key.environment_variable(),
        })
    }

    /// Every key held, for reporting provenance without values.
    pub fn keys(&self) -> impl Iterator<Item = (&SecretKey, SecretSource)> {
        self.entries.iter().map(|(key, entry)| (key, entry.source))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

#[cfg(unix)]
fn file_mode(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn file_mode(_metadata: &std::fs::Metadata) -> u32 {
    // No POSIX mode to inspect. Reporting a mode that cannot be enforced would
    // be worse than reporting an impossible one: this value never equals
    // `REQUIRED_MODE`, so a non-Unix host refuses the file rather than reading
    // it under an unverifiable permission claim.
    0o777
}

/// A required secret that is not present.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretMissing {
    pub key: SecretKey,
    pub variable: String,
}

impl fmt::Display for SecretMissing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "secret '{}' is not configured; set it in the secrets file or as {}",
            self.key, self.variable
        )
    }
}

impl Error for SecretMissing {}

/// Why a secrets file could not be loaded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretLoadError {
    /// The file is readable by more than its owner. Carries the observed mode
    /// so the operator does not have to go and look.
    PermissionsTooOpen {
        observed: u32,
    },
    NotAFile,
    TooLarge,
    Unreadable,
    Malformed,
    NotAString,
    Key(SecretKeyError),
}

impl fmt::Display for SecretLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermissionsTooOpen { observed } => write!(
                formatter,
                "secrets file mode is {observed:04o}; it must be {REQUIRED_MODE:04o}"
            ),
            Self::NotAFile => formatter.write_str("the secrets path is not a regular file"),
            Self::TooLarge => write!(formatter, "the secrets file exceeds {MAX_FILE_BYTES} bytes"),
            Self::Unreadable => formatter.write_str("the secrets file could not be read"),
            Self::Malformed => formatter.write_str("the secrets file is not a TOML table"),
            Self::NotAString => formatter.write_str("every secret value must be a string"),
            Self::Key(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for SecretLoadError {}

#[cfg(test)]
mod tests {
    use super::{
        SecretKey, SecretKeyError, SecretLoadError, SecretSource, SecretStore, ENVIRONMENT_PREFIX,
        REQUIRED_MODE,
    };
    use std::path::{Path, PathBuf};

    const PLAINTEXT: &str = "correct-horse-battery-staple";

    fn no_environment() -> impl Fn(&str) -> Option<String> {
        |_| None
    }

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |key| {
            owned
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, value)| value.clone())
        }
    }

    /// Writes a secrets file with an explicit mode and returns its path.
    fn write_secrets(name: &str, contents: &str, mode: u32) -> PathBuf {
        let directory = std::env::temp_dir().join("surveillance-secret-store-tests");
        std::fs::create_dir_all(&directory).expect("test directory is creatable");
        let path = directory.join(name);
        std::fs::write(&path, contents).expect("test file is writable");
        set_mode(&path, mode);
        path
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .expect("test file mode is settable");
    }

    #[cfg(not(unix))]
    fn set_mode(_path: &Path, _mode: u32) {}

    fn key(value: &str) -> SecretKey {
        SecretKey::new(value).expect("test key is valid")
    }

    #[test]
    fn keys_must_be_namespaced() {
        assert!(SecretKey::new("camera.front-door.rtsp_password").is_ok());
        assert!(SecretKey::new("livekit.api_secret").is_ok());
        assert_eq!(
            SecretKey::new("flat").err(),
            Some(SecretKeyError::NotNamespaced)
        );
        assert_eq!(SecretKey::new("").err(), Some(SecretKeyError::Empty));
        assert_eq!(
            SecretKey::new("camera..password").err(),
            Some(SecretKeyError::EmptySegment)
        );
        assert_eq!(
            SecretKey::new("camera.front door.password").err(),
            Some(SecretKeyError::IllegalCharacter)
        );
    }

    #[test]
    fn a_per_camera_key_is_expressible() {
        // The Phase 7 environment layer had no syntax for this, which is why
        // cameras could not have different credentials.
        let first = SecretKey::camera_rtsp_password("front-door").expect("valid");
        let second = SecretKey::camera_rtsp_password("back-gate").expect("valid");
        assert_ne!(first, second);
        assert_eq!(first.as_str(), "camera.front-door.rtsp_password");
    }

    #[test]
    fn the_environment_variable_name_is_derived_deterministically() {
        assert_eq!(
            key("camera.front-door.rtsp_password").environment_variable(),
            format!("{ENVIRONMENT_PREFIX}CAMERA_FRONT_DOOR_RTSP_PASSWORD")
        );
        assert_eq!(
            key("livekit.api_secret").environment_variable(),
            format!("{ENVIRONMENT_PREFIX}LIVEKIT_API_SECRET")
        );
    }

    #[test]
    fn a_correctly_permissioned_file_loads() {
        let path = write_secrets(
            "good.toml",
            &format!("\"camera.front-door.rtsp_password\" = \"{PLAINTEXT}\"\n"),
            REQUIRED_MODE,
        );
        let store = SecretStore::load(Some(&path), no_environment()).expect("file loads");
        let entry = store
            .require(&key("camera.front-door.rtsp_password"))
            .expect("key is present");
        assert_eq!(entry.value().expose(), PLAINTEXT);
        assert_eq!(entry.source(), SecretSource::File);
        assert_eq!(store.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_world_readable_file_is_refused_and_names_the_observed_mode() {
        let path = write_secrets(
            "open.toml",
            &format!("\"livekit.api_secret\" = \"{PLAINTEXT}\"\n"),
            0o644,
        );
        let error = SecretStore::load(Some(&path), no_environment())
            .expect_err("a too-open file must be refused");
        assert_eq!(
            error,
            SecretLoadError::PermissionsTooOpen { observed: 0o644 }
        );
        // The operator should not have to go and look up what it currently is.
        assert!(error.to_string().contains("0644"));
        assert!(error.to_string().contains("0600"));
    }

    #[cfg(unix)]
    #[test]
    fn a_group_readable_file_is_refused_too() {
        let path = write_secrets(
            "group.toml",
            &format!("\"livekit.api_secret\" = \"{PLAINTEXT}\"\n"),
            0o640,
        );
        assert_eq!(
            SecretStore::load(Some(&path), no_environment()).err(),
            Some(SecretLoadError::PermissionsTooOpen { observed: 0o640 })
        );
    }

    #[test]
    fn a_refusal_never_renders_the_secret_value() {
        let path = write_secrets(
            "leak.toml",
            &format!("\"livekit.api_secret\" = \"{PLAINTEXT}\"\n"),
            0o644,
        );
        if let Err(error) = SecretStore::load(Some(&path), no_environment()) {
            assert!(!error.to_string().contains(PLAINTEXT));
        }
    }

    #[test]
    fn an_absent_file_is_not_an_error() {
        // A container may inject every secret by environment.
        let store = SecretStore::load(
            Some(Path::new("/nonexistent/surveillance/secrets.toml")),
            no_environment(),
        )
        .expect("an absent file is legal");
        assert!(store.is_empty());
    }

    #[test]
    fn the_environment_overrides_a_file_value_and_records_the_source() {
        let path = write_secrets(
            "override.toml",
            "\"camera.front-door.rtsp_password\" = \"from-file\"\n",
            REQUIRED_MODE,
        );
        let store = SecretStore::load(
            Some(&path),
            environment(&[(
                &format!("{ENVIRONMENT_PREFIX}CAMERA_FRONT_DOOR_RTSP_PASSWORD"),
                "from-environment",
            )]),
        )
        .expect("file loads");
        let entry = store
            .require(&key("camera.front-door.rtsp_password"))
            .expect("present");
        assert_eq!(entry.value().expose(), "from-environment");
        assert_eq!(entry.source(), SecretSource::Environment);
    }

    #[test]
    fn an_environment_variable_alone_cannot_introduce_an_undeclared_key() {
        // A typo would otherwise create a silently unused secret, and the
        // deployment would look configured when it was not.
        let store = SecretStore::load(
            None,
            environment(&[(
                &format!("{ENVIRONMENT_PREFIX}CAMERA_TYPO_RTSP_PASSWORD"),
                PLAINTEXT,
            )]),
        )
        .expect("loads");
        assert!(store.is_empty());
    }

    #[test]
    fn a_declared_key_may_come_from_the_environment_alone() {
        let mut store = SecretStore::empty();
        let lookup = environment(&[(
            &format!("{ENVIRONMENT_PREFIX}LIVEKIT_API_SECRET"),
            PLAINTEXT,
        )]);
        store.declare(key("livekit.api_secret"), &lookup);
        assert_eq!(
            store
                .require(&key("livekit.api_secret"))
                .expect("present")
                .value()
                .expose(),
            PLAINTEXT
        );
    }

    #[test]
    fn a_missing_required_secret_names_itself_and_never_substitutes() {
        let store = SecretStore::empty();
        let missing = store
            .require(&key("camera.front-door.rtsp_password"))
            .expect_err("an absent secret must be an error");
        assert_eq!(missing.key.as_str(), "camera.front-door.rtsp_password");
        let rendered = missing.to_string();
        assert!(rendered.contains("camera.front-door.rtsp_password"));
        assert!(rendered.contains("SURVEILLANCE_SECRET_CAMERA_FRONT_DOOR_RTSP_PASSWORD"));
    }

    #[test]
    fn a_non_string_value_is_refused() {
        let path = write_secrets(
            "number.toml",
            "\"livekit.api_secret\" = 12345\n",
            REQUIRED_MODE,
        );
        assert_eq!(
            SecretStore::load(Some(&path), no_environment()).err(),
            Some(SecretLoadError::NotAString)
        );
    }

    #[test]
    fn a_malformed_file_is_refused() {
        let path = write_secrets("bad.toml", "this is not toml = = =\n", REQUIRED_MODE);
        assert_eq!(
            SecretStore::load(Some(&path), no_environment()).err(),
            Some(SecretLoadError::Malformed)
        );
    }

    #[test]
    fn an_invalid_key_in_the_file_is_refused() {
        let path = write_secrets("badkey.toml", "flat = \"value\"\n", REQUIRED_MODE);
        assert_eq!(
            SecretStore::load(Some(&path), no_environment()).err(),
            Some(SecretLoadError::Key(SecretKeyError::NotNamespaced))
        );
    }

    #[test]
    fn provenance_is_reportable_without_values() {
        let path = write_secrets(
            "provenance.toml",
            "\"camera.a.rtsp_password\" = \"a\"\n\"camera.b.rtsp_password\" = \"b\"\n",
            REQUIRED_MODE,
        );
        let store = SecretStore::load(Some(&path), no_environment()).expect("loads");
        let reported: Vec<String> = store
            .keys()
            .map(|(key, source)| format!("{key}={}", source.as_str()))
            .collect();
        assert_eq!(
            reported,
            vec![
                "camera.a.rtsp_password=file".to_owned(),
                "camera.b.rtsp_password=file".to_owned()
            ]
        );
        // Two cameras, two different credentials: the thing Phase 7 could not
        // express.
        assert_ne!(
            store
                .require(&key("camera.a.rtsp_password"))
                .expect("present")
                .value()
                .expose(),
            store
                .require(&key("camera.b.rtsp_password"))
                .expect("present")
                .value()
                .expose()
        );
    }

    #[test]
    fn the_debug_rendering_never_contains_a_value() {
        let path = write_secrets(
            "debug.toml",
            &format!("\"livekit.api_secret\" = \"{PLAINTEXT}\"\n"),
            REQUIRED_MODE,
        );
        let store = SecretStore::load(Some(&path), no_environment()).expect("loads");
        let rendered = format!("{store:?}");
        assert!(!rendered.contains(PLAINTEXT), "leaked: {rendered}");
        assert!(rendered.contains("livekit.api_secret"));
    }
}
