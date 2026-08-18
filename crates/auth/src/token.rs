//! Bearer token representation, generation, and parsing.
//!
//! A presented token is `srv_<id>_<secret>`:
//!
//! - `id` is 16 lowercase hex characters (64 bits). It is the *public* half. It
//!   is indexed, it may be logged, and it is what an operator names when
//!   revoking.
//! - `secret` is 64 lowercase hex characters (256 bits). It is never stored and
//!   never logged. The store holds an Argon2id hash of it.
//!
//! Splitting the two halves is what lets verification look up exactly one row
//! instead of hashing the presented value against every row in the table.

use std::{error::Error, fmt};

use surveillance_common::secret::Secret;

use crate::clock::Timestamp;

/// The literal that prefixes every token this system issues.
pub const TOKEN_PREFIX: &str = "srv_";

/// Bytes of entropy in the public identifier.
const ID_BYTES: usize = 8;
/// Bytes of entropy in the secret half.
const SECRET_BYTES: usize = 32;

/// Hex-character length of the public identifier.
pub const TOKEN_ID_LEN: usize = ID_BYTES * 2;
/// Hex-character length of the secret half.
pub const TOKEN_SECRET_LEN: usize = SECRET_BYTES * 2;

/// The public half of a bearer token.
///
/// Safe to log, index, and return in an API response. Knowing it does not
/// permit authenticating.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TokenId(String);

impl TokenId {
    /// Accepts exactly [`TOKEN_ID_LEN`] lowercase hex characters.
    ///
    /// # Errors
    /// Returns [`TokenParseError::MalformedId`] otherwise.
    pub fn new(value: &str) -> Result<Self, TokenParseError> {
        if value.len() != TOKEN_ID_LEN || !is_lowercase_hex(value) {
            return Err(TokenParseError::MalformedId);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TokenId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The private half of a bearer token.
///
/// Wrapped in [`Secret`] so `Debug`, `Display`, and `Serialize` all redact. The
/// only way to read it is `expose`, which keeps every read site greppable.
pub struct TokenSecret(Secret<String>);

impl TokenSecret {
    /// Accepts exactly [`TOKEN_SECRET_LEN`] lowercase hex characters.
    ///
    /// # Errors
    /// Returns [`TokenParseError::MalformedSecret`] otherwise.
    pub fn new(value: &str) -> Result<Self, TokenParseError> {
        if value.len() != TOKEN_SECRET_LEN || !is_lowercase_hex(value) {
            return Err(TokenParseError::MalformedSecret);
        }
        Ok(Self(Secret::new(value.to_owned())))
    }

    /// Borrows the secret material. Named to keep read sites greppable.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.expose()
    }
}

impl fmt::Debug for TokenSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "TokenSecret({:?})", self.0)
    }
}

/// A token as presented by a client, split into its two halves.
#[derive(Debug)]
pub struct PresentedToken {
    id: TokenId,
    secret: TokenSecret,
}

impl PresentedToken {
    /// Splits `srv_<id>_<secret>` into its halves.
    ///
    /// # Errors
    /// Returns [`TokenParseError`] naming which part was wrong. That detail is
    /// for the audit trail and metrics only; the HTTP layer collapses every
    /// variant to one response.
    pub fn parse(presented: &str) -> Result<Self, TokenParseError> {
        let body = presented
            .strip_prefix(TOKEN_PREFIX)
            .ok_or(TokenParseError::MissingPrefix)?;
        let (id, secret) = body
            .split_once('_')
            .ok_or(TokenParseError::MissingSeparator)?;
        Ok(Self {
            id: TokenId::new(id)?,
            secret: TokenSecret::new(secret)?,
        })
    }

    #[must_use]
    pub const fn id(&self) -> &TokenId {
        &self.id
    }

    #[must_use]
    pub const fn secret(&self) -> &TokenSecret {
        &self.secret
    }
}

