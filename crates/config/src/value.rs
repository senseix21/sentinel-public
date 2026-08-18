//! Setting values, and the kind-directed parsing that produces them.
//!
//! Parsing is driven by [`SettingId::kind`], never by whatever shape the input
//! happened to have. A layer that supplies a string where the setting is an
//! unsigned integer is rejected with the setting named, rather than coerced.

use std::{error::Error, fmt, path::PathBuf, time::Duration};

use crate::{
    classification::{Secrecy, SettingId, ValueKind},
    secret::{Fingerprint, Secret, REDACTED},
};

/// A validated value for one setting.
///
/// [`SettingValue::Secret`] exists so that secrecy is carried by the value, not
/// only by the setting identity. A rendering path that has a value but has lost
/// track of which setting it came from still cannot leak it.
pub enum SettingValue {
    Boolean(bool),
    Unsigned(u64),
    Duration(Duration),
    Text(String),
    Path(PathBuf),
    Secret(Secret<String>),
}

impl SettingValue {
    /// The kind this value satisfies. Secrets are textual.
    #[must_use]
    pub const fn kind(&self) -> ValueKind {
        match self {
            Self::Boolean(_) => ValueKind::Boolean,
            Self::Unsigned(_) => ValueKind::Unsigned,
            Self::Duration(_) => ValueKind::Duration,
            Self::Text(_) | Self::Secret(_) => ValueKind::Text,
            Self::Path(_) => ValueKind::Path,
        }
    }

    #[must_use]
    pub const fn is_secret(&self) -> bool {
        matches!(self, Self::Secret(_))
    }

    /// Renders for display, redacting secrets.
    ///
    /// This is the only rendering used by the API and by logs. There is no
    /// variant that renders a secret, so a caller cannot leak one by choosing
    /// the wrong method.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Boolean(value) => value.to_string(),
            Self::Unsigned(value) => value.to_string(),
            Self::Duration(value) => render_duration(*value),
            Self::Text(value) => value.clone(),
            Self::Path(value) => value.to_string_lossy().into_owned(),
            Self::Secret(_) => REDACTED.to_owned(),
        }
    }

    /// Renders for the settings store.
    ///
    /// Returns `None` for secrets, which are not persisted in Phase 7. The
    /// caller cannot accidentally write one, because there is no value to
    /// write.
    #[must_use]
    pub fn persisted_text(&self) -> Option<String> {
        if self.is_secret() {
            None
        } else {
            Some(self.render())
        }
    }

    /// The audit representation: a plaintext value, or a fingerprint.
    ///
    /// Exactly one side is `Some`, which is the invariant the `settings_audit`
    /// CHECK constraints also encode.
    #[must_use]
    pub fn audit_representation(&self, salt: &Secret<String>) -> AuditRepresentation {
        match self {
            Self::Secret(value) => {
                AuditRepresentation::Fingerprint(Fingerprint::of(salt, value.expose().as_bytes()))
            }
            other => AuditRepresentation::Value(other.render()),
        }
    }

    /// Parses raw text for a setting, according to that setting's kind.
    ///
    /// # Errors
    /// Returns a typed error naming the setting and what was expected. The
    /// offending text is not included for secret settings.
    pub fn parse(id: SettingId, raw: &str) -> Result<Self, ValueParseError> {
        let trimmed = raw.trim();
        match id.kind() {
            ValueKind::Boolean => match trimmed {
                "true" => Ok(Self::Boolean(true)),
                "false" => Ok(Self::Boolean(false)),
                _ => Err(ValueParseError::expected(id, "`true` or `false`")),
            },
            ValueKind::Unsigned => trimmed
                .parse::<u64>()
                .map(Self::Unsigned)
                .map_err(|_| ValueParseError::expected(id, "a non-negative integer")),
            ValueKind::Duration => parse_duration(trimmed).map(Self::Duration).ok_or_else(|| {
                ValueParseError::expected(
                    id,
                    "a duration with a unit, such as `250ms`, `30s`, `5m`, `2h`, or `7d`",
                )
            }),
            ValueKind::Path => {
                if trimmed.is_empty() {
                    Err(ValueParseError::expected(id, "a non-empty path"))
                } else {
                    Ok(Self::Path(PathBuf::from(trimmed)))
                }
            }
            ValueKind::Text => {
                if trimmed.is_empty() {
                    return Err(ValueParseError::expected(id, "non-empty text"));
                }
                Ok(match id.secrecy() {
                    // The untrimmed value is kept for secrets: leading or
                    // trailing whitespace can be significant in a credential,
                    // and silently altering one produces an authentication
                    // failure that looks like a wrong password.
                    Secrecy::Secret => Self::Secret(Secret::new(raw.to_owned())),
                    Secrecy::Public => Self::Text(trimmed.to_owned()),
                })
            }
        }
    }

    /// Parses a TOML value for a setting, according to that setting's kind.
    ///
    /// # Errors
    /// Returns a typed error naming the setting and what was expected.
    pub fn from_toml(id: SettingId, value: &toml::Value) -> Result<Self, ValueParseError> {
        match (id.kind(), value) {
            (ValueKind::Boolean, toml::Value::Boolean(flag)) => Ok(Self::Boolean(*flag)),
            (ValueKind::Unsigned, toml::Value::Integer(number)) => u64::try_from(*number)
                .map(Self::Unsigned)
                .map_err(|_| ValueParseError::expected(id, "a non-negative integer")),
            // Durations, paths, and text all arrive as TOML strings and reuse
            // the textual parser, so file and environment layers cannot
            // disagree about what a value means.
            (
                ValueKind::Duration | ValueKind::Path | ValueKind::Text,
                toml::Value::String(text),
            ) => Self::parse(id, text),
            (kind, other) => Err(ValueParseError {
                id,
                expected: expectation_for(kind),
                found: Some(other.type_str()),
            }),
        }
    }

    /// Whether two values are equal.
    ///
    /// Defined as a method rather than [`PartialEq`] because secret comparison
    /// requires a salt and therefore cannot satisfy the trait's signature.
    #[must_use]
    pub fn same_as(&self, other: &Self, salt: &Secret<String>) -> bool {
        match (self, other) {
            (Self::Boolean(left), Self::Boolean(right)) => left == right,
            (Self::Unsigned(left), Self::Unsigned(right)) => left == right,
            (Self::Duration(left), Self::Duration(right)) => left == right,
            (Self::Text(left), Self::Text(right)) => left == right,
            (Self::Path(left), Self::Path(right)) => left == right,
            (Self::Secret(_), Self::Secret(_)) => {
                self.audit_representation(salt) == other.audit_representation(salt)
            }
            _ => false,
        }
    }
}

