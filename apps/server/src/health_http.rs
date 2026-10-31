//! Loopback-only bounded health, metrics, and Phase 3 camera API adapter.

use std::{fmt::Write as _, io, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use surveillance_camera::connection::ConnectionState;
use surveillance_common::types::CameraId;
use surveillance_config::{
    cache::CacheState,
    change::{ChangeReport, ProposedChange, UpdateRejection},
    classification::{Secrecy, SettingId},
    resolution::SettingKey,
    settings::{CameraCredentialsSettings, OneCameraConfig, OneCameraSettings},
    value::SettingValue,
};
use surveillance_database::PostgresRecordingRepository;
use surveillance_detector::inference::InferenceDescriptor;
use surveillance_livekit::{config::LiveKitSettings, token::DevelopmentTokenService};
use surveillance_storage::recordings::{
    PlaybackPlan, RecordingCursor, RecordingResultLimit, RecordingSearch, RecordingTimeRange,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};

use crate::camera_supervisor::{
    CameraHealthRegistry, CameraStatusSnapshot, CameraSupervisorHandle, PipelineState,
    SupervisorRequestError,
};
use surveillance_auth::{
    clock::{Clock, SystemClock},
    principal::{CameraScope, Principal, PrincipalId, PrincipalKind, Role},
    token::{IssuedToken, TokenId},
    verify::{HashParams, Verifier},
};
use surveillance_database::{
    access_audit::ANONYMOUS_ACTOR, AccessAuditRecord, AuditCategory, AuditOutcome, ExportExtent,
    PostgresAccessAuditRepository, PostgresAuthRepository, RepositoryErrorCategory,
};

use crate::{
    authenticate::{
        AuthMetrics, PresentedCredentials, RequestAuthenticator, SecurityContext, SecurityOwner,
    },
    authorize::{authorize, Action, Authorized, Denied, MutatingSurface, RequestContext},
    metadata_writer::MetadataWriterHandle,
    phase4_metrics::Phase4Metrics,
    playback::{ClipDuration, PlaybackAssembler, PlaybackStream, RecordingVideoExtent},
    settings_service::{ConfigurationMetricsView, SettingsService},
    trust_boundary::{
        TrustBoundary, AUTHENTICATED_PRINCIPAL_HEADER, FORWARDED_FOR_HEADER, PROOF_HEADER,
    },
};

/// Renders the P8.7 authentication and authorization series.
///
/// Cardinality is bounded by construction: the failure label comes from a
/// seven-variant enum and the denial label from a five-variant one. A principal
/// identifier or token identifier is deliberately never a label — it would be
/// unbounded, and it would put an identity into every scrape and dashboard.
fn render_auth_metrics(output: &mut String, auth: Option<&AuthMetrics>) {
    use std::fmt::Write as _;

    // Rendered as zeros when no identity store is configured, never omitted.
    // An absent series makes `rate()` empty rather than 0, so an alert built on
    // it silently never fires — worse than a series that reads zero.
    //
    // Found by running the server: with no database, `/metrics` exported no
    // security series at all, including `surveillance_authz_decision_total` and
    // `surveillance_proxy_proof_total`, which count global facts that have
    // nothing to do with whether an identity store exists.
    let succeeded = auth.map_or(0, AuthMetrics::succeeded);
    let asserted = auth.map_or(0, AuthMetrics::asserted);
    let unavailable = auth.map_or(0, AuthMetrics::unavailable);
    let failures = auth.map_or_else(AuthMetrics::zeroed_failures, AuthMetrics::failures);

    output.push_str(
        "# HELP surveillance_auth_success_total Credentials accepted.\n\
         # TYPE surveillance_auth_success_total counter\n",
    );
    let _ = writeln!(output, "surveillance_auth_success_total {succeeded}");

    // A strict subset of the successes above: those that arrived as a proxy
    // assertion rather than a bearer token this server verified itself. The
    // difference between the two series is what tells an operator whether the
    // browser path is in use, and whether it is in use when it should not be.
    output.push_str(
        "# HELP surveillance_auth_proxy_asserted_total Successes asserted by the trusted proxy.\n\
         # TYPE surveillance_auth_proxy_asserted_total counter\n",
    );
    let _ = writeln!(output, "surveillance_auth_proxy_asserted_total {asserted}");

    output.push_str(
        "# HELP surveillance_auth_failure_total Credentials refused, by reason.\n\
         # TYPE surveillance_auth_failure_total counter\n",
    );
    for (reason, count) in failures {
        let _ = writeln!(
            output,
            "surveillance_auth_failure_total{{reason=\"{reason}\"}} {count}"
        );
    }

    output.push_str(
        "# HELP surveillance_auth_unavailable_total Identity store could not be consulted.\n\
         # TYPE surveillance_auth_unavailable_total counter\n",
    );
    let _ = writeln!(output, "surveillance_auth_unavailable_total {unavailable}");

    output.push_str(
        "# HELP surveillance_authz_decision_total Authorization decisions, by outcome.\n\
         # TYPE surveillance_authz_decision_total counter\n",
    );
    for (outcome, count) in crate::authorize::decision_counts() {
        let _ = writeln!(
            output,
            "surveillance_authz_decision_total{{outcome=\"{outcome}\"}} {count}"
        );
    }

    output.push_str(
        "# HELP surveillance_proxy_proof_total Proof-of-origin outcomes, by result.\n\
         # TYPE surveillance_proxy_proof_total counter\n",
    );
    for (outcome, count) in crate::trust_boundary::proof_counts() {
        let _ = writeln!(
            output,
            "surveillance_proxy_proof_total{{outcome=\"{outcome}\"}} {count}"
        );
    }
}

/// The request's identity plus where its audit rows go.
///
/// Bundled because they always travel together: an authorization decision that
/// is not recorded is exactly what P8.6 exists to prevent.
pub(crate) struct Guard<'a> {
    context: RequestContext,
    audit: Option<&'a PostgresAccessAuditRepository>,
}

impl Guard<'_> {
    pub(crate) const fn context(&self) -> &RequestContext {
        &self.context
    }
}

/// Authorizes and records the decision.
///
/// The single place an audit row is written for a protected request, for the
/// same reason `authorize` is the single place a decision is made: an audit
/// call per handler is an audit call that gets forgotten in a handler. This was
/// forgotten wholesale — the audit schema and repository shipped with nothing
/// writing to them, which running the server revealed as an empty table.
///
/// Returns the rendered refusal on `Err`, because every call site immediately
/// renders one.
///
/// The read/write asymmetry is deliberate (see `crates/database/src/access_audit.rs`):
/// a failed audit write **fails a mutating** operation, so "changed but
/// unlogged" stays unrepresentable; a failed audit write on a **read** is
/// counted and the read proceeds, because refusing reads when the audit store
/// is down turns an audit outage into an observability outage during exactly
/// the incident that needs both.
async fn authorize_audited(guard: &Guard<'_>, action: Action) -> Result<Authorized, String> {
    let mutating = action.is_mutating();
    let category = audit_category(&action);
    // Captured before `authorize` consumes the action. A denial row that says
    // only "denied" tells an incident review that someone was refused but not
    // what they reached for, which is the part that matters.
    let label = action.as_str();
    let camera = camera_of(&action).map(CameraId::as_str).map(str::to_owned);
    let outcome = authorize(&guard.context, action);

    let (result, reason) = match &outcome {
        Ok(_) => (AuditOutcome::Granted, None),
        Err(denied) => (AuditOutcome::Denied, Some(denial_reason(*denied))),
    };

    if let Some(repository) = guard.audit {
        let actor = match &outcome {
            Ok(authorized) => authorized.actor().as_str().to_owned(),
            Err(_) => guard
                .context
                .authentication()
                .principal()
                .map_or_else(|| ANONYMOUS_ACTOR.to_owned(), Principal::audit_identity),
        };
        let mut record = AccessAuditRecord::new(category, label, &actor, result)
            .with_client(guard.context.client());
        if let Some(camera) = camera.as_deref() {
            record = record.with_camera(camera);
        }
        if let Some(reason) = reason {
            record = record.with_reason(reason);
        }
        if let Err(error) = repository.record(&record).await {
            if mutating {
                tracing::error!(%error, "audit write failed; the mutating operation was refused");
                return Err(json_error("503 Service Unavailable", "audit_unavailable"));
            }
            tracing::warn!(%error, "audit write failed for a read; the read proceeded");
        }
    }

    outcome.map_err(denial_response)
}

/// Routes `/api/identities`.
///
/// This surface exists because P8.6 requires auditing "authorization changes",
/// and a change nothing can observe cannot be audited. Before this, the only
/// way to create a principal or a token was raw SQL, which leaves no trail at
/// all — the audit requirement was vacuous rather than met.
///
/// Every mutating route here is `ManageIdentities` (Administrator, and gated by
/// `MutatingSurface` like any other reconfiguration). The listing is
/// `ViewIdentities`, which is Administrator but not surface-gated: reading who
/// has access must not require opening the mutating surface.
async fn route_identities(
    method: &str,
    path: &str,
    body: &[u8],
    auth: Option<&PostgresAuthRepository>,
    guard: &Guard<'_>,
) -> String {
    // Deliberately *after* the chokepoint in every arm below, not here. An
    // early `503` would answer an unauthenticated caller before authorization
    // ran, telling them whether an identity store is configured and skipping
    // the chokepoint entirely. That is the same shape as the playback defect:
    // a return path above the check.
    let segments: Vec<_> = path.trim_matches('/').split('/').collect();

    match (method, segments.as_slice()) {
        ("GET", ["api", "identities"]) => {
            match authorize_audited(guard, Action::ViewIdentities).await {
                Ok(_authorized) => match auth {
                    Some(repository) => list_identities(repository).await,
                    None => json_error("503 Service Unavailable", "identity_store_unavailable"),
                },
                Err(response) => response,
            }
        }
        ("POST", ["api", "identities"]) => {
            match authorize_audited(guard, Action::ManageIdentities).await {
                Ok(authorized) => match auth {
                    Some(repository) => create_identity(body, repository, authorized).await,
                    None => json_error("503 Service Unavailable", "identity_store_unavailable"),
                },
                Err(response) => response,
            }
        }
        ("GET", ["api", "identities", id, "tokens"]) => {
            match authorize_audited(guard, Action::ViewIdentities).await {
                Ok(_authorized) => match auth {
                    Some(repository) => list_identity_tokens(id, repository).await,
                    None => json_error("503 Service Unavailable", "identity_store_unavailable"),
                },
                Err(response) => response,
            }
        }
        ("POST", ["api", "identities", id, "tokens"]) => {
            match authorize_audited(guard, Action::ManageIdentities).await {
                Ok(authorized) => match auth {
                    Some(repository) => mint_identity_token(id, body, repository, authorized).await,
                    None => json_error("503 Service Unavailable", "identity_store_unavailable"),
                },
                Err(response) => response,
            }
        }
        ("DELETE", ["api", "identities", id, "tokens", token]) => {
            match authorize_audited(guard, Action::ManageIdentities).await {
                Ok(authorized) => match auth {
                    Some(repository) => {
                        revoke_identity_token(id, token, repository, authorized).await
                    }
                    None => json_error("503 Service Unavailable", "identity_store_unavailable"),
                },
                Err(response) => response,
            }
        }
        ("POST", ["api", "identities", id, "disable"]) => {
            match authorize_audited(guard, Action::ManageIdentities).await {
                Ok(authorized) => match auth {
                    Some(repository) => {
                        set_identity_enabled(id, false, repository, authorized).await
                    }
                    None => json_error("503 Service Unavailable", "identity_store_unavailable"),
                },
                Err(response) => response,
            }
        }
        ("POST", ["api", "identities", id, "enable"]) => {
            match authorize_audited(guard, Action::ManageIdentities).await {
                Ok(authorized) => match auth {
                    Some(repository) => {
                        set_identity_enabled(id, true, repository, authorized).await
                    }
                    None => json_error("503 Service Unavailable", "identity_store_unavailable"),
                },
                Err(response) => response,
            }
        }
        ("GET" | "POST" | "DELETE", _) => json_error("404 Not Found", "not_found"),
        _ => json_error("405 Method Not Allowed", "method_not_allowed"),
    }
}

async fn list_identities(repository: &PostgresAuthRepository) -> String {
    match repository.list_principals().await {
        Ok(principals) => {
            let rendered: Vec<serde_json::Value> = principals
                .iter()
                .map(|principal| {
                    serde_json::json!({
                        "principal_id": principal.principal_id,
                        "kind": principal.kind,
                        "display_name": principal.display_name,
                        "role": principal.role,
                        "scope": if principal.scope_all { serde_json::Value::String("all".to_owned()) }
                                 else { serde_json::json!(principal.cameras) },
                        "enabled": principal.enabled,
                        "live_tokens": principal.live_tokens,
                    })
                })
                .collect();
            json_response("200 OK", &serde_json::Value::Array(rendered))
        }
        Err(_) => json_error("503 Service Unavailable", "identity_store_unavailable"),
    }
}

async fn list_identity_tokens(id: &str, repository: &PostgresAuthRepository) -> String {
    let Ok(principal) = PrincipalId::new(id) else {
        return json_error("400 Bad Request", "invalid_principal_id");
    };
    match repository.list_tokens(&principal).await {
        Ok(tokens) => {
            let rendered: Vec<serde_json::Value> = tokens
                .iter()
                .map(|token| {
                    serde_json::json!({
                        // The public half only. Never the secret, which the
                        // store does not hold in a usable form anyway.
                        "token_id": token.token_id,
                        "label": token.label,
                        "expires_at_unix": token.expires_at_unix,
                        "revoked_at_unix": token.revoked_at_unix,
                    })
                })
                .collect();
            json_response("200 OK", &serde_json::Value::Array(rendered))
        }
        Err(_) => json_error("503 Service Unavailable", "identity_store_unavailable"),
    }
}

/// Creates or replaces a principal.
///
/// Takes [`Authorized`] by value, so it cannot run without the chokepoint.
async fn create_identity(
    body: &[u8],
    repository: &PostgresAuthRepository,
    _authorized: Authorized,
) -> String {
    let Ok(request) = serde_json::from_slice::<serde_json::Value>(body) else {
        return json_error("400 Bad Request", "invalid_body");
    };
    let (Some(id), Some(role)) = (
        request
            .get("principal_id")
            .and_then(serde_json::Value::as_str),
        request.get("role").and_then(serde_json::Value::as_str),
    ) else {
        return json_error("400 Bad Request", "missing_field");
    };
    let Ok(principal_id) = PrincipalId::new(id) else {
        return json_error("400 Bad Request", "invalid_principal_id");
    };
    let Ok(role) = Role::parse(role) else {
        return json_error("400 Bad Request", "invalid_role");
    };
    let kind = match request.get("kind").and_then(serde_json::Value::as_str) {
        None | Some("human") => PrincipalKind::Human,
        Some("service") => PrincipalKind::Service,
        Some(_) => return json_error("400 Bad Request", "invalid_kind"),
    };
    let display_name = request
        .get("display_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(id);

    // Absent `cameras` means every camera; an explicit list — including an
    // empty one — means exactly that list. An empty scope is legal and is how a
    // principal is held without camera access.
    let scope = match request.get("cameras") {
        None => CameraScope::All,
        Some(serde_json::Value::Array(entries)) => {
            let mut cameras = Vec::with_capacity(entries.len());
            for entry in entries {
                let Some(name) = entry.as_str() else {
                    return json_error("400 Bad Request", "invalid_camera_id");
                };
                let Ok(camera) = CameraId::new(name) else {
                    return json_error("400 Bad Request", "invalid_camera_id");
                };
                cameras.push(camera);
            }
            match CameraScope::only(cameras) {
                Ok(scope) => scope,
                Err(_) => return json_error("400 Bad Request", "scope_too_large"),
            }
        }
        Some(_) => return json_error("400 Bad Request", "invalid_scope"),
    };

    let enabled = request
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);

    let Ok(principal) = Principal::new(principal_id, kind, display_name, role, scope, enabled)
    else {
        return json_error("400 Bad Request", "invalid_display_name");
    };

    match repository.upsert_principal(&principal).await {
        Ok(()) => json_response(
            "201 Created",
            &serde_json::json!({ "principal_id": principal.id().as_str() }),
        ),
        // A scope naming a camera that does not exist violates the foreign key.
        // That is the caller's mistake, not an outage, and reporting it as
        // "store unavailable" sends an operator to the database runbook for a
        // typo. Found live: creating a principal scoped to an unknown camera
        // answered 503.
        Err(error) if error.category() == RepositoryErrorCategory::Constraint => {
            json_error("400 Bad Request", "unknown_camera_in_scope")
        }
        Err(_) => json_error("503 Service Unavailable", "identity_store_unavailable"),
    }
}

/// Mints one token and returns it exactly once.
async fn mint_identity_token(
    id: &str,
    body: &[u8],
    repository: &PostgresAuthRepository,
    _authorized: Authorized,
) -> String {
    let Ok(principal) = PrincipalId::new(id) else {
        return json_error("400 Bad Request", "invalid_principal_id");
    };
    match repository.principal_exists(&principal).await {
        Ok(true) => {}
        Ok(false) => return json_error("404 Not Found", "principal_not_found"),
        Err(_) => return json_error("503 Service Unavailable", "identity_store_unavailable"),
    }

    let request: serde_json::Value = serde_json::from_slice(body).unwrap_or(serde_json::json!({}));
    let lifetime_days = request
        .get("lifetime_days")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(90);
    if lifetime_days <= 0 || lifetime_days > 3_650 {
        return json_error("400 Bad Request", "invalid_lifetime");
    }
    let label = request.get("label").and_then(serde_json::Value::as_str);

    let expires_at = SystemClock
        .now()
        .saturating_add_seconds(lifetime_days.saturating_mul(24 * 60 * 60));
    let Ok(token) = IssuedToken::generate(expires_at) else {
        return json_error("503 Service Unavailable", "entropy_unavailable");
    };
    let Ok(verifier) = Verifier::new(HashParams::default()) else {
        return json_error("500 Internal Server Error", "verifier_unavailable");
    };
    let Ok(hash) = verifier.hash(token.secret()) else {
        return json_error("500 Internal Server Error", "hashing_failed");
    };

    match repository
        .insert_token(&token, &principal, &hash, label)
        .await
    {
        // The plaintext appears here and nowhere else, ever. The store holds
        // only the Argon2id hash, so this response is not recoverable later.
        Ok(()) => json_response(
            "201 Created",
            &serde_json::json!({
                "token_id": token.id().as_str(),
                "token": token.render().expose(),
                "expires_at_unix": expires_at.as_unix_seconds(),
            }),
        ),
        Err(_) => json_error("503 Service Unavailable", "identity_store_unavailable"),
    }
}

async fn revoke_identity_token(
    id: &str,
    token: &str,
    repository: &PostgresAuthRepository,
    _authorized: Authorized,
) -> String {
    let Ok(_principal) = PrincipalId::new(id) else {
        return json_error("400 Bad Request", "invalid_principal_id");
    };
    let Ok(token_id) = TokenId::new(token) else {
        return json_error("400 Bad Request", "invalid_token_id");
    };
    match repository.revoke_token(&token_id, SystemClock.now()).await {
        Ok(true) => response("204 No Content", "application/json", ""),
        // Already revoked, or no such token. Reported the same way so this
        // endpoint cannot be used to probe which identifiers exist.
        Ok(false) => json_error("404 Not Found", "token_not_found"),
        Err(_) => json_error("503 Service Unavailable", "identity_store_unavailable"),
    }
}

async fn set_identity_enabled(
    id: &str,
    enabled: bool,
    repository: &PostgresAuthRepository,
    _authorized: Authorized,
) -> String {
    let Ok(principal) = PrincipalId::new(id) else {
        return json_error("400 Bad Request", "invalid_principal_id");
    };
    match repository.set_principal_enabled(&principal, enabled).await {
        Ok(true) => json_response(
            "200 OK",
            &serde_json::json!({ "principal_id": id, "enabled": enabled }),
        ),
        Ok(false) => json_error("404 Not Found", "principal_not_found"),
        Err(_) => json_error("503 Service Unavailable", "identity_store_unavailable"),
    }
}

/// Records what a playback request actually handed over.
///
/// Separate from [`authorize_audited`] because the interesting fields — how
/// many bytes and over which range — are only known after streaming, and the
/// authorization row is written before it. Two rows per export is the honest
/// shape: one says who was allowed, one says what left.
async fn record_export(
    guard: &Guard<'_>,
    camera: &CameraId,
    exported: Option<Exported>,
    succeeded: bool,
) {
    let Some(repository) = guard.audit else {
        return;
    };
    // The schema refuses an export row without a complete extent, so a request
    // that never named a valid range is recorded as a plain authorization
    // failure rather than with a fabricated range.
    let Some(extent) = exported.and_then(Exported::extent) else {
        return;
    };

    let actor = guard
        .context
        .authentication()
        .principal()
        .map_or_else(|| ANONYMOUS_ACTOR.to_owned(), Principal::audit_identity);
    let outcome = if succeeded {
        AuditOutcome::Succeeded
    } else {
        AuditOutcome::Failed
    };
    let record = AccessAuditRecord::export(&actor, camera.as_str(), outcome, extent)
        .with_client(guard.context.client());

    if let Err(error) = repository.record(&record).await {
        // The footage has already left; refusing now would change nothing and
        // the connection is finished. Loud, and counted by the caller's own
        // failure metric.
        tracing::error!(
            %error,
            camera = camera.as_str(),
            bytes = exported.map_or(0, Exported::bytes),
            "export audit write failed; footage left the system unrecorded"
        );
    }
}

/// Which audit category an authorization *decision* belongs to.
///
/// `ExportRecording` maps to `Authorization`, not `Export`. The `export`
/// category is reserved for the row that carries the extent — who, which
/// camera, which range, how many bytes — and the schema refuses an export row
/// without all of it. Categorising the decision row as `export` made every
/// playback request violate `access_audit_export_fields_together`; because an
/// export is a read, the failure was logged and swallowed, so the request
/// succeeded and produced no audit row at all. Found live by requesting
/// playback and getting an empty table.
const fn audit_category(action: &Action) -> AuditCategory {
    match action {
        // Both, so "show me everything that touched identities" is one query.
        Action::ManageIdentities | Action::ViewIdentities => AuditCategory::Identity,
        Action::StartCamera(_)
        | Action::StopCamera(_)
        | Action::AddCamera
        | Action::RemoveCamera(_) => AuditCategory::Camera,
        _ => AuditCategory::Authorization,
    }
}

/// The camera an action names, for the audit row's own column.
const fn camera_of(action: &Action) -> Option<&CameraId> {
    match action.scope() {
        crate::authorize::ActionScope::Camera(camera) => Some(camera),
        crate::authorize::ActionScope::Global => None,
    }
}

/// A bounded reason label. `&'static str` so request text can never reach the
/// audit column.
const fn denial_reason(denied: Denied) -> &'static str {
    match denied {
        Denied::SurfaceDisabled => "surface_disabled",
        Denied::Unauthenticated(failure) => failure.as_str(),
        Denied::Unauthorized => "unauthorized",
        Denied::NotInScope => "not_in_scope",
        Denied::ShuttingDown => "shutting_down",
    }
}

/// Establishes who is calling, from a parsed request head.
///
/// Shared by [`route_with_recordings`] and by the playback short-circuit in
/// [`handle_client`]. Playback streams a response body, so it cannot go through
/// the string-returning router — and a second path into the handlers is exactly
/// how an endpoint ends up unauthenticated. It did: before this function
/// existed, `GET /api/cameras/{id}/recordings/playback` never reached the
/// chokepoint at all, which made the most sensitive endpoint in the system —
/// the one that streams actual footage — the only unauthenticated one.
///
/// Returns `None` when an identity store is configured but unreachable. That is
/// an outage and the caller renders `503`, never `401`.
async fn establish_context(
    head: &str,
    peer: Option<std::net::IpAddr>,
    security: &SecurityContext<'_>,
    surface: MutatingSurface,
) -> Option<RequestContext> {
    let resolved = security.boundary.resolve(
        peer,
        header_value(head, FORWARDED_FOR_HEADER).as_deref(),
        header_value(head, PROOF_HEADER).as_deref(),
    );
    // The asserted-principal header is read out of the request unconditionally
    // but handed to `from_outcome`, which discards it unless `resolved` proves
    // the request came from the configured proxy. Reading it here and deciding
    // there keeps the decision in one place; the value never reaches
    // `authenticate` without its proof.
    let authentication = security
        .authenticate(
            PresentedCredentials::from_outcome(
                resolved.outcome(),
                header_value(head, "authorization").as_deref(),
                header_value(head, AUTHENTICATED_PRINCIPAL_HEADER).as_deref(),
            ),
            peer,
        )
        .await
        .ok()?;
    Some(RequestContext::new(
        peer,
        resolved.client(),
        authentication,
        surface,
        security.shutting_down,
    ))
}

