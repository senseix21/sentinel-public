//! Principal and bearer-token persistence.
//!
//! Implements [`TokenStore`] against `PostgreSQL`. Follows the
//! `recording_segments` and `settings` repositories: a bounded pool, forward
//! migrations owned by the repository, and multi-statement writes inside an
//! explicit `pool.begin()`.
//!
//! This module holds no policy. It does not decide whether a token is valid —
//! it returns the row and lets [`surveillance_auth::Verifier`] decide, so that
//! expiry, revocation, and secret comparison live in exactly one place.
//!
//! A row whose stored `role` or `credential_hash` no longer parses is a
//! **lookup failure**, not a miss. Returning `Ok(None)` for a corrupt row would
//! report "no such token" for what is really a data-integrity fault, and the
//! operator would chase the wrong problem.

use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};

use surveillance_auth::{
    clock::Timestamp,
    principal::{CameraScope, Principal, PrincipalId, PrincipalKind, Role},
    store::{PrincipalStore, TokenStore},
    token::{IssuedToken, TokenId},
    verify::{CredentialHash, StoredToken},
};
use surveillance_common::types::CameraId;

use crate::{
    config::DatabaseConfig,
    recordings::{RepositoryError, MIGRATOR},
};

/// One principal, as an administrative listing shows it.
///
/// Carries no hash and no token identifier: listing identities must not become
/// a way to enumerate credentials.
#[derive(Clone, Debug)]
pub struct PrincipalSummary {
    pub principal_id: String,
    pub kind: String,
    pub display_name: String,
    pub role: String,
    pub scope_all: bool,
    pub cameras: Vec<String>,
    pub enabled: bool,
    pub live_tokens: i64,
}

/// One token's public half and lifecycle state.
#[derive(Clone, Debug)]
pub struct TokenSummary {
    pub token_id: String,
    pub label: Option<String>,
    pub expires_at_unix: i64,
    pub revoked_at_unix: Option<i64>,
}

/// Reads and writes identities and their bearer tokens.
pub struct PostgresAuthRepository {
    pool: PgPool,
}

impl PostgresAuthRepository {
    /// Creates a bounded pool without requiring `PostgreSQL` to be available.
    ///
    /// Lazy for the same reason the settings repository is: the server must
    /// come up whether or not the store is reachable. An unreachable identity
    /// store means every request is refused, which is the correct failure
    /// direction, and it is visible rather than silent.
    #[must_use]
    pub fn connect_lazy(config: &DatabaseConfig) -> Self {
        let pool = PgPoolOptions::new()
            .max_connections(config.maximum_connections())
            .acquire_timeout(config.acquire_timeout())
            .connect_lazy_with(config.connect_options());
        Self { pool }
    }