impl Clone for SettingValue {
    fn clone(&self) -> Self {
        match self {
            Self::Boolean(value) => Self::Boolean(*value),
            Self::Unsigned(value) => Self::Unsigned(*value),
            Self::Duration(value) => Self::Duration(*value),
            Self::Text(value) => Self::Text(value.clone()),
            Self::Path(value) => Self::Path(value.clone()),
            Self::Secret(value) => Self::Secret(value.clone()),
        }
    }
}

impl fmt::Debug for SettingValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}({})", self.kind(), self.render())
    }
}

/// How a value appears in an audit row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuditRepresentation {
    Value(String),
    Fingerprint(Fingerprint),
}

impl AuditRepresentation {
    /// The plaintext column, or `None` when this side is fingerprinted.
    #[must_use]
    pub fn value_column(&self) -> Option<&str> {
        match self {
            Self::Value(text) => Some(text),
            Self::Fingerprint(_) => None,
        }
    }

    /// The fingerprint column, or `None` when this side is plaintext.
    #[must_use]
    pub fn fingerprint_column(&self) -> Option<String> {
        match self {
            Self::Value(_) => None,
            Self::Fingerprint(fingerprint) => Some(fingerprint.to_hex()),
        }
    }
}

/// Parses `250ms`, `30s`, `5m`, `2h`, `7d`.
///
/// A bare integer is rejected on purpose. The unit-suffixed setting names this
/// replaces — `segment_duration_seconds`, `reconnect_initial_milliseconds` —
/// encoded the unit in the key, and dropping the key suffix without requiring
/// an explicit unit would make `30` ambiguous between the two.
#[must_use]
pub fn parse_duration(raw: &str) -> Option<Duration> {
    let trimmed = raw.trim();
    let split = trimmed.find(|character: char| !character.is_ascii_digit())?;
    if split == 0 {
        return None;
    }
    let (digits, unit) = trimmed.split_at(split);
    let magnitude: u64 = digits.parse().ok()?;
    match unit {
        "ms" => Some(Duration::from_millis(magnitude)),
        "s" => Some(Duration::from_secs(magnitude)),
        "m" => magnitude.checked_mul(60).map(Duration::from_secs),
        "h" => magnitude.checked_mul(60 * 60).map(Duration::from_secs),
        "d" => magnitude.checked_mul(24 * 60 * 60).map(Duration::from_secs),
        _ => None,
    }
}