/// Splits a raw request into its head, for header reading.
fn request_head(request: &[u8]) -> Option<&str> {
    let header_end = find_header_end(request)?;
    std::str::from_utf8(&request[..header_end]).ok()
}

/// Reads one request header, case-insensitively.
///
/// The hand-rolled parser elsewhere in this module reads the request line; this
/// reads a named header from the same head. Returns the first occurrence:
/// duplicate `Authorization` headers are a smuggling smell, and taking the
/// first means an appended one cannot override an earlier one.
fn header_value(head: &str, name: &str) -> Option<String> {
    head.lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .find(|(field, _)| field.trim().eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim().to_owned())
}

/// Which optional surfaces this deployment actually wired in.
///
/// Every field is read from whether the corresponding collaborator was passed
/// to the router, not from configuration intent. A surface that is configured
/// but failed to construct reports `false`, which is the useful answer: it says
/// what the API will actually do, not what someone meant it to do.
// No longer Copy: the detection claim is taken from the live backends and
// carries their identity, which is textual.
#[derive(Clone, Debug, Serialize)]
// Five independent yes/no facts about one deployment, serialised by name. The
// excessive-bools lint guards against positional confusion at call sites; there
// is one call site and it names every field, and the JSON keys are the whole
// point of the shape.
#[allow(
    clippy::struct_excessive_bools,
    reason = "five independent surfaces, serialised by name at the single call site"
)]
struct SurfaceAvailability {
    recording_index: bool,
    viewer_tokens: bool,
    settings: bool,
    identities: bool,
    access_audit: bool,
    /// What the running detectors are, taken from the live pipelines.
    detection: DetectionCapability,
}

/// Describes detection from the backends that are actually running.
///
/// Empty means no detector is running, which is still reported as a fixture.
/// The field exists for a client to branch on, and `false` there reads as "these
/// results are genuine"; saying that about a deployment producing no results at
/// all is the direction that misleads. A deployment where any detector is a
/// fixture is reported as a fixture deployment for the same reason: one stream
/// of fabricated detections is enough, and under-claiming is the only safe way
/// to be wrong here.
fn detection_capability(backends: &[Option<InferenceDescriptor>]) -> DetectionCapability {
    let live: Vec<&InferenceDescriptor> = backends.iter().flatten().collect();
    let Some(first) = live.first() else {
        return DetectionCapability {
            mode: "none".to_owned(),
            model_name: "none".to_owned(),
            model_version: "none".to_owned(),
            is_production_model: false,
            results_are_fixture: true,
        };
    };
    let all_production = live.iter().all(|descriptor| descriptor.is_production());
    // Uniform means every running detector is the same thing. Differing
    // production status counts even when the names coincide: one camera on a
    // real model and another on a fixture is the mixture that matters most, and
    // naming one of them would describe a deployment that does not exist.
    let uniform = live.iter().all(|descriptor| {
        descriptor.mode() == first.mode()
            && descriptor.model_name() == first.model_name()
            && descriptor.model_version() == first.model_version()
            && descriptor.is_production() == first.is_production()
    });
    DetectionCapability {
        mode: if uniform {
            first.mode().as_str().to_owned()
        } else {
            "mixed".to_owned()
        },
        model_name: if uniform {
            first.model_name().to_owned()
        } else {
            "mixed".to_owned()
        },
        model_version: if uniform {
            first.model_version().to_owned()
        } else {
            "mixed".to_owned()
        },
        is_production_model: all_production,
        results_are_fixture: !all_production,
    }
}

fn live_detection_backend(
    detection: &surveillance_detector::detector::DetectionHealthSnapshot,
) -> Option<InferenceDescriptor> {
    use surveillance_detector::detector::DetectionState;
    match detection.state {
        DetectionState::Starting | DetectionState::Running | DetectionState::Degraded => {
            detection.backend.clone()
        }
        DetectionState::Disabled
        | DetectionState::Terminal
        | DetectionState::Stopping
        | DetectionState::Stopped => None,
    }
}

/// Describes what this deployment can and cannot do.
///
/// This endpoint exists for one reason above the others: **detection results
/// are a fixture**, and nothing in a `DetectionSet` reveals that. A UI built
/// against correctly-shaped, correctly-timestamped fake detections will show a
/// person count that looks entirely real. Making the fixture a machine-readable
/// field means a client can refuse to render it, instead of relying on whoever
/// builds the client having read the documentation.
///
/// The playback fields serve the same purpose for a subtler lie: the stream is
/// assembled on demand and segment-aligned, so a frame-accurate scrubber is not
/// something the server can honour.
///
/// `authorized` is unused beyond proving the chokepoint was passed — the type
/// has no public constructor, so requiring it here is what makes an
/// unauthenticated call to this handler fail to compile.
/// Authorizes and renders `GET /api/system`.
///
/// Split out only to keep `route_with_recordings` under the line limit; the
/// authorization shape is identical to every other read.
async fn route_system(surfaces: SurfaceAvailability, guard: &Guard<'_>) -> String {
    match authorize_audited(guard, Action::ViewSystemCapabilities).await {
        Ok(authorized) => system_capabilities(surfaces, &authorized),
        Err(response) => response,
    }
}

fn system_capabilities(surfaces: SurfaceAvailability, _authorized: &Authorized) -> String {
    // Taken from the backends the pipelines actually built, not from what this
    // build could construct. A build can carry the ONNX backend and still run
    // with no weights, so only the live object knows whether these results mean
    // anything, and a hardcoded string would keep saying "fake" after a real
    // model landed — or be edited to say "production" with nothing else
    // changing.
    let detection = surfaces.detection.clone();
    json_response(
        "200 OK",
        &SystemCapabilities {
            detection,
            playback: PlaybackCapability {
                accuracy: "segment-aligned",
                seekable: false,
                transport: "chunked-stream",
            },
            surfaces,
        },
    )
}

#[derive(Serialize)]
struct SystemCapabilities {
    detection: DetectionCapability,
    playback: PlaybackCapability,
    surfaces: SurfaceAvailability,
}

#[derive(Clone, Debug, Serialize)]
struct DetectionCapability {
    mode: String,
    model_name: String,
    model_version: String,
    is_production_model: bool,
    /// The field a client is expected to branch on. Named for what it means to
    /// a consumer rather than for the implementation behind it.
    results_are_fixture: bool,
}

#[derive(Serialize)]
struct PlaybackCapability {
    /// `segment-aligned`: a request is served from whole segments, so the media
    /// returned may begin before, and end after, the requested instant.
    accuracy: &'static str,
    /// Always `false`. The endpoint answers `Accept-Ranges: none` to match.
    seekable: bool,
    transport: &'static str,
}

/// Renders a refusal.
///
/// The status codes carry meaning that the threat model depends on:
///
/// - `SurfaceDisabled` is `404` so a deployment that has not opted in does not
///   advertise the endpoint;
/// - `NotInScope` is `404` so a scoped principal cannot enumerate cameras by
///   distinguishing "forbidden" from "absent";
/// - `Unauthenticated` is `401` with a constant body, so the specific
///   [`AuthFailure`](surveillance_auth::verify::AuthFailure) — expired, revoked,
///   unknown — stays in the audit trail and out of the response.
fn denial_response(denied: Denied) -> String {
    match denied {
        Denied::SurfaceDisabled | Denied::NotInScope => json_error("404 Not Found", "not_found"),
        Denied::Unauthenticated(_) => json_error("401 Unauthorized", "unauthenticated"),
        Denied::Unauthorized => json_error("403 Forbidden", "unauthorized"),
        Denied::ShuttingDown => json_error("503 Service Unavailable", "shutting_down"),
    }
}

/// The camera snapshots this principal is scoped to see.
///
/// Filtering the *listing* matters as much as refusing a direct read: an
/// unfiltered list hands a scoped operator the estate inventory that
/// [`Denied::NotInScope`]'s `404` is there to withhold.
fn visible_snapshots(
    registry: &CameraHealthRegistry,
    context: &RequestContext,
) -> Vec<CameraStatusSnapshot> {
    let snapshots = registry.snapshots();
    let Some(principal) = context.authentication().principal() else {
        return Vec::new();
    };
    if principal.scope().is_all() {
        return snapshots;
    }
    snapshots
        .into_iter()
        .filter(|snapshot| principal.scope().permits(&snapshot.camera_id))
        .collect()
}

/// Fallbacks used only when no settings service is present.
///
/// These equal the compiled defaults in `crates/config/src/defaults.rs`, which
/// a test asserts, so removing the service changes configurability without
/// changing behavior.
const DEFAULT_MAX_REQUEST_BYTES: usize = 16 * 1024;
const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_PLAYBACK_STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// One allowlisted operator-UI asset compiled into the binary.
///
/// Request paths are data, not filesystem paths. The router performs an exact
/// lookup in [`UI_ASSETS`]; it never joins, decodes, or reads a caller-supplied
/// path from disk.
#[derive(Clone, Copy)]
struct UiAsset {
    path: &'static str,
    content_type: &'static str,
    body: &'static str,
}

const fn ui_asset(path: &'static str, content_type: &'static str, body: &'static str) -> UiAsset {
    UiAsset {
        path,
        content_type,
        body,
    }
}

const HTML: &str = "text/html; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";
const JAVASCRIPT: &str = "application/javascript; charset=utf-8";
const INDEX_HTML: &str = include_str!("../ui/index.html");

/// The `LiveKit` browser SDK, compiled into the binary for the same reasons.
///
/// Vendored rather than fetched from a CDN: the dashboard makes no external
/// request, so a live player cannot depend on one either. Serving it as its own
/// same-origin asset instead of inlining it into the page keeps the HTML
/// readable and keeps half a megabyte off every page load — the dashboard
/// fetches it on the first "Watch live" press, not at boot. It is `no-store`
/// like everything else here, so an upgrade cannot leave a browser running the
/// previous SDK against the current server.
///
/// Provenance, version, and hash are recorded in `ui/vendor/README.md`.
const LIVEKIT_CLIENT: &str = include_str!("../ui/vendor/livekit-client.umd.js");

const UI_ASSETS: &[UiAsset] = &[
    ui_asset("/", HTML, INDEX_HTML),
    ui_asset("/ui", HTML, INDEX_HTML),
    ui_asset(
        "/ui/styles/base.css",
        CSS,
        include_str!("../ui/styles/base.css"),
    ),
    ui_asset(
        "/ui/styles/layout.css",
        CSS,
        include_str!("../ui/styles/layout.css"),
    ),
    ui_asset(
        "/ui/styles/components.css",
        CSS,
        include_str!("../ui/styles/components.css"),
    ),
    ui_asset(
        "/ui/styles/screens.css",
        CSS,
        include_str!("../ui/styles/screens.css"),
    ),
    ui_asset("/ui/js/app.js", JAVASCRIPT, include_str!("../ui/js/app.js")),
    ui_asset("/ui/js/api.js", JAVASCRIPT, include_str!("../ui/js/api.js")),
    ui_asset(
        "/ui/js/formatting.js",
        JAVASCRIPT,
        include_str!("../ui/js/formatting.js"),
    ),
    ui_asset(
        "/ui/js/metrics.js",
        JAVASCRIPT,
        include_str!("../ui/js/metrics.js"),
    ),
    ui_asset(
        "/ui/js/livekit.js",
        JAVASCRIPT,
        include_str!("../ui/js/livekit.js"),
    ),
    ui_asset(
        "/ui/js/detection-overlay.js",
        JAVASCRIPT,
        include_str!("../ui/js/detection-overlay.js"),
    ),
    ui_asset(
        "/ui/js/playback.js",
        JAVASCRIPT,
        include_str!("../ui/js/playback.js"),
    ),
    ui_asset(
        "/ui/js/router.js",
        JAVASCRIPT,
        include_str!("../ui/js/router.js"),
    ),
    ui_asset(
        "/ui/js/screens/cameras.js",
        JAVASCRIPT,
        include_str!("../ui/js/screens/cameras.js"),
    ),
    ui_asset(
        "/ui/js/screens/dashboard.js",
        JAVASCRIPT,
        include_str!("../ui/js/screens/dashboard.js"),
    ),
    ui_asset(
        "/ui/js/screens/events.js",
        JAVASCRIPT,
        include_str!("../ui/js/screens/events.js"),
    ),
    ui_asset(
        "/ui/js/screens/heatmap.js",
        JAVASCRIPT,
        include_str!("../ui/js/screens/heatmap.js"),
    ),
    ui_asset(
        "/ui/js/screens/live.js",
        JAVASCRIPT,
        include_str!("../ui/js/screens/live.js"),
    ),
    ui_asset(
        "/ui/js/screens/playback.js",
        JAVASCRIPT,
        include_str!("../ui/js/screens/playback.js"),
    ),
    ui_asset(
        "/ui/js/screens/settings.js",
        JAVASCRIPT,
        include_str!("../ui/js/screens/settings.js"),
    ),
    ui_asset(
        "/ui/js/screens/storage.js",
        JAVASCRIPT,
        include_str!("../ui/js/screens/storage.js"),
    ),
    ui_asset(
        "/ui/js/screens/system.js",
        JAVASCRIPT,
        include_str!("../ui/js/screens/system.js"),
    ),
    ui_asset(
        "/ui/js/state.js",
        JAVASCRIPT,
        include_str!("../ui/js/state.js"),
    ),
    ui_asset(
        "/ui/js/theme.js",
        JAVASCRIPT,
        include_str!("../ui/js/theme.js"),
    ),
    ui_asset(
        "/ui/js/components/banner.js",
        JAVASCRIPT,
        include_str!("../ui/js/components/banner.js"),
    ),
    ui_asset(
        "/ui/js/components/camera-tile.js",
        JAVASCRIPT,
        include_str!("../ui/js/components/camera-tile.js"),
    ),
    ui_asset(
        "/ui/js/components/empty-state.js",
        JAVASCRIPT,
        include_str!("../ui/js/components/empty-state.js"),
    ),
    ui_asset("/ui/livekit-client.js", JAVASCRIPT, LIVEKIT_CLIENT),
];

fn find_ui_asset(path: &str) -> Option<UiAsset> {
    UI_ASSETS.iter().copied().find(|asset| asset.path == path)
}

/// Reads the request bounds for this request.
///
/// `api.max_request_bytes` and `api.io_timeout` are the two settings declared
/// `Immediate`, so they are read per request rather than captured at startup.
async fn request_bounds(settings: Option<&SettingsService>) -> (usize, Duration) {
    let Some(service) = settings else {
        return (DEFAULT_MAX_REQUEST_BYTES, DEFAULT_IO_TIMEOUT);
    };
    let bytes = match service
        .effective_value(SettingId::ApiMaxRequestBytes, None)
        .await
    {
        Some(SettingValue::Unsigned(value)) => {
            usize::try_from(value).unwrap_or(DEFAULT_MAX_REQUEST_BYTES)
        }
        _ => DEFAULT_MAX_REQUEST_BYTES,
    };
    let timeout = match service.effective_value(SettingId::ApiIoTimeout, None).await {
        Some(SettingValue::Duration(value)) => value,
        _ => DEFAULT_IO_TIMEOUT,
    };
    (bytes, timeout)
}

/// Reads how long a playback body may make no progress before it is abandoned.
///
/// Deliberately not [`request_bounds`]'s `api.io_timeout`. That one bounds a
/// request or a JSON response, where a client that goes quiet for five seconds
/// really has gone away. A playback body is drained by a media element, which
/// stops reading the moment its buffer is full and does not read again until the
/// buffer drains — behaviour every browser has. Bounding it at `api.io_timeout`
/// cancelled the assembly under any paced reader, and the client, which had been
/// handed only the few seconds it managed to buffer before the socket closed,
/// treated that as the whole recording and rewound to its start.
///
/// Read per request for the same reason `api.io_timeout` is: it is `Immediate`.
async fn playback_stall_timeout(settings: Option<&SettingsService>) -> Duration {
    let Some(service) = settings else {
        return DEFAULT_PLAYBACK_STALL_TIMEOUT;
    };
    match service
        .effective_value(SettingId::ApiPlaybackStallTimeout, None)
        .await
    {
        Some(SettingValue::Duration(value)) => value,
        _ => DEFAULT_PLAYBACK_STALL_TIMEOUT,
    }
}

/// The deployment-wide template a camera created through the API is built from.
///
/// Neither half is a per-camera fact. Every camera on a unit records beneath the
/// same root, and a unit publishes to one `LiveKit` deployment, so the endpoint,
/// credentials, namespace, adapter, and queue tuning are identical for every
/// camera that publishes at all. What varies per camera is one bit: whether it
/// publishes.
///
/// That is why this arrives from the process owner rather than from a request
/// body. `LiveKitSettings` carries an API secret, and `POST /api/cameras` must
/// never be a route by which a caller supplies one, learns one, or substitutes
/// an endpoint of their choosing.
pub struct CameraDefaults {
    recording_root: PathBuf,
    livekit: Option<LiveKitSettings>,
}

impl CameraDefaults {
    /// Builds the template. `livekit` is `None` when the deployment publishes
    /// nowhere, which makes [`Self::publishes_live`] false and every request to
    /// enable publication a rejection rather than a silent no-op.
    #[must_use]
    pub const fn new(recording_root: PathBuf, livekit: Option<LiveKitSettings>) -> Self {
        Self {
            recording_root,
            livekit,
        }
    }

    #[must_use]
    pub fn recording_root(&self) -> &std::path::Path {
        &self.recording_root
    }

    /// Whether this deployment publishes to `LiveKit` at all.
    #[must_use]
    pub const fn publishes_live(&self) -> bool {
        self.livekit.is_some()
    }

    /// A copy of the deployment template for one camera. Private: the secret it
    /// carries stays inside this module, and the only caller is the add path.
    fn livekit(&self) -> Option<LiveKitSettings> {
        self.livekit.clone()
    }
}

/// Runs one bounded sequential loopback adapter until requested shutdown.
///
/// Listener/client failure is returned to its process owner and never sends a
/// camera-supervisor shutdown command.
///
/// # Errors
///
/// Returns an I/O error for a non-loopback bind, listener failure, or shutdown
/// channel failure. Individual malformed clients are contained and discarded.
#[allow(
    clippy::too_many_arguments,
    reason = "the process owner passes each independently owned adapter boundary explicitly"
)]
pub async fn serve_health(
    address: SocketAddr,
    camera_defaults: CameraDefaults,
    registry: CameraHealthRegistry,
    supervisor: CameraSupervisorHandle,
    recordings: Option<PostgresRecordingRepository>,
    playback: Option<PlaybackAssembler>,
    metadata: Option<MetadataWriterHandle>,
    phase4: Option<Phase4Metrics>,
    viewer_tokens: Option<DevelopmentTokenService>,
    settings: Option<Arc<SettingsService>>,
    security: Option<Arc<SecurityOwner>>,
    access_audit: Option<PostgresAccessAuditRepository>,
    identities: Option<Arc<PostgresAuthRepository>>,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    if !address.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Phase 3 camera API must bind to a loopback address",
        ));
    }
    let listener = TcpListener::bind(address).await?;
    tracing::info!(%address, "loopback health and camera API listening");
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow_and_update() {
                    return Ok(());
                }
            }
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                // Read fresh per request: a mutation that arrives after the
                // drain begins must be refused, not admitted because the
                // listener started before the signal.
                let draining = *shutdown.borrow();
                if let Err(error) = Box::pin(handle_client(stream, &camera_defaults, &registry, &supervisor, recordings.as_ref(), playback.as_ref(), metadata.as_ref(), phase4.as_ref(), viewer_tokens.as_ref(), settings.as_deref(), Some(peer.ip()), security.as_deref(), access_audit.as_ref(), identities.as_deref(), draining)).await {
                    tracing::debug!(%peer, %error, "API client request discarded");
                }
            }
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "adapter dependencies retain singular owners"
)]
async fn handle_client(
    mut stream: TcpStream,
    camera_defaults: &CameraDefaults,
    registry: &CameraHealthRegistry,
    supervisor: &CameraSupervisorHandle,
    recordings: Option<&PostgresRecordingRepository>,
    playback: Option<&PlaybackAssembler>,
    metadata: Option<&MetadataWriterHandle>,
    phase4: Option<&Phase4Metrics>,
    viewer_tokens: Option<&DevelopmentTokenService>,
    settings: Option<&SettingsService>,
    peer: Option<std::net::IpAddr>,
    security: Option<&SecurityOwner>,
    access_audit: Option<&PostgresAccessAuditRepository>,
    identities: Option<&PostgresAuthRepository>,
    shutting_down: bool,
) -> io::Result<()> {
    let (max_request_bytes, io_timeout) = request_bounds(settings).await;
    let request = read_request(&mut stream, max_request_bytes, io_timeout).await?;
    if let (Some(repository), Some(assembler), Some(target)) =
        (recordings, playback, playback_target(&request))
    {
        // Playback streams a response body, so it cannot go through the
        // string-returning router — but it still has to reach the chokepoint.
        // It streams actual footage, which makes it the endpoint that least
        // tolerates being missed.
        let default_security = SecurityOwner::loopback_development();
        let owner = security.unwrap_or(&default_security);
        let context = match request_head(&request) {
            Some(head) => {
                establish_context(
                    head,
                    peer,
                    &owner.context(shutting_down),
                    settings.map_or(MutatingSurface::Disabled, SettingsService::surface),
                )
                .await
            }
            None => None,
        };
        let Some(context) = context else {
            return write_response(
                &mut stream,
                io_timeout,
                &json_error("503 Service Unavailable", "identity_store_unavailable"),
            )
            .await;
        };
        // `ExportRecording`, not `ViewRecordings`: this is the only path by
        // which media leaves the system, so calling it a "view" would be a
        // fiction — the client can save whatever it is streamed. The role stays
        // Viewer (see `Action::required_role`); what the stronger action buys
        // is the export audit row below, which the threat model (B7) names as
        // the only available control on insider export.
        let guard = Guard {
            context,
            audit: access_audit,
        };
        let camera_id = target.camera_id().clone();
        if let Err(response) =
            authorize_audited(&guard, Action::ExportRecording(camera_id.clone())).await
        {
            return write_response(&mut stream, io_timeout, &response).await;
        }
        let exported = match target {
            PlaybackTarget::Range { query, .. } => {
                Box::pin(stream_playback(
                    &mut stream,
                    camera_id.clone(),
                    query,
                    repository,
                    assembler,
                    phase4,
                    playback_stall_timeout(settings).await,
                ))
                .await
            }
            PlaybackTarget::Recording {
                recording_id,
                query,
                ..
            } => {
                Box::pin(stream_recording_video(
                    &mut stream,
                    camera_id.clone(),
                    recording_id,
                    query,
                    repository,
                    assembler,
                    phase4,
                    playback_stall_timeout(settings).await,
                ))
                .await
            }
        };

        // Written after streaming, because the byte count is only known then.
        // A failure mid-stream still produces a row: partial footage left the
        // system, and an audit trail that recorded only clean successes would
        // be exactly the one an incident cannot use.
        let delivered = match &exported {
            Ok(exported) => Some(*exported),
            Err(partial) => partial.delivered,
        };
        record_export(&guard, &camera_id, delivered, exported.is_ok()).await;
        return exported.map(|_| ()).map_err(|partial| partial.error);
    }
    let response = route_with_recordings(
        &request,
        camera_defaults,
        registry,
        supervisor,
        recordings,
        metadata,
        phase4,
        viewer_tokens,
        settings,
        peer,
        security.map(|owner| owner.context(shutting_down)).as_ref(),
        access_audit,
        identities,
    )
    .await;
    write_response(&mut stream, io_timeout, &response).await
}

