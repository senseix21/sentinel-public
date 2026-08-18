-- Phase 8 (P8.2): identities and bearer tokens.
--
-- ADR 009 Decision 1. Two properties this schema enforces independently of the
-- application, in the same spirit as 0002's refusal to store credential keys:
--
--   1. A stored credential must be an Argon2id PHC string. A caller that tries
--      to write a plaintext token, or a hash from a weaker scheme, is refused
--      by the database, not merely by review.
--   2. Every token has an expiry, and the expiry follows issuance. A
--      never-expiring token is unrepresentable.

CREATE TYPE principal_kind AS ENUM ('human', 'service');
CREATE TYPE principal_role AS ENUM ('viewer', 'operator', 'administrator');

CREATE TABLE principals (
    principal_id  text PRIMARY KEY,
    kind          principal_kind NOT NULL,
    display_name  text NOT NULL,
    role          principal_role NOT NULL,
    -- TRUE means every camera, including cameras added later. FALSE means
    -- exactly the rows in principal_camera_scope, which may legally be none:
    -- an empty scope is how an operator is suspended from cameras without
    -- deleting the identity the audit trail references.
    scope_all     boolean NOT NULL,
    enabled       boolean NOT NULL DEFAULT TRUE,
    created_at    timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at    timestamptz NOT NULL DEFAULT clock_timestamp(),

    -- Mirrors PrincipalId::new: alphanumerics, '-', '_', '.', not on a
    -- boundary. Bounded so that 'principal:<kind>:<id>' fits the 128-byte
    -- settings_audit.actor column.
    CONSTRAINT principals_id_shape CHECK (
        principal_id ~ '^[A-Za-z0-9]([A-Za-z0-9._-]*[A-Za-z0-9])?$'
        AND octet_length(principal_id) BETWEEN 1 AND 64
    ),
    CONSTRAINT principals_display_name_bounded CHECK (
        octet_length(display_name) BETWEEN 1 AND 128
    )
);

-- An explicit camera scope. ON DELETE CASCADE from cameras is deliberate: a
-- scope entry naming a camera that no longer exists would silently widen
-- nothing but would accumulate, and re-adding a camera under the same id must
-- not silently restore an old grant.
CREATE TABLE principal_camera_scope (
    principal_id  text NOT NULL REFERENCES principals (principal_id) ON DELETE CASCADE,
    camera_id     text NOT NULL REFERENCES cameras (camera_id) ON DELETE CASCADE,
    PRIMARY KEY (principal_id, camera_id)
);

CREATE INDEX principal_camera_scope_by_camera
    ON principal_camera_scope (camera_id);

CREATE TABLE auth_tokens (
    -- The public half of the token: 16 lowercase hex characters. Safe to log.
    token_id         text PRIMARY KEY,
    principal_id     text NOT NULL REFERENCES principals (principal_id) ON DELETE CASCADE,
    -- Argon2id PHC string. Encodes its own parameters, so raising the defaults
    -- does not invalidate existing rows.
    credential_hash  text NOT NULL,
    label            text,
    issued_at        timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at       timestamptz NOT NULL,
    revoked_at       timestamptz,

    CONSTRAINT auth_tokens_id_shape CHECK (token_id ~ '^[0-9a-f]{16}$'),

    -- The schema-level refusal to store anything but Argon2id. A plaintext
    -- token, a SHA-256 digest, or a bcrypt hash all fail this.
    CONSTRAINT auth_tokens_hash_is_argon2id CHECK (credential_hash ~ '^\$argon2id\$'),
    CONSTRAINT auth_tokens_hash_bounded CHECK (
        octet_length(credential_hash) BETWEEN 1 AND 512
    ),

    -- Expiry is mandatory (NOT NULL) and meaningful (after issuance).
    CONSTRAINT auth_tokens_expiry_after_issue CHECK (expires_at > issued_at),
    CONSTRAINT auth_tokens_revocation_after_issue CHECK (
        revoked_at IS NULL OR revoked_at >= issued_at
    ),
    CONSTRAINT auth_tokens_label_bounded CHECK (
        label IS NULL OR octet_length(label) BETWEEN 1 AND 128
    )
);

CREATE INDEX auth_tokens_by_principal ON auth_tokens (principal_id);

-- Supports the expiry sweep without scanning the table.
CREATE INDEX auth_tokens_live_by_expiry
    ON auth_tokens (expires_at)
    WHERE revoked_at IS NULL;