/// Renders a duration using the largest unit that divides it exactly.
///
/// Exact division keeps the round trip stable: a value written `7d` reads back
/// as `7d`, so a configuration file that is written out does not drift into
/// less legible units.
#[must_use]
pub fn render_duration(value: Duration) -> String {
    let milliseconds = value.as_millis();
    for (unit, factor) in [
        ("d", 24 * 60 * 60 * 1_000_u128),
        ("h", 60 * 60 * 1_000),
        ("m", 60 * 1_000),
        ("s", 1_000),
    ] {
        if milliseconds >= factor && milliseconds.is_multiple_of(factor) {
            return format!("{}{unit}", milliseconds / factor);
        }
    }
    format!("{milliseconds}ms")
}

const fn expectation_for(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Boolean => "a boolean",
        ValueKind::Unsigned => "a non-negative integer",
        ValueKind::Duration => "a duration string such as `30s`",
        ValueKind::Text => "a string",
        ValueKind::Path => "a path string",
    }
}

/// A value that could not be parsed for its setting.
///
/// Carries the setting identity so that every rejection names the offending
/// setting, as P7.4 requires. It never carries the offending text, because that
/// text may be a credential.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValueParseError {
    pub id: SettingId,
    pub expected: &'static str,
    pub found: Option<&'static str>,
}

impl ValueParseError {
    const fn expected(id: SettingId, expected: &'static str) -> Self {
        Self {
            id,
            expected,
            found: None,
        }
    }
}

impl fmt::Display for ValueParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "setting {} expects {}", self.id, self.expected)?;
        if let Some(found) = self.found {
            write!(formatter, ", found {found}")?;
        }
        Ok(())
    }
}

impl Error for ValueParseError {}

#[cfg(test)]
mod tests {
    use super::{parse_duration, render_duration, AuditRepresentation, SettingValue};
    use crate::{classification::SettingId, secret::Secret};
    use std::time::Duration;

    fn salt() -> Secret<String> {
        Secret::new("unit-salt".to_owned())
    }