    #[must_use]
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Applies repository-owned forward migrations.
    ///
    /// # Errors
    /// Returns a migration failure without exposing the DSN.
    pub async fn migrate(&self) -> Result<(), RepositoryError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(RepositoryError::migration)
    }

    /// Creates a principal, replacing any existing row with the same id.
    ///
    /// The principal and its camera scope commit together: a principal that
    /// exists with a half-written scope would be a silent privilege change.
    ///
    /// # Errors
    /// Returns a classified `PostgreSQL` failure.
    pub async fn upsert_principal(&self, principal: &Principal) -> Result<(), RepositoryError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(RepositoryError::from_sqlx)?;

        sqlx::query(
            "INSERT INTO principals \
                 (principal_id, kind, display_name, role, scope_all, enabled) \
             VALUES ($1, $2::principal_kind, $3, $4::principal_role, $5, $6) \
             ON CONFLICT (principal_id) DO UPDATE SET \
                 kind = EXCLUDED.kind, \
                 display_name = EXCLUDED.display_name, \
                 role = EXCLUDED.role, \
                 scope_all = EXCLUDED.scope_all, \
                 enabled = EXCLUDED.enabled, \
                 updated_at = clock_timestamp()",
        )
        .bind(principal.id().as_str())
        .bind(principal.kind().as_str())
        .bind(principal.display_name())
        .bind(principal.role().as_str())
        .bind(principal.scope().is_all())
        .bind(principal.is_enabled())
        .execute(&mut *transaction)
        .await
        .map_err(RepositoryError::from_sqlx)?;

        // Replace the scope wholesale rather than diffing. A diff that drops a
        // revocation is a privilege leak; a full replace cannot.
        sqlx::query("DELETE FROM principal_camera_scope WHERE principal_id = $1")
            .bind(principal.id().as_str())
            .execute(&mut *transaction)
            .await
            .map_err(RepositoryError::from_sqlx)?;

        if let CameraScope::Only(cameras) = principal.scope() {
            for camera in cameras {
                sqlx::query(
                    "INSERT INTO principal_camera_scope (principal_id, camera_id) \
                     VALUES ($1, $2) ON CONFLICT DO NOTHING",
                )
                .bind(principal.id().as_str())
                .bind(camera.as_str())
                .execute(&mut *transaction)
                .await
                .map_err(RepositoryError::from_sqlx)?;
            }
        }

        transaction
            .commit()
            .await
            .map_err(RepositoryError::from_sqlx)
    }

    /// Stores a freshly issued token against a principal.
    ///
    /// Takes the [`CredentialHash`], never the secret. The plaintext exists
    /// only inside [`IssuedToken`] and is delivered to its owner once.
    ///
    /// # Errors
    /// Returns a classified `PostgreSQL` failure, including the schema's
    /// refusal of a non-Argon2id credential.
    pub async fn insert_token(
        &self,
        token: &IssuedToken,
        principal: &PrincipalId,
        hash: &CredentialHash,
        label: Option<&str>,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO auth_tokens \
                 (token_id, principal_id, credential_hash, label, expires_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(token.id().as_str())
        .bind(principal.as_str())
        .bind(hash.as_str())
        .bind(label)
        .bind(to_datetime(token.expires_at())?)
        .execute(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;
        Ok(())
    }

    /// Marks a token revoked. Idempotent: revoking an already-revoked token
    /// keeps the original instant, so the audit trail records when access was
    /// actually withdrawn rather than when someone last clicked the button.
    ///
    /// # Errors
    /// Returns a classified `PostgreSQL` failure.
    pub async fn revoke_token(&self, id: &TokenId, at: Timestamp) -> Result<bool, RepositoryError> {
        // `GREATEST(.., issued_at)` because `Timestamp` is whole-second while
        // `issued_at` defaults to sub-second `clock_timestamp()`. Revoking a
        // token in the same second it was issued would otherwise stamp a
        // `revoked_at` fractionally *before* `issued_at`, trip the
        // `auth_tokens_revocation_after_issue` constraint, and fail the
        // revocation — leaving a token the operator believes is dead. Found by
        // revoking a freshly minted token against a real database.
        let result = sqlx::query(
            "UPDATE auth_tokens SET revoked_at = GREATEST($2, issued_at) \
             WHERE token_id = $1 AND revoked_at IS NULL",
        )
        .bind(id.as_str())
        .bind(to_datetime(at)?)
        .execute(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;
        Ok(result.rows_affected() > 0)
    }

    /// Whether any enabled principal exists.
    ///
    /// The non-loopback bind unlock (ADR 009) requires this: binding a routable
    /// address with an empty identity table would authenticate nobody and
    /// refuse everything, which looks like an outage rather than a
    /// misconfiguration.
    ///
    /// # Errors
    /// Returns a classified `PostgreSQL` failure.
    pub async fn has_enabled_principal(&self) -> Result<bool, RepositoryError> {
        let row = sqlx::query("SELECT EXISTS (SELECT 1 FROM principals WHERE enabled) AS present")
            .fetch_one(&self.pool)
            .await
            .map_err(RepositoryError::from_sqlx)?;
        row.try_get::<bool, _>("present")
            .map_err(RepositoryError::from_sqlx)
    }

    /// Lists every principal, with a live token count.
    ///
    /// Returns no hashes and no token identifiers. Listing identities is an
    /// administrative read, and it must not become a way to enumerate
    /// credentials.
    ///
    /// # Errors
    /// Returns a classified `PostgreSQL` failure.
    pub async fn list_principals(&self) -> Result<Vec<PrincipalSummary>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT p.principal_id, p.kind::text AS kind, p.display_name, \
                    p.role::text AS role, p.scope_all, p.enabled, \
                    (SELECT count(*) FROM auth_tokens t \
                     WHERE t.principal_id = p.principal_id \
                       AND t.revoked_at IS NULL \
                       AND t.expires_at > now()) AS live_tokens \
             FROM principals p ORDER BY p.principal_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;

        let mut summaries = Vec::with_capacity(rows.len());
        for row in rows {
            let principal_id: String = row
                .try_get("principal_id")
                .map_err(RepositoryError::from_sqlx)?;
            let scope_all: bool = row
                .try_get("scope_all")
                .map_err(RepositoryError::from_sqlx)?;
            let live_tokens: i64 = row
                .try_get("live_tokens")
                .map_err(RepositoryError::from_sqlx)?;
            let identifier =
                PrincipalId::new(&principal_id).map_err(|_| RepositoryError::invariant())?;
            let cameras = if scope_all {
                Vec::new()
            } else {
                self.scope_camera_ids(&identifier).await?
            };
            summaries.push(PrincipalSummary {
                principal_id,
                kind: row.try_get("kind").map_err(RepositoryError::from_sqlx)?,
                display_name: row
                    .try_get("display_name")
                    .map_err(RepositoryError::from_sqlx)?,
                role: row.try_get("role").map_err(RepositoryError::from_sqlx)?,
                scope_all,
                cameras,
                enabled: row.try_get("enabled").map_err(RepositoryError::from_sqlx)?,
                live_tokens,
            });
        }
        Ok(summaries)
    }

    /// Enables or disables a principal.
    ///
    /// Separate from deleting it so an account can be suspended without
    /// orphaning the audit rows that reference it. Returns whether a row
    /// changed, so the caller can answer `404` for an unknown principal rather
    /// than reporting a success that did nothing.
    ///
    /// # Errors
    /// Returns a classified `PostgreSQL` failure.
    pub async fn set_principal_enabled(
        &self,
        principal: &PrincipalId,
        enabled: bool,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query(
            "UPDATE principals SET enabled = $2, updated_at = clock_timestamp() \
             WHERE principal_id = $1",
        )
        .bind(principal.as_str())
        .bind(enabled)
        .execute(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;
        Ok(result.rows_affected() > 0)
    }

    /// Whether a principal exists.
    ///
    /// # Errors
    /// Returns a classified `PostgreSQL` failure.
    pub async fn principal_exists(&self, principal: &PrincipalId) -> Result<bool, RepositoryError> {
        let row = sqlx::query(
            "SELECT EXISTS (SELECT 1 FROM principals WHERE principal_id = $1) AS present",
        )
        .bind(principal.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;
        row.try_get::<bool, _>("present")
            .map_err(RepositoryError::from_sqlx)
    }

    /// The token identifiers belonging to a principal, with their state.
    ///
    /// Identifiers only — the public half. Knowing one does not permit
    /// authenticating, and an operator needs it to revoke.
    ///
    /// # Errors
    /// Returns a classified `PostgreSQL` failure.
    pub async fn list_tokens(
        &self,
        principal: &PrincipalId,
    ) -> Result<Vec<TokenSummary>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT token_id, label, expires_at, revoked_at \
             FROM auth_tokens WHERE principal_id = $1 ORDER BY issued_at DESC",
        )
        .bind(principal.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;

        let mut summaries = Vec::with_capacity(rows.len());
        for row in rows {
            let expires_at: DateTime<Utc> = row
                .try_get("expires_at")
                .map_err(RepositoryError::from_sqlx)?;
            let revoked_at: Option<DateTime<Utc>> = row
                .try_get("revoked_at")
                .map_err(RepositoryError::from_sqlx)?;
            summaries.push(TokenSummary {
                token_id: row
                    .try_get("token_id")
                    .map_err(RepositoryError::from_sqlx)?,
                label: row.try_get("label").map_err(RepositoryError::from_sqlx)?,
                expires_at_unix: expires_at.timestamp(),
                revoked_at_unix: revoked_at.map(|instant| instant.timestamp()),
            });
        }
        Ok(summaries)
    }

    async fn scope_camera_ids(
        &self,
        principal: &PrincipalId,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT camera_id FROM principal_camera_scope \
             WHERE principal_id = $1 ORDER BY camera_id",
        )
        .bind(principal.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;
        rows.into_iter()
            .map(|row| {
                row.try_get::<String, _>("camera_id")
                    .map_err(RepositoryError::from_sqlx)
            })
            .collect()
    }

    async fn load(&self, id: &TokenId) -> Result<Option<StoredToken>, RepositoryError> {
        let Some(row) = sqlx::query(
            "SELECT t.token_id, t.credential_hash, t.expires_at, t.revoked_at, \
                    p.principal_id, p.kind::text AS kind, p.display_name, \
                    p.role::text AS role, p.scope_all, p.enabled \
             FROM auth_tokens t \
             JOIN principals p ON p.principal_id = t.principal_id \
             WHERE t.token_id = $1",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?
        else {
            return Ok(None);
        };

        let principal_id: String = row
            .try_get("principal_id")
            .map_err(RepositoryError::from_sqlx)?;
        let kind: String = row.try_get("kind").map_err(RepositoryError::from_sqlx)?;
        let display_name: String = row
            .try_get("display_name")
            .map_err(RepositoryError::from_sqlx)?;
        let role: String = row.try_get("role").map_err(RepositoryError::from_sqlx)?;
        let scope_all: bool = row
            .try_get("scope_all")
            .map_err(RepositoryError::from_sqlx)?;
        let enabled: bool = row.try_get("enabled").map_err(RepositoryError::from_sqlx)?;
        let credential_hash: String = row
            .try_get("credential_hash")
            .map_err(RepositoryError::from_sqlx)?;
        let expires_at: DateTime<Utc> = row
            .try_get("expires_at")
            .map_err(RepositoryError::from_sqlx)?;
        let revoked_at: Option<DateTime<Utc>> = row
            .try_get("revoked_at")
            .map_err(RepositoryError::from_sqlx)?;

        // Every conversion below is `invariant` rather than `None`: the schema
        // constrains all of these, so a failure here means the data violates a
        // constraint that is supposed to hold, and that is worth an operator's
        // attention rather than a silent authentication refusal.
        let principal_id =
            PrincipalId::new(&principal_id).map_err(|_| RepositoryError::invariant())?;
        let kind = match kind.as_str() {
            "human" => PrincipalKind::Human,
            "service" => PrincipalKind::Service,
            _ => return Err(RepositoryError::invariant()),
        };
        let role = Role::parse(&role).map_err(|_| RepositoryError::invariant())?;
        let hash =
            CredentialHash::parse(&credential_hash).map_err(|_| RepositoryError::invariant())?;

        let scope = if scope_all {
            CameraScope::All
        } else {
            self.load_scope(&principal_id).await?
        };

        let principal = Principal::new(principal_id, kind, &display_name, role, scope, enabled)
            .map_err(|_| RepositoryError::invariant())?;

        Ok(Some(StoredToken {
            id: id.clone(),
            principal,
            hash,
            expires_at: from_datetime(expires_at),
            revoked_at: revoked_at.map(from_datetime),
        }))
    }

    /// Loads one principal by identifier, with no credential involved.
    ///
    /// Backs the proxy-asserted authentication path (ADR 009 Decision 6). It
    /// deliberately does not join `auth_tokens`: a human authenticated by the
    /// proxy has no token here, and requiring one would make the browser path
    /// impossible for exactly the identities it exists to serve.
    ///
    /// The row is returned whether or not it is enabled, so the caller can tell
    /// "suspended" from "never existed" in the audit trail.
    async fn load_principal(&self, id: &PrincipalId) -> Result<Option<Principal>, RepositoryError> {
        let Some(row) = sqlx::query(
            "SELECT principal_id, kind::text AS kind, display_name, \
                    role::text AS role, scope_all, enabled \
             FROM principals WHERE principal_id = $1",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?
        else {
            return Ok(None);
        };

        let kind: String = row.try_get("kind").map_err(RepositoryError::from_sqlx)?;
        let display_name: String = row
            .try_get("display_name")
            .map_err(RepositoryError::from_sqlx)?;
        let role: String = row.try_get("role").map_err(RepositoryError::from_sqlx)?;
        let scope_all: bool = row
            .try_get("scope_all")
            .map_err(RepositoryError::from_sqlx)?;
        let enabled: bool = row.try_get("enabled").map_err(RepositoryError::from_sqlx)?;

        // As in `load`: `invariant` rather than `None`, because the schema
        // constrains every one of these and a violation is an operator's
        // problem, not a silent authentication refusal.
        let kind = match kind.as_str() {
            "human" => PrincipalKind::Human,
            "service" => PrincipalKind::Service,
            _ => return Err(RepositoryError::invariant()),
        };
        let role = Role::parse(&role).map_err(|_| RepositoryError::invariant())?;
        let scope = if scope_all {
            CameraScope::All
        } else {
            self.load_scope(id).await?
        };

        Principal::new(id.clone(), kind, &display_name, role, scope, enabled)
            .map(Some)
            .map_err(|_| RepositoryError::invariant())
    }

    async fn load_scope(&self, principal: &PrincipalId) -> Result<CameraScope, RepositoryError> {
        let rows = sqlx::query(
            "SELECT camera_id FROM principal_camera_scope \
             WHERE principal_id = $1 ORDER BY camera_id",
        )
        .bind(principal.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;

        let mut cameras = Vec::with_capacity(rows.len());
        for row in rows {
            let raw: String = row
                .try_get("camera_id")
                .map_err(RepositoryError::from_sqlx)?;
            cameras.push(CameraId::new(raw).map_err(|_| RepositoryError::invariant())?);
        }
        CameraScope::only(cameras).map_err(|_| RepositoryError::invariant())
    }
}

impl TokenStore for PostgresAuthRepository {
    type Error = RepositoryError;

    async fn find(&self, id: &TokenId) -> Result<Option<StoredToken>, Self::Error> {
        self.load(id).await
    }
}

impl PrincipalStore for PostgresAuthRepository {
    type Error = RepositoryError;

    async fn find_principal(&self, id: &PrincipalId) -> Result<Option<Principal>, Self::Error> {
        self.load_principal(id).await
    }
}

fn to_datetime(timestamp: Timestamp) -> Result<DateTime<Utc>, RepositoryError> {
    DateTime::from_timestamp(timestamp.as_unix_seconds(), 0).ok_or_else(RepositoryError::invariant)
}

fn from_datetime(value: DateTime<Utc>) -> Timestamp {
    Timestamp::from_unix_seconds(value.timestamp())
}

#[cfg(test)]
mod tests {
    use super::{from_datetime, to_datetime};
    use chrono::{DateTime, Utc};
    use surveillance_auth::clock::Timestamp;

    #[test]
    fn timestamps_round_trip_through_the_database_representation() {
        let original = Timestamp::from_unix_seconds(1_800_000_000);
        let stored = to_datetime(original).expect("representable");
        assert_eq!(from_datetime(stored), original);
    }

    #[test]
    fn a_timestamp_outside_the_representable_range_is_an_invariant_failure() {
        // Not a panic and not a silent clamp: an unrepresentable expiry means
        // something upstream is wrong.
        assert!(to_datetime(Timestamp::from_unix_seconds(i64::MAX)).is_err());
    }

    #[test]
    fn the_epoch_round_trips() {
        let epoch: DateTime<Utc> = DateTime::from_timestamp(0, 0).expect("epoch is representable");
        assert_eq!(from_datetime(epoch), Timestamp::from_unix_seconds(0));
    }
}
