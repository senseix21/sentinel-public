//! The environment layer: highest precedence, and the only source of secrets.
//!
//! Canonical names are mechanical: `SURVEILLANCE_` followed by the setting key
//! uppercased with dots replaced by underscores, so `retention.max_age` is
//! `SURVEILLANCE_RETENTION_MAX_AGE`. Nothing needs to be looked up in a table.
//!
//! The variables that predate this module keep working through an explicit
//! alias table. Some of them encode a unit in the name — `RETENTION_DAYS`,
//! `DETECTION_MAX_GAP_MS` — so an alias may carry the unit its bare integer
//! implies. Setting both a canonical name and its alias is an error rather than
//! a silent preference, because the two disagreeing is exactly the situation an
//! operator needs told about.
//!
//! # Scope limitation
//!
//! Environment entries are all-camera templates. There is no per-camera
//! environment syntax, so a deployment whose cameras need *different*
//! credentials cannot express that here. That gap is tied to the production
//! secret store, which ADR 003 still leaves open; inventing an encoding for
//! camera identifiers inside variable names would decide it by accident.

use std::{error::Error, fmt};

use crate::{
    classification::{Origin, SettingId},
    resolution::{Layer, SettingKey},
    value::{SettingValue, ValueParseError},
};

/// The prefix every configuration variable carries.
pub const PREFIX: &str = "SURVEILLANCE_";

/// A unit implied by a legacy variable's name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImpliedUnit {
    /// The value is already in the setting's own syntax.
    None,
    Milliseconds,
    Seconds,
    Hours,
    Days,
}

impl ImpliedUnit {
    fn apply(self, raw: &str) -> String {
        let suffix = match self {
            Self::None => return raw.to_owned(),
            Self::Milliseconds => "ms",
            Self::Seconds => "s",
            Self::Hours => "h",
            Self::Days => "d",
        };
        format!("{}{suffix}", raw.trim())
    }
}

/// A variable name that predates the canonical scheme.
#[derive(Clone, Copy, Debug)]
pub struct Alias {
    pub name: &'static str,
    pub id: SettingId,
    pub implied: ImpliedUnit,
}

/// Variable names in use before Phase 7, kept working.
///
/// `SURVEILLANCE_CAMERA_ID` is absent on purpose: a camera identifier is the
/// scope key, not a setting.
pub const LEGACY_ALIASES: &[Alias] = &[
    Alias {
        name: "SURVEILLANCE_RTSP_URL",
        id: SettingId::CameraRtspEndpoint,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_RTSP_USERNAME",
        id: SettingId::CameraRtspUsername,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_RTSP_PASSWORD",
        id: SettingId::CameraRtspPassword,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_VIDEO_WIDTH",
        id: SettingId::CameraVideoWidth,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_VIDEO_HEIGHT",
        id: SettingId::CameraVideoHeight,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_RECORDING_ROOT",
        id: SettingId::StorageRecordingRoot,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_HEALTH_BIND",
        id: SettingId::ServerHealthBind,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_MAXIMUM_CAMERAS",
        id: SettingId::SupervisionMaximumCameras,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_COMMAND_QUEUE_ITEMS",
        id: SettingId::SupervisionCommandQueueItems,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_COMMAND_TIMEOUT_SECONDS",
        id: SettingId::SupervisionCommandTimeout,
        implied: ImpliedUnit::Seconds,
    },
    // SURVEILLANCE_DATABASE_URL and SURVEILLANCE_DATABASE_MAX_CONNECTIONS need
    // no alias: they already equal their canonical names. The same holds for
    // SURVEILLANCE_RETENTION_BATCH_FILES and SURVEILLANCE_RETENTION_BATCH_BYTES.
    Alias {
        name: "SURVEILLANCE_DATABASE_ACQUIRE_TIMEOUT_SECONDS",
        id: SettingId::DatabaseAcquireTimeout,
        implied: ImpliedUnit::Seconds,
    },
    Alias {
        name: "SURVEILLANCE_RETENTION_DAYS",
        id: SettingId::RetentionMaxAge,
        implied: ImpliedUnit::Days,
    },
    Alias {
        name: "SURVEILLANCE_RETENTION_PROTECTED_HOURS",
        id: SettingId::RetentionProtectedRecent,
        implied: ImpliedUnit::Hours,
    },
    Alias {
        name: "SURVEILLANCE_RETENTION_INTERVAL_SECONDS",
        id: SettingId::RetentionInterval,
        implied: ImpliedUnit::Seconds,
    },
    Alias {
        name: "SURVEILLANCE_FFMPEG",
        id: SettingId::PlaybackFfmpeg,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_LIVEKIT_URL",
        id: SettingId::CameraLiveKitEndpoint,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_LIVEKIT_API_KEY",
        id: SettingId::CameraLiveKitApiKey,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_LIVEKIT_API_SECRET",
        id: SettingId::CameraLiveKitApiSecret,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_LIVEKIT_NAMESPACE",
        id: SettingId::CameraLiveKitDeploymentNamespace,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_LIVEKIT_CLI",
        id: SettingId::CameraLiveKitCliPath,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_LIVEKIT_CLI_VERSION",
        id: SettingId::CameraLiveKitCliVersion,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_DETECTION_ENABLED",
        id: SettingId::CameraDetectionEnabled,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_DETECTION_QUEUE_ITEMS",
        id: SettingId::CameraDetectionQueueItems,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_DETECTION_SAMPLE_FPS",
        id: SettingId::CameraDetectionSampleFps,
        implied: ImpliedUnit::None,
    },
    Alias {
        name: "SURVEILLANCE_DETECTION_MAX_GAP_MS",
        id: SettingId::CameraDetectionMaxGap,
        implied: ImpliedUnit::Milliseconds,
    },
];

