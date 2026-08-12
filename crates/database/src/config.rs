//! Secret-safe bounded `PostgreSQL` configuration.

use std::{error::Error, fmt, num::NonZeroU32, str::FromStr, time::Duration};

use sqlx::postgres::PgConnectOptions;

pub struct DatabaseConfig {
    connect_options: PgConnectOptions,
    maximum_connections: NonZeroU32,
    acquire_timeout: Duration,
}

impl DatabaseConfig {
    /// Parses a `PostgreSQL` DSN without retaining it as displayable text.
    ///
    /// # Errors
    /// Returns [`DatabaseConfigError`] for malformed connection settings or a
    /// zero acquisition timeout.
    pub fn new(
        dsn: &str,
        maximum_connections: NonZeroU32,
        acquire_timeout: Duration,
    ) -> Result<Self, DatabaseConfigError> {
        if acquire_timeout.is_zero() {
            return Err(DatabaseConfigError::ZeroAcquireTimeout);
        }
        let connect_options = PgConnectOptions::from_str(dsn)
            .map_err(|_| DatabaseConfigError::InvalidConnectionString)?;
        Ok(Self {
            connect_options,
            maximum_connections,
            acquire_timeout,
        })
    }

    pub(crate) fn connect_options(&self) -> PgConnectOptions {
        self.connect_options.clone()
    }

    pub(crate) const fn maximum_connections(&self) -> u32 {
        self.maximum_connections.get()
    }

    pub(crate) const fn acquire_timeout(&self) -> Duration {
        self.acquire_timeout
    }
}

impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DatabaseConfig")
            .field("connection", &"[REDACTED]")
            .field("maximum_connections", &self.maximum_connections)
            .field("acquire_timeout", &self.acquire_timeout)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatabaseConfigError {
    InvalidConnectionString,
    ZeroAcquireTimeout,
}

impl fmt::Display for DatabaseConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid database configuration: {self:?}")
    }
}

impl Error for DatabaseConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_exposes_dsn_credentials() {
        let config = DatabaseConfig::new(
            "postgres://secret-user:secret-password@localhost/surveillance",
            NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
            Duration::from_secs(5),
        )
        .unwrap_or_else(|error| panic!("valid configuration: {error}"));
        let debug = format!("{config:?}");
        assert!(!debug.contains("secret-user"));
        assert!(!debug.contains("secret-password"));
        assert!(debug.contains("REDACTED"));
    }
}
