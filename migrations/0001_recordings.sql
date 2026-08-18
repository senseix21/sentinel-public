-- Establishes cameras and the indexed recording-segment lifecycle schema.

CREATE TABLE cameras (
    camera_id text PRIMARY KEY,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT cameras_id_valid CHECK (
        camera_id ~ '^[a-z0-9]+(-[a-z0-9]+)*$'
        AND octet_length(camera_id) BETWEEN 1 AND 64
    )
);

CREATE TYPE recording_status AS ENUM (
    'open',
    'finalizing',
    'completed',
    'failed',
    'incomplete',
    'deletion_pending',
    'deleted',
    'quarantined',
    'inconsistent'
);

CREATE TYPE recording_provenance AS ENUM ('recorder', 'reconciliation');

CREATE TABLE recording_segments (
    recording_id text PRIMARY KEY,
    camera_id text NOT NULL REFERENCES cameras(camera_id),
    started_at timestamptz NOT NULL,
    ended_at timestamptz,
    storage_locator text NOT NULL UNIQUE,
    container text,
    video_codec text,
    width integer,
    height integer,
    size_bytes bigint,
    status recording_status NOT NULL,
    provenance recording_provenance NOT NULL,
    failure_category text,
    status_changed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT recording_id_valid CHECK (
        recording_id ~ '^[a-z0-9]+(-[a-z0-9]+)*$'
        AND octet_length(recording_id) BETWEEN 1 AND 64
    ),
    CONSTRAINT recording_interval_valid CHECK (ended_at IS NULL OR ended_at > started_at),
    CONSTRAINT recording_locator_relative CHECK (
        storage_locator <> ''
        AND storage_locator !~ '^/'
        AND storage_locator !~ '(^|/)\.\.(/|$)'
        AND storage_locator !~ '(^|/)\.(/|$)'
    ),
    CONSTRAINT recording_playable_lifecycle_evidence CHECK (
        status NOT IN ('completed', 'deletion_pending', 'deleted')
        OR (
            ended_at > started_at
            AND storage_locator LIKE '%.mp4'
            AND storage_locator NOT LIKE '%.partial.mp4'
            AND container = 'fragmented_mp4'
            AND video_codec = 'h264'
            AND width > 0
            AND height > 0
            AND size_bytes > 0
            AND failure_category IS NULL
        )
    ),
    CONSTRAINT recording_failure_evidence CHECK (
        status NOT IN ('failed', 'incomplete') OR failure_category IS NOT NULL
    )
);

CREATE INDEX recording_segments_completed_overlap_idx
    ON recording_segments (camera_id, started_at, recording_id)
    INCLUDE (ended_at)
    WHERE status = 'completed';

CREATE INDEX recording_segments_reconciliation_idx
    ON recording_segments (status, status_changed_at, recording_id)
    WHERE status IN ('incomplete', 'deletion_pending', 'inconsistent');

CREATE INDEX recording_segments_retention_idx
    ON recording_segments (ended_at, recording_id)
    WHERE status = 'completed';
