-- Persistent operational settings and their audit trail.
--
-- ADR 008: a settings change and its audit record commit in one transaction.
-- An audit trail with gaps is worse than none, because it invites the inference
-- that an unlogged change did not happen.
--
-- Secret-valued settings are absent by construction. ADR 003 leaves the
-- production secret store open, so Phase 7 does not persist credentials; the
-- application layer refuses them, and `settings_no_secret_keys` refuses them
-- again here in case a future caller forgets.

CREATE TABLE settings (
    setting_key text NOT NULL,
    -- NULL means the value applies to every camera, or the setting is global.
    camera_id text REFERENCES cameras(camera_id),
    value text NOT NULL,
    generation bigint NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT settings_key_valid CHECK (
        setting_key ~ '^[a-z0-9]+([._][a-z0-9]+)*$'
        AND octet_length(setting_key) BETWEEN 1 AND 128
    ),
    CONSTRAINT settings_value_bounded CHECK (octet_length(value) BETWEEN 1 AND 4096),
    CONSTRAINT settings_generation_positive CHECK (generation > 0),
    -- Credentials never reach this table. Keys are stable identities, so
    -- matching on them is not a heuristic.
    CONSTRAINT settings_no_secret_keys CHECK (
        setting_key NOT IN (
            'database.url',
            'audit.fingerprint_salt',
            'camera.rtsp.username',
            'camera.rtsp.password',
            'camera.livekit.api_key',
            'camera.livekit.api_secret'
        )
    )
);

-- PostgreSQL treats NULLs as distinct in a unique index, so a primary key over
-- (setting_key, camera_id) would NOT prevent duplicate global rows. Both
-- indexes are required: the partial unique index is what actually enforces one
-- row per global setting.
CREATE UNIQUE INDEX settings_scoped_unique
    ON settings (setting_key, camera_id)
    WHERE camera_id IS NOT NULL;

CREATE UNIQUE INDEX settings_global_unique
    ON settings (setting_key)
    WHERE camera_id IS NULL;

CREATE INDEX settings_by_camera ON settings (camera_id) WHERE camera_id IS NOT NULL;

CREATE TYPE settings_action AS ENUM ('set', 'clear');

CREATE TABLE settings_audit (
    audit_id bigserial PRIMARY KEY,
    occurred_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    generation bigint NOT NULL,
    -- Phase 7 records the loopback origin and a placeholder identity. Phase 8
    -- replaces the content, not the shape.
    actor text NOT NULL,
    action settings_action NOT NULL,
    setting_key text NOT NULL,
    camera_id text,
    origin text NOT NULL,
    outcome text NOT NULL,
    old_value text,
    new_value text,
    old_fingerprint text,
    new_fingerprint text,
    CONSTRAINT settings_audit_actor_bounded CHECK (octet_length(actor) BETWEEN 1 AND 128),
    CONSTRAINT settings_audit_generation_positive CHECK (generation > 0),
    -- Each side carries either a plaintext value or a fingerprint, never both.
    -- A bug that wrote a secret into new_value while also fingerprinting it is
    -- rejected by the database rather than discovered in an export.
    CONSTRAINT settings_audit_old_single_representation CHECK (
        old_value IS NULL OR old_fingerprint IS NULL
    ),
    CONSTRAINT settings_audit_new_single_representation CHECK (
        new_value IS NULL OR new_fingerprint IS NULL
    ),
    CONSTRAINT settings_audit_fingerprints_are_sha256 CHECK (
        (old_fingerprint IS NULL OR old_fingerprint ~ '^[0-9a-f]{64}$')
        AND (new_fingerprint IS NULL OR new_fingerprint ~ '^[0-9a-f]{64}$')
    ),
    -- A 'set' must say what it set. Without this, a row recording nothing would
    -- still satisfy every other constraint.
    CONSTRAINT settings_audit_set_has_a_new_side CHECK (
        action <> 'set' OR new_value IS NOT NULL OR new_fingerprint IS NOT NULL
    )
);

CREATE INDEX settings_audit_by_time ON settings_audit (occurred_at DESC);
CREATE INDEX settings_audit_by_key ON settings_audit (setting_key, occurred_at DESC);
CREATE INDEX settings_audit_by_generation ON settings_audit (generation);

-- Monotonic configuration version. A sequence rather than max(generation)+1 so
-- that concurrent transactions cannot select the same number.
CREATE SEQUENCE settings_generation_sequence AS bigint START 1;