/// Writes a rendered response under the configured I/O timeout.
async fn write_response(
    stream: &mut TcpStream,
    io_timeout: std::time::Duration,
    response: &str,
) -> io::Result<()> {
    tokio::time::timeout(io_timeout, stream.write_all(response.as_bytes()))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "API response timed out"))??;
    Ok(())
}

enum PlaybackTarget<'a> {
    Range {
        camera_id: CameraId,
        query: &'a str,
    },
    Recording {
        camera_id: CameraId,
        recording_id: surveillance_common::types::RecordingId,
        query: &'a str,
    },
}

impl PlaybackTarget<'_> {
    fn camera_id(&self) -> &CameraId {
        match self {
            Self::Range { camera_id, .. } | Self::Recording { camera_id, .. } => camera_id,
        }
    }
}

fn playback_target(request: &[u8]) -> Option<PlaybackTarget<'_>> {
    let header_end = find_header_end(request)?;
    let head = std::str::from_utf8(&request[..header_end]).ok()?;
    let mut line = head.lines().next()?.split_whitespace();
    if line.next()? != "GET" {
        return None;
    }
    let target = line.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    if parts.first() != Some(&"api")
        || parts.get(1) != Some(&"cameras")
        || parts.get(3) != Some(&"recordings")
    {
        return None;
    }
    let camera_id = CameraId::new(parts[2]).ok()?;
    if parts.len() == 5 && parts[4] == "playback" {
        return Some(PlaybackTarget::Range { camera_id, query });
    }
    if parts.len() == 6 && parts[5] == "video" {
        let recording_id = surveillance_common::types::RecordingId::new(parts[4]).ok()?;
        return Some(PlaybackTarget::Recording {
            camera_id,
            recording_id,
            query,
        });
    }
    None
}

/// The response headers for a playback stream.
///
/// Extracted so the honesty of this response is testable without a database,
/// an assembler, and a socket. What it must say, and why:
///
/// - `Accept-Ranges: none` is the machine-readable half of what
///   `X-Playback-Accuracy` says in prose. The body is assembled on demand and
///   streamed chunked, so there is no addressable byte range to seek into. A
///   client offering a scrubber is promising something this endpoint cannot
///   do; saying so in a standard header means a browser media element will not
///   try, rather than failing obscurely mid-seek.
/// - `X-Playback-Accuracy: segment-aligned` because the boundaries come from
///   whole segments and their keyframes, not from the requested instants. The
///   requested instants are echoed back so a caller can see the difference
///   between what it asked for and what the segment index could honour.
/// - No `Content-Length`: the length is not known until the last segment has
///   been read, which is after the response has to start.
fn playback_headers(start: impl std::fmt::Display, end: impl std::fmt::Display) -> String {
    format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: video/mp4\r\n\
         Transfer-Encoding: chunked\r\n\
         Accept-Ranges: none\r\n\
         X-Playback-Accuracy: segment-aligned\r\n\
         X-Requested-Start: {start}\r\n\
         X-Requested-End: {end}\r\n\
         Connection: close\r\n\r\n"
    )
}

fn recording_video_headers(
    recording_id: &surveillance_common::types::RecordingId,
    extent: RecordingVideoExtent,
) -> String {
    let requested = match extent {
        RecordingVideoExtent::Full => "full".to_owned(),
        RecordingVideoExtent::Leading(duration) => format!("{}s", duration.seconds()),
    };
    format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: video/mp4\r\n\
         Transfer-Encoding: chunked\r\n\
         Accept-Ranges: none\r\n\
         X-Playback-Accuracy: packet-bounded\r\n\
         X-Recording-Id: {recording_id}\r\n\
         X-Requested-Duration: {requested}\r\n\
         Connection: close\r\n\r\n"
    )
}

fn parse_recording_video_extent(query: &str) -> Result<RecordingVideoExtent, &'static str> {
    let parameters: Vec<_> = url::form_urlencoded::parse(query.as_bytes()).collect();
    if parameters.is_empty() {
        return Ok(RecordingVideoExtent::Full);
    }
    if parameters.len() != 1 || parameters[0].0 != "duration_seconds" {
        return Err("invalid_recording_video_query");
    }
    let seconds = parameters[0]
        .1
        .parse::<u32>()
        .map_err(|_| "invalid_recording_video_duration")?;
    ClipDuration::new(seconds)
        .map(RecordingVideoExtent::Leading)
        .map_err(|_| "invalid_recording_video_duration")
}

/// Writes a JSON error response and reports that nothing was exported.
///
/// Ten call sites repeated this five-line shape; collapsing it keeps both
/// streaming handlers inside the workspace line budget and leaves one place
/// where a refusal's response and its audit meaning are decided together.
async fn refuse(stream: &mut TcpStream, status: &str, code: &str) -> ExportResult {
    stream
        .write_all(json_error(status, code).as_bytes())
        .await
        .map(|()| Exported::refused())
        .map_err(PartialExport::from)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the streaming adapter receives each independently owned boundary explicitly"
)]
async fn stream_recording_video(
    stream: &mut TcpStream,
    camera_id: CameraId,
    recording_id: surveillance_common::types::RecordingId,
    query: &str,
    repository: &PostgresRecordingRepository,
    assembler: &PlaybackAssembler,
    metrics: Option<&Phase4Metrics>,
    io_timeout: Duration,
) -> ExportResult {
    let extent = match parse_recording_video_extent(query) {
        Ok(extent) => extent,
        Err(code) => {
            return refuse(stream, "400 Bad Request", code).await;
        }
    };
    let recording = match repository.find_completed(&camera_id, &recording_id).await {
        Ok(Some(recording)) => recording,
        Ok(None) => {
            return refuse(stream, "404 Not Found", "recording_not_found").await;
        }
        Err(error) if error.retryable() => {
            return refuse(
                stream,
                "503 Service Unavailable",
                "recording_index_unavailable",
            )
            .await;
        }
        Err(_) => {
            return refuse(
                stream,
                "500 Internal Server Error",
                "recording_query_failed",
            )
            .await;
        }
    };
    let available_duration = recording_duration(&recording)?;
    let requested_duration = match extent {
        RecordingVideoExtent::Full => available_duration,
        RecordingVideoExtent::Leading(duration) => {
            Duration::from_secs(u64::from(duration.seconds()))
        }
    };
    if requested_duration > available_duration {
        return refuse(
            stream,
            "400 Bad Request",
            "recording_video_duration_exceeds_recording",
        )
        .await;
    }
    let Ok(mut playback) = assembler.start_recording(&recording, extent).await else {
        if let Some(metrics) = metrics {
            metrics.playback_failed();
        }
        return refuse(stream, "409 Conflict", "recording_media_unavailable").await;
    };
    if let Some(metrics) = metrics {
        metrics.playback_started();
    }
    stream
        .write_all(recording_video_headers(&recording_id, extent).as_bytes())
        .await?;
    let (exported_bytes, outcome) = stream_chunked_media(
        stream,
        &mut playback,
        io_timeout,
        "recording video client stalled",
    )
    .await;
    let delivered = |bytes| {
        recording_extent(&recording, requested_duration)
            .map(|(start, end)| Exported::delivered(start, end, bytes))
    };
    if let Err(error) = outcome {
        playback.cancel().await;
        if error.kind() == io::ErrorKind::TimedOut {
            if let Some(metrics) = metrics {
                metrics.playback_cancelled();
                metrics.playback_closed();
            }
        }
        return Err(PartialExport::after(delivered(exported_bytes), error));
    }
    if playback.finish().await.is_err() {
        if let Some(metrics) = metrics {
            metrics.playback_failed();
            metrics.playback_closed();
        }
        return Err(PartialExport::after(
            delivered(exported_bytes),
            io::Error::other("recording video assembly failed"),
        ));
    }
    if let Some(metrics) = metrics {
        metrics.playback_completed();
        metrics.playback_closed();
    }
    let (start, end) = recording_extent(&recording, requested_duration)
        .ok_or_else(|| io::Error::other("recording extent invalid"))?;
    stream
        .write_all(b"0\r\n\r\n")
        .await
        .map(|()| Exported::delivered(start, end, exported_bytes))
        .map_err(PartialExport::from)
}

/// The audit extent of one recording-video response.
fn recording_extent(
    recording: &surveillance_storage::metadata::CompletedRecording,
    requested_duration: Duration,
) -> Option<(jiff::Timestamp, jiff::Timestamp)> {
    let start = jiff::Timestamp::try_from(recording.started_at()).ok()?;
    let end = jiff::Timestamp::try_from(recording.started_at() + requested_duration).ok()?;
    Some((start, end))
}

fn recording_duration(
    recording: &surveillance_storage::metadata::CompletedRecording,
) -> io::Result<Duration> {
    recording
        .ended_at()
        .duration_since(recording.started_at())
        .map_err(|_| io::Error::other("recording interval invalid"))
}

/// Writes assembled media as chunked transfer encoding.
///
/// Returns the media bytes delivered alongside the outcome, because the two are
/// independent: a stall or a broken pipe partway through still handed footage
/// over, and the export audit row has to be able to say how much. Media bytes
/// only -- the chunk framing and headers are transport, not footage, and an
/// audit row that counted them would misstate what left.
///
/// Shared by both streaming handlers, which had the same loop written twice.
async fn stream_chunked_media(
    stream: &mut TcpStream,
    playback: &mut PlaybackStream,
    io_timeout: Duration,
    stall_message: &'static str,
) -> (u64, io::Result<()>) {
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    let mut exported_bytes = 0_u64;
    loop {
        let read = match playback.read(&mut buffer).await {
            Ok(read) => read,
            Err(error) => return (exported_bytes, Err(error)),
        };
        if read == 0 {
            break;
        }
        exported_bytes = exported_bytes.saturating_add(read as u64);
        let chunk_header = format!("{read:x}\r\n");
        let write = async {
            stream.write_all(chunk_header.as_bytes()).await?;
            stream.write_all(&buffer[..read]).await?;
            stream.write_all(b"\r\n").await
        };
        if let Ok(result) = tokio::time::timeout(io_timeout, write).await {
            if let Err(error) = result {
                return (exported_bytes, Err(error));
            }
        } else {
            return (
                exported_bytes,
                Err(io::Error::new(io::ErrorKind::TimedOut, stall_message)),
            );
        }
    }
    (exported_bytes, Ok(()))
}

async fn stream_playback(
    stream: &mut TcpStream,
    camera_id: CameraId,
    query: &str,
    repository: &PostgresRecordingRepository,
    assembler: &PlaybackAssembler,
    metrics: Option<&Phase4Metrics>,
    io_timeout: Duration,
) -> ExportResult {
    let parameters: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
    let Some((start, end, range)) = parse_range(&parameters) else {
        if let Some(metrics) = metrics {
            metrics.range_rejected();
        }
        return refuse(stream, "400 Bad Request", "invalid_recording_range").await;
    };
    let search = RecordingSearch::new(
        camera_id,
        range,
        RecordingResultLimit::new(1_000).map_err(|_| io::Error::other("playback limit invalid"))?,
        None,
    );
    let Ok(rows) = repository.find_overlapping_completed(&search).await else {
        if let Some(metrics) = metrics {
            metrics.query_failed();
        }
        return refuse(
            stream,
            "503 Service Unavailable",
            "recording_index_unavailable",
        )
        .await;
    };
    if let Some(metrics) = metrics {
        metrics.query(rows.len());
    }
    let Ok(plan) = PlaybackPlan::new(range, rows) else {
        return refuse(stream, "409 Conflict", "recording_range_not_contiguous").await;
    };
    let Ok(mut playback) = assembler.start(&plan).await else {
        if let Some(metrics) = metrics {
            metrics.playback_failed();
        }
        return refuse(stream, "409 Conflict", "recording_media_unavailable").await;
    };
    if let Some(metrics) = metrics {
        metrics.playback_started();
    }
    stream
        .write_all(playback_headers(start, end).as_bytes())
        .await?;
    let delivered = |bytes| Some(Exported::delivered(start, end, bytes));
    let (exported_bytes, outcome) =
        stream_chunked_media(stream, &mut playback, io_timeout, "playback client stalled").await;
    if let Err(error) = outcome {
        playback.cancel().await;
        if error.kind() == io::ErrorKind::TimedOut {
            if let Some(metrics) = metrics {
                metrics.playback_cancelled();
                metrics.playback_closed();
            }
        }
        // The client may have got part of the footage before failing. The audit
        // row must say so: reporting zero would understate what left, and
        // reporting success would overstate it.
        return Err(PartialExport::after(delivered(exported_bytes), error));
    }
    if playback.finish().await.is_err() {
        if let Some(metrics) = metrics {
            metrics.playback_failed();
            metrics.playback_closed();
        }
        return Err(PartialExport::after(
            delivered(exported_bytes),
            io::Error::other("playback assembly failed"),
        ));
    }
    if let Some(metrics) = metrics {
        metrics.playback_completed();
        metrics.playback_closed();
    }
    stream
        .write_all(b"0\r\n\r\n")
        .await
        .map(|()| Exported::delivered(start, end, exported_bytes))
        .map_err(PartialExport::from)
}

/// What a playback request actually handed over.
///
/// Returned so the caller can write a truthful export audit row. A refused or
/// failed request still produces a row — "someone asked for this footage and did
/// not get it" is as audit-relevant as a success, and omitting it would let a
/// probe for what exists leave no trace.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Exported {
    range: Option<(jiff::Timestamp, jiff::Timestamp)>,
    bytes: u64,
}

impl Exported {
    const fn refused() -> Self {
        Self {
            range: None,
            bytes: 0,
        }
    }

    const fn delivered(start: jiff::Timestamp, end: jiff::Timestamp, bytes: u64) -> Self {
        Self {
            range: Some((start, end)),
            bytes,
        }
    }

    /// The audit extent, when a range was actually parsed and served.
    ///
    /// `None` means the request never named a valid range, so there is no
    /// extent to record — the schema refuses a partially-filled export row, and
    /// inventing a range to satisfy it would be fabricated metadata.
    fn extent(self) -> Option<ExportExtent> {
        let (start, end) = self.range?;
        Some(ExportExtent {
            range_start: chrono::DateTime::from_timestamp(start.as_second(), 0)?,
            range_end: chrono::DateTime::from_timestamp(end.as_second(), 0)?,
            byte_count: i64::try_from(self.bytes).unwrap_or(i64::MAX),
        })
    }

    const fn bytes(self) -> u64 {
        self.bytes
    }
}

/// An export that failed after footage had already been handed over.
///
/// The delivered extent and the failure are independent facts. Both call sites
/// already documented the intent -- "a failure mid-stream still produces a
/// row", "reporting zero would understate what left" -- but the extent was
/// dropped with the error, so `record_export` received `None` and wrote
/// nothing. Carrying it inside the error is what makes those comments true.
///
/// `From<io::Error>` exists so `?` keeps working on the many write paths that
/// fail before anything was delivered.
pub(crate) struct PartialExport {
    delivered: Option<Exported>,
    error: io::Error,
}

impl PartialExport {
    fn after(delivered: Option<Exported>, error: io::Error) -> Self {
        Self { delivered, error }
    }
}

impl From<io::Error> for PartialExport {
    fn from(error: io::Error) -> Self {
        Self {
            delivered: None,
            error,
        }
    }
}

type ExportResult = Result<Exported, PartialExport>;

/// The query parameters naming a recording time range.
///
/// Named once because they are read here, in [`recording_search`], and written
/// by the dashboard — three places, two languages, and nothing in the build
/// checks that the third agrees. It did not: the page sent `from`/`to`, so every
/// search answered `missing_recording_range` whatever range was asked for. These
/// constants let a test assert the shipped page against the names actually read,
/// which is the only joint that catches a rename on either side.
const RANGE_START: &str = "start";
const RANGE_END: &str = "end";

fn parse_range(
    parameters: &std::collections::BTreeMap<String, String>,
) -> Option<(jiff::Timestamp, jiff::Timestamp, RecordingTimeRange)> {
    let start = parameters
        .get(RANGE_START)?
        .parse::<jiff::Timestamp>()
        .ok()?;
    let end = parameters.get(RANGE_END)?.parse::<jiff::Timestamp>().ok()?;
    let range = RecordingTimeRange::new(
        std::time::SystemTime::from(start),
        std::time::SystemTime::from(end),
        Duration::from_secs(24 * 60 * 60),
    )
    .ok()?;
    Some((start, end, range))
}

async fn read_request(
    stream: &mut TcpStream,
    max_request_bytes: usize,
    io_timeout: Duration,
) -> io::Result<Vec<u8>> {
    let mut request = Vec::with_capacity(1_024);
    loop {
        if request.len() >= max_request_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "API request exceeds configured bound",
            ));
        }
        let mut chunk = [0_u8; 1_024];
        let remaining = max_request_bytes.saturating_sub(request.len());
        let read =
            tokio::time::timeout(io_timeout, stream.read(&mut chunk[..remaining.min(1_024)]))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "API request timed out"))??;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        if let Some(header_end) = find_header_end(&request) {
            let content_length = content_length(&request[..header_end])?;
            let total = header_end
                .checked_add(4)
                .and_then(|value| value.checked_add(content_length))
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid request size")
                })?;
            if total > max_request_bytes {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "API request body exceeds configured bound",
                ));
            }
            if request.len() >= total {
                request.truncate(total);
                return Ok(request);
            }
        }
    }
    Ok(request)
}

fn find_header_end(request: &[u8]) -> Option<usize> {
    request.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(headers: &[u8]) -> io::Result<usize> {
    let headers = std::str::from_utf8(headers)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request headers are not UTF-8"))?;
    for line in headers.lines().skip(1) {
        if let Some(value) = line
            .split_once(':')
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .map(|(_, value)| value.trim())
        {
            return value
                .parse()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid content length"));
        }
    }
    Ok(0)
}

/// The deployment template the routing tests run against.
///
/// No `LiveKit`, which is the honest default for a test that never starts one:
/// it makes `publishes_live()` false, so a camera added without the field is
/// created without publication and one added with `true` is refused. The tests
/// that exercise publication build their own template.
#[cfg(test)]
fn test_defaults(recording_root: &str) -> CameraDefaults {
    CameraDefaults::new(PathBuf::from(recording_root), None)
}

#[cfg(test)]
async fn route(
    request: &[u8],
    camera_defaults: &CameraDefaults,
    registry: &CameraHealthRegistry,
    supervisor: &CameraSupervisorHandle,
) -> String {
    route_with_recordings(
        request,
        camera_defaults,
        registry,
        supervisor,
        None,
        None,
        None,
        None,
        None,
        // A real accepted connection always has a peer address, and since
        // ADR 009 the development identity is granted only to a loopback one.
        // Passing `None` here would test a request shape that cannot occur.
        Some(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        None,
        None,
        None,
    )
    .await
}

/// Routes with a settings service and a stated peer, for the P7.6 tests.
#[cfg(test)]
async fn route_with_settings(
    request: &[u8],
    camera_defaults: &CameraDefaults,
    registry: &CameraHealthRegistry,
    supervisor: &CameraSupervisorHandle,
    settings: &SettingsService,
    peer: Option<std::net::IpAddr>,
) -> String {
    route_with_recordings(
        request,
        camera_defaults,
        registry,
        supervisor,
        None,
        None,
        None,
        None,
        Some(settings),
        peer,
        None,
        None,
        None,
    )
    .await
}

#[allow(
    clippy::too_many_arguments,
    reason = "API adapter receives explicit independently owned boundaries"
)]
#[allow(
    clippy::too_many_lines,
    reason = "one match is the route table; splitting it would hide which paths \
              exist and which of them reach the authorization chokepoint"
)]
async fn route_with_recordings(
    request: &[u8],
    camera_defaults: &CameraDefaults,
    registry: &CameraHealthRegistry,
    supervisor: &CameraSupervisorHandle,
    recordings: Option<&PostgresRecordingRepository>,
    metadata: Option<&MetadataWriterHandle>,
    phase4: Option<&Phase4Metrics>,
    viewer_tokens: Option<&DevelopmentTokenService>,
    settings: Option<&SettingsService>,
    peer: Option<std::net::IpAddr>,
    security: Option<&SecurityContext<'_>>,
    access_audit: Option<&PostgresAccessAuditRepository>,
    identities: Option<&PostgresAuthRepository>,
) -> String {
    let Some(header_end) = find_header_end(request) else {
        return json_error("400 Bad Request", "invalid_request");
    };
    let Ok(head) = std::str::from_utf8(&request[..header_end]) else {
        return json_error("400 Bad Request", "invalid_request");
    };
    let Some(line) = head.lines().next() else {
        return json_error("400 Bad Request", "invalid_request");
    };
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target), Some(_version)) = (parts.next(), parts.next(), parts.next())
    else {
        return json_error("400 Bad Request", "invalid_request");
    };
    let body = &request[header_end + 4..];
    let (path, query) = target
        .split_once('?')
        .map_or((target, ""), |(path, query)| (path, query));
    // Gathered before dispatch so `/metrics` takes the configuration lock once.
    let configuration_view = match (method, path, settings) {
        ("GET", "/metrics", Some(service)) => Some(service.metrics_view().await),
        _ => None,
    };

    // Establish who is calling exactly once, before dispatch. Doing it per
    // handler would mean an Argon2id verification per handler, and would make
    // "every protected route authenticates" a matter of review rather than of
    // structure.
    //
    // `/live`, `/ready`, and `/metrics` are reached below without an
    // `Authorized`, which is the whole of ADR 009 Decision 2: they are the
    // three unauthenticated endpoints, all constant or aggregate, and none
    // names a recording or a time range.
    let default_security = SecurityContext {
        boundary: &TrustBoundary::Loopback,
        authenticator: None,
        shutting_down: false,
    };
    let security = security.unwrap_or(&default_security);
    // Read once, before dispatch, so `/metrics` renders a consistent view.
    let auth_metrics = security.authenticator.map(RequestAuthenticator::metrics);
    let Some(context) = establish_context(
        head,
        peer,
        security,
        settings.map_or(MutatingSurface::Disabled, SettingsService::surface),
    )
    .await
    else {
        return json_error("503 Service Unavailable", "identity_store_unavailable");
    };
    let guard = Guard {
        context,
        audit: access_audit,
    };

    if method == "GET" {
        if let Some(asset) = find_ui_asset(path) {
            return response("200 OK", asset.content_type, asset.body);
        }
    }

    match (method, path) {
        // The dashboard shell, served without an `Authorized` for the same
        // reason as the three below it (ADR 009 Decision 2): it is a
        // compile-time constant, byte-identical for every caller, and names no
        // camera, no principal, and no time range. It carries no data — every
        // value it displays arrives later through an authenticated call.
        //
        // It must also be reachable by a caller this system will refuse, or an
        // unprovisioned person would meet a raw JSON `401` instead of the
        // screen that explains signing in again cannot help them.
        ("GET", "/live") => response("200 OK", "text/plain; charset=utf-8", "live\n"),
        ("GET", "/ready") => {
            readiness_response(&registry.snapshots(), recordings, metadata, phase4).await
        }
        ("GET", "/metrics") => response(
            "200 OK",
            "text/plain; version=0.0.4; charset=utf-8",
            &render_metrics(
                &registry.snapshots(),
                recordings,
                metadata,
                phase4,
                Some(&supervisor.event_bus()),
                configuration_view.as_ref(),
                auth_metrics.as_deref(),
            ),
        ),
        ("GET", "/api/cameras") => match authorize_audited(&guard, Action::ListCameras).await {
            // The listing is filtered to what this principal may see, so a
            // scoped operator cannot enumerate the estate by listing it.
            Ok(_authorized) => json_response(
                "200 OK",
                &camera_list(&visible_snapshots(registry, guard.context())),
            ),
            Err(response) => response,
        },
        ("POST", "/api/cameras") => match authorize_audited(&guard, Action::AddCamera).await {
            Ok(authorized) => add_camera(body, camera_defaults, supervisor, authorized).await,
            Err(response) => response,
        },
        ("GET", "/api/system") => {
            let backends: Vec<Option<InferenceDescriptor>> = registry
                .snapshots()
                .into_iter()
                .map(|camera| {
                    camera
                        .health
                        .and_then(|health| live_detection_backend(&health.detection))
                })
                .collect();
            let surfaces = SurfaceAvailability {
                recording_index: recordings.is_some(),
                viewer_tokens: viewer_tokens.is_some(),
                settings: settings.is_some(),
                identities: identities.is_some(),
                access_audit: access_audit.is_some(),
                detection: detection_capability(&backends),
            };
            route_system(surfaces, &guard).await
        }
        (_, path) if path == "/api/settings" || path.starts_with("/api/settings/") => {
            route_settings(method, path, body, settings, &guard).await
        }
        (_, path) if path == "/api/identities" || path.starts_with("/api/identities/") => {
            route_identities(method, path, body, identities, &guard).await
        }
        _ => {
            route_camera(
                method,
                path,
                query,
                camera_defaults,
                registry,
                supervisor,
                recordings,
                viewer_tokens,
                &guard,
            )
            .await
        }
    }
}

