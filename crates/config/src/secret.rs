//! Secret-carrying wrapper and salted fingerprints.
//!
//! Introduced before any persisted or API path exists so that redaction is
//! structural. Retrofitting redaction after those paths are written means
//! auditing every one of them.

use std::fmt;

use serde::{Serialize, Serializer};
use sha2::{Digest, Sha256};

/// The single rendering used everywhere a secret would otherwise appear.
pub const REDACTED: &str = "***";

/// Wraps a value that must never be rendered, logged, or serialized.
///
/// [`Debug`], [`Display`](fmt::Display), and [`Serialize`] all render
/// [`REDACTED`]. The only way to obtain the value is [`Secret::expose`], whose
/// name makes every read site greppable.
///
/// [`PartialEq`] is deliberately not implemented. Comparing secrets by value
/// invites both timing-dependent comparisons and accidental logging of the
/// operands; compare [`Fingerprint`]s instead.
pub struct Secret<T>(T);

impl<T> Secret<T> {
    /// Wraps a value as secret.
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Borrows the protected value.
    ///
    /// Named `expose` so that `grep -rn "\.expose()"` enumerates every site at
    /// which a secret is read. That result is expected to stay small, and
    /// reviewing it is the intended workflow.
    pub const fn expose(&self) -> &T {
        &self.0
    }

    /// Consumes the wrapper and returns the protected value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: Clone> Clone for Secret<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Secret({REDACTED})")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}

impl<T> Serialize for Secret<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(REDACTED)
    }
}

impl<T> From<T> for Secret<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

/// A salted SHA-256 fingerprint of a secret value.
///
/// Audit rows record fingerprints rather than values. A fingerprint shows that
/// a value changed, and that it changed back to a previously used value, both
/// of which are audit-relevant, without revealing the value.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    /// Computes `SHA-256(salt || 0x00 || value)`.
    ///
    /// The separator byte removes the concatenation ambiguity in which a
    /// different `(salt, value)` split hashes to the same digest.
    ///
    /// The salt is per-deployment so fingerprints are neither comparable across
    /// units nor open to precomputation against a small candidate space.
    #[must_use]
    pub fn of(salt: &Secret<String>, value: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(salt.expose().as_bytes());
        hasher.update([0u8]);
        hasher.update(value);
        Self(hasher.finalize().into())
    }

    /// Renders the fingerprint as lowercase hexadecimal.
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut rendered = String::with_capacity(64);
        for byte in self.0 {
            // Two hex digits per byte; `from_digit` cannot fail for a radix-16
            // nibble, so the fallback is unreachable rather than a silent zero.
            for nibble in [byte >> 4, byte & 0x0f] {
                rendered.push(char::from_digit(u32::from(nibble), 16).unwrap_or('?'));
            }
        }
        rendered
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Fingerprint({})", self.to_hex())
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl Serialize for Fingerprint {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::{Fingerprint, Secret, REDACTED};

    const PLAINTEXT: &str = "correct-horse-battery-staple";

    fn salt(value: &str) -> Secret<String> {
        Secret::new(value.to_owned())
    }

    #[test]
    fn debug_never_contains_the_plaintext() {
        let secret = Secret::new(PLAINTEXT.to_owned());
        let rendered = format!("{secret:?}");
        assert!(!rendered.contains(PLAINTEXT), "debug leaked: {rendered}");
        assert!(rendered.contains(REDACTED));
    }

    #[test]
    fn display_never_contains_the_plaintext() {
        let secret = Secret::new(PLAINTEXT.to_owned());
        let rendered = format!("{secret}");
        assert!(!rendered.contains(PLAINTEXT), "display leaked: {rendered}");
        assert_eq!(rendered, REDACTED);
    }

    #[test]
    fn serialization_never_contains_the_plaintext() {
        let secret = Secret::new(PLAINTEXT.to_owned());
        let rendered = toml::to_string(&toml::value::Table::from_iter([(
            "value".to_owned(),
            toml::Value::try_from(&secret).unwrap_or(toml::Value::Boolean(false)),
        )]))
        .unwrap_or_default();
        assert!(
            !rendered.contains(PLAINTEXT),
            "serialization leaked: {rendered}"
        );
        assert!(rendered.contains(REDACTED));
    }

    #[test]
    fn nested_debug_of_a_struct_holding_a_secret_stays_redacted() {
        #[derive(Debug)]
        struct Credentials {
            username: String,
            password: Secret<String>,
        }
        let credentials = Credentials {
            username: "operator".to_owned(),
            password: Secret::new(PLAINTEXT.to_owned()),
        };
        let rendered = format!("{credentials:?}");
        assert!(rendered.contains("operator"));
        assert!(!rendered.contains(PLAINTEXT), "nested debug leaked");
        // The value is still reachable deliberately; only its rendering is not.
        assert_eq!(credentials.username, "operator");
        assert_eq!(credentials.password.expose(), PLAINTEXT);
    }

    #[test]
    fn matches_published_sha256_vectors() {
        // Empty salt and separator reduce the input to `0x00 || value`, so the
        // vectors are computed over that exact byte string rather than assumed.
        let empty = salt("");
        assert_eq!(
            Fingerprint::of(&empty, b"").to_hex(),
            // SHA-256 of the single byte 0x00.
            "6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d"
        );
        assert_eq!(
            Fingerprint::of(&empty, b"abc").to_hex(),
            // SHA-256 of 0x00 || "abc".
            "609f6e36d2405585188d5cfd761f407c7cc46a7d3f314c88270469dde315fcd1"
        );
    }

    #[test]
    fn hex_is_lowercase_and_fixed_width() {
        let rendered = Fingerprint::of(&salt("deployment"), b"value").to_hex();
        assert_eq!(rendered.len(), 64);
        assert!(rendered
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase()));
    }

    #[test]
    fn identical_values_fingerprint_differently_under_different_salts() {
        let first = Fingerprint::of(&salt("unit-a"), PLAINTEXT.as_bytes());
        let second = Fingerprint::of(&salt("unit-b"), PLAINTEXT.as_bytes());
        assert_ne!(first, second);
    }

    #[test]
    fn identical_values_fingerprint_identically_under_one_salt() {
        let deployment = salt("unit-a");
        assert_eq!(
            Fingerprint::of(&deployment, PLAINTEXT.as_bytes()),
            Fingerprint::of(&deployment, PLAINTEXT.as_bytes()),
        );
    }

    #[test]
    fn separator_removes_concatenation_ambiguity() {
        // Without the 0x00 separator these two would hash identically.
        let first = Fingerprint::of(&salt("ab"), b"c");
        let second = Fingerprint::of(&salt("a"), b"bc");
        assert_ne!(first, second);
    }

    #[test]
    fn fingerprint_debug_does_not_contain_the_plaintext() {
        let fingerprint = Fingerprint::of(&salt("unit"), PLAINTEXT.as_bytes());
        let rendered = format!("{fingerprint:?}");
        assert!(!rendered.contains(PLAINTEXT));
    }
}
