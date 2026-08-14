//! TOML configuration files as a real precedence layer.
//!
//! A file's dotted table path *is* the setting key, so `[retention] max_age`
//! supplies `retention.max_age`. There is no translation table to drift out of
//! sync with the classification table, and the keys an operator writes in a file
//! are the keys they see in the API, in metrics, and in audit rows.
//!
//! Per-camera settings appear twice:
//!
//! ```toml
//! [camera.recording]           # template: applies to every camera
//! segment_duration = "30s"
//!
//! [cameras."lobby-01".rtsp]    # override: applies to one camera
//! endpoint = "rtsp://10.0.0.4:554/stream"
//! ```
//!
//! Unknown keys are rejected rather than ignored. A typo in a configuration
//! file that silently does nothing is the failure mode this avoids.

use std::{error::Error, fmt, path::Path};

use surveillance_common::types::{CameraId, IdentifierError};

use crate::{
    classification::{Origin, Scope, SettingId},
    resolution::{Layer, SettingKey},
    value::{SettingValue, ValueParseError},
};

/// The table under which per-camera overrides live.
const CAMERAS_TABLE: &str = "cameras";

/// Parses TOML text into a layer.
///
/// # Errors
/// Returns the first structural or value problem, naming the offending path.
pub fn parse(origin: Origin, text: &str) -> Result<Layer, FileError> {
    let document: toml::Value = toml::from_str(text).map_err(|error| FileError::Syntax {
        message: error.message().to_owned(),
    })?;
    let table = document.as_table().ok_or(FileError::NotATable)?;
    let mut layer = Layer::new(origin);

    for (name, value) in table {
        if name == CAMERAS_TABLE {
            parse_camera_overrides(&mut layer, value)?;
        } else {
            let mut path = vec![name.clone()];
            flatten(&mut layer, &mut path, value, None)?;
        }
    }
    Ok(layer)
}

/// Reads and parses a configuration file.
///
/// # Errors
/// Returns [`FileError::Unreadable`] when the file cannot be read, or a parse
/// error naming the offending key.
pub fn load(origin: Origin, path: &Path) -> Result<Layer, FileError> {
    let text = std::fs::read_to_string(path).map_err(|error| FileError::Unreadable {
        path: path.display().to_string(),
        kind: error.kind(),
    })?;
    parse(origin, &text)
}

fn parse_camera_overrides(layer: &mut Layer, value: &toml::Value) -> Result<(), FileError> {
    let table = value.as_table().ok_or_else(|| FileError::ExpectedTable {
        path: CAMERAS_TABLE.to_owned(),
    })?;
    for (name, body) in table {
        let camera = CameraId::new(name.clone()).map_err(|error| FileError::InvalidCameraId {
            name: name.clone(),
            source: error,
        })?;
        let body = body.as_table().ok_or_else(|| FileError::ExpectedTable {
            path: format!("{CAMERAS_TABLE}.{name}"),
        })?;
        for (section, contents) in body {
            // Overrides are written relative to the camera, so the `camera.`
            // prefix is supplied here rather than repeated in every file.
            let mut path = vec!["camera".to_owned(), section.clone()];
            flatten(layer, &mut path, contents, Some(&camera))?;
        }
    }
    Ok(())
}

fn flatten(
    layer: &mut Layer,
    path: &mut Vec<String>,
    value: &toml::Value,
    camera: Option<&CameraId>,
) -> Result<(), FileError> {
    if let Some(table) = value.as_table() {
        for (name, child) in table {
            path.push(name.clone());
            flatten(layer, path, child, camera)?;
            path.pop();
        }
        return Ok(());
    }

    let key = path.join(".");
    let id = SettingId::from_key(&key).ok_or_else(|| FileError::UnknownKey { key: key.clone() })?;
    if camera.is_some() && id.scope() == Scope::Global {
        return Err(FileError::GlobalUnderCamera { id });
    }
    let parsed = SettingValue::from_toml(id, value).map_err(FileError::Value)?;
    let setting_key = match camera {
        Some(camera) => SettingKey::scoped(id, camera.clone())
            .map_err(|_| FileError::GlobalUnderCamera { id })?,
        None => SettingKey::unscoped(id),
    };
    layer
        .insert(setting_key, parsed)
        .map_err(|error| FileError::Layer {
            key,
            message: error.to_string(),
        })
}

