-- Separate read-only archive indexes from active recorder-owned footage.
CREATE SEQUENCE recording_source_sequence;
CREATE TABLE recording_sources (
    source_id text PRIMARY KEY DEFAULT ('archive-' || nextval('recording_source_sequence')),
    root text UNIQUE,
    label text NOT NULL,
    read_only boolean NOT NULL DEFAULT true,
    index_state text NOT NULL DEFAULT 'not_indexed' CHECK (index_state IN ('not_indexed', 'indexing', 'ready', 'partial', 'failed', 'interrupted')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (source_id ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND octet_length(source_id) <= 64),
    CHECK ((source_id = 'primary' AND root IS NULL AND NOT read_only)
        OR (source_id <> 'primary' AND root IS NOT NULL AND read_only))
);
INSERT INTO recording_sources(source_id, root, label, read_only)
VALUES ('primary', NULL, 'Current recordings', false);
ALTER TABLE recording_segments ADD COLUMN source_id text NOT NULL DEFAULT 'primary'
    REFERENCES recording_sources(source_id);
ALTER TABLE recording_segments DROP CONSTRAINT recording_segments_pkey;
ALTER TABLE recording_segments DROP CONSTRAINT recording_segments_storage_locator_key;
ALTER TABLE recording_segments ADD PRIMARY KEY (source_id, recording_id);
ALTER TABLE recording_segments ADD UNIQUE (source_id, storage_locator);
CREATE INDEX recording_segments_source_overlap_idx
    ON recording_segments(source_id, camera_id, started_at, recording_id)
    INCLUDE (ended_at) WHERE status = 'completed';

ALTER TABLE access_audit ADD COLUMN recording_source_id text REFERENCES recording_sources(source_id);