/// A freshly minted token, returned once and never reconstructible.
///
/// The rendered form exists only here. Once this value is dropped the plaintext
/// is gone, which is the property that makes "the store never holds a usable
/// token" true rather than aspirational.
#[derive(Debug)]
pub struct IssuedToken {
    id: TokenId,
    secret: TokenSecret,
    expires_at: Timestamp,
}

impl IssuedToken {
    /// Generates a token from the operating system CSPRNG.
    ///
    /// # Errors
    /// Returns [`TokenGenerateError::Entropy`] if the CSPRNG is unavailable.
    /// This is not retried and not worked around: a token generated from a
    /// degraded source is worse than no token.
    pub fn generate(expires_at: Timestamp) -> Result<Self, TokenGenerateError> {
        let mut id_bytes = [0u8; ID_BYTES];
        let mut secret_bytes = [0u8; SECRET_BYTES];
        getrandom::fill(&mut id_bytes).map_err(|_| TokenGenerateError::Entropy)?;
        getrandom::fill(&mut secret_bytes).map_err(|_| TokenGenerateError::Entropy)?;
        Ok(Self {
            id: TokenId(to_hex(&id_bytes)),
            secret: TokenSecret(Secret::new(to_hex(&secret_bytes))),
            expires_at,
        })
    }

    #[must_use]
    pub const fn id(&self) -> &TokenId {
        &self.id
    }

    #[must_use]
    pub const fn secret(&self) -> &TokenSecret {
        &self.secret
    }

    #[must_use]
    pub const fn expires_at(&self) -> Timestamp {
        self.expires_at
    }

    /// Renders the token for one-time delivery to its owner.
    ///
    /// Returns a [`Secret`] so that an accidental log of the return value
    /// redacts. Reaching the plaintext still requires `expose`.
    #[must_use]
    pub fn render(&self) -> Secret<String> {
        Secret::new(format!(
            "{TOKEN_PREFIX}{}_{}",
            self.id.as_str(),
            self.secret.expose()
        ))
    }
}

/// Why a presented token could not be split into halves.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenParseError {
    MissingPrefix,
    MissingSeparator,
    MalformedId,
    MalformedSecret,
}

impl fmt::Display for TokenParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPrefix => write!(formatter, "token does not begin with {TOKEN_PREFIX}"),
            Self::MissingSeparator => formatter.write_str("token has no identifier separator"),
            Self::MalformedId => formatter.write_str("token identifier is malformed"),
            Self::MalformedSecret => formatter.write_str("token secret is malformed"),
        }
    }
}

impl Error for TokenParseError {}

/// Why a token could not be generated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenGenerateError {
    Entropy,
}

impl fmt::Display for TokenGenerateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the operating system entropy source is unavailable")
    }
}

impl Error for TokenGenerateError {}