/// Settings resources.
///
/// Reads are unauthenticated in Phase 7: the listener is loopback-only and
/// observing effective configuration carries no write risk. Writes always route
/// through [`authorize`], and when the mutating surface is disabled they answer
/// `404`, matching an unknown path exactly — a disabled surface does not
/// advertise itself.
async fn route_settings(
    method: &str,
    path: &str,
    body: &[u8],
    settings: Option<&SettingsService>,
    guard: &Guard<'_>,
) -> String {
    let Some(service) = settings else {
        return json_error("503 Service Unavailable", "settings_unavailable");
    };
    let key = path
        .strip_prefix("/api/settings/")
        .filter(|key| !key.is_empty());

    // Reads are authorized too (ADR 009 Decision 2): effective configuration
    // names storage roots and retention, which is not public information.
    match (method, key) {
        ("GET", None) => match authorize_audited(guard, Action::ViewSettings).await {
            Ok(_authorized) => settings_list(service).await,
            Err(response) => response,
        },
        ("GET", Some(key)) => match authorize_audited(guard, Action::ViewSettings).await {
            Ok(_authorized) => match SettingId::from_key(key) {
                Some(id) => settings_detail(service, id).await,
                None => json_error("404 Not Found", "unknown_setting"),
            },
            Err(response) => response,
        },
        ("PUT", None) => apply_settings_update(body, service, guard).await,
        _ => json_error("405 Method Not Allowed", "method_not_allowed"),
    }
}

/// The one mutating settings handler.
///
/// Takes [`Authorized`] by value, so it cannot run without the chokepoint
/// having granted this specific action.
async fn apply_settings_update(
    body: &[u8],
    service: &SettingsService,
    guard: &Guard<'_>,
) -> String {
    let authorized = match authorize_audited(guard, Action::UpdateGlobalSettings).await {
        Ok(authorized) => authorized,
        Err(response) => return response,
    };

    let Ok(text) = std::str::from_utf8(body) else {
        return json_error("400 Bad Request", "invalid_request");
    };
    let changes = match parse_settings_body(text) {
        Ok(changes) => changes,
        Err(reason) => return json_error("400 Bad Request", reason),
    };

    match service.update(authorized, changes).await {
        Ok(report) => json_response("200 OK", &settings_report(&report)),
        Err(rejection) => settings_rejection(&rejection),
    }
}

/// Parses `{"changes":{"<key>":"<value>"}}`.
///
/// Values arrive as strings for every kind so that one wire form covers
/// durations, counts, and paths without the client having to know which is
/// which. Parsing is still kind-directed on the server.
fn parse_settings_body(text: &str) -> Result<Vec<ProposedChange>, &'static str> {
    let document: serde_json::Value = match serde_json::from_str(text) {
        Ok(document) => document,
        Err(_) => return Err("invalid_body"),
    };
    let table = document
        .get("changes")
        .and_then(serde_json::Value::as_object)
        .ok_or("missing_changes")?;

    let mut changes = Vec::with_capacity(table.len());
    for (key, value) in table {
        let raw = value.as_str().ok_or("value_must_be_a_string")?;
        let (name, camera) = key
            .split_once('@')
            .map_or((key.as_str(), None), |(name, camera)| (name, Some(camera)));
        let id = SettingId::from_key(name).ok_or("unknown_setting")?;
        let parsed = SettingValue::parse(id, raw).map_err(|_| "invalid_value")?;
        let setting_key = match camera {
            Some(camera) => {
                let camera = CameraId::new(camera).map_err(|_| "invalid_camera_id")?;
                SettingKey::scoped(id, camera).map_err(|_| "not_camera_scoped")?
            }
            None => SettingKey::unscoped(id),
        };
        changes.push(ProposedChange {
            key: setting_key,
            value: parsed,
        });
    }
    Ok(changes)
}

fn settings_rejection(rejection: &UpdateRejection) -> String {
    let (status, code) = match rejection {
        UpdateRejection::UnknownKey { .. } => ("400 Bad Request", "unknown_setting"),
        UpdateRejection::NotOperational { .. } => ("400 Bad Request", "not_operational"),
        UpdateRejection::Duplicate { .. } => ("400 Bad Request", "duplicate_setting"),
        UpdateRejection::Malformed { .. } => ("400 Bad Request", "invalid_value"),
        UpdateRejection::Scope { .. } => ("400 Bad Request", "invalid_scope"),
        UpdateRejection::GroupInvariant { .. } => ("409 Conflict", "group_invariant"),
        UpdateRejection::Unauthorized => ("403 Forbidden", "unauthorized"),
        UpdateRejection::SurfaceDisabled => ("404 Not Found", "not_found"),
        UpdateRejection::ShuttingDown => ("503 Service Unavailable", "shutting_down"),
    };
    // The detail names the offending setting, which is the point of the typed
    // error. It is safe to render because a rejection carries an identity,
    // never a value.
    json_response(
        status,
        &serde_json::json!({ "error": code, "detail": rejection.to_string() }),
    )
}

async fn settings_list(service: &SettingsService) -> String {
    let snapshot = service.snapshot().await;
    let mut entries = Vec::new();
    for id in SettingId::ALL.iter().copied() {
        if let Some((value, origin)) = service.effective(id, None).await {
            entries.push(serde_json::json!({
                "key": id.key(),
                // `value` is already redacted for secrets: SettingValue::render
                // is the only rendering, and it returns "***".
                "value": value,
                "origin": origin.as_str(),
                "boundary": id.boundary().as_str(),
                "class": if id.api_editable() { "operational" } else { "bootstrap" },
                "secret": id.secrecy() == Secrecy::Secret,
            }));
        }
    }
    json_response(
        "200 OK",
        &serde_json::json!({
            "generation": snapshot.generation,
            "freshness": if snapshot.freshness.is_stale() { "stale" } else { "fresh" },
            "cache": snapshot.cache.as_str(),
            "settings": entries,
        }),
    )
}

/// Reports stored and effective values separately.
///
/// A setting whose stored and effective values differ is exactly the condition
/// an operator needs surfaced, so the two are never collapsed into one number.
async fn settings_detail(service: &SettingsService, id: SettingId) -> String {
    let effective = service.effective(id, None).await;
    let stored = service.stored(id, None).await;
    json_response(
        "200 OK",
        &serde_json::json!({
            "key": id.key(),
            "effective": effective.as_ref().map(|(value, _)| value.clone()),
            "origin": effective.as_ref().map(|(_, origin)| origin.as_str()),
            "stored": stored,
            "boundary": id.boundary().as_str(),
            "editable": id.api_editable(),
            "secret": id.secrecy() == Secrecy::Secret,
        }),
    )
}

fn settings_report(report: &ChangeReport) -> String {
    let outcomes: Vec<serde_json::Value> = report
        .outcomes
        .iter()
        .map(|(key, outcome)| {
            serde_json::json!({
                "key": key.id().key(),
                "camera_id": key.camera().map(|camera| camera.as_str().to_owned()),
                "outcome": outcome.as_str(),
            })
        })
        .collect();
    json_response(
        "200 OK",
        &serde_json::json!({ "generation": report.generation, "outcomes": outcomes }),
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "camera routing keeps resource dependencies explicit"
)]
async fn route_camera(
    method: &str,
    path: &str,
    query: &str,
    camera_defaults: &CameraDefaults,
    registry: &CameraHealthRegistry,
    supervisor: &CameraSupervisorHandle,
    recordings: Option<&PostgresRecordingRepository>,
    viewer_tokens: Option<&DevelopmentTokenService>,
    guard: &Guard<'_>,
) -> String {
    let segments: Vec<_> = path.trim_matches('/').split('/').collect();
    if segments.len() < 3 || segments[0] != "api" || segments[1] != "cameras" {
        return json_error("404 Not Found", "not_found");
    }
    let Ok(camera_id) = CameraId::new(segments[2]) else {
        return json_error("400 Bad Request", "invalid_camera_id");
    };
    // Every arm below authorizes before touching a repository or the
    // supervisor, so an out-of-scope camera cannot be probed for existence
    // through a timing or error-shape difference.
    if method == "GET" && segments.as_slice() == ["api", "cameras", segments[2], "recordings"] {
        return match authorize_audited(guard, Action::ViewRecordings(camera_id.clone())).await {
            Ok(_authorized) => {
                recording_search(
                    camera_id,
                    query,
                    camera_defaults.recording_root(),
                    recordings,
                )
                .await
            }
            Err(response) => response,
        };
    }
    if method == "GET" && segments.len() == 5 && segments[3] == "recordings" {
        let Ok(recording_id) = surveillance_common::types::RecordingId::new(segments[4]) else {
            return json_error("400 Bad Request", "invalid_recording_id");
        };
        return match authorize_audited(guard, Action::ViewRecordings(camera_id.clone())).await {
            Ok(_authorized) => {
                recording_detail(
                    camera_id,
                    recording_id,
                    camera_defaults.recording_root(),
                    recordings,
                )
                .await
            }
            Err(response) => response,
        };
    }
    if method == "POST" && segments.as_slice() == ["api", "cameras", segments[2], "viewer-token"] {
        if let Err(response) =
            authorize_audited(guard, Action::IssueViewerToken(camera_id.clone())).await
        {
            return response;
        }
        if registry.snapshot(&camera_id).is_none() {
            return json_error("404 Not Found", "camera_not_found");
        }
        let Some(service) = viewer_tokens else {
            return json_error(
                "503 Service Unavailable",
                "livekit_token_issuer_unavailable",
            );
        };
        return match service.issue_for_camera(&camera_id) {
            Ok(access) => json_response(
                "200 OK",
                &ViewerAccessResponse {
                    endpoint: access.endpoint(),
                    room: access.room(),
                    token: access.token(),
                    expires_in_seconds: access.expires_in_seconds(),
                    development_only: true,
                },
            ),
            Err(_) => json_error("503 Service Unavailable", "livekit_token_issue_failed"),
        };
    }
    let action = match (method, segments.as_slice()) {
        ("GET", ["api", "cameras", _] | ["api", "cameras", _, "health"]) => {
            Action::ViewCamera(camera_id.clone())
        }
        ("POST", ["api", "cameras", _, "start"]) => Action::StartCamera(camera_id.clone()),
        ("POST", ["api", "cameras", _, "stop"]) => Action::StopCamera(camera_id.clone()),
        ("DELETE", ["api", "cameras", _]) => Action::RemoveCamera(camera_id.clone()),
        _ => return json_error("404 Not Found", "not_found"),
    };
    let authorized = match authorize_audited(guard, action).await {
        Ok(authorized) => authorized,
        Err(response) => return response,
    };

    match authorized.action() {
        Action::ViewCamera(_) => registry.snapshot(&camera_id).map_or_else(
            || json_error("404 Not Found", "camera_not_found"),
            |snapshot| json_response("200 OK", &CameraResponse::from(&snapshot)),
        ),
        Action::StartCamera(_) => operation_response(supervisor.start(camera_id).await, "200 OK"),
        Action::StopCamera(_) => operation_response(supervisor.stop(camera_id).await, "200 OK"),
        Action::RemoveCamera(_) => {
            operation_response(supervisor.remove(camera_id).await, "204 No Content")
        }
        // Unreachable: `action` was built from the match above, which produces
        // exactly these four variants. Answering 404 rather than panicking
        // keeps the request path free of a panic if that ever stops holding.
        _ => json_error("404 Not Found", "not_found"),
    }
}

#[derive(Serialize)]
struct ViewerAccessResponse<'a> {
    endpoint: &'a str,
    room: &'a str,
    token: &'a str,
    expires_in_seconds: u64,
    development_only: bool,
}

#[derive(Serialize)]
struct RecordingSearchResponse {
    recordings: Vec<RecordingResponse>,
    next_cursor: Option<String>,
}

#[derive(Serialize)]
struct RecordingResponse {
    recording_id: String,
    camera_id: String,
    started_at: String,
    ended_at: String,
    codec: &'static str,
    width: u32,
    height: u32,
    size_bytes: u64,
    status: &'static str,
    playback: String,
    video: String,
}

async fn recording_search(
    camera_id: CameraId,
    query: &str,
    recording_root: &std::path::Path,
    repository: Option<&PostgresRecordingRepository>,
) -> String {
    let Some(repository) = repository else {
        return json_error("503 Service Unavailable", "recording_index_unavailable");
    };
    let parameters: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
    let (Some(start), Some(end)) = (parameters.get(RANGE_START), parameters.get(RANGE_END)) else {
        return json_error("400 Bad Request", "missing_recording_range");
    };
    let Ok(start) = start.parse::<jiff::Timestamp>() else {
        return json_error("400 Bad Request", "invalid_recording_start");
    };
    let Ok(end) = end.parse::<jiff::Timestamp>() else {
        return json_error("400 Bad Request", "invalid_recording_end");
    };
    let (start, end) = (
        std::time::SystemTime::from(start),
        std::time::SystemTime::from(end),
    );
    let Ok(range) = RecordingTimeRange::new(start, end, Duration::from_secs(24 * 60 * 60)) else {
        return json_error("400 Bad Request", "invalid_recording_range");
    };
    let limit = match parameters.get("limit") {
        Some(value) => value.parse::<usize>().ok(),
        None => Some(100),
    };
    let Some(limit) = limit.and_then(|value| RecordingResultLimit::new(value).ok()) else {
        return json_error("400 Bad Request", "invalid_recording_limit");
    };
    let cursor = match parameters.get("cursor") {
        Some(value) => match decode_recording_cursor(value) {
            Ok(cursor) => Some(cursor),
            Err(()) => return json_error("400 Bad Request", "invalid_recording_cursor"),
        },
        None => None,
    };
    let search = RecordingSearch::new(camera_id, range, limit, cursor);
    match repository.find_overlapping_completed(&search).await {
        Ok(rows) => {
            if !recordings_are_present(repository, recording_root, &rows).await {
                return json_error("409 Conflict", "recording_storage_inconsistent");
            }
            let next_cursor = (rows.len() == limit.get())
                .then(|| {
                    rows.last()
                        .map(|row| encode_recording_cursor(row.started_at(), row.recording_id()))
                })
                .flatten();
            let recordings = rows.iter().map(recording_response).collect();
            json_response(
                "200 OK",
                &RecordingSearchResponse {
                    recordings,
                    next_cursor,
                },
            )
        }
        Err(error) if error.retryable() => {
            json_error("503 Service Unavailable", "recording_index_unavailable")
        }
        Err(_) => json_error("500 Internal Server Error", "recording_query_failed"),
    }
}

async fn recording_detail(
    camera_id: CameraId,
    recording_id: surveillance_common::types::RecordingId,
    recording_root: &std::path::Path,
    repository: Option<&PostgresRecordingRepository>,
) -> String {
    let Some(repository) = repository else {
        return json_error("503 Service Unavailable", "recording_index_unavailable");
    };
    match repository.find_completed(&camera_id, &recording_id).await {
        Ok(Some(row))
            if recordings_are_present(repository, recording_root, std::slice::from_ref(&row))
                .await =>
        {
            json_response("200 OK", &recording_response(&row))
        }
        Ok(Some(_)) => json_error("409 Conflict", "recording_storage_inconsistent"),
        Ok(None) => json_error("404 Not Found", "recording_not_found"),
        Err(error) if error.retryable() => {
            json_error("503 Service Unavailable", "recording_index_unavailable")
        }
        Err(_) => json_error("500 Internal Server Error", "recording_query_failed"),
    }
}

async fn recordings_are_present(
    repository: &PostgresRecordingRepository,
    root: &std::path::Path,
    rows: &[surveillance_storage::metadata::CompletedRecording],
) -> bool {
    let mut all_present = true;
    for row in rows {
        let path = root.join(row.locator().as_path());
        let present = tokio::fs::symlink_metadata(path)
            .await
            .is_ok_and(|metadata| {
                metadata.file_type().is_file() && !metadata.file_type().is_symlink()
            });
        if !present {
            let _ = repository.mark_inconsistent(row.recording_id()).await;
            all_present = false;
        }
    }
    all_present
}

fn recording_response(
    row: &surveillance_storage::metadata::CompletedRecording,
) -> RecordingResponse {
    RecordingResponse {
        recording_id: row.recording_id().as_str().to_owned(),
        camera_id: row.camera_id().as_str().to_owned(),
        started_at: jiff::Timestamp::try_from(row.started_at())
            .map_or_else(|_| "invalid".to_owned(), |value| value.to_string()),
        ended_at: jiff::Timestamp::try_from(row.ended_at())
            .map_or_else(|_| "invalid".to_owned(), |value| value.to_string()),
        codec: "h264",
        width: row.width(),
        height: row.height(),
        size_bytes: row.size_bytes(),
        status: "completed",
        playback: format!(
            "/api/cameras/{}/recordings/playback?start={}&end={}",
            row.camera_id(),
            jiff::Timestamp::try_from(row.started_at())
                .map_or_else(|_| "invalid".to_owned(), |value| value.to_string()),
            jiff::Timestamp::try_from(row.ended_at())
                .map_or_else(|_| "invalid".to_owned(), |value| value.to_string()),
        ),
        video: format!(
            "/api/cameras/{}/recordings/{}/video",
            row.camera_id(),
            row.recording_id()
        ),
    }
}

fn encode_recording_cursor(
    started_at: std::time::SystemTime,
    recording_id: &surveillance_common::types::RecordingId,
) -> String {
    let timestamp = jiff::Timestamp::try_from(started_at).map_or(0, jiff::Timestamp::as_nanosecond);
    // Signed LowerHex formats negatives as two's complement. The decoder uses
    // signed radix parsing, so preserve an explicit minus sign before the epoch.
    if timestamp < 0 {
        format!("-{:x}.{}", timestamp.unsigned_abs(), recording_id.as_str())
    } else {
        format!("{timestamp:x}.{}", recording_id.as_str())
    }
}

fn decode_recording_cursor(value: &str) -> Result<RecordingCursor, ()> {
    let (timestamp, recording_id) = value.split_once('.').ok_or(())?;
    let timestamp = i128::from_str_radix(timestamp, 16).map_err(|_| ())?;
    // Reject untrusted integers before conversion: jiff-core 0.1.0 can panic
    // internally when very large nanosecond values narrow to epoch seconds.
    if !(jiff::Timestamp::MIN.as_nanosecond()..=jiff::Timestamp::MAX.as_nanosecond())
        .contains(&timestamp)
    {
        return Err(());
    }
    let timestamp = jiff::Timestamp::from_nanosecond(timestamp).map_err(|_| ())?;
    let started_at = std::time::SystemTime::from(timestamp);
    let recording_id =
        surveillance_common::types::RecordingId::new(recording_id).map_err(|_| ())?;
    Ok(RecordingCursor::new(started_at, recording_id))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddCameraRequest {
    camera_id: String,
    rtsp_endpoint: String,
    username: Option<String>,
    password: Option<String>,
    width: u32,
    height: u32,
    #[serde(default = "default_detection_enabled")]
    detection_enabled: bool,
    /// Whether this camera publishes to the deployment's `LiveKit`.
    ///
    /// Absent means "whatever this deployment does": publish when `LiveKit` is
    /// configured, and do not when it is not. Before this field existed the
    /// answer was always *no*, so every camera added through the API was
    /// silently created unable to publish while the bootstrap camera published
    /// — a fleet that could not be watched, with nothing anywhere saying why.
    ///
    /// It carries no endpoint and no credential. Those are deployment facts
    /// held in [`CameraDefaults`]; this is the one bit that is per-camera.
    live_publication_enabled: Option<bool>,
}

const fn default_detection_enabled() -> bool {
    true
}

/// Whether a request asks for this camera to publish.
///
/// Split out because the interesting case is the one nobody writes down: an
/// absent field. Defaulting it to `false` is what produced a fleet of cameras
/// that could not be watched, and defaulting it to `true` would make a
/// deployment with no `LiveKit` reject every add. Following the deployment is the
/// only answer that is right in both.
const fn wants_live_publication(requested: Option<bool>, deployment_publishes: bool) -> bool {
    match requested {
        Some(explicit) => explicit,
        None => deployment_publishes,
    }
}

/// Adds a camera.
///
/// Takes [`Authorized`] by value, so it cannot run without the chokepoint
/// having granted this specific action.
async fn add_camera(
    body: &[u8],
    defaults: &CameraDefaults,
    supervisor: &CameraSupervisorHandle,
    _authorized: Authorized,
) -> String {
    let Ok(request) = serde_json::from_slice::<AddCameraRequest>(body) else {
        return json_error("400 Bad Request", "invalid_camera_configuration");
    };
    let credentials = match (request.username, request.password) {
        (None, None) => None,
        (Some(username), Some(password)) => Some(CameraCredentialsSettings { username, password }),
        _ => return json_error("400 Bad Request", "incomplete_camera_credentials"),
    };
    let mut settings = OneCameraSettings::h264_mvp(
        request.camera_id,
        request.rtsp_endpoint,
        credentials,
        request.width,
        request.height,
        defaults.recording_root().to_path_buf(),
    );
    settings.detection_enabled = request.detection_enabled;
    if wants_live_publication(request.live_publication_enabled, defaults.publishes_live()) {
        // Refused rather than ignored. Accepting the camera without publication
        // would answer 201 to a request that asked for something this
        // deployment cannot do, and the operator would find out by watching a
        // tile stay dark. An absent field never reaches here without LiveKit,
        // so only an explicit `true` can be rejected this way.
        let Some(livekit) = defaults.livekit() else {
            return json_error("400 Bad Request", "live_publication_not_configured");
        };
        settings.livekit = Some(livekit);
    }
    let Ok(config) = OneCameraConfig::validate(settings) else {
        return json_error("400 Bad Request", "invalid_camera_configuration");
    };
    operation_response(supervisor.add(Arc::new(config)).await, "201 Created")
}

fn operation_response(result: Result<(), SupervisorRequestError>, success: &str) -> String {
    match result {
        Ok(()) if success.starts_with("204") => response(success, "application/json", ""),
        Ok(()) => json_response(success, &OperationResponse { status: "ok" }),
        Err(SupervisorRequestError::Operation(error)) => match error {
            crate::camera_supervisor::SupervisorOperationError::DuplicateCamera => {
                json_error("409 Conflict", "camera_already_exists")
            }
            crate::camera_supervisor::SupervisorOperationError::UnknownCamera => {
                json_error("404 Not Found", "camera_not_found")
            }
            crate::camera_supervisor::SupervisorOperationError::RegistryFull => {
                json_error("409 Conflict", "camera_capacity_reached")
            }
            _ => json_error("409 Conflict", "camera_operation_failed"),
        },
        Err(SupervisorRequestError::Full | SupervisorRequestError::TimedOut) => {
            json_error("503 Service Unavailable", "camera_supervisor_busy")
        }
        Err(SupervisorRequestError::Closed) => {
            json_error("503 Service Unavailable", "camera_supervisor_unavailable")
        }
    }
}

#[derive(Serialize)]
struct OperationResponse<'a> {
    status: &'a str,
}

#[derive(Serialize)]
struct ErrorResponse<'a> {
    error: &'a str,
}