    #[test]
    fn parses_every_duration_unit() {
        assert_eq!(parse_duration("250ms"), Some(Duration::from_millis(250)));
        assert_eq!(parse_duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7_200)));
        assert_eq!(parse_duration("7d"), Some(Duration::from_secs(604_800)));
    }

    #[test]
    fn rejects_durations_without_a_unit() {
        assert_eq!(parse_duration("30"), None);
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("s"), None);
        assert_eq!(parse_duration("30x"), None);
        assert_eq!(parse_duration("-5s"), None);
    }

    #[test]
    fn duration_rendering_round_trips() {
        for text in ["250ms", "30s", "5m", "2h", "7d"] {
            let parsed = parse_duration(text).unwrap_or(Duration::ZERO);
            assert_eq!(
                render_duration(parsed),
                text,
                "round trip failed for {text}"
            );
        }
    }

    #[test]
    fn duration_rendering_falls_back_to_milliseconds() {
        assert_eq!(render_duration(Duration::from_millis(1_500)), "1500ms");
        assert_eq!(render_duration(Duration::ZERO), "0ms");
    }

    #[test]
    fn rejects_a_string_where_an_integer_is_required() {
        let error = SettingValue::from_toml(
            SettingId::RetentionBatchFiles,
            &toml::Value::String("many".to_owned()),
        )
        .expect_err("a string is not an integer");
        assert_eq!(error.id, SettingId::RetentionBatchFiles);
        assert_eq!(error.found, Some("string"));
    }

    #[test]
    fn rejects_a_negative_integer_for_an_unsigned_setting() {
        assert!(
            SettingValue::from_toml(SettingId::CameraVideoWidth, &toml::Value::Integer(-1))
                .is_err()
        );
    }

    #[test]
    fn never_coerces_an_out_of_range_value() {
        // The rejection must name the setting rather than clamp to a bound.
        let error = SettingValue::parse(SettingId::StorageWatermarkWarning, "not-a-number")
            .expect_err("text is not unsigned");
        assert_eq!(error.id, SettingId::StorageWatermarkWarning);
    }

    #[test]
    fn secret_settings_parse_into_a_secret_value() {
        let value = SettingValue::parse(SettingId::CameraRtspPassword, "hunter2")
            .unwrap_or(SettingValue::Boolean(false));
        assert!(value.is_secret());
        assert_eq!(value.render(), "***");
        assert_eq!(value.persisted_text(), None);
    }

    #[test]
    fn secret_values_never_render_their_plaintext() {
        let value = SettingValue::parse(SettingId::CameraRtspPassword, "hunter2")
            .unwrap_or(SettingValue::Boolean(false));
        assert!(!format!("{value:?}").contains("hunter2"));
        assert!(!value.render().contains("hunter2"));
    }

    #[test]
    fn secret_whitespace_is_preserved_but_public_text_is_trimmed() {
        let secret = SettingValue::parse(SettingId::CameraRtspPassword, " pad ")
            .unwrap_or(SettingValue::Boolean(false));
        match secret {
            SettingValue::Secret(value) => assert_eq!(value.expose(), " pad "),
            _ => panic!("expected a secret"),
        }
        let public = SettingValue::parse(SettingId::CameraLiveKitEndpoint, " ws://127.0.0.1:7880 ")
            .unwrap_or(SettingValue::Boolean(false));
        assert_eq!(public.render(), "ws://127.0.0.1:7880");
    }

    #[test]
    fn audit_representation_uses_exactly_one_column_per_side() {
        let public = SettingValue::Unsigned(80);
        let public_side = public.audit_representation(&salt());
        assert!(public_side.value_column().is_some());
        assert!(public_side.fingerprint_column().is_none());

        let secret = SettingValue::Secret(Secret::new("hunter2".to_owned()));
        let secret_side = secret.audit_representation(&salt());
        assert!(secret_side.value_column().is_none());
        let fingerprint = secret_side
            .fingerprint_column()
            .unwrap_or_else(|| "missing".to_owned());
        assert_eq!(fingerprint.len(), 64);
        assert!(!fingerprint.contains("hunter2"));
    }

    #[test]
    fn audit_fingerprints_reveal_a_change_and_a_revert() {
        let first = SettingValue::Secret(Secret::new("one".to_owned()));
        let second = SettingValue::Secret(Secret::new("two".to_owned()));
        let back = SettingValue::Secret(Secret::new("one".to_owned()));
        let deployment = salt();
        assert_ne!(
            first.audit_representation(&deployment),
            second.audit_representation(&deployment)
        );
        assert_eq!(
            first.audit_representation(&deployment),
            back.audit_representation(&deployment),
            "a revert to a previously used value must be visible"
        );
    }

    #[test]
    fn same_as_compares_secrets_without_rendering_them() {
        let deployment = salt();
        let left = SettingValue::Secret(Secret::new("same".to_owned()));
        let right = SettingValue::Secret(Secret::new("same".to_owned()));
        let other = SettingValue::Secret(Secret::new("different".to_owned()));
        assert!(left.same_as(&right, &deployment));
        assert!(!left.same_as(&other, &deployment));
    }

    #[test]
    fn same_as_is_false_across_kinds() {
        let deployment = salt();
        assert!(!SettingValue::Unsigned(1).same_as(&SettingValue::Boolean(true), &deployment));
    }

    #[test]
    fn audit_representation_is_stable_for_equal_public_values() {
        let deployment = salt();
        assert_eq!(
            SettingValue::Unsigned(80).audit_representation(&deployment),
            AuditRepresentation::Value("80".to_owned())
        );
    }

    #[test]
    fn empty_text_and_paths_are_rejected() {
        assert!(SettingValue::parse(SettingId::PlaybackFfmpeg, "   ").is_err());
        assert!(SettingValue::parse(SettingId::CameraLiveKitEndpoint, "").is_err());
    }

    #[test]
    fn boolean_parsing_is_exact() {
        assert!(SettingValue::parse(SettingId::CameraDetectionEnabled, "true").is_ok());
        assert!(SettingValue::parse(SettingId::CameraDetectionEnabled, "false").is_ok());
        for rejected in ["True", "yes", "1", "on", ""] {
            assert!(
                SettingValue::parse(SettingId::CameraDetectionEnabled, rejected).is_err(),
                "{rejected} should not parse as a boolean"
            );
        }
    }

    #[test]
    fn toml_and_text_layers_agree_on_duration_meaning() {
        let from_text = SettingValue::parse(SettingId::RetentionInterval, "5m")
            .unwrap_or(SettingValue::Boolean(false));
        let from_toml = SettingValue::from_toml(
            SettingId::RetentionInterval,
            &toml::Value::String("5m".to_owned()),
        )
        .unwrap_or(SettingValue::Boolean(false));
        assert!(from_text.same_as(&from_toml, &salt()));
    }
}