/// A configuration file that could not be turned into a layer.
#[derive(Debug)]
pub enum FileError {
    Unreadable {
        path: String,
        kind: std::io::ErrorKind,
    },
    Syntax {
        message: String,
    },
    NotATable,
    ExpectedTable {
        path: String,
    },
    /// A key that does not name a setting. Rejected rather than ignored so that
    /// a typo cannot silently do nothing.
    UnknownKey {
        key: String,
    },
    InvalidCameraId {
        name: String,
        source: IdentifierError,
    },
    /// A global setting written under a camera override.
    GlobalUnderCamera {
        id: SettingId,
    },
    Value(ValueParseError),
    Layer {
        key: String,
        message: String,
    },
}

impl fmt::Display for FileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, kind } => {
                write!(
                    formatter,
                    "configuration file {path} is unreadable: {kind:?}"
                )
            }
            Self::Syntax { message } => write!(formatter, "configuration syntax error: {message}"),
            Self::NotATable => formatter.write_str("configuration root is not a table"),
            Self::ExpectedTable { path } => write!(formatter, "{path} must be a table"),
            Self::UnknownKey { key } => write!(formatter, "unknown configuration key {key}"),
            Self::InvalidCameraId { name, source } => {
                write!(formatter, "invalid camera identifier {name}: {source:?}")
            }
            Self::GlobalUnderCamera { id } => {
                write!(
                    formatter,
                    "setting {id} is global and cannot be scoped to a camera"
                )
            }
            Self::Value(error) => write!(formatter, "{error}"),
            Self::Layer { key, message } => write!(formatter, "{key}: {message}"),
        }
    }
}