/// The canonical variable name for a setting.
#[must_use]
pub fn canonical_name(id: SettingId) -> String {
    format!("{PREFIX}{}", id.key().to_uppercase().replace('.', "_"))
}

/// Builds the environment layer from an arbitrary lookup.
///
/// # Errors
/// Returns a conflict when a setting is supplied under both its canonical name
/// and a legacy alias, or a parse failure naming the setting.
pub fn layer_from(lookup: impl Fn(&str) -> Option<String>) -> Result<Layer, EnvironmentError> {
    let mut layer = Layer::new(Origin::Environment);

    for id in SettingId::ALL.iter().copied() {
        let canonical = canonical_name(id);
        let direct = lookup(&canonical).map(|raw| (canonical.clone(), raw));
        let aliased = LEGACY_ALIASES
            .iter()
            .filter(|alias| alias.id == id)
            .find_map(|alias| {
                lookup(alias.name).map(|raw| (alias.name.to_owned(), alias.implied.apply(&raw)))
            });

        let (name, raw) = match (direct, aliased) {
            (Some(direct), Some(aliased)) => {
                return Err(EnvironmentError::Conflicting {
                    id,
                    canonical: direct.0,
                    alias: aliased.0,
                })
            }
            (Some(found), None) | (None, Some(found)) => found,
            (None, None) => continue,
        };

        let value = SettingValue::parse(id, &raw)
            .map_err(|source| EnvironmentError::Value { name, source })?;
        layer
            .insert(SettingKey::unscoped(id), value)
            .map_err(|error| EnvironmentError::Layer {
                id,
                message: error.to_string(),
            })?;
    }

    Ok(layer)
}

/// Builds the environment layer from the process environment.
///
/// # Errors
/// See [`layer_from`].
pub fn layer() -> Result<Layer, EnvironmentError> {
    layer_from(|name| std::env::var(name).ok())
}

/// An environment layer that could not be built.
#[derive(Debug)]
pub enum EnvironmentError {
    /// A setting supplied under both its canonical name and a legacy alias.
    Conflicting {
        id: SettingId,
        canonical: String,
        alias: String,
    },
    Value {
        name: String,
        source: ValueParseError,
    },
    Layer {
        id: SettingId,
        message: String,
    },
}

impl fmt::Display for EnvironmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflicting {
                id,
                canonical,
                alias,
            } => write!(
                formatter,
                "setting {id} is supplied by both {canonical} and {alias}; \
                 remove one so the effective value is unambiguous"
            ),
            Self::Value { name, source } => write!(formatter, "{name}: {source}"),
            Self::Layer { id, message } => write!(formatter, "{id}: {message}"),
        }
    }
}