#[derive(Serialize)]
struct CameraResponse {
    camera_id: String,
    pipeline_state: &'static str,
    connection_state: Option<&'static str>,
    connection_attempts: u64,
    connection_epochs: u64,
    reconnects: u64,
    terminal_failures: u64,
    last_connected_unix_ms: Option<u64>,
    last_media_unix_ms: Option<u64>,
    last_actionable_failure: Option<String>,
    recorder_queue_depth: Option<usize>,
    recorder_queue_capacity: Option<usize>,
    recorder_rejected: u64,
    live_queue_depth: Option<usize>,
    live_queue_capacity: Option<usize>,
    /// Access units the distributor offered this camera's live consumer.
    ///
    /// The denominator for `live_published_access_units`. Publication health
    /// can read as healthy while almost nothing reaches a subscriber, and the
    /// two numbers side by side are what make that visible without scraping
    /// `/metrics`.
    live_offered: u64,
    live_dropped: u64,
    detection_enabled: bool,
    detection_mode: Option<&'static str>,
    detection_overlay_max_age_ms: u64,
    detection_person_state: Option<String>,
    detection_face_state: Option<String>,
    detection_state: Option<String>,
    detection_queue_depth: Option<usize>,
    detection_queue_capacity: Option<usize>,
    detection_dropped: u64,
    detection_inferred: u64,
    detections_published: u64,
    recorder_completed_segments: u64,
    recorder_failed_segments: u64,
    live_publication_enabled: bool,
    live_publication_state: Option<String>,
    live_publication_failure: Option<String>,
    live_publication_timestamp_authority: Option<&'static str>,
    live_published_access_units: u64,
    live_published_bytes: u64,
    /// Access units the publisher offered the transport and it declined.
    ///
    /// Not a failure: the session is intact and the next access unit may well
    /// be published. It is media a subscriber did not receive, which is
    /// otherwise invisible from a healthy-looking publication state.
    live_publication_adapter_drops: u64,
}

impl From<&CameraStatusSnapshot> for CameraResponse {
    #[allow(
        clippy::too_many_lines,
        reason = "flat credential-free API projection keeps bounded health fields explicit"
    )]
    fn from(snapshot: &CameraStatusSnapshot) -> Self {
        let connection = snapshot.health.as_ref().map(|health| &health.connection);
        let counters =
            connection.map(surveillance_camera::connection::ConnectionSnapshot::counters);
        let operational = snapshot.health.as_ref().map(|health| health.camera);
        Self {
            camera_id: snapshot.camera_id.to_string(),
            pipeline_state: pipeline_state_name(snapshot.pipeline_state),
            connection_state: connection.map(|value| connection_state_name(value.state())),
            connection_attempts: counters.map_or(0, |value| value.attempts),
            connection_epochs: counters.map_or(0, |value| value.connected),
            reconnects: counters.map_or(0, |value| value.reconnects),
            terminal_failures: counters.map_or(0, |value| value.terminal_failures),
            last_connected_unix_ms: operational
                .and_then(|value| value.last_connected_at)
                .map(unix_ms),
            last_media_unix_ms: operational
                .and_then(|value| value.last_media_at)
                .map(unix_ms),
            last_actionable_failure: operational
                .and_then(|value| value.last_actionable_failure)
                .map(|value| format!("{value:?}"))
                .or_else(|| snapshot.last_failure.map(|value| format!("{value:?}"))),
            recorder_queue_depth: snapshot
                .health
                .as_ref()
                .map(|value| value.recorder_queue.depth),
            recorder_queue_capacity: snapshot
                .health
                .as_ref()
                .map(|value| value.recorder_queue.capacity),
            recorder_rejected: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.recorder_queue.rejected_full),
            live_queue_depth: snapshot.health.as_ref().map(|value| value.live_queue.depth),
            live_queue_capacity: snapshot
                .health
                .as_ref()
                .map(|value| value.live_queue.capacity),
            live_offered: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.live_queue.received),
            live_dropped: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.live_queue.dropped_oldest),
            detection_enabled: snapshot.health.as_ref().is_some_and(|value| {
                !matches!(
                    value.detection.state,
                    surveillance_detector::detector::DetectionState::Disabled
                )
            }),
            detection_mode: snapshot
                .health
                .as_ref()
                .and_then(|h| h.detection.backend.as_ref())
                .map(|b| b.mode().as_str()),
            detection_person_state: snapshot
                .health
                .as_ref()
                .and_then(|h| h.detection.person_lane)
                .map(|s| format!("{s:?}")),
            detection_face_state: snapshot
                .health
                .as_ref()
                .and_then(|h| h.detection.face_lane)
                .map(|s| format!("{s:?}")),
            detection_overlay_max_age_ms: snapshot.health.as_ref().map_or(3000, |h| {
                u64::try_from(h.detection.overlay_max_age.as_millis()).unwrap_or(u64::MAX)
            }),
            detection_state: snapshot
                .health
                .as_ref()
                .map(|value| format!("{:?}", value.detection.state)),
            detection_queue_depth: snapshot
                .health
                .as_ref()
                .map(|value| value.detection_queue.depth),
            detection_queue_capacity: snapshot
                .health
                .as_ref()
                .map(|value| value.detection_queue.capacity),
            detection_dropped: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.detection_queue.dropped_oldest),
            detection_inferred: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.detection.counters.inferred),
            detections_published: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.detection.counters.events_published),
            recorder_completed_segments: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.recorder.completed_segments),
            recorder_failed_segments: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.recorder.failed_segments),
            live_publication_enabled: snapshot
                .health
                .as_ref()
                .is_some_and(|value| value.publication.enabled),
            live_publication_state: snapshot
                .health
                .as_ref()
                .map(|value| format!("{:?}", value.publication.state)),
            live_publication_failure: snapshot
                .health
                .as_ref()
                .and_then(|value| value.publication.last_failure)
                .map(|value| format!("{value:?}")),
            live_publication_timestamp_authority: snapshot.health.as_ref().map(|value| {
                timestamp_authority_name(value.publication.counters.timestamp_authority)
            }),
            live_published_access_units: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.publication.counters.access_units),
            live_published_bytes: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.publication.counters.bytes),
            live_publication_adapter_drops: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.publication.counters.adapter_drops),
        }
    }
}

fn camera_list(snapshots: &[CameraStatusSnapshot]) -> Vec<CameraResponse> {
    snapshots.iter().map(CameraResponse::from).collect()
}

async fn readiness_response(
    snapshots: &[CameraStatusSnapshot],
    recordings: Option<&PostgresRecordingRepository>,
    metadata: Option<&MetadataWriterHandle>,
    phase4: Option<&Phase4Metrics>,
) -> String {
    let ready = !snapshots.is_empty()
        && snapshots.iter().all(|snapshot| {
            snapshot.pipeline_state == PipelineState::Running
                && snapshot.health.as_ref().is_some_and(|health| {
                    matches!(health.connection.state(), ConnectionState::Connected { .. })
                })
        });
    let database_ready = match recordings {
        Some(repository) => repository.health().await.is_ok(),
        None => true,
    };
    let metadata_ready = metadata.is_none_or(|handle| {
        let snapshot = handle.snapshot();
        snapshot.rejected_full == 0 && snapshot.permanent_failures == 0
    });
    let storage_ready = phase4.is_none_or(|metrics| {
        metrics.snapshot().storage_pressure
            < surveillance_storage::retention::StoragePressure::Critical
    });
    if ready && database_ready && metadata_ready && storage_ready {
        response("200 OK", "text/plain; charset=utf-8", "ready\n")
    } else {
        response(
            "503 Service Unavailable",
            "text/plain; charset=utf-8",
            if !database_ready {
                "recording index is degraded; media recording may continue\n"
            } else if !metadata_ready {
                "recording metadata delivery is degraded; filesystem reconciliation remains available\n"
            } else if !storage_ready {
                "recording storage pressure is critical\n"
            } else {
                "one or more cameras are degraded\n"
            },
        )
    }
}

/// Emits the configuration metrics required by `docs/phase-7/contracts.md` §6.
///
/// Audit-write and notification outcomes are separate series on purpose: a
/// change that is durably recorded but whose notification was dropped is
/// correct behavior, and collapsing them would make it read as a failure.
fn render_configuration_metrics(output: &mut String, view: &ConfigurationMetricsView) {
    let _ = writeln!(
        output,
        "surveillance_configuration_generation {}",
        view.generation
    );
    let _ = writeln!(
        output,
        "surveillance_configuration_freshness {}",
        view.freshness.gauge()
    );
    let _ = writeln!(
        output,
        "surveillance_configuration_store_available {}",
        u8::from(view.store_available)
    );
    let _ = writeln!(
        output,
        "surveillance_configuration_mutating_surface_enabled {}",
        u8::from(view.mutating_surface_enabled)
    );
    let _ = writeln!(
        output,
        "surveillance_configuration_unreadable_rows {}",
        view.unreadable_rows
    );
    if let Some(age) = view.last_success_age_seconds {
        // Absent rather than zero when no load has ever succeeded: zero would
        // read as "just loaded".
        let _ = writeln!(
            output,
            "surveillance_configuration_last_success_age_seconds {age}"
        );
    }
    for state in CacheState::ALL {
        let _ = writeln!(
            output,
            "surveillance_configuration_cache_state{{state=\"{}\"}} {}",
            state.as_str(),
            u8::from(*state == view.cache)
        );
    }
    for (origin, count) in &view.by_origin {
        let _ = writeln!(
            output,
            "surveillance_configuration_origin{{origin=\"{}\"}} {count}",
            origin.as_str()
        );
    }
    for (boundary, count) in &view.pending_by_boundary {
        let _ = writeln!(
            output,
            "surveillance_configuration_pending{{boundary=\"{}\"}} {count}",
            boundary.as_str()
        );
    }
    for (name, value) in [
        ("applied_total", view.applied),
        ("rejected_total", view.rejected),
        ("audit_write_total", view.audit_written),
        ("audit_write_failed_total", view.audit_failed),
        ("notify_published_total", view.notify_published),
        ("notify_dropped_total", view.notify_dropped),
        ("cache_write_total", view.cache_writes),
        ("cache_write_failed_total", view.cache_write_failures),
    ] {
        let _ = writeln!(output, "surveillance_configuration_{name} {value}");
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the flat exporter keeps bounded metric names and labels directly auditable"
)]
fn render_metrics(
    snapshots: &[CameraStatusSnapshot],
    recordings: Option<&PostgresRecordingRepository>,
    metadata: Option<&MetadataWriterHandle>,
    phase4: Option<&Phase4Metrics>,
    event_bus: Option<&surveillance_events::bus::EventBus>,
    configuration: Option<&ConfigurationMetricsView>,
    auth: Option<&AuthMetrics>,
) -> String {
    let mut output = String::with_capacity(snapshots.len().saturating_mul(1_024));
    render_auth_metrics(&mut output, auth);
    if let Some(configuration) = configuration {
        render_configuration_metrics(&mut output, configuration);
    }
    if let Some(event_bus) = event_bus {
        let events = event_bus.metrics();
        let _ = writeln!(
            output,
            "surveillance_event_bus_subscribers {}",
            events.subscribers
        );
        let _ = writeln!(
            output,
            "surveillance_event_publications_total {}",
            events.publications
        );
        let _ = writeln!(
            output,
            "surveillance_event_deliveries_total {}",
            events.outcomes.accepted
        );
        let _ = writeln!(
            output,
            "surveillance_event_rejected_full_total {}",
            events.outcomes.rejected_full
        );
        let _ = writeln!(
            output,
            "surveillance_event_dropped_newest_total {}",
            events.outcomes.dropped_newest
        );
        let _ = writeln!(
            output,
            "surveillance_event_replaced_oldest_total {}",
            events.outcomes.replaced_oldest
        );
        let _ = writeln!(
            output,
            "surveillance_event_coalesced_total {}",
            events.outcomes.coalesced
        );
        for (kind, value) in [
            "camera_state_changed",
            "recording_segment_completed",
            "recording_segment_failed",
            "storage_pressure_changed",
            "person_detection_observed",
            "detector_state_changed",
            "configuration_changed",
            "face_detection_observed",
            "person_presence_observed",
        ]
        .into_iter()
        .zip(events.event_kinds)
        {
            let _ = writeln!(
                output,
                "surveillance_event_publications_by_kind_total{{kind=\"{kind}\"}} {value}"
            );
        }
        for (class, value) in [
            "critical_lifecycle",
            "latest_state",
            "telemetry",
            "detection",
        ]
        .into_iter()
        .zip(events.delivery_classes)
        {
            let _ = writeln!(
                output,
                "surveillance_event_publications_by_class_total{{class=\"{class}\"}} {value}"
            );
        }
    }
    if let Some(repository) = recordings {
        let pool = repository.pool_snapshot();
        let _ = writeln!(output, "surveillance_database_pool_size {}", pool.size);
        let _ = writeln!(output, "surveillance_database_pool_idle {}", pool.idle);
        let _ = writeln!(
            output,
            "surveillance_database_pool_maximum {}",
            pool.maximum
        );
    }
    if let Some(metadata) = metadata {
        let writer = metadata.snapshot();
        let _ = writeln!(
            output,
            "surveillance_metadata_queue_capacity {}",
            writer.queue_capacity
        );
        let _ = writeln!(
            output,
            "surveillance_metadata_accepted_total {}",
            writer.accepted
        );
        let _ = writeln!(
            output,
            "surveillance_metadata_rejected_full_total {}",
            writer.rejected_full
        );
        let _ = writeln!(
            output,
            "surveillance_metadata_persisted_total {}",
            writer.persisted
        );
        let _ = writeln!(
            output,
            "surveillance_metadata_retryable_failures_total {}",
            writer.retryable_failures
        );
    }
    if let Some(metrics) = phase4 {
        let value = metrics.snapshot();
        for (name, metric_value) in [
            (
                "surveillance_reconciliation_active",
                value.reconciliation_active,
            ),
            (
                "surveillance_reconciliation_passes_total",
                value.reconciliation_passes,
            ),
            (
                "surveillance_reconciliation_failures_total",
                value.reconciliation_failures,
            ),
            (
                "surveillance_reconciliation_examined_total",
                value.reconciliation_examined,
            ),
            (
                "surveillance_reconciliation_imported_total",
                value.reconciliation_imported,
            ),
            (
                "surveillance_reconciliation_incomplete_total",
                value.reconciliation_incomplete,
            ),
            (
                "surveillance_reconciliation_inconsistent_total",
                value.reconciliation_inconsistent,
            ),
            (
                "surveillance_reconciliation_skipped_indexed_total",
                value.reconciliation_skipped_indexed,
            ),
            (
                "surveillance_recording_queries_total",
                value.recording_queries,
            ),
            (
                "surveillance_recording_query_failures_total",
                value.recording_query_failures,
            ),
            (
                "surveillance_recording_query_results_total",
                value.recording_query_results,
            ),
            (
                "surveillance_recording_ranges_rejected_total",
                value.recording_ranges_rejected,
            ),
            ("surveillance_playback_active", value.playback_active),
            (
                "surveillance_playback_started_total",
                value.playback_started,
            ),
            (
                "surveillance_playback_completed_total",
                value.playback_completed,
            ),
            (
                "surveillance_playback_cancelled_total",
                value.playback_cancelled,
            ),
            (
                "surveillance_playback_failures_total",
                value.playback_failures,
            ),
            (
                "surveillance_playback_buffer_bytes",
                value.playback_buffer_bytes,
            ),
            (
                "surveillance_retention_passes_total",
                value.retention_passes,
            ),
            (
                "surveillance_retention_failures_total",
                value.retention_failures,
            ),
            (
                "surveillance_retention_candidates_total",
                value.retention_candidates,
            ),
            (
                "surveillance_retention_deleted_total",
                value.retention_deleted,
            ),
            (
                "surveillance_retention_delete_failures_total",
                value.retention_delete_failures,
            ),
            (
                "surveillance_retention_bytes_reclaimed_total",
                value.retention_bytes_reclaimed,
            ),
            (
                "surveillance_storage_total_bytes",
                value.storage_total_bytes,
            ),
            ("surveillance_storage_free_bytes", value.storage_free_bytes),
            (
                "surveillance_storage_pressure",
                value.storage_pressure as u64,
            ),
        ] {
            let _ = writeln!(output, "{name} {metric_value}");
        }
    }
    for status in snapshots {
        let camera = status.camera_id.as_str();
        metric(
            &mut output,
            "surveillance_camera_pipeline_state",
            camera,
            pipeline_state_value(status.pipeline_state),
        );
        let Some(snapshot) = &status.health else {
            continue;
        };
        let connection = snapshot.connection.counters();
        metric(
            &mut output,
            "surveillance_camera_connected",
            camera,
            u8::from(matches!(
                snapshot.connection.state(),
                ConnectionState::Connected { .. }
            )),
        );
        metric(
            &mut output,
            "surveillance_camera_connection_attempts_total",
            camera,
            connection.attempts,
        );
        metric(
            &mut output,
            "surveillance_camera_connection_epochs_total",
            camera,
            connection.connected,
        );
        metric(
            &mut output,
            "surveillance_camera_reconnects_total",
            camera,
            connection.reconnects,
        );
        metric(
            &mut output,
            "surveillance_camera_terminal_failures_total",
            camera,
            connection.terminal_failures,
        );
        metric(
            &mut output,
            "surveillance_camera_access_units_total",
            camera,
            snapshot.camera.access_units,
        );
        metric(
            &mut output,
            "surveillance_camera_packet_loss_events_total",
            camera,
            snapshot.camera.packet_loss_events,
        );
        metric(
            &mut output,
            "surveillance_camera_lost_packets_total",
            camera,
            snapshot.camera.lost_packets,
        );
        metric(
            &mut output,
            "surveillance_camera_live_evictions_total",
            camera,
            snapshot.camera.live_evictions,
        );
        metric(
            &mut output,
            "surveillance_camera_teardown_failures_total",
            camera,
            snapshot.camera.teardown_failures,
        );
        metric(
            &mut output,
            "surveillance_recorder_segments_completed_total",
            camera,
            snapshot.recorder.completed_segments,
        );
        metric(
            &mut output,
            "surveillance_recorder_segments_failed_total",
            camera,
            snapshot.recorder.failed_segments,
        );
        queue_metrics(&mut output, camera, "recorder", snapshot.recorder_queue);
        queue_metrics(&mut output, camera, "live", snapshot.live_queue);
        queue_metrics(&mut output, camera, "detection", snapshot.detection_queue);
        let detection = &snapshot.detection;
        metric(
            &mut output,
            "surveillance_detector_enabled",
            camera,
            u8::from(!matches!(
                detection.state,
                surveillance_detector::detector::DetectionState::Disabled
            )),
        );
        metric(
            &mut output,
            "surveillance_detector_state",
            camera,
            match detection.state {
                surveillance_detector::detector::DetectionState::Disabled => 0_u8,
                surveillance_detector::detector::DetectionState::Starting => 1,
                surveillance_detector::detector::DetectionState::Running => 2,
                surveillance_detector::detector::DetectionState::Degraded => 3,
                surveillance_detector::detector::DetectionState::Terminal => 4,
                surveillance_detector::detector::DetectionState::Stopping => 5,
                surveillance_detector::detector::DetectionState::Stopped => 6,
            },
        );
        for (lane, state, counters) in [
            ("person", detection.person_lane, detection.person_counters),
            ("face", detection.face_lane, detection.face_counters),
        ] {
            let state = state.map_or(0, |s| match s {
                surveillance_detector::detector::DetectionState::Running => 2,
                surveillance_detector::detector::DetectionState::Starting => 1,
                surveillance_detector::detector::DetectionState::Degraded => 3,
                surveillance_detector::detector::DetectionState::Terminal => 4,
                _ => 0,
            });
            for (name, value) in [
                ("state", state),
                ("offered_total", counters.offered),
                ("dropped_total", counters.replaced),
                ("stale_total", counters.stale),
                ("completed_total", counters.completed),
                ("failures_total", counters.failed),
                ("preprocessing_micros_total", counters.preprocessing_micros),
                ("inference_micros_total", counters.inference_micros),
            ] {
                let _=writeln!(output,"surveillance_detection_lane_{name}{{camera_id=\"{camera}\",lane=\"{lane}\"}} {value}");
            }
            for (bound, value) in [
                "10000", "50000", "100000", "250000", "500000", "1000000", "+Inf",
            ]
            .into_iter()
            .zip(counters.latency_buckets)
            {
                let _=writeln!(output,"surveillance_detection_lane_inference_micros_bucket{{camera_id=\"{camera}\",lane=\"{lane}\",le=\"{bound}\"}} {value}");
            }
        }
        detection_gate_metrics(&mut output, camera, detection.counters);
        for (name, value) in [
            (
                "surveillance_detector_encoded_considered_total",
                detection.sampler.considered,
            ),
            (
                "surveillance_detector_sampled_total",
                detection.sampler.sampled,
            ),
            (
                "surveillance_detector_skipped_by_rate_total",
                detection.sampler.skipped_by_rate,
            ),
            (
                "surveillance_detector_duplicate_timestamps_total",
                detection.sampler.duplicate,
            ),
            (
                "surveillance_detector_resets_total",
                detection.sampler.resets,
            ),
            (
                "surveillance_detector_decode_attempts_total",
                detection.counters.decode_attempts,
            ),
            (
                "surveillance_detector_decoded_total",
                detection.counters.decoded,
            ),
            (
                "surveillance_detector_decode_failures_total",
                detection.counters.decode_failed,
            ),
            (
                "surveillance_detector_inference_attempts_total",
                detection.counters.inference_attempts,
            ),
            (
                "surveillance_detector_inferred_total",
                detection.counters.inferred,
            ),
            (
                "surveillance_detector_detections_produced_total",
                detection.counters.detections_produced,
            ),
            (
                "surveillance_detector_inference_failures_total",
                detection.counters.inference_failed,
            ),
            (
                "surveillance_detector_events_published_total",
                detection.counters.events_published,
            ),
            (
                "surveillance_detector_event_delivery_losses_total",
                detection.counters.event_delivery_losses,
            ),
            (
                "surveillance_detector_decode_duration_micros_total",
                detection.counters.decode_duration_micros,
            ),
            (
                "surveillance_detector_inference_duration_micros_total",
                detection.counters.inference_duration_micros,
            ),
            (
                "surveillance_detector_end_to_event_duration_micros_total",
                detection.counters.end_to_event_duration_micros,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_0_10ms_total",
                detection.counters.latency_le_10ms,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_10_50ms_total",
                detection.counters.latency_le_50ms,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_50_100ms_total",
                detection.counters.latency_le_100ms,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_100_250ms_total",
                detection.counters.latency_le_250ms,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_250_500ms_total",
                detection.counters.latency_le_500ms,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_500_1000ms_total",
                detection.counters.latency_le_1000ms,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_over_1000ms_total",
                detection.counters.latency_over_1000ms,
            ),
        ] {
            metric(&mut output, name, camera, value);
        }
        let publication = snapshot.publication;
        metric(
            &mut output,
            "surveillance_livekit_enabled",
            camera,
            u8::from(publication.enabled),
        );
        metric(
            &mut output,
            "surveillance_livekit_state",
            camera,
            publication_state_value(publication.state),
        );
        metric(
            &mut output,
            "surveillance_livekit_media_timestamp_authority",
            camera,
            u8::from(matches!(
                publication.counters.timestamp_authority,
                surveillance_livekit::adapter::TimestampAuthority::Media
            )),
        );
        // A gauge, not a total, and deliberately not in the array below: it
        // goes down. Zero here with a connected state is the viewer-gated
        // publisher idling on purpose, which is the only way to tell that
        // apart from a publisher that has quietly stopped producing media.
        metric(
            &mut output,
            "surveillance_livekit_viewers",
            camera,
            publication.counters.adapter_viewers,
        );
        for (name, value) in [
            (
                "surveillance_livekit_connection_attempts_total",
                publication.counters.connection_attempts,
            ),
            (
                "surveillance_livekit_connection_successes_total",
                publication.counters.connection_successes,
            ),
            (
                "surveillance_livekit_reconnects_total",
                publication.counters.reconnects,
            ),
            (
                "surveillance_livekit_access_units_total",
                publication.counters.access_units,
            ),
            (
                "surveillance_livekit_bytes_total",
                publication.counters.bytes,
            ),
            (
                "surveillance_livekit_keyframes_total",
                publication.counters.keyframes,
            ),
            (
                "surveillance_livekit_parameter_set_waits_total",
                publication.counters.parameter_set_waits,
            ),
            (
                "surveillance_livekit_stale_drops_total",
                publication.counters.stale_drops,
            ),
            (
                // Resynchronisations forced by the media timeline itself. A
                // reconnecting camera supplies a new RTP base, so this rising
                // once per reconnect is expected; rising while the camera holds
                // one session is a source whose timestamps cannot be trusted for
                // pacing. Before this counter existed the forward-jump case was
                // invisible: the publisher slept out the jump and stopped
                // draining the live lane while still reporting `Publishing`.
                "surveillance_livekit_timestamp_discontinuities_total",
                publication.counters.timestamp_discontinuities,
            ),
            (
                "surveillance_livekit_media_rejections_total",
                publication.counters.media_rejections,
            ),
            (
                // Access units the publisher offered to the transport and the
                // transport declined. Against
                // `surveillance_livekit_access_units_total` this is the
                // difference between offering media and publishing it: a
                // publisher can report itself healthy while a subscriber sees
                // nothing, and this counter is where that shows up.
                "surveillance_livekit_adapter_drops_total",
                publication.counters.adapter_drops,
            ),
            (
                // Media the adapter produced from what it was offered. For a
                // transcoding adapter these are decoded pictures handed to the
                // encoder, so this over the offered total — `access_units` plus
                // `adapter_drops`, since `access_units` counts only what was
                // published — is the fraction of the stream a subscriber saw.
                // May continue briefly with zero viewers during the configured
                // warm-session grace; it stops after that bound.
                "surveillance_livekit_published_frames_total",
                publication.counters.adapter_media_produced,
            ),
            (
                // Machinery the adapter restarted without failing the session.
                // Climbing here while the state stays connected is a degraded
                // live view that reports itself healthy.
                "surveillance_livekit_adapter_restarts_total",
                publication.counters.adapter_restarts,
            ),
            (
                "surveillance_livekit_detection_packets_dropped_total",
                publication.counters.detection_packets_dropped,
            ),
            (
                "surveillance_livekit_send_failures_total",
                publication.counters.send_failures,
            ),
            (
                "surveillance_livekit_shutdown_failures_total",
                publication.counters.shutdown_failures,
            ),
        ] {
            metric(&mut output, name, camera, value);
        }
    }
    output
}