fn is_lowercase_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut rendered = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Both indices are masked into 0..16, so indexing cannot be out of
        // bounds and the conversion from an ASCII byte cannot fail.
        rendered.push(char::from(DIGITS[usize::from(byte >> 4)]));
        rendered.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::{
        IssuedToken, PresentedToken, TokenId, TokenParseError, TokenSecret, TOKEN_ID_LEN,
        TOKEN_PREFIX, TOKEN_SECRET_LEN,
    };
    use crate::clock::Timestamp;

    fn expiry() -> Timestamp {
        Timestamp::from_unix_seconds(1_800_000_000)
    }

    fn issued() -> IssuedToken {
        IssuedToken::generate(expiry()).expect("the test host has a working CSPRNG")
    }

    #[test]
    fn a_generated_token_has_the_documented_shape() {
        let token = issued();
        assert_eq!(token.id().as_str().len(), TOKEN_ID_LEN);
        assert_eq!(token.secret().expose().len(), TOKEN_SECRET_LEN);
        let rendered = token.render();
        assert!(rendered.expose().starts_with(TOKEN_PREFIX));
        assert_eq!(
            rendered.expose().len(),
            TOKEN_PREFIX.len() + TOKEN_ID_LEN + 1 + TOKEN_SECRET_LEN
        );
    }

    #[test]
    fn a_generated_token_round_trips_through_parsing() {
        let token = issued();
        let rendered = token.render();
        let presented = PresentedToken::parse(rendered.expose()).expect("round trip parses");
        assert_eq!(presented.id(), token.id());
        assert_eq!(presented.secret().expose(), token.secret().expose());
    }

    #[test]
    fn two_generated_tokens_differ_in_both_halves() {
        let first = issued();
        let second = issued();
        assert_ne!(first.id(), second.id());
        assert_ne!(first.secret().expose(), second.secret().expose());
    }

    #[test]
    fn the_secret_never_appears_in_debug_output() {
        let token = issued();
        let plaintext = token.secret().expose().to_owned();

        for rendered in [
            format!("{token:?}"),
            format!("{:?}", token.secret()),
            format!("{:?}", token.render()),
            format!("{}", token.render()),
        ] {
            assert!(
                !rendered.contains(&plaintext),
                "secret leaked into: {rendered}"
            );
        }
    }

    #[test]
    fn the_public_identifier_is_deliberately_not_redacted() {
        // The id is what an operator names when revoking; hiding it would make
        // revocation impossible from a log.
        let token = issued();
        assert!(format!("{token:?}").contains(token.id().as_str()));
    }

    #[test]
    fn parsing_names_which_part_was_wrong() {
        let good_id = "0123456789abcdef";
        let good_secret = "0".repeat(TOKEN_SECRET_LEN);

        assert_eq!(
            PresentedToken::parse("nope_0123_4567").err(),
            Some(TokenParseError::MissingPrefix)
        );
        assert_eq!(
            PresentedToken::parse("srv_nosep").err(),
            Some(TokenParseError::MissingSeparator)
        );
        assert_eq!(
            PresentedToken::parse(&format!("srv_short_{good_secret}")).err(),
            Some(TokenParseError::MalformedId)
        );
        assert_eq!(
            PresentedToken::parse(&format!("srv_{good_id}_short")).err(),
            Some(TokenParseError::MalformedSecret)
        );
    }

    #[test]
    fn uppercase_hex_is_refused_so_one_token_has_one_representation() {
        // Two spellings of one token would mean two index lookups and an
        // opportunity for a case-folding mismatch between store and verifier.
        assert_eq!(
            TokenId::new("0123456789ABCDEF").err(),
            Some(TokenParseError::MalformedId)
        );
        assert_eq!(
            TokenSecret::new(&"A".repeat(TOKEN_SECRET_LEN)).err(),
            Some(TokenParseError::MalformedSecret)
        );
    }

    #[test]
    fn non_hex_characters_are_refused() {
        assert_eq!(
            TokenId::new("0123456789abcdeg").err(),
            Some(TokenParseError::MalformedId)
        );
        // A same-length string of non-hex bytes must not slip through on length
        // alone.
        assert_eq!(
            TokenId::new("................").err(),
            Some(TokenParseError::MalformedId)
        );
    }

    #[test]
    fn an_empty_presentation_is_refused() {
        assert_eq!(
            PresentedToken::parse("").err(),
            Some(TokenParseError::MissingPrefix)
        );
        assert_eq!(
            PresentedToken::parse(TOKEN_PREFIX).err(),
            Some(TokenParseError::MissingSeparator)
        );
    }

    #[test]
    fn hex_rendering_covers_every_nibble() {
        // Guards the hand-rolled hex writer against a transposed-nibble bug,
        // which would silently halve the effective entropy space of an id.
        assert_eq!(super::to_hex(&[0x00, 0x0f, 0xf0, 0xff]), "000ff0ff");
        assert_eq!(super::to_hex(&[0x12, 0x34, 0xab, 0xcd]), "1234abcd");
    }
}