impl Error for FileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Value(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, FileError};
    use crate::{
        classification::{Origin, SettingId},
        resolution::ResolvedSettings,
    };
    use surveillance_common::types::CameraId;

    fn camera(id: &str) -> CameraId {
        CameraId::new(id.to_owned()).expect("valid test camera identifier")
    }

    #[test]
    fn a_dotted_table_path_is_the_setting_key() {
        let layer = parse(
            Origin::File,
            r#"
            [retention]
            max_age = "14d"
            interval = "90s"
            "#,
        )
        .expect("valid file");
        let resolved = ResolvedSettings::resolve(vec![layer]);
        assert_eq!(
            resolved
                .effective(SettingId::RetentionMaxAge, None)
                .map(|value| value.value().render()),
            Some("14d".to_owned())
        );
        assert_eq!(
            resolved
                .effective(SettingId::RetentionInterval, None)
                .map(|value| value.value().render()),
            Some("90s".to_owned())
        );
    }

    #[test]
    fn nested_tables_flatten_into_dotted_keys() {
        let layer = parse(
            Origin::File,
            r#"
            [camera.recording]
            segment_duration = "45s"
            "#,
        )
        .expect("valid file");
        let resolved = ResolvedSettings::resolve(vec![layer]);
        assert_eq!(
            resolved
                .effective(SettingId::CameraRecordingSegmentDuration, None)
                .map(|value| value.value().render()),
            Some("45s".to_owned())
        );
    }

    #[test]
    fn camera_overrides_beat_templates() {
        let layer = parse(
            Origin::File,
            r#"
            [camera.detection]
            sample_fps = 5

            [cameras."lobby-01".detection]
            sample_fps = 15
            "#,
        )
        .expect("valid file");
        let resolved = ResolvedSettings::resolve(vec![layer]);
        assert_eq!(
            resolved
                .effective(
                    SettingId::CameraDetectionSampleFps,
                    Some(&camera("lobby-01"))
                )
                .map(|value| value.value().render()),
            Some("15".to_owned())
        );
        assert_eq!(
            resolved
                .effective(
                    SettingId::CameraDetectionSampleFps,
                    Some(&camera("dock-02"))
                )
                .map(|value| value.value().render()),
            Some("5".to_owned())
        );
    }

    #[test]
    fn an_unknown_key_is_rejected_rather_than_ignored() {
        let error = parse(
            Origin::File,
            r#"
            [retention]
            max_agee = "14d"
            "#,
        )
        .expect_err("a typo must not be silently ignored");
        match error {
            FileError::UnknownKey { key } => assert_eq!(key, "retention.max_agee"),
            other => panic!("expected an unknown key, got {other}"),
        }
    }

    #[test]
    fn a_value_of_the_wrong_type_names_the_setting() {
        let error = parse(
            Origin::File,
            r#"
            [retention]
            batch_files = "many"
            "#,
        )
        .expect_err("a string is not a count");
        match error {
            FileError::Value(value) => assert_eq!(value.id, SettingId::RetentionBatchFiles),
            other => panic!("expected a value error, got {other}"),
        }
    }

    #[test]
    fn a_duration_without_a_unit_is_rejected() {
        let error = parse(
            Origin::File,
            r#"
            [retention]
            interval = "60"
            "#,
        )
        .expect_err("a bare number is an ambiguous duration");
        assert!(matches!(error, FileError::Value(_)));
    }

    #[test]
    fn a_global_setting_cannot_be_written_under_a_camera() {
        let error = parse(
            Origin::File,
            r#"
            [cameras."lobby-01".retention]
            max_age = "1d"
            "#,
        )
        .expect_err("retention is global");
        // The key resolves under the `camera.` prefix, so it is not a setting
        // at all; either rejection is correct, but it must be a rejection.
        assert!(matches!(
            error,
            FileError::UnknownKey { .. } | FileError::GlobalUnderCamera { .. }
        ));
    }

    #[test]
    fn an_invalid_camera_identifier_is_rejected() {
        let error = parse(
            Origin::File,
            r#"
            [cameras."".detection]
            sample_fps = 5
            "#,
        )
        .expect_err("an empty camera identifier is invalid");
        assert!(matches!(error, FileError::InvalidCameraId { .. }));
    }

    #[test]
    fn malformed_toml_is_reported_as_syntax() {
        let error = parse(Origin::File, "[retention").expect_err("unterminated table");
        assert!(matches!(error, FileError::Syntax { .. }));
    }

    #[test]
    fn a_secret_in_a_persisted_file_is_refused_by_the_layer() {
        // The persisted layer never carries credentials; a file claiming that
        // origin must not smuggle one in.
        let error = parse(
            Origin::Persisted,
            r#"
            [cameras."lobby-01".rtsp]
            password = "hunter2"
            "#,
        )
        .expect_err("secrets are not persisted");
        match error {
            FileError::Layer { key, .. } => assert_eq!(key, "camera.rtsp.password"),
            other => panic!("expected a layer refusal, got {other}"),
        }
    }

    #[test]
    fn an_empty_file_produces_an_empty_layer() {
        let layer = parse(Origin::File, "").expect("an empty file is valid");
        assert!(layer.is_empty());
    }

    /// `config/default.toml` is a real input, not documentation. This is the
    /// test that keeps it from drifting back into documentation.
    #[test]
    fn shipped_default_file_matches_the_compiled_defaults() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/default.toml");
        let text = std::fs::read_to_string(path).expect("config/default.toml is readable");
        let from_file = ResolvedSettings::resolve(vec![
            parse(Origin::File, &text).expect("config/default.toml parses")
        ]);
        let compiled = ResolvedSettings::resolve(vec![crate::defaults::layer()]);
        let salt = crate::secret::Secret::new("comparison".to_owned());

        for id in SettingId::ALL {
            let shipped = from_file.effective(*id, None);
            let expected = compiled.effective(*id, None);
            match (shipped, expected) {
                (Some(shipped), Some(expected)) => assert!(
                    shipped.value().same_as(expected.value(), &salt),
                    "{id}: file has {}, compiled default has {}",
                    shipped.value().render(),
                    expected.value().render()
                ),
                (None, None) => {}
                (Some(_), None) => {
                    panic!("{id} is in config/default.toml but has no compiled default")
                }
                (None, Some(_)) => {
                    panic!("{id} has a compiled default but is missing from config/default.toml")
                }
            }
        }
    }

    #[test]
    fn shipped_example_file_parses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/example.toml");
        let text = std::fs::read_to_string(path).expect("config/example.toml is readable");
        parse(Origin::File, &text).expect("config/example.toml parses");
    }
}