const fn timestamp_authority_name(
    authority: surveillance_livekit::adapter::TimestampAuthority,
) -> &'static str {
    use surveillance_livekit::adapter::TimestampAuthority;
    match authority {
        TimestampAuthority::Media => "media",
        TimestampAuthority::AdapterClock => "adapter_clock",
    }
}

const fn publication_state_value(state: surveillance_livekit::domain::PublicationState) -> u8 {
    use surveillance_livekit::domain::PublicationState as S;
    match state {
        S::Disabled => 0,
        S::Starting => 1,
        S::Connecting => 2,
        S::Publishing => 3,
        S::Backoff => 4,
        S::Degraded => 5,
        S::Terminal => 6,
        S::Stopping => 7,
        S::Stopped => 8,
    }
}

fn metric(output: &mut String, name: &str, camera: &str, value: impl std::fmt::Display) {
    let _ = writeln!(output, "{name}{{camera_id=\"{camera}\"}} {value}");
}

fn detection_gate_metrics(
    output: &mut String,
    camera: &str,
    counters: surveillance_detector::detector::DetectionCounters,
) {
    metric(
        output,
        "surveillance_detector_output_capacity_dropped_total",
        camera,
        counters.output_capacity_dropped,
    );
    metric(
        output,
        "surveillance_detector_gate_skipped_total",
        camera,
        counters.gate_skipped,
    );
    metric(
        output,
        "surveillance_detector_gate_heartbeat_total",
        camera,
        counters.gate_heartbeat,
    );
}

fn queue_metrics(
    output: &mut String,
    camera: &str,
    consumer: &str,
    queue: surveillance_media::distribution::QueueMetrics,
) {
    for (suffix, value) in [
        ("capacity", queue.capacity as u64),
        ("depth", queue.depth as u64),
        ("accepted_total", queue.accepted),
        ("received_total", queue.received),
        ("rejected_full_total", queue.rejected_full),
        ("dropped_oldest_total", queue.dropped_oldest),
        ("dropped_undecodable_total", queue.dropped_undecodable),
        ("dropped_on_close_total", queue.dropped_on_close),
    ] {
        let _ = writeln!(output, "surveillance_media_queue_{suffix}{{camera_id=\"{camera}\",consumer=\"{consumer}\"}} {value}");
    }
}

const fn pipeline_state_name(state: PipelineState) -> &'static str {
    match state {
        PipelineState::Configured => "configured",
        PipelineState::Starting => "starting",
        PipelineState::Running => "running",
        PipelineState::Stopping => "stopping",
        PipelineState::Stopped => "stopped",
        PipelineState::Degraded => "degraded",
        PipelineState::Terminal => "terminal",
    }
}

const fn pipeline_state_value(state: PipelineState) -> u8 {
    match state {
        PipelineState::Configured => 0,
        PipelineState::Starting => 1,
        PipelineState::Running => 2,
        PipelineState::Stopping => 3,
        PipelineState::Stopped => 4,
        PipelineState::Degraded => 5,
        PipelineState::Terminal => 6,
    }
}

const fn connection_state_name(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::Connecting { .. } => "connecting",
        ConnectionState::Connected { .. } => "connected",
        ConnectionState::Reconnecting { .. } => "reconnecting",
        ConnectionState::Stopped { .. } => "stopped",
    }
}

fn unix_ms(value: std::time::SystemTime) -> u64 {
    value
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn json_response(status: &str, value: &impl Serialize) -> String {
    match serde_json::to_string(value) {
        Ok(body) => response(status, "application/json", &body),
        Err(_) => json_error("500 Internal Server Error", "response_encoding_failed"),
    }
}

fn json_error(status: &str, error: &str) -> String {
    json_response(status, &ErrorResponse { error })
}

fn response(status: &str, content_type: &str, body: &str) -> String {
    // `no-store` on every response this helper builds. The JSON here is
    // per-principal and describes footage, so it does not belong in a shared or
    // on-disk browser cache; and the dashboard is a compile-time constant that
    // changes on upgrade, where a cached copy means an operator reading a stale
    // interface with no way to know it. Found by upgrading the page underneath
    // a running browser, which kept serving the previous one.
    format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}", body.len())
}

/// Exercises the production playback parsers without sockets, files, or a database.
///
/// Only the isolated fuzz workspace enables this entry point. Input is bounded
/// by the ordinary default HTTP request limit; no media or model work is started.
#[cfg(feature = "fuzzing")]
#[doc(hidden)]
pub fn fuzz_playback_request(input: &[u8]) {
    if input.len() > DEFAULT_MAX_REQUEST_BYTES {
        return;
    }
    let _ = content_length(input);
    if let Some(head) = request_head(input) {
        let _ = header_value(head, "content-length");
    }
    if let Some(target) = playback_target(input) {
        let query = match target {
            PlaybackTarget::Range { query, .. } | PlaybackTarget::Recording { query, .. } => query,
        };
        fuzz_playback_query(query);
    }
    if let Ok(text) = std::str::from_utf8(input) {
        fuzz_playback_query(text);
    }
}