impl Error for EnvironmentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Value { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_name, layer_from, EnvironmentError, ImpliedUnit, LEGACY_ALIASES, PREFIX,
    };
    use crate::{classification::SettingId, resolution::ResolvedSettings};
    use std::collections::BTreeMap;

    fn from(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn resolve(pairs: &[(&str, &str)]) -> ResolvedSettings {
        let environment = from(pairs);
        let layer = layer_from(|name| environment.get(name).cloned()).expect("valid environment");
        ResolvedSettings::resolve(vec![layer])
    }

    #[test]
    fn canonical_names_are_mechanical() {
        assert_eq!(
            canonical_name(SettingId::RetentionMaxAge),
            "SURVEILLANCE_RETENTION_MAX_AGE"
        );
        assert_eq!(
            canonical_name(SettingId::CameraDetectionSampleFps),
            "SURVEILLANCE_CAMERA_DETECTION_SAMPLE_FPS"
        );
    }

    #[test]
    fn canonical_names_are_unique_and_prefixed() {
        let names: std::collections::BTreeSet<String> = SettingId::ALL
            .iter()
            .map(|id| canonical_name(*id))
            .collect();
        assert_eq!(
            names.len(),
            SettingId::ALL.len(),
            "collision in variable names"
        );
        assert!(names.iter().all(|name| name.starts_with(PREFIX)));
    }

    #[test]
    fn a_canonical_variable_is_read() {
        let resolved = resolve(&[("SURVEILLANCE_RETENTION_MAX_AGE", "3d")]);
        assert_eq!(
            resolved
                .effective(SettingId::RetentionMaxAge, None)
                .map(|value| value.value().render()),
            Some("3d".to_owned())
        );
    }

    #[test]
    fn legacy_aliases_still_work() {
        let resolved = resolve(&[("SURVEILLANCE_RETENTION_DAYS", "7")]);
        assert_eq!(
            resolved
                .effective(SettingId::RetentionMaxAge, None)
                .map(|value| value.value().render()),
            Some("7d".to_owned()),
            "a pre-Phase-7 deployment variable must keep working"
        );
    }

    #[test]
    fn legacy_unit_suffixes_are_applied() {
        let resolved = resolve(&[
            ("SURVEILLANCE_RETENTION_PROTECTED_HOURS", "2"),
            ("SURVEILLANCE_RETENTION_INTERVAL_SECONDS", "90"),
            ("SURVEILLANCE_DETECTION_MAX_GAP_MS", "1500"),
        ]);
        assert_eq!(
            resolved
                .effective(SettingId::RetentionProtectedRecent, None)
                .map(|value| value.value().render()),
            Some("2h".to_owned())
        );
        assert_eq!(
            resolved
                .effective(SettingId::RetentionInterval, None)
                .map(|value| value.value().render()),
            Some("90s".to_owned())
        );
        assert_eq!(
            resolved
                .effective(SettingId::CameraDetectionMaxGap, None)
                .map(|value| value.value().render()),
            Some("1500ms".to_owned())
        );
    }

    #[test]
    fn supplying_both_a_canonical_name_and_an_alias_is_an_error() {
        let environment = from(&[
            ("SURVEILLANCE_RETENTION_MAX_AGE", "3d"),
            ("SURVEILLANCE_RETENTION_DAYS", "7"),
        ]);
        let error = layer_from(|name| environment.get(name).cloned())
            .expect_err("an ambiguous environment must be rejected");
        match error {
            EnvironmentError::Conflicting { id, .. } => {
                assert_eq!(id, SettingId::RetentionMaxAge);
            }
            other => panic!("expected a conflict, got {other}"),
        }
    }

    #[test]
    fn an_invalid_value_names_the_variable() {
        let environment = from(&[("SURVEILLANCE_RETENTION_BATCH_FILES", "lots")]);
        let error = layer_from(|name| environment.get(name).cloned()).expect_err("not a count");
        match error {
            EnvironmentError::Value { name, source } => {
                assert_eq!(name, "SURVEILLANCE_RETENTION_BATCH_FILES");
                assert_eq!(source.id, SettingId::RetentionBatchFiles);
            }
            other => panic!("expected a value error, got {other}"),
        }
    }

    #[test]
    fn secrets_are_accepted_from_the_environment() {
        let resolved = resolve(&[("SURVEILLANCE_RTSP_PASSWORD", "hunter2")]);
        let value = resolved
            .effective(SettingId::CameraRtspPassword, None)
            .expect("password present");
        assert!(value.value().is_secret());
        assert_eq!(value.value().render(), "***");
    }

    #[test]
    fn an_empty_environment_produces_an_empty_layer() {
        let layer = layer_from(|_| None).expect("an empty environment is valid");
        assert!(layer.is_empty());
    }

    #[test]
    fn every_alias_names_a_real_setting_and_is_unique() {
        let mut names = std::collections::BTreeSet::new();
        for alias in LEGACY_ALIASES {
            assert!(
                names.insert(alias.name),
                "duplicate legacy alias {}",
                alias.name
            );
            assert!(
                SettingId::from_key(alias.id.key()).is_some(),
                "alias {} names a setting that no longer exists",
                alias.name
            );
        }
    }

    #[test]
    fn no_alias_collides_with_a_canonical_name() {
        // A collision would make the conflict check fire on a single variable.
        let canonical: std::collections::BTreeSet<String> = SettingId::ALL
            .iter()
            .map(|id| canonical_name(*id))
            .collect();
        for alias in LEGACY_ALIASES {
            assert!(
                !canonical.contains(alias.name),
                "{} is both an alias and a canonical name",
                alias.name
            );
        }
    }

    #[test]
    fn implied_units_only_appear_on_duration_settings() {
        for alias in LEGACY_ALIASES {
            if alias.implied != ImpliedUnit::None {
                assert_eq!(
                    alias.id.kind(),
                    crate::classification::ValueKind::Duration,
                    "{} implies a unit for a non-duration setting",
                    alias.name
                );
            }
        }
    }

    #[test]
    fn every_pre_phase_seven_variable_is_covered() {
        // The variables `apps/server/src/runtime_config.rs` reads today. Losing
        // one silently would break a deployment on upgrade.
        for name in [
            "SURVEILLANCE_RTSP_URL",
            "SURVEILLANCE_RTSP_USERNAME",
            "SURVEILLANCE_RTSP_PASSWORD",
            "SURVEILLANCE_VIDEO_WIDTH",
            "SURVEILLANCE_VIDEO_HEIGHT",
            "SURVEILLANCE_RECORDING_ROOT",
            "SURVEILLANCE_HEALTH_BIND",
            "SURVEILLANCE_MAXIMUM_CAMERAS",
            "SURVEILLANCE_COMMAND_QUEUE_ITEMS",
            "SURVEILLANCE_COMMAND_TIMEOUT_SECONDS",
            "SURVEILLANCE_DATABASE_URL",
            "SURVEILLANCE_DATABASE_MAX_CONNECTIONS",
            "SURVEILLANCE_DATABASE_ACQUIRE_TIMEOUT_SECONDS",
            "SURVEILLANCE_RETENTION_DAYS",
            "SURVEILLANCE_RETENTION_PROTECTED_HOURS",
            "SURVEILLANCE_RETENTION_BATCH_FILES",
            "SURVEILLANCE_RETENTION_BATCH_BYTES",
            "SURVEILLANCE_RETENTION_INTERVAL_SECONDS",
            "SURVEILLANCE_FFMPEG",
            "SURVEILLANCE_LIVEKIT_URL",
            "SURVEILLANCE_LIVEKIT_API_KEY",
            "SURVEILLANCE_LIVEKIT_API_SECRET",
            "SURVEILLANCE_LIVEKIT_NAMESPACE",
            "SURVEILLANCE_LIVEKIT_CLI",
            "SURVEILLANCE_LIVEKIT_CLI_VERSION",
            "SURVEILLANCE_DETECTION_ENABLED",
            "SURVEILLANCE_DETECTION_QUEUE_ITEMS",
            "SURVEILLANCE_DETECTION_SAMPLE_FPS",
            "SURVEILLANCE_DETECTION_MAX_GAP_MS",
        ] {
            let aliased = LEGACY_ALIASES.iter().any(|alias| alias.name == name);
            let already_canonical = SettingId::ALL.iter().any(|id| canonical_name(*id) == name);
            assert!(
                aliased || already_canonical,
                "{name} is read by the server today but is neither canonical nor aliased"
            );
        }
    }
}
