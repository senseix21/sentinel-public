-- Phase 8 (P8.6): audit beyond configuration.
--
-- 0002 audits configuration changes. This audits *access*: authentication
-- outcomes, authorization decisions, camera lifecycle operations, recording
-- export, and identity changes.
--
-- Same shape and same discipline as settings_audit:
--
--   * a fingerprint, never a value, for anything sensitive;
--   * bounded columns, so an audit row cannot be used to store a payload;
--   * enumerated categories and outcomes checked by the database, so a caller
--     cannot invent a category and quietly land outside every report.
--
-- Recording export is the row to care most about. It is the operation that
-- removes footage from the system's control, it is the one an insider abuses,
-- and no control prevents it — detection is the control. So an export row
-- carries who, which camera, the exact range, and how many bytes left.

CREATE TABLE access_audit (
    id                  bigserial PRIMARY KEY,
    occurred_at         timestamptz NOT NULL DEFAULT clock_timestamp(),

    -- What kind of event this is.
    category            text NOT NULL,
    -- The bounded action label. For authorization rows this is
    -- `Action::as_str`, which deliberately excludes the camera identifier
    -- because camera_id is its own column.
    action              text NOT NULL,
    -- 'principal:<kind>:<id>', or 'anonymous' when nobody authenticated.
    actor               text NOT NULL,
    -- NULL for actions that are not camera-scoped.
    camera_id           text,
    outcome             text NOT NULL,
    -- A bounded failure label (AuthFailure::as_str, Denied), never a value and
    -- never free text from a request.
    reason              text,
    -- Who the request came from, after proof of origin. NULL when unknown.
    client_address      inet,

    -- Export only. NULL elsewhere; enforced below.
    range_start         timestamptz,
    range_end           timestamptz,
    byte_count          bigint,

    -- A salted fingerprint of anything sensitive the row refers to. Never the
    -- value itself.
    detail_fingerprint  text,

    CONSTRAINT access_audit_category_known CHECK (
        category IN ('authentication', 'authorization', 'camera', 'export', 'identity')
    ),
    CONSTRAINT access_audit_outcome_known CHECK (
        outcome IN ('granted', 'denied', 'succeeded', 'failed')
    ),
    CONSTRAINT access_audit_action_bounded CHECK (octet_length(action) BETWEEN 1 AND 64),
    CONSTRAINT access_audit_actor_bounded CHECK (octet_length(actor) BETWEEN 1 AND 128),
    CONSTRAINT access_audit_reason_bounded CHECK (
        reason IS NULL OR octet_length(reason) BETWEEN 1 AND 64
    ),
    CONSTRAINT access_audit_camera_bounded CHECK (
        camera_id IS NULL OR octet_length(camera_id) BETWEEN 1 AND 64
    ),

    -- Same fingerprint shape as settings_audit: 64 lowercase hex characters.
    CONSTRAINT access_audit_fingerprint_shape CHECK (
        detail_fingerprint IS NULL OR detail_fingerprint ~ '^[0-9a-f]{64}$'
    ),

    -- Export columns belong to export rows and nowhere else, and an export row
    -- must carry all three. A partially-filled export row would look like
    -- evidence while proving nothing.
    CONSTRAINT access_audit_export_fields_together CHECK (
        (category = 'export' AND range_start IS NOT NULL AND range_end IS NOT NULL
             AND byte_count IS NOT NULL)
        OR (category <> 'export' AND range_start IS NULL AND range_end IS NULL
             AND byte_count IS NULL)
    ),
    CONSTRAINT access_audit_export_range_ordered CHECK (
        range_start IS NULL OR range_end >= range_start
    ),
    CONSTRAINT access_audit_byte_count_non_negative CHECK (
        byte_count IS NULL OR byte_count >= 0
    )
);

-- "What did this principal do" is the question an incident asks first.
CREATE INDEX access_audit_by_actor ON access_audit (actor, occurred_at DESC);

-- "Who touched this camera" is the second.
CREATE INDEX access_audit_by_camera
    ON access_audit (camera_id, occurred_at DESC)
    WHERE camera_id IS NOT NULL;

-- Authentication-failure rate, for the P8.7 alert.
CREATE INDEX access_audit_failures_by_time
    ON access_audit (occurred_at DESC)
    WHERE outcome IN ('denied', 'failed');

-- Every export, cheaply.
CREATE INDEX access_audit_exports
    ON access_audit (occurred_at DESC)
    WHERE category = 'export';