#[cfg(feature = "fuzzing")]
fn fuzz_playback_query(query: &str) {
    if let Ok(RecordingVideoExtent::Leading(duration)) = parse_recording_video_extent(query) {
        assert!((1..=ClipDuration::MAX_SECONDS).contains(&duration.seconds()));
    }
    let parameters = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    if let Some((start, end, range)) = parse_range(&parameters) {
        assert!(start < end);
        assert_eq!(range.start(), std::time::SystemTime::from(start));
        assert_eq!(range.end(), std::time::SystemTime::from(end));
        assert!(range
            .end()
            .duration_since(range.start())
            .is_ok_and(|span| !span.is_zero() && span <= Duration::from_secs(24 * 60 * 60)));
    }
    for value in std::iter::once(query).chain(parameters.get("cursor").map(String::as_str)) {
        if let Ok(cursor) = decode_recording_cursor(value) {
            let encoded = encode_recording_cursor(cursor.started_at(), cursor.recording_id());
            assert_eq!(decode_recording_cursor(&encoded).ok(), Some(cursor));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_recording_cursor, encode_recording_cursor, handle_client, parse_range,
        parse_recording_video_extent, playback_target, recording_video_headers, route,
        route_with_settings, serve_health, test_defaults, CameraDefaults, PlaybackTarget,
        SecurityOwner, LIVEKIT_CLIENT, UI_ASSETS,
    };
    use crate::authorize::{authorize_call_count, MutatingSurface};
    use crate::camera_supervisor::{CameraSupervisor, PipelineState};
    use crate::settings_service::SettingsService;
    use crate::trust_boundary::TrustBoundary;
    use std::{
        net::{IpAddr, Ipv4Addr, SocketAddr},
        num::NonZeroUsize,
        path::PathBuf,
        sync::Arc,
        time::Duration,
    };
    use surveillance_config::{
        cache::{Cache, StaleRetry},
        classification::{Origin, SettingId},
        defaults,
        resolution::{Layer, SettingKey},
        secret::Secret,
        settings::{OneCameraConfig, OneCameraSettings},
        value::SettingValue,
    };
    use tokio::sync::watch;

    use crate::playback::{ClipDuration, RecordingVideoExtent};

    fn loopback() -> IpAddr {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    }

    /// A service with no store: enough to exercise routing, authorization, and
    /// read rendering without a database.
    fn settings_service(surface: MutatingSurface) -> SettingsService {
        let mut environment = Layer::new(Origin::Environment);
        // A secret supplied by the environment, so the read tests have
        // something that must never be rendered.
        environment
            .insert(
                SettingKey::unscoped(SettingId::CameraRtspPassword),
                SettingValue::Secret(Secret::new("top-secret-password".to_owned())),
            )
            .unwrap_or_else(|error| panic!("test entry: {error}"));
        // Pin one setting from the environment so the shadowing path is
        // reachable in tests.
        environment
            .insert(
                SettingKey::unscoped(SettingId::RetentionInterval),
                SettingValue::Duration(Duration::from_secs(600)),
            )
            .unwrap_or_else(|error| panic!("test entry: {error}"));

        let mut cache_path = std::env::temp_dir();
        cache_path.push(format!("surveillance-api-cache-{}", std::process::id()));
        SettingsService::new(
            vec![defaults::layer()],
            environment,
            None,
            Cache::new(cache_path),
            Secret::new("test-salt".to_owned()),
            None,
            StaleRetry::new(Duration::from_secs(5), Duration::from_secs(30)),
            surface,
        )
    }

    fn get(path: &str) -> String {
        format!("GET {path} HTTP/1.1\r\n\r\n")
    }

    fn put(path: &str, body: &str) -> String {
        format!(
            "PUT {path} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    /// Exit gate 15: moving a value into configuration must change
    /// configurability, not behavior.
    #[tokio::test]
    async fn moved_api_bounds_default_to_the_constants_they_replaced() {
        let service = settings_service(MutatingSurface::Disabled);
        let (bytes, timeout) = super::request_bounds(Some(&service)).await;
        assert_eq!(bytes, super::DEFAULT_MAX_REQUEST_BYTES);
        assert_eq!(timeout, super::DEFAULT_IO_TIMEOUT);
        assert_eq!(bytes, 16 * 1024, "the previous MAX_REQUEST_BYTES");
        assert_eq!(timeout, Duration::from_secs(5), "the previous IO_TIMEOUT");
        let stall = super::playback_stall_timeout(Some(&service)).await;
        assert_eq!(stall, super::DEFAULT_PLAYBACK_STALL_TIMEOUT);
    }

    /// The playback body must not be bounded by `api.io_timeout`.
    ///
    /// It was, and the effect was not a slow stream but a wrong one: a media
    /// element stops reading once its buffer is full, the five-second budget
    /// expired against that perfectly healthy client, the assembler was
    /// cancelled and the socket closed, and the browser treated the few seconds
    /// it had buffered as the entire recording — observed in the dashboard as
    /// playback that ran for about three seconds and rewound, on a run of 12
    /// segments covering six minutes. This asserts the two budgets are read
    /// separately and that the playback one is the larger; it does not assert
    /// the number, which `crates/config/src/defaults.rs` documents as a guess.
    #[tokio::test]
    async fn playback_is_not_bounded_by_the_json_response_timeout() {
        let service = settings_service(MutatingSurface::Disabled);
        let (_, io_timeout) = super::request_bounds(Some(&service)).await;
        let stall = super::playback_stall_timeout(Some(&service)).await;
        assert!(
            stall > io_timeout,
            "a paced media reader must outlast api.io_timeout ({io_timeout:?}), got {stall:?}"
        );
        assert_eq!(
            super::playback_stall_timeout(None).await,
            super::DEFAULT_PLAYBACK_STALL_TIMEOUT,
            "the no-settings fallback must not silently drop back to the JSON budget"
        );
    }

    /// `api.max_request_bytes` and `api.io_timeout` are the two settings
    /// declared `Immediate`, so they must be read per request rather than
    /// captured once.
    #[tokio::test]
    async fn immediate_api_bounds_are_read_from_configuration() {
        let mut file = Layer::new(Origin::File);
        file.insert(
            SettingKey::unscoped(SettingId::ApiMaxRequestBytes),
            SettingValue::Unsigned(4_096),
        )
        .unwrap_or_else(|error| panic!("test entry: {error}"));
        file.insert(
            SettingKey::unscoped(SettingId::ApiIoTimeout),
            SettingValue::Duration(Duration::from_secs(11)),
        )
        .unwrap_or_else(|error| panic!("test entry: {error}"));
        file.insert(
            SettingKey::unscoped(SettingId::ApiPlaybackStallTimeout),
            SettingValue::Duration(Duration::from_secs(123)),
        )
        .unwrap_or_else(|error| panic!("test entry: {error}"));

        let mut cache_path = std::env::temp_dir();
        cache_path.push(format!("surveillance-bounds-cache-{}", std::process::id()));
        let service = SettingsService::new(
            vec![defaults::layer(), file],
            Layer::new(Origin::Environment),
            None,
            Cache::new(cache_path),
            Secret::new(String::new()),
            None,
            StaleRetry::new(Duration::from_secs(5), Duration::from_secs(30)),
            MutatingSurface::Disabled,
        );
        let (bytes, timeout) = super::request_bounds(Some(&service)).await;
        assert_eq!(bytes, 4_096);
        assert_eq!(timeout, Duration::from_secs(11));
        assert_eq!(
            super::playback_stall_timeout(Some(&service)).await,
            Duration::from_secs(123)
        );
    }

    /// `docs/phase-7/contracts.md` §6. Without this the counters increment but
    /// nothing observes them, and the contract document would be describing
    /// metrics that are not exposed.
    #[tokio::test]
    async fn configuration_metrics_are_exposed_with_bounded_cardinality() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Disabled);

        let response = route_with_settings(
            get("/metrics").as_bytes(),
            &test_defaults("target/phase7-tests"),
            &handle.health_registry(),
            &handle,
            &service,
            Some(loopback()),
        )
        .await;

        for series in [
            "surveillance_configuration_generation",
            "surveillance_configuration_freshness",
            "surveillance_configuration_store_available",
            "surveillance_configuration_mutating_surface_enabled",
            "surveillance_configuration_cache_state{state=\"valid\"}",
            "surveillance_configuration_origin{origin=\"default\"}",
            "surveillance_configuration_pending{boundary=\"next_segment\"}",
            "surveillance_configuration_audit_write_total",
            "surveillance_configuration_audit_write_failed_total",
            "surveillance_configuration_notify_published_total",
            "surveillance_configuration_notify_dropped_total",
            "surveillance_configuration_cache_write_failed_total",
        ] {
            assert!(response.contains(series), "missing {series}");
        }

        // Audit and notification outcomes must be separate series: a change
        // that committed but whose notification dropped is correct behavior.
        assert_ne!(
            response.find("surveillance_configuration_audit_write_total"),
            response.find("surveillance_configuration_notify_published_total")
        );

        // Bounded cardinality: no setting value, and no camera identifier, may
        // appear as a label.
        assert!(
            !response.contains("top-secret-password"),
            "metrics leaked a credential"
        );
        for line in response.lines() {
            if line.starts_with("surveillance_configuration_") {
                let labels = line.matches('{').count();
                assert!(labels <= 1, "unbounded label structure: {line}");
            }
        }

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    #[tokio::test]
    async fn settings_reads_never_render_a_secret() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Enabled);

        for path in ["/api/settings", "/api/settings/camera.rtsp.password"] {
            let response = route_with_settings(
                get(path).as_bytes(),
                &test_defaults("target/phase7-tests"),
                &handle.health_registry(),
                &handle,
                &service,
                Some(loopback()),
            )
            .await;
            assert!(
                !response.contains("top-secret-password"),
                "{path} leaked a credential: {response}"
            );
            assert!(response.contains("200 OK"), "{path} failed: {response}");
        }

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    #[tokio::test]
    async fn a_disabled_mutating_surface_answers_404_and_writes_nothing() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Disabled);

        let response = route_with_settings(
            put(
                "/api/settings",
                r#"{"changes":{"retention.batch_files":"25"}}"#,
            )
            .as_bytes(),
            &test_defaults("target/phase7-tests"),
            &handle.health_registry(),
            &handle,
            &service,
            Some(loopback()),
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 404"),
            "a disabled surface must not advertise itself: {response}"
        );
        assert!(
            !response.contains("unauthorized"),
            "404 must be indistinguishable from an unknown path: {response}"
        );

        // Reads stay available: observing configuration carries no write risk.
        let read = route_with_settings(
            get("/api/settings").as_bytes(),
            &test_defaults("target/phase7-tests"),
            &handle.health_registry(),
            &handle,
            &service,
            Some(loopback()),
        )
        .await;
        assert!(read.starts_with("HTTP/1.1 200"), "reads stay available");

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    #[tokio::test]
    async fn a_non_loopback_caller_is_refused() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Enabled);

        let response = route_with_settings(
            put(
                "/api/settings",
                r#"{"changes":{"retention.batch_files":"25"}}"#,
            )
            .as_bytes(),
            &test_defaults("target/phase7-tests"),
            &handle.health_registry(),
            &handle,
            &service,
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))),
        )
        .await;
        // ADR 009 changed the code, not the control: a caller with no
        // credential is *unauthenticated*, so `401` is the accurate answer.
        // Phase 7 said `403` because peer address was the only identity
        // signal there was. The refusal itself is what this test guards.
        assert!(
            response.starts_with("HTTP/1.1 401"),
            "a remote caller must be refused: {response}"
        );

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    /// Failure-matrix item 12: proven by test, not by inspection.
    ///
    /// The type system already makes it impossible for a handler to *skip*
    /// `authorize` — [`Authorized`] has no public constructor. This proves the
    /// mutating route actually *runs* it.
    #[tokio::test]
    async fn every_mutating_settings_request_reaches_the_chokepoint() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Disabled);

        let before = authorize_call_count();
        for body in [
            r#"{"changes":{"retention.batch_files":"25"}}"#,
            r#"{"changes":{}}"#,
            "not json at all",
        ] {
            let _ = route_with_settings(
                put("/api/settings", body).as_bytes(),
                &test_defaults("target/phase7-tests"),
                &handle.health_registry(),
                &handle,
                &service,
                Some(loopback()),
            )
            .await;
        }
        assert!(
            authorize_call_count() >= before + 3,
            "every mutating request must reach the chokepoint, including malformed ones"
        );

        // A read does not go through the mutating path. Counting calls would
        // race with other tests sharing the process-wide counter, so this
        // asserts the observable consequence instead: with the surface
        // disabled, anything routed through `authorize` answers 404, and this
        // read does not.
        let read = route_with_settings(
            get("/api/settings").as_bytes(),
            &test_defaults("target/phase7-tests"),
            &handle.health_registry(),
            &handle,
            &service,
            Some(loopback()),
        )
        .await;
        assert!(
            read.starts_with("HTTP/1.1 200"),
            "a read must not be gated by the mutating surface: {read}"
        );

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    /// The playback stream reaches the chokepoint.
    ///
    /// This drives `handle_client` over a real socket rather than entering at
    /// `route_with_recordings`, because the playback path short-circuits
    /// *above* the router. The chokepoint test enters below that branch and so
    /// was structurally incapable of seeing it — which is how this endpoint,
    /// the one that streams actual footage, was the only unauthenticated one.
    ///
    /// A regression here is not "a test went red". It is footage served to
    /// anyone who can reach the socket.
    #[tokio::test]
    async fn the_playback_stream_reaches_the_chokepoint() {
        use super::{handle_client, PlaybackAssembler, PostgresRecordingRepository};
        use std::num::NonZeroU32;
        use tokio::io::AsyncReadExt as _;

        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Enabled);

        // Lazy: neither needs the database to be reachable, and the request is
        // refused before either is consulted.
        let database = surveillance_database::DatabaseConfig::new(
            "postgres://localhost/surveillance-phase8-test",
            NonZeroU32::new(1).expect("one is non-zero"),
            std::time::Duration::from_secs(1),
        )
        .expect("test database configuration is valid");
        let recordings = PostgresRecordingRepository::connect_lazy(&database);
        let assembler = PlaybackAssembler::new(
            std::path::PathBuf::from("target/phase8-tests"),
            std::path::PathBuf::from("ffmpeg"),
            std::time::Duration::from_secs(5),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener binds");
        let address = listener.local_addr().expect("listener has an address");

        let registry = handle.health_registry();
        let served = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("a client connects");
            // No `SecurityOwner`, and a non-loopback peer, so the development
            // identity does not apply and nothing is authenticated.
            let _ = handle_client(
                stream,
                &test_defaults("target/phase8-tests"),
                &registry,
                &handle,
                Some(&recordings),
                Some(&assembler),
                None,
                None,
                None,
                Some(&service),
                Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))),
                None,
                None,
                None,
                false,
            )
            .await;
            handle
        });

        let mut client = tokio::net::TcpStream::connect(address)
            .await
            .expect("client connects");
        let request = "GET /api/cameras/camera-01/recordings/playback?start=2026-01-01T00:00:00Z\
                       &end=2026-01-01T00:01:00Z HTTP/1.1\r\nHost: localhost\r\n\r\n";
        tokio::io::AsyncWriteExt::write_all(&mut client, request.as_bytes())
            .await
            .expect("request is written");

        let mut response = String::new();
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.read_to_string(&mut response),
        )
        .await;

        assert!(
            response.starts_with("HTTP/1.1 401"),
            "unauthenticated playback must be refused, got: {response}"
        );
        // And specifically not media: a body of footage here would mean the
        // refusal happened after the stream had already started.
        assert!(
            !response.contains("video/"),
            "footage was served to an unauthenticated caller: {response}"
        );

        let handle = served.await.expect("the server task completes");
        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    /// Builds a security owner whose boundary trusts one proxy address.
    ///
    /// Holds three principals the proxy may assert: `alice`, an administrator
    /// over every camera; `bob`, a viewer scoped to `camera-01` alone; and
    /// `carol`, an administrator who has been suspended.
    fn proxy_security(peer: IpAddr, proof: &str) -> SecurityOwner {
        let boundary = TrustBoundary::reverse_proxy(
            vec![peer],
            surveillance_common::secret::Secret::new(proof.to_owned()),
        )
        .expect("the test boundary is well formed");
        SecurityOwner::new(boundary, Some(identity_store()))
    }

    /// The same identity store behind a *loopback* boundary.
    ///
    /// Distinct from [`SecurityOwner::loopback_development`], which has no
    /// store at all and therefore grants the development identity. Here there
    /// is a store, so nothing is granted for free — which is what makes it
    /// possible to see whether the asserted header was consulted.
    fn loopback_security() -> SecurityOwner {
        SecurityOwner::new(TrustBoundary::Loopback, Some(identity_store()))
    }

    fn identity_store() -> Arc<dyn crate::authenticate::RequestAuthenticator> {
        use crate::authenticate::{Authenticator, VerifyConcurrency};
        use surveillance_auth::{
            clock::SystemClock,
            principal::{CameraScope, Principal, PrincipalId, PrincipalKind, Role},
            store::InMemoryTokenStore,
            verify::{HashParams, Verifier},
        };
        use surveillance_common::types::CameraId;

        let store = InMemoryTokenStore::new();
        let insert = |id: &str, name: &str, role, scope, enabled| {
            store.insert_principal(
                Principal::new(
                    PrincipalId::new(id).expect("valid principal id"),
                    PrincipalKind::Human,
                    name,
                    role,
                    scope,
                    enabled,
                )
                .expect("principal is valid"),
            );
        };
        insert(
            "alice",
            "Alice",
            Role::Administrator,
            CameraScope::All,
            true,
        );
        insert(
            "bob",
            "Bob",
            Role::Viewer,
            CameraScope::only([CameraId::new("camera-01").expect("valid camera id")])
                .expect("a one-camera scope is within bounds"),
            true,
        );
        insert(
            "carol",
            "Carol",
            Role::Administrator,
            CameraScope::All,
            false,
        );

        // Cheap parameters: this test is about who is trusted, not about how
        // expensive hashing is, and no hashing happens on the asserted path.
        let params = HashParams::new(64, 1, 1).expect("cheap test parameters are valid");
        let verifier = Arc::new(Verifier::new(params).expect("verifier builds"));
        let authenticator = Authenticator::new(
            store,
            SystemClock,
            verifier,
            VerifyConcurrency::from_budget(params, 1024 * 1024),
        );
        Arc::new(authenticator)
    }

    /// Drives one request through a real socket and returns the raw response.
    ///
    /// Entering at the socket rather than at a routing function is the whole
    /// point: the defects this phase found all lived *above* the seam the tests
    /// entered at, so a test that cannot see `handle_client` cannot see them.
    async fn requests_through_socket(
        security: SecurityOwner,
        peer: IpAddr,
        requests: &[String],
    ) -> Vec<String> {
        use tokio::io::AsyncReadExt as _;

        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let mut responses = Vec::with_capacity(requests.len());

        for request in requests {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("test listener binds");
            let address = listener.local_addr().expect("listener has an address");
            let registry = handle.health_registry();

            // Joined rather than spawned so `security` may be borrowed: it is
            // the subject of these tests and must be the same owner throughout.
            let serve = async {
                let (stream, _) = listener.accept().await.expect("a client connects");
                let _ = handle_client(
                    stream,
                    &test_defaults("target/phase8-tests"),
                    &registry,
                    &handle,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(peer),
                    Some(&security),
                    None,
                    None,
                    false,
                )
                .await;
            };
            let client_side = async {
                let mut client = tokio::net::TcpStream::connect(address)
                    .await
                    .expect("client connects");
                tokio::io::AsyncWriteExt::write_all(&mut client, request.as_bytes())
                    .await
                    .expect("request is written");
                let mut response = String::new();
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    client.read_to_string(&mut response),
                )
                .await;
                response
            };
            let ((), response) = tokio::join!(serve, client_side);
            responses.push(response);
        }

        let _ = handle.shutdown().await;
        let _ = task.await;
        responses
    }

    /// One request, for the common case.
    async fn request_through_socket(
        security: SecurityOwner,
        peer: IpAddr,
        request: &str,
    ) -> String {
        requests_through_socket(security, peer, &[request.to_owned()])
            .await
            .pop()
            .unwrap_or_default()
    }

    const PROXY_PEER: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7));
    const PROXY_PROOF: &str = "0123456789abcdef0123456789abcdef";

    /// The property the whole proxy-authentication design rests on.
    ///
    /// The asserted-principal header is worth exactly as much as the proof of
    /// origin beside it. Without the proof it must be *discarded*, not merely
    /// distrusted later, because anything that can reach the socket can set a
    /// header. If this test fails, naming a principal is enough to become one.
    #[tokio::test]
    async fn an_asserted_principal_without_proof_of_origin_is_ignored() {
        let security = proxy_security(PROXY_PEER, PROXY_PROOF);
        let response = request_through_socket(
            security,
            PROXY_PEER,
            "GET /api/cameras HTTP/1.1\r\nHost: localhost\r\n\
             X-Authenticated-Principal: alice\r\n\r\n",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 401"),
            "an unproved assertion authenticated a caller: {response}"
        );
    }

    /// The same request from an address that is not the configured proxy.
    ///
    /// Distinct from the case above: here the proof header is *correct*, and
    /// the only thing wrong is where the connection came from. Both halves of
    /// the proof have to hold independently.
    #[tokio::test]
    async fn a_correct_proof_from_the_wrong_peer_does_not_authenticate() {
        let security = proxy_security(PROXY_PEER, PROXY_PROOF);
        let response = request_through_socket(
            security,
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)),
            &format!(
                "GET /api/cameras HTTP/1.1\r\nHost: localhost\r\n\
                 X-Forwarded-Proof: {PROXY_PROOF}\r\n\
                 X-Authenticated-Principal: alice\r\n\r\n"
            ),
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 401"),
            "a request from an unlisted peer was authenticated: {response}"
        );
    }

    /// A wrong proof secret from the right peer, for the same reason reversed.
    #[tokio::test]
    async fn a_wrong_proof_from_the_right_peer_does_not_authenticate() {
        let security = proxy_security(PROXY_PEER, PROXY_PROOF);
        let response = request_through_socket(
            security,
            PROXY_PEER,
            "GET /api/cameras HTTP/1.1\r\nHost: localhost\r\n\
             X-Forwarded-Proof: fedcba9876543210fedcba9876543210\r\n\
             X-Authenticated-Principal: alice\r\n\r\n",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 401"),
            "a mismatched proof was accepted: {response}"
        );
    }

    /// The path actually working, so the tests above are proving a restriction
    /// rather than an endpoint that refuses everything.
    #[tokio::test]
    async fn a_proved_proxy_assertion_authenticates_the_named_principal() {
        let security = proxy_security(PROXY_PEER, PROXY_PROOF);
        let response = request_through_socket(
            security,
            PROXY_PEER,
            &format!(
                "GET /api/cameras HTTP/1.1\r\nHost: localhost\r\n\
                 X-Forwarded-Proof: {PROXY_PROOF}\r\n\
                 X-Authenticated-Principal: alice\r\n\r\n"
            ),
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "a proved assertion was refused: {response}"
        );
    }

    /// A proxy that authenticates somebody this system has never heard of.
    ///
    /// Refused rather than admitted with a default role: the proxy proves
    /// *that* it authenticated someone, never *what they may do*. Roles and
    /// camera scope stay in the database on this side of the boundary.
    #[tokio::test]
    async fn a_proved_assertion_for_an_unknown_principal_is_refused() {
        let security = proxy_security(PROXY_PEER, PROXY_PROOF);
        let response = request_through_socket(
            security,
            PROXY_PEER,
            &format!(
                "GET /api/cameras HTTP/1.1\r\nHost: localhost\r\n\
                 X-Forwarded-Proof: {PROXY_PROOF}\r\n\
                 X-Authenticated-Principal: mallory\r\n\r\n"
            ),
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 401"),
            "an unknown principal was admitted by proxy assertion: {response}"
        );
    }

    /// Under a loopback boundary the header must never be consulted at all.
    ///
    /// There is no proxy here, so `ProofOutcome::NotConfigured` — which
    /// deliberately does not prove origin — must drop the assertion. The store
    /// holds `alice`, so if the header *were* consulted the request would
    /// succeed; a 401 is the only outcome that proves it was not.
    #[tokio::test]
    async fn a_loopback_deployment_never_reads_an_asserted_principal() {
        let response = request_through_socket(
            loopback_security(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            "GET /api/cameras HTTP/1.1\r\nHost: localhost\r\n\
             X-Authenticated-Principal: alice\r\n\r\n",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 401"),
            "a loopback deployment honoured an asserted principal: {response}"
        );
    }

    /// And the development posture itself still works: no store, no header,
    /// loopback caller, granted as the conspicuous development identity.
    ///
    /// Paired with the test above so a regression cannot be "fixed" by making
    /// loopback refuse everything.
    #[tokio::test]
    async fn a_loopback_deployment_without_a_store_keeps_the_development_identity() {
        let response = request_through_socket(
            SecurityOwner::loopback_development(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            "GET /api/cameras HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "the loopback development posture regressed: {response}"
        );
    }

    /// `/api/system` must report detection as a fixture.
    ///
    /// The single most dangerous thing a UI could do with this system is render
    /// a person count, because the fake detections are correctly shaped and
    /// correctly timestamped and look entirely real. This field is what lets a
    /// client refuse to.
    #[tokio::test]
    async fn the_capabilities_endpoint_reports_detection_as_a_fixture() {
        let response = request_through_socket(
            SecurityOwner::loopback_development(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            "GET /api/system HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "the capabilities endpoint did not answer: {response}"
        );
        assert!(
            response.contains("\"results_are_fixture\":true"),
            "detection was not reported as a fixture: {response}"
        );
        assert!(
            response.contains("\"is_production_model\":false"),
            "a non-production model was not declared as such: {response}"
        );
        // And the playback honesty, in the same place.
        assert!(
            response.contains("\"seekable\":false"),
            "playback was not declared unseekable: {response}"
        );
    }

    /// A camera you may not see and a camera that does not exist must be
    /// **byte-identical** in the response.
    ///
    /// Not merely "both are 404". If the two differ in any byte — a different
    /// error code, a different message, a different header, a different length
    /// — then the difference is an oracle: a viewer scoped to one camera can
    /// enumerate the rest of the installation by asking about each in turn and
    /// watching which answer comes back. That is a floor plan of a building
    /// under surveillance, handed out to someone entitled to one camera.
    ///
    /// This is why the UI can never say "you lack access to camera-07": the
    /// server has taken care not to tell it that camera-07 exists.
    #[tokio::test]
    async fn an_out_of_scope_camera_is_indistinguishable_from_one_that_does_not_exist() {
        let security = proxy_security(PROXY_PEER, PROXY_PROOF);
        // Alice creates camera-02. Bob is scoped to camera-01 alone, so
        // camera-02 exists but is not his; camera-99 was never created.
        let body = br#"{"camera_id":"camera-02","rtsp_endpoint":"rtsp://127.0.0.1:19222/camera-02","width":640,"height":360}"#;
        let create = format!(
            "POST /api/cameras HTTP/1.1\r\nHost: localhost\r\n\
             X-Forwarded-Proof: {PROXY_PROOF}\r\n\
             X-Authenticated-Principal: alice\r\n\
             Content-Length: {}\r\n\r\n{}",
            body.len(),
            String::from_utf8_lossy(body)
        );
        let ask = |camera: &str| {
            format!(
                "GET /api/cameras/{camera} HTTP/1.1\r\nHost: localhost\r\n\
                 X-Forwarded-Proof: {PROXY_PROOF}\r\n\
                 X-Authenticated-Principal: bob\r\n\r\n"
            )
        };

        let responses = requests_through_socket(
            security,
            PROXY_PEER,
            &[create, ask("camera-02"), ask("camera-99")],
        )
        .await;

        assert!(
            responses[0].starts_with("HTTP/1.1 20"),
            "the administrator could not create the camera: {}",
            responses[0]
        );
        assert_eq!(
            responses[1], responses[2],
            "an out-of-scope camera is distinguishable from an absent one:\n\
             out of scope: {}\nabsent:       {}",
            responses[1], responses[2]
        );
        assert!(
            responses[1].starts_with("HTTP/1.1 404"),
            "neither answer was a plain 404: {}",
            responses[1]
        );
        assert!(
            !responses[1].contains("camera-02"),
            "the response names the camera the caller may not see: {}",
            responses[1]
        );
    }

    /// A suspended principal the proxy still authenticates.
    ///
    /// The proxy has no way to know the identity store disabled somebody, so
    /// this must be caught on this side of the boundary. If it is not, then
    /// revoking access means editing proxy configuration, and this system's
    /// own suspension control is decorative.
    #[tokio::test]
    async fn a_suspended_principal_is_refused_even_when_the_proxy_asserts_them() {
        let security = proxy_security(PROXY_PEER, PROXY_PROOF);
        let response = request_through_socket(
            security,
            PROXY_PEER,
            &format!(
                "GET /api/cameras HTTP/1.1\r\nHost: localhost\r\n\
                 X-Forwarded-Proof: {PROXY_PROOF}\r\n\
                 X-Authenticated-Principal: carol\r\n\r\n"
            ),
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 401"),
            "a suspended principal was admitted by proxy assertion: {response}"
        );
    }

    /// Playback must declare itself unseekable in the response itself.
    ///
    /// `X-Playback-Accuracy: segment-aligned` was prose; a browser does not
    /// read prose. `Accept-Ranges: none` is the same statement in a form the
    /// HTTP stack acts on, and it is what stops a client from rendering a
    /// scrubber over a stream that cannot honour a seek.
    #[test]
    fn the_playback_response_declares_itself_unseekable() {
        let headers = super::playback_headers("2026-01-01T00:00:00Z", "2026-01-01T00:01:00Z");
        assert!(
            headers.contains("Accept-Ranges: none\r\n"),
            "playback did not refuse range requests: {headers}"
        );
        assert!(
            headers.contains("X-Playback-Accuracy: segment-aligned\r\n"),
            "playback did not declare its accuracy: {headers}"
        );
        assert!(
            !headers.contains("Content-Length"),
            "a chunked stream must not claim a length: {headers}"
        );
    }

    #[test]
    fn recording_video_route_identifies_one_recording() {
        let request = b"GET /api/cameras/camera-01/recordings/recording-01/video?duration_seconds=5 HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let Some(PlaybackTarget::Recording {
            camera_id,
            recording_id,
            query,
        }) = playback_target(request)
        else {
            panic!("recording video route was not recognized");
        };
        assert_eq!(camera_id.as_str(), "camera-01");
        assert_eq!(recording_id.as_str(), "recording-01");
        assert_eq!(query, "duration_seconds=5");
    }

    #[test]
    fn recording_video_duration_is_optional_strict_and_bounded() {
        assert_eq!(
            parse_recording_video_extent(""),
            Ok(RecordingVideoExtent::Full)
        );
        assert_eq!(
            parse_recording_video_extent("duration_seconds=10"),
            Ok(RecordingVideoExtent::Leading(
                ClipDuration::new(10).unwrap_or_else(|_| panic!("ten seconds must be valid"))
            ))
        );
        for query in [
            "duration_seconds=0",
            "duration_seconds=61",
            "duration_seconds=5.5",
            "duration_seconds=5&duration_seconds=10",
            "other=5",
        ] {
            assert!(
                parse_recording_video_extent(query).is_err(),
                "query unexpectedly accepted: {query}"
            );
        }
    }

    #[test]
    fn recording_video_response_is_honest_about_transport_and_accuracy() {
        let recording_id = surveillance_common::types::RecordingId::new("recording-01")
            .unwrap_or_else(|_| panic!("fixture id must be valid"));
        let headers = recording_video_headers(
            &recording_id,
            RecordingVideoExtent::Leading(
                ClipDuration::new(5).unwrap_or_else(|_| panic!("fixture duration must be valid")),
            ),
        );
        assert!(headers.contains("Accept-Ranges: none"));
        assert!(headers.contains("X-Playback-Accuracy: packet-bounded"));
        assert!(headers.contains("X-Requested-Duration: 5s"));
        assert!(headers.contains("X-Recording-Id: recording-01"));
    }

    /// The capabilities endpoint must describe the backend that is actually
    /// compiled in, not a string somebody remembered to update.
    ///
    /// A hardcoded `"fake"` would keep saying `fake` after a real model landed,
    /// and — worse — could be edited to say `production` without a single line
    /// of inference changing. Asking the implementation is the only version of
    /// this that stays true by construction.
    /// DI.R7: the reported claim is derived from the live backends.
    ///
    /// This replaces `the_compiled_backend_matches_what_the_pipeline_builds`,
    /// which asserted that the endpoint described the one implementation the
    /// build contained. That invariant does not survive a real backend: a build
    /// can carry ONNX support and run with no weights, so what the build
    /// contains stopped being evidence of what is running. Each assertion below
    /// carries one of the claims that test used to make, restated against the
    /// live object.
    #[test]
    fn detection_is_reported_from_the_running_backends() {
        use super::detection_capability;
        use surveillance_detector::inference::{
            DeterministicFakeInference, InferenceDescriptor, PersonInference,
        };

        /// A backend that claims production, so the aggregate can be exercised
        /// without a model file. Nothing in the tree reports production unless
        /// it is actually running inference.
        struct ProductionLike(DeterministicFakeInference);
        impl PersonInference for ProductionLike {
            fn model_name(&self) -> &surveillance_events::event::ModelName {
                self.0.model_name()
            }
            fn model_version(&self) -> &surveillance_events::event::ModelVersion {
                self.0.model_version()
            }
            fn infer(
                &mut self,
                frame: &surveillance_media::frame::DecodedFrame,
            ) -> Result<
                surveillance_events::event::DetectionSet,
                surveillance_detector::inference::InferenceError,
            > {
                self.0.infer(frame)
            }
            fn is_production_model(&self) -> bool {
                true
            }
        }

        let fixture = InferenceDescriptor::of(
            &DeterministicFakeInference::new().expect("the fixture backend builds"),
        );
        let production = InferenceDescriptor::of(&ProductionLike(
            DeterministicFakeInference::new().expect("the fixture backend builds"),
        ));

        // No detector running still reports a fixture. A client branches on
        // this field, and `false` would tell it the results are genuine — about
        // a deployment producing none at all.
        let none = detection_capability(&[]);
        assert!(!none.is_production_model);
        assert!(none.results_are_fixture);

        // A fixture backend reports itself, which is what the old test pinned.
        let fixtures = detection_capability(&[Some(fixture.clone()), Some(fixture.clone())]);
        assert!(!fixtures.is_production_model);
        assert!(fixtures.results_are_fixture);

        // A camera with detection disabled does not drag a production
        // deployment down, and does not lift a fixture one up.
        let production_only = detection_capability(&[Some(production.clone()), None]);
        assert!(production_only.is_production_model);
        assert!(!production_only.results_are_fixture);

        // Mixed deployments under-claim. One stream of fabricated detections is
        // enough to mislead whoever reads the results.
        let mixed = detection_capability(&[Some(production), Some(fixture)]);
        assert!(
            !mixed.is_production_model,
            "a deployment with any fixture backend must not report production"
        );
        assert!(mixed.results_are_fixture);
        assert_eq!(mixed.model_name, "mixed");
    }

    #[test]
    fn terminal_detector_is_excluded_from_live_backend_status() {
        use super::live_detection_backend;
        use surveillance_detector::{
            detector::{DetectionHealth, DetectionState},
            inference::{DeterministicFakeInference, InferenceDescriptor},
        };

        let descriptor = InferenceDescriptor::of(
            &DeterministicFakeInference::new().expect("the fixture backend builds"),
        );
        let mut terminal = DetectionHealth::disabled_snapshot();
        terminal.state = DetectionState::Terminal;
        terminal.backend = Some(descriptor.clone());

        assert!(live_detection_backend(&terminal).is_none());

        terminal.state = DetectionState::Running;
        assert_eq!(live_detection_backend(&terminal), Some(descriptor));
    }

    #[test]
    fn detector_gate_counters_are_exported_with_bounded_camera_labels() {
        let counters = surveillance_detector::detector::DetectionCounters {
            gate_skipped: 7,
            gate_heartbeat: 2,
            ..Default::default()
        };
        let mut output = String::new();

        super::detection_gate_metrics(&mut output, "camera-01", counters);

        assert!(
            output.contains("surveillance_detector_gate_skipped_total{camera_id=\"camera-01\"} 7")
        );
        assert!(output
            .contains("surveillance_detector_gate_heartbeat_total{camera_id=\"camera-01\"} 2"));
    }

    /// The capabilities endpoint is authenticated like every other read.
    #[tokio::test]
    async fn the_capabilities_endpoint_reaches_the_chokepoint() {
        let security = proxy_security(PROXY_PEER, PROXY_PROOF);
        let response = request_through_socket(
            security,
            PROXY_PEER,
            "GET /api/system HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 401"),
            "the capabilities endpoint answered an anonymous caller: {response}"
        );
    }

    /// No authorization decision may claim the `export` category.
    ///
    /// `access_audit` refuses an `export` row that lacks range and byte count,
    /// and a decision row cannot have them — the bytes are not known until
    /// after streaming. Mapping `ExportRecording` to `export` therefore made
    /// every playback request fail its audit write; because an export is a
    /// read, the failure was logged and swallowed and the request still
    /// succeeded, so the only symptom was an empty table.
    ///
    /// This asserts the invariant directly, so the mapping cannot drift back.
    #[test]
    fn no_authorization_decision_is_categorised_as_an_export() {
        use super::{Action, AuditCategory, CameraId};
        let camera = CameraId::new("camera-01").expect("valid");
        let every = [
            Action::ListCameras,
            Action::ViewCamera(camera.clone()),
            Action::ViewRecordings(camera.clone()),
            Action::IssueViewerToken(camera.clone()),
            Action::ExportRecording(camera.clone()),
            Action::ViewSettings,
            Action::ViewIdentities,
            Action::StartCamera(camera.clone()),
            Action::StopCamera(camera.clone()),
            Action::AddCamera,
            Action::RemoveCamera(camera.clone()),
            Action::UpdateGlobalSettings,
            Action::UpdateCameraSettings(camera),
            Action::ManageIdentities,
        ];
        for action in every {
            assert_ne!(
                super::audit_category(&action),
                AuditCategory::Export,
                "{} would write an export row with no extent, which the schema refuses",
                action.as_str()
            );
        }
    }

    /// The export row itself carries a complete extent, or is not written.
    #[test]
    fn an_export_row_is_only_built_from_a_delivered_range() {
        // Refused: no range was ever parsed, so there is nothing truthful to
        // record and no row is produced. Inventing a range to satisfy the
        // schema would be fabricated metadata.
        assert!(super::Exported::refused().extent().is_none());

        let start = "2026-01-01T00:00:00Z"
            .parse::<jiff::Timestamp>()
            .expect("valid");
        let end = "2026-01-01T00:01:00Z"
            .parse::<jiff::Timestamp>()
            .expect("valid");
        let extent = super::Exported::delivered(start, end, 4_096)
            .extent()
            .expect("a delivered range yields an extent");
        assert_eq!(extent.byte_count, 4_096);
        assert!(extent.range_end >= extent.range_start);
    }

    /// P8.7: the new security series are bounded and name no identity.
    #[test]
    fn security_metrics_have_bounded_cardinality_and_leak_no_identity() {
        let metrics = crate::authenticate::AuthMetrics::default();
        let mut rendered = String::new();
        super::render_auth_metrics(&mut rendered, Some(&metrics));

        // The no-identity-store deployment must still export every series.
        // Running the server is what caught this: `None` here previously meant
        // the whole security block was omitted.
        let mut without_store = String::new();
        super::render_auth_metrics(&mut without_store, None);
        for series in [
            "surveillance_auth_success_total",
            "surveillance_auth_failure_total",
            "surveillance_auth_unavailable_total",
            "surveillance_authz_decision_total",
            "surveillance_proxy_proof_total",
            "surveillance_auth_proxy_asserted_total",
        ] {
            assert!(
                without_store.contains(series),
                "{series} is omitted when no identity store is configured"
            );
        }

        // One line per enum variant, and the variants are fixed. Nothing here
        // scales with the number of principals, tokens, cameras, or requests.
        // Eight since the proxy-asserted path added `unknown_principal`: a
        // proxy that authenticated somebody this system does not hold.
        assert_eq!(
            rendered
                .lines()
                .filter(|line| line.starts_with("surveillance_auth_failure_total{"))
                .count(),
            8
        );
        assert!(
            rendered.contains("surveillance_auth_failure_total{reason=\"unknown_principal\"}"),
            "the proxy-asserted failure reason is not exported: {rendered}"
        );
        assert_eq!(
            rendered
                .lines()
                .filter(|line| line.starts_with("surveillance_authz_decision_total{"))
                .count(),
            6
        );
        assert_eq!(
            rendered
                .lines()
                .filter(|line| line.starts_with("surveillance_proxy_proof_total{"))
                .count(),
            6
        );

        // The only label key in the security series is a fixed one. A
        // principal, token, or camera label would be unbounded cardinality and
        // would put an identity into every scrape and dashboard.
        for line in rendered.lines().filter(|line| line.contains('{')) {
            let Some((_, labels)) = line.split_once('{') else {
                continue;
            };
            assert!(
                labels.starts_with("reason=") || labels.starts_with("outcome="),
                "unexpected label in security series: {line}"
            );
        }
        // `principal_disabled` and `unknown_token` are bounded *reason* values
        // and belong here. What must never appear is an actual identity: the
        // `principal:<kind>:<id>` audit form, or a token identifier.
        assert!(
            !rendered.contains("principal:"),
            "an audit identity reached a metric label"
        );
        assert!(
            !rendered.contains(surveillance_auth::token::TOKEN_PREFIX),
            "a token reached a metric label"
        );

        // Every alert expression in docs/phase-8/alerts.yml names one of these.
        for series in [
            "surveillance_auth_failure_total",
            "surveillance_auth_unavailable_total",
            "surveillance_authz_decision_total",
            "surveillance_proxy_proof_total",
        ] {
            assert!(rendered.contains(series), "{series} is not exported");
        }
    }

    /// The P8.3 acceptance test: every protected route reaches the chokepoint.
    ///
    /// The Phase 7 version covered the three mutating settings requests. This
    /// covers the whole protected surface, so a route added without a check
    /// fails here rather than shipping.
    ///
    /// It works by refusing the request at the chokepoint — no credential, and
    /// a non-loopback peer so the development identity does not apply — and
    /// asserting the refusal. A handler reached without `authorize` would
    /// answer `200`, `201`, `404 camera_not_found`, or `503`, none of which is
    /// an authorization refusal.
    #[tokio::test]
    async fn every_protected_route_reaches_the_chokepoint() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Enabled);
        let remote = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9));

        // (method, path, body). Every route under /api.
        let protected: Vec<(&str, &str, &str)> = vec![
            ("GET", "/api/cameras", ""),
            ("POST", "/api/cameras", r#"{"camera_id":"camera-01"}"#),
            ("GET", "/api/cameras/camera-01", ""),
            ("GET", "/api/cameras/camera-01/health", ""),
            ("GET", "/api/cameras/camera-01/recordings", ""),
            ("GET", "/api/cameras/camera-01/recordings/rec-01", ""),
            ("POST", "/api/cameras/camera-01/viewer-token", ""),
            ("POST", "/api/cameras/camera-01/start", ""),
            ("POST", "/api/cameras/camera-01/stop", ""),
            ("DELETE", "/api/cameras/camera-01", ""),
            ("GET", "/api/settings", ""),
            ("GET", "/api/settings/retention.batch_files", ""),
            (
                "PUT",
                "/api/settings",
                r#"{"changes":{"retention.batch_files":"25"}}"#,
            ),
            // P8.6 identity surface. These reach the chokepoint even with no
            // identity store configured: an early "store unavailable" would
            // answer before authorization ran and tell an anonymous caller
            // whether a store exists.
            ("GET", "/api/identities", ""),
            (
                "POST",
                "/api/identities",
                r#"{"principal_id":"x","role":"viewer"}"#,
            ),
            ("GET", "/api/identities/operator/tokens", ""),
            ("POST", "/api/identities/operator/tokens", "{}"),
            (
                "DELETE",
                "/api/identities/operator/tokens/0123456789abcdef",
                "",
            ),
            ("POST", "/api/identities/operator/disable", ""),
            ("POST", "/api/identities/operator/enable", ""),
            // P8 capabilities. It exposes no footage and no identity, but it
            // does describe the deployment's shape, which is reconnaissance.
            ("GET", "/api/system", ""),
        ];

        let before = authorize_call_count();
        for (method, path, body) in &protected {
            let request = format!(
                "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let response = route_with_settings(
                request.as_bytes(),
                &test_defaults("target/phase8-tests"),
                &handle.health_registry(),
                &handle,
                &service,
                Some(remote),
            )
            .await;
            assert!(
                response.starts_with("HTTP/1.1 401")
                    || response.starts_with("HTTP/1.1 403")
                    || response.starts_with("HTTP/1.1 404 "),
                "{method} {path} did not reach the chokepoint: {response}"
            );
        }
        assert!(
            authorize_call_count() >= before + protected.len() as u64,
            "each protected route must call authorize exactly once or more"
        );

        // The unauthenticated endpoints of ADR 009 Decision 2 stay open. The
        // dashboard shell joins them on the same test: a constant response that
        // names nothing. Everything it later displays is fetched through a
        // route in the table above.
        for path in ["/live", "/ready", "/metrics", "/", "/ui"] {
            let response = route_with_settings(
                get(path).as_bytes(),
                &test_defaults("target/phase8-tests"),
                &handle.health_registry(),
                &handle,
                &service,
                Some(remote),
            )
            .await;
            assert!(
                response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.1 503"),
                "{path} must not require a credential: {response}"
            );
        }

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    // ---------------------------------------------------------------------
    // The operator dashboard.
    //
    // These are assertions about the shipped bytes, not about the interface.
    // There is no JavaScript engine in this workspace, so nothing below
    // executes the page, and none of it is evidence that a screen renders
    // correctly. What it does catch is the small set of edits that would make
    // the interface lie — each of which is a plausible, well-intentioned
    // change that no compiler would object to.
    // ---------------------------------------------------------------------

    #[tokio::test]
    async fn the_dashboard_is_served_at_the_root() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Enabled);

        for path in ["/", "/ui"] {
            let response = route_with_settings(
                get(path).as_bytes(),
                &test_defaults("target/phase8-tests"),
                &handle.health_registry(),
                &handle,
                &service,
                None,
            )
            .await;
            assert!(
                response.starts_with("HTTP/1.1 200 OK"),
                "{path}: {response}"
            );
            assert!(
                response.contains("Content-Type: text/html; charset=utf-8"),
                "{path} must be served as HTML"
            );
            assert!(
                response.contains("<title>SENTINEL Surveillance System</title>"),
                "{path} did not return the dashboard"
            );
        }

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    #[tokio::test]
    async fn every_embedded_ui_asset_has_a_unique_exact_route() {
        let mut paths = std::collections::HashSet::new();
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Enabled);

        for asset in UI_ASSETS {
            assert!(
                paths.insert(asset.path),
                "duplicate UI route: {}",
                asset.path
            );
            assert!(asset.path == "/" || asset.path.starts_with("/ui"));
            let response = route_with_settings(
                get(asset.path).as_bytes(),
                &test_defaults("target/ui-asset-tests"),
                &handle.health_registry(),
                &handle,
                &service,
                None,
            )
            .await;
            assert!(
                response.starts_with("HTTP/1.1 200 OK"),
                "{} was not served: {response}",
                asset.path
            );
            assert!(
                response.contains(&format!("Content-Type: {}", asset.content_type)),
                "{} had the wrong content type",
                asset.path
            );
            assert!(response.contains("Cache-Control: no-store"));
            assert!(
                response.ends_with(asset.body),
                "{} was not served byte-for-byte",
                asset.path
            );
        }

        for unknown in [
            "/ui/js/not-shipped.js",
            "/ui/js/screens/../app.js",
            "/ui/styles/base.css/extra",
        ] {
            let response = route_with_settings(
                get(unknown).as_bytes(),
                &test_defaults("target/ui-asset-tests"),
                &handle.health_registry(),
                &handle,
                &service,
                None,
            )
            .await;
            assert!(
                response.starts_with("HTTP/1.1 404 Not Found"),
                "unexpected UI route matched {unknown}: {response}"
            );
        }

        let response = route_with_settings(
            b"POST /ui/js/app.js HTTP/1.1\r\nContent-Length: 0\r\n\r\n",
            &test_defaults("target/ui-asset-tests"),
            &handle.health_registry(),
            &handle,
            &service,
            None,
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 404 Not Found"),
            "UI assets must be GET-only: {response}"
        );

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    #[tokio::test]
    async fn no_response_may_be_cached_by_a_browser() {
        // An operator reading a cached dashboard after an upgrade has no way to
        // tell. The same header keeps per-principal JSON out of a shared cache.
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Enabled);

        for path in ["/", "/ui", "/live", "/api/cameras"] {
            let response = route_with_settings(
                get(path).as_bytes(),
                &test_defaults("target/phase8-tests"),
                &handle.health_registry(),
                &handle,
                &service,
                None,
            )
            .await;
            assert!(
                response.contains("Cache-Control: no-store"),
                "{path} must not be cacheable: {response}"
            );
        }

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    #[tokio::test]
    async fn the_live_player_sdk_is_served_as_the_bytes_that_were_vendored() {
        // The page cannot reach a CDN, so the SDK ships inside the binary. That
        // moves the trust question from "is the CDN honest today" to "are these
        // the bytes that were reviewed" — which is answerable, and answered
        // here against the digest recorded in `ui/vendor/README.md`. An edit to
        // the bundle without an edit to its provenance fails this test.
        use sha2::{Digest, Sha256};

        let readme = include_str!("../ui/vendor/README.md");
        let recorded = readme
            .lines()
            .find_map(|line| line.strip_prefix("- SHA-256: `"))
            .and_then(|rest| rest.strip_suffix('`'))
            .expect("ui/vendor/README.md must record the bundle digest");
        let actual = format!("{:x}", Sha256::digest(LIVEKIT_CLIENT.as_bytes()));
        assert_eq!(
            recorded, actual,
            "the vendored LiveKit SDK does not match its recorded provenance"
        );

        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Enabled);

        let response = route_with_settings(
            get("/ui/livekit-client.js").as_bytes(),
            &test_defaults("target/phase8-tests"),
            &handle.health_registry(),
            &handle,
            &service,
            None,
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(
            response.contains("Content-Type: application/javascript; charset=utf-8"),
            "the SDK must be served as JavaScript"
        );
        assert!(
            response.ends_with(LIVEKIT_CLIENT),
            "the SDK must be served unmodified"
        );

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    #[tokio::test]
    async fn a_setting_detail_reports_stored_and_effective_separately() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Enabled);

        let response = route_with_settings(
            get("/api/settings/retention.interval").as_bytes(),
            &test_defaults("target/phase7-tests"),
            &handle.health_registry(),
            &handle,
            &service,
            Some(loopback()),
        )
        .await;
        // The environment pins this setting, so effective is the environment
        // value and stored is null.
        assert!(response.contains("\"effective\":\"10m\""), "{response}");
        assert!(
            response.contains("\"origin\":\"environment\""),
            "{response}"
        );
        assert!(response.contains("\"stored\":null"), "{response}");

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    #[tokio::test]
    async fn an_unknown_setting_key_is_404_not_500() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Enabled);

        let response = route_with_settings(
            get("/api/settings/retention.max_agee").as_bytes(),
            &test_defaults("target/phase7-tests"),
            &handle.health_registry(),
            &handle,
            &service,
            Some(loopback()),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 404"), "{response}");

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    #[tokio::test]
    async fn a_bootstrap_setting_is_refused_through_the_api() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Enabled);

        let response = route_with_settings(
            put(
                "/api/settings",
                r#"{"changes":{"server.health_bind":"0.0.0.0:80"}}"#,
            )
            .as_bytes(),
            &test_defaults("target/phase7-tests"),
            &handle.health_registry(),
            &handle,
            &service,
            Some(loopback()),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 400"), "{response}");
        assert!(response.contains("not_operational"), "{response}");

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    #[tokio::test]
    async fn a_group_invariant_breach_is_409_and_names_the_group() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let service = settings_service(MutatingSurface::Enabled);

        let response = route_with_settings(
            put(
                "/api/settings",
                r#"{"changes":{"storage.watermark_warning":"99"}}"#,
            )
            .as_bytes(),
            &test_defaults("target/phase7-tests"),
            &handle.health_registry(),
            &handle,
            &service,
            Some(loopback()),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 409"), "{response}");
        assert!(response.contains("storage_watermarks"), "{response}");

        let _ = handle.shutdown().await;
        let _ = task.await;
    }

    #[test]
    fn recording_cursor_round_trips_across_unix_epoch() {
        let recording_id = surveillance_common::types::RecordingId::new("cursor-recording")
            .unwrap_or_else(|error| panic!("recording ID: {error}"));
        for nanos in [-1_000_000_000_i128, -1, 0, 1, 1_000_000_000] {
            let timestamp = jiff::Timestamp::from_nanosecond(nanos)
                .unwrap_or_else(|error| panic!("fixture timestamp: {error}"));
            let started_at = std::time::SystemTime::from(timestamp);
            let encoded = encode_recording_cursor(started_at, &recording_id);
            let decoded = decode_recording_cursor(&encoded)
                .unwrap_or_else(|()| panic!("cursor must decode for {nanos}: {encoded}"));
            assert_eq!(decoded.started_at(), started_at);
            assert_eq!(decoded.recording_id(), &recording_id);
        }
    }

    #[test]
    fn recording_cursor_rejects_extreme_timestamps_without_panicking() {
        for value in [
            "-1CCCCCCCCCCCCCCCCCCCCCC.segment-1".to_owned(),
            "1CCCCCCCCCCCCCCCCCCCCCC.segment-1".to_owned(),
            format!("{:x}.segment-1", i128::MAX),
            format!("-{:x}.segment-1", i128::MAX),
        ] {
            assert!(decode_recording_cursor(&value).is_err(), "{value}");
        }
        let id = surveillance_common::types::RecordingId::new("segment-1")
            .unwrap_or_else(|error| panic!("recording ID: {error}"));
        for timestamp in [jiff::Timestamp::MIN, jiff::Timestamp::MAX] {
            let started_at = std::time::SystemTime::from(timestamp);
            let encoded = encode_recording_cursor(started_at, &id);
            let decoded = decode_recording_cursor(&encoded)
                .unwrap_or_else(|()| panic!("boundary cursor: {encoded}"));
            assert_eq!(decoded.started_at(), started_at);
        }
    }

    #[test]
    fn recording_range_normalizes_offsets_and_cursor_round_trips() {
        let parameters = std::collections::BTreeMap::from([
            ("start".to_owned(), "2026-08-11T12:00:00+06:00".to_owned()),
            ("end".to_owned(), "2026-08-11T06:01:00Z".to_owned()),
        ]);
        let (_, _, range) =
            parse_range(&parameters).unwrap_or_else(|| panic!("offset range must parse"));
        assert_eq!(
            range
                .end()
                .duration_since(range.start())
                .unwrap_or(Duration::ZERO),
            Duration::from_secs(60)
        );
        let recording_id = surveillance_common::types::RecordingId::new("cursor-recording")
            .unwrap_or_else(|error| panic!("recording ID: {error}"));
        let encoded = encode_recording_cursor(range.start(), &recording_id);
        let decoded =
            decode_recording_cursor(&encoded).unwrap_or_else(|()| panic!("cursor must decode"));
        assert_eq!(decoded.started_at(), range.start());
        assert_eq!(decoded.recording_id(), &recording_id);
    }

    fn supervisor() -> (
        CameraSupervisor,
        crate::camera_supervisor::CameraSupervisorHandle,
    ) {
        CameraSupervisor::new(
            NonZeroUsize::new(8).unwrap_or_else(|| panic!("command bound")),
            NonZeroUsize::new(10).unwrap_or_else(|| panic!("registry bound")),
            Duration::from_secs(2),
        )
    }

    #[tokio::test]
    async fn camera_api_validates_commands_and_never_returns_source_or_credentials() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let body = br#"{"camera_id":"camera-21","rtsp_endpoint":"rtsp://127.0.0.1:19221/camera-21","username":"operator","password":"top-secret","width":640,"height":360}"#;
        let request = format!(
            "POST /api/cameras HTTP/1.1\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(body).unwrap_or_else(|error| panic!("test JSON: {error}"))
        );
        let response = route(
            request.as_bytes(),
            &test_defaults("target/phase3-tests"),
            &handle.health_registry(),
            &handle,
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 201"));
        let response = route(
            b"GET /api/cameras/camera-21 HTTP/1.1\r\n\r\n",
            &test_defaults("target/phase3-tests"),
            &handle.health_registry(),
            &handle,
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("camera-21"));
        assert!(!response.contains("top-secret"));
        assert!(!response.contains("rtsp://"));
        assert!(!response.contains("recording_root"));

        let invalid = route(
            b"POST /api/cameras/../start HTTP/1.1\r\n\r\n",
            &test_defaults("target/phase3-tests"),
            &handle.health_registry(),
            &handle,
        )
        .await;
        assert!(invalid.starts_with("HTTP/1.1 400"));
        handle
            .shutdown()
            .await
            .unwrap_or_else(|error| panic!("shutdown: {error}"));
        task.await
            .unwrap_or_else(|error| panic!("supervisor task: {error}"));
    }

    #[tokio::test]
    async fn api_listener_failure_does_not_cancel_pipeline_supervision() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let mut settings = OneCameraSettings::h264_mvp(
            "camera-22".to_owned(),
            "rtsp://127.0.0.1:19222/camera-22".to_owned(),
            None,
            640,
            360,
            "target/phase3-tests/camera-22".into(),
        );
        settings.connect_timeout = Duration::from_millis(50);
        let config = Arc::new(
            OneCameraConfig::validate(settings)
                .unwrap_or_else(|error| panic!("valid config: {error}")),
        );
        handle
            .add(Arc::clone(&config))
            .await
            .unwrap_or_else(|error| panic!("add: {error}"));
        handle
            .start(config.camera_id.clone())
            .await
            .unwrap_or_else(|error| panic!("start: {error}"));
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let public_address: SocketAddr = "0.0.0.0:0"
            .parse()
            .unwrap_or_else(|error| panic!("test address: {error}"));
        assert!(serve_health(
            public_address,
            CameraDefaults::new("target/phase3-tests".into(), None),
            handle.health_registry(),
            handle.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            shutdown_rx
        )
        .await
        .is_err());
        assert_eq!(
            handle
                .health_registry()
                .snapshot(&config.camera_id)
                .map(|snapshot| snapshot.pipeline_state),
            Some(PipelineState::Running)
        );
        handle
            .shutdown()
            .await
            .unwrap_or_else(|error| panic!("shutdown: {error}"));
        task.await
            .unwrap_or_else(|error| panic!("supervisor task: {error}"));
    }

    /// A deployment `LiveKit` template, parameterised on the one field that
    /// decides whether `OneCameraConfig::validate` accepts it.
    ///
    /// The endpoint is the lever these tests pull because stamping the template
    /// onto a camera is otherwise unobservable through the API: publication
    /// state is reported from a running pipeline's health, and a unit test
    /// starts no pipeline and no `LiveKit` server. A template that cannot
    /// validate turns "was this attached?" into a status code.
    fn livekit_template(endpoint: &str) -> super::LiveKitSettings {
        super::LiveKitSettings {
            decoder_program: "ffmpeg".to_owned(),
            max_publish_height: 0,
            viewer_idle_grace: Duration::from_secs(5),
            endpoint: endpoint.to_owned(),
            api_key: "devkey".to_owned(),
            api_secret: "development-secret-of-at-least-32-bytes".to_owned(),
            deployment_namespace: "surveillance".to_owned(),
            queue_items: 8,
            max_lag: Duration::from_secs(2),
            max_access_unit_bytes: 2 * 1024 * 1024,
            shutdown_timeout: Duration::from_secs(5),
            reconnect_initial: Duration::from_millis(250),
            reconnect_maximum: Duration::from_secs(30),
            reconnect_stable_reset: Duration::from_secs(60),
        }
    }

    fn publishing_defaults(endpoint: &str) -> CameraDefaults {
        CameraDefaults::new(
            PathBuf::from("target/phase9-tests"),
            Some(livekit_template(endpoint)),
        )
    }

    fn add_camera_request(camera_id: &str, extra: &str) -> String {
        let body = format!(
            r#"{{"camera_id":"{camera_id}","rtsp_endpoint":"rtsp://127.0.0.1:19301/{camera_id}","width":640,"height":360{extra}}}"#
        );
        format!(
            "POST /api/cameras HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    /// The truth table for the field nobody sets.
    ///
    /// An absent field is the common case — the dashboard omits it, and so does
    /// every hand-written `curl`. Before this existed the answer was a hard
    /// `false`, which is why a fleet added through the API could not be watched.
    #[test]
    fn an_absent_publication_field_follows_the_deployment() {
        assert!(super::wants_live_publication(None, true));
        assert!(!super::wants_live_publication(None, false));
        // An explicit choice is never overridden by the deployment, in either
        // direction: a deployment that publishes must still be able to hold one
        // camera back, and asking to publish where nothing can is an error the
        // caller has to be told about rather than a silently downgraded add.
        assert!(super::wants_live_publication(Some(true), true));
        assert!(super::wants_live_publication(Some(true), false));
        assert!(!super::wants_live_publication(Some(false), true));
        assert!(!super::wants_live_publication(Some(false), false));
    }

    #[tokio::test]
    async fn a_camera_added_through_the_api_publishes_when_the_deployment_does() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());

        // A usable deployment template: the camera is accepted with publication
        // attached. This is the case that did not exist before — every camera
        // added here was created unable to publish, whatever the deployment did.
        let response = route(
            add_camera_request("camera-31", "").as_bytes(),
            &publishing_defaults("ws://127.0.0.1:7880"),
            &handle.health_registry(),
            &handle,
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 201"),
            "a camera on a publishing deployment must be accepted: {response}"
        );

        // The same request against a template that cannot validate is refused,
        // which is only reachable if the template was attached to this camera.
        let response = route(
            add_camera_request("camera-32", "").as_bytes(),
            &publishing_defaults("not-a-url"),
            &handle.health_registry(),
            &handle,
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 400")
                && response.contains("invalid_camera_configuration"),
            "an absent field must take the deployment's template, not skip it: {response}"
        );

        // And declining publication skips the template entirely, so the same
        // unusable settings no longer stand between the operator and a camera
        // they never asked to publish.
        let response = route(
            add_camera_request("camera-32", r#","live_publication_enabled":false"#).as_bytes(),
            &publishing_defaults("not-a-url"),
            &handle.health_registry(),
            &handle,
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 201"),
            "declining publication must not consult the template: {response}"
        );

        handle
            .shutdown()
            .await
            .unwrap_or_else(|error| panic!("shutdown: {error}"));
        task.await
            .unwrap_or_else(|error| panic!("supervisor task: {error}"));
    }

    #[tokio::test]
    async fn asking_to_publish_where_the_deployment_cannot_is_refused_not_ignored() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());

        let response = route(
            add_camera_request("camera-33", r#","live_publication_enabled":true"#).as_bytes(),
            &test_defaults("target/phase9-tests"),
            &handle.health_registry(),
            &handle,
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 400")
                && response.contains("live_publication_not_configured"),
            "asking for publication a deployment cannot do must be an error: {response}"
        );

        // Refused, not half-applied. A 201 here would hand back a camera that
        // silently does less than was asked for, and the operator would find
        // out by watching a tile stay dark.
        let response = route(
            b"GET /api/cameras/camera-33 HTTP/1.1\r\n\r\n",
            &test_defaults("target/phase9-tests"),
            &handle.health_registry(),
            &handle,
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 404"),
            "the refused camera must not have been created: {response}"
        );

        handle
            .shutdown()
            .await
            .unwrap_or_else(|error| panic!("shutdown: {error}"));
        task.await
            .unwrap_or_else(|error| panic!("supervisor task: {error}"));
    }

    /// The endpoint and credentials are the deployment's, and this route is not
    /// a way to supply, substitute, or learn them. `deny_unknown_fields` is what
    /// enforces it; this pins the consequence so a later field addition cannot
    /// relax it by accident.
    #[tokio::test]
    async fn the_add_route_refuses_a_livekit_endpoint_or_credential() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());

        for smuggled in [
            r#","livekit_endpoint":"ws://attacker.example/""#,
            r#","api_secret":"stolen""#,
            r#","deployment_namespace":"other""#,
        ] {
            let response = route(
                add_camera_request("camera-34", smuggled).as_bytes(),
                &publishing_defaults("ws://127.0.0.1:7880"),
                &handle.health_registry(),
                &handle,
            )
            .await;
            assert!(
                response.starts_with("HTTP/1.1 400"),
                "the add route must not accept {smuggled}: {response}"
            );
        }

        handle
            .shutdown()
            .await
            .unwrap_or_else(|error| panic!("shutdown: {error}"));
        task.await
            .unwrap_or_else(|error| panic!("supervisor task: {error}"));
    }
}
