//! Stable cross-domain identifiers only.

use std::{error::Error, fmt};

macro_rules! domain_id {
    ($name:ident, $label:literal) => {
        #[doc = concat!("A validated ", $label, " identifier.")]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Validates a ", $label, " identifier.")]
            ///
            /// # Errors
            ///
            /// Returns [`IdentifierError`] unless the value is 1–64 lowercase
            /// ASCII letters, digits, or hyphens, with no leading/trailing
            /// hyphen or consecutive hyphens.
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                validate_identifier(&value)?;
                Ok(Self(value))
            }

            /// Returns the validated identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

domain_id!(CameraId, "camera");
domain_id!(RecordingId, "recording");

/// Invalid stable domain identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentifierError {
    Empty,
    TooLong,
    InvalidCharacter,
    InvalidHyphenPlacement,
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("identifier must not be empty"),
            Self::TooLong => formatter.write_str("identifier must not exceed 64 bytes"),
            Self::InvalidCharacter => formatter.write_str(
                "identifier must contain only lowercase ASCII letters, digits, and hyphens",
            ),
            Self::InvalidHyphenPlacement => formatter
                .write_str("identifier must not start, end, or contain consecutive hyphens"),
        }
    }
}

impl Error for IdentifierError {}

fn validate_identifier(value: &str) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::Empty);
    }
    if value.len() > 64 {
        return Err(IdentifierError::TooLong);
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(IdentifierError::InvalidCharacter);
    }
    if value.starts_with('-') || value.ends_with('-') || value.contains("--") {
        return Err(IdentifierError::InvalidHyphenPlacement);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CameraId, IdentifierError, RecordingId};

    #[test]
    fn accepts_safe_stable_identifiers() {
        let camera = CameraId::new("camera-01")
            .unwrap_or_else(|error| panic!("valid camera identifier: {error}"));
        let recording = RecordingId::new("01k2a7f4m2t8")
            .unwrap_or_else(|error| panic!("valid recording identifier: {error}"));
        assert_eq!(camera.as_str(), "camera-01");
        assert_eq!(recording.as_str(), "01k2a7f4m2t8");
    }

    #[test]
    fn rejects_path_and_ambiguous_hyphen_forms() {
        assert_eq!(
            CameraId::new("../camera"),
            Err(IdentifierError::InvalidCharacter)
        );
        assert_eq!(
            CameraId::new("camera--01"),
            Err(IdentifierError::InvalidHyphenPlacement)
        );
        assert_eq!(
            CameraId::new("Camera-01"),
            Err(IdentifierError::InvalidCharacter)
        );
    }
}
