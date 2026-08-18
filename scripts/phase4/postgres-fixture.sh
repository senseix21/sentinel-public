#!/usr/bin/env bash

# Qualifies PostgreSQL indexing, playback, reconciliation, retention, and outage recovery.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
fixture_root=$(mktemp -d "${TMPDIR:-/tmp}/surveillance-pg.XXXXXX")
data_dir="$fixture_root/data"
socket_dir="$fixture_root/socket"
log_file="$fixture_root/postgres.log"
port=$((55000 + ($$ % 1000)))
mkdir -p "$socket_dir"

cleanup() {
    if [[ -f "$data_dir/postmaster.pid" ]]; then
        pg_ctl -D "$data_dir" -m fast -w stop >/dev/null
    fi
    rm -rf "$fixture_root"
}
trap cleanup EXIT

initdb -D "$data_dir" --auth=trust --no-locale --encoding=UTF8 >/dev/null
pg_ctl -D "$data_dir" -l "$log_file" -o "-h 127.0.0.1 -k $socket_dir -p $port" -w start >/dev/null

psql_fixture=(psql -X -v ON_ERROR_STOP=1 -h "$socket_dir" -p "$port" -d postgres)
"${psql_fixture[@]}" -Atc 'SELECT version()'
"${psql_fixture[@]}" -c 'CREATE DATABASE surveillance_phase4' >/dev/null
psql_fixture=(psql -X -v ON_ERROR_STOP=1 -h "$socket_dir" -p "$port" -d surveillance_phase4)

apply_migrations() {
    "${psql_fixture[@]}" <<'SQL' >/dev/null
CREATE TABLE IF NOT EXISTS phase4_schema_migrations (
    version text PRIMARY KEY,
    applied_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
SQL
    for migration in "$repo_root"/migrations/*.sql; do
        version=$(basename "$migration")
        applied=$("${psql_fixture[@]}" -Atc "SELECT 1 FROM phase4_schema_migrations WHERE version = '$version'")
        if [[ "$applied" != "1" ]]; then
            "${psql_fixture[@]}" -f "$migration" >/dev/null
            "${psql_fixture[@]}" -c \
                "INSERT INTO phase4_schema_migrations(version) VALUES ('$version')" >/dev/null
        fi
    done
}

apply_migrations
apply_migrations

"${psql_fixture[@]}" <<'SQL' >/dev/null
INSERT INTO cameras(camera_id) VALUES ('camera-a'), ('camera-b');

INSERT INTO recording_segments (
    recording_id, camera_id, started_at, ended_at, storage_locator,
    container, video_codec, width, height, size_bytes, status, provenance
) VALUES
    ('segment-1', 'camera-a', '2026-08-11T00:00:00Z', '2026-08-11T00:01:00Z', 'camera-a/segment-1.mp4', 'fragmented_mp4', 'h264', 1920, 1080, 1000, 'completed', 'recorder'),
    ('segment-2', 'camera-a', '2026-08-11T00:01:00Z', '2026-08-11T00:02:00Z', 'camera-a/segment-2.mp4', 'fragmented_mp4', 'h264', 1920, 1080, 1000, 'completed', 'recorder'),
    ('segment-3', 'camera-a', '2026-08-11T00:02:00Z', '2026-08-11T00:03:00Z', 'camera-a/segment-3.mp4', 'fragmented_mp4', 'h264', 1920, 1080, 1000, 'completed', 'reconciliation'),
    ('segment-b', 'camera-b', '2026-08-11T00:01:00Z', '2026-08-11T00:02:00Z', 'camera-b/segment-b.mp4', 'fragmented_mp4', 'h264', 1920, 1080, 1000, 'completed', 'recorder');

DO $$
DECLARE selected text[];
BEGIN
    SELECT array_agg(recording_id ORDER BY started_at, recording_id)
      INTO selected
      FROM recording_segments
     WHERE camera_id = 'camera-a'
       AND status = 'completed'
       AND started_at < '2026-08-11T00:02:30Z'
       AND ended_at > '2026-08-11T00:00:30Z';
    IF selected <> ARRAY['segment-1', 'segment-2', 'segment-3'] THEN
        RAISE EXCEPTION 'unexpected overlap result: %', selected;
    END IF;

    BEGIN
        INSERT INTO recording_segments (
            recording_id, camera_id, started_at, ended_at, storage_locator,
            container, video_codec, width, height, size_bytes, status, provenance
        ) VALUES ('bad-interval', 'camera-a', now(), now(), 'camera-a/bad.mp4',
                  'fragmented_mp4', 'h264', 1, 1, 1, 'completed', 'recorder');
        RAISE EXCEPTION 'invalid interval was accepted';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    BEGIN
        INSERT INTO recording_segments (
            recording_id, camera_id, started_at, ended_at, storage_locator,
            container, video_codec, width, height, size_bytes, status, provenance
        ) VALUES ('bad-file', 'camera-a', now(), now() + interval '1 second', '../bad.partial.mp4',
                  'fragmented_mp4', 'h264', 1, 1, 1, 'completed', 'recorder');
        RAISE EXCEPTION 'unsafe partial locator was accepted';
    EXCEPTION WHEN check_violation THEN NULL;
    END;
END $$;
SQL

"${psql_fixture[@]}" -c 'ANALYZE recording_segments' >/dev/null
"${psql_fixture[@]}" -c "SET enable_seqscan = off; EXPLAIN (COSTS OFF) SELECT recording_id FROM recording_segments WHERE camera_id = 'camera-a' AND status = 'completed' AND started_at < '2026-08-11T00:02:30Z' AND ended_at > '2026-08-11T00:00:30Z' ORDER BY started_at, recording_id;"
echo "Phase 4 PostgreSQL migration, constraint, overlap, and index-plan fixture passed"

"${psql_fixture[@]}" -c 'CREATE DATABASE surveillance_repository' >/dev/null
PHASE4_DATABASE_URL="postgres://127.0.0.1:$port/surveillance_repository" \
    cargo test --manifest-path "$repo_root/Cargo.toml" -p surveillance-database \
    --test postgres_repository -- --ignored --test-threads=1

playback_root="$fixture_root/playback"
mkdir -p "$playback_root/playback-camera"
for segment in 1 2 3; do
    ffmpeg -hide_banner -loglevel error -f lavfi -i "testsrc=size=320x240:rate=15" \
        -t 2 -c:v libx264 -pix_fmt yuv420p -g 30 -bf 0 \
        -movflags frag_keyframe+empty_moov+default_base_moof \
        "$playback_root/playback-camera/segment-$segment.mp4"
done
PHASE4_DATABASE_URL="postgres://127.0.0.1:$port/surveillance_repository" \
PHASE4_PLAYBACK_ROOT="$playback_root" \
    cargo test --manifest-path "$repo_root/Cargo.toml" -p surveillance-server \
    --test playback_assembler -- --ignored --test-threads=1

reconciliation_root="$fixture_root/reconciliation"
retention_root="$fixture_root/retention"
mkdir -p "$reconciliation_root" "$retention_root"
PHASE4_DATABASE_URL="postgres://127.0.0.1:$port/surveillance_repository" \
PHASE4_RECONCILIATION_ROOT="$reconciliation_root" \
PHASE4_OUTAGE_MEDIA="$playback_root/playback-camera/segment-1.mp4" \
    cargo test --manifest-path "$repo_root/Cargo.toml" -p surveillance-server \
    --test reconciliation_owner -- --ignored --test-threads=1
PHASE4_DATABASE_URL="postgres://127.0.0.1:$port/surveillance_repository" \
PHASE4_RETENTION_ROOT="$retention_root" \
    cargo test --manifest-path "$repo_root/Cargo.toml" -p surveillance-server \
    --test retention_owner -- --ignored --test-threads=1

outage_coordination="$fixture_root/outage"
mkdir -p "$outage_coordination"
PHASE4_DATABASE_URL="postgres://127.0.0.1:$port/surveillance_repository" \
PHASE4_OUTAGE_COORDINATION="$outage_coordination" \
PHASE4_OUTAGE_MEDIA="$playback_root/playback-camera/segment-1.mp4" \
    cargo test --manifest-path "$repo_root/Cargo.toml" -p surveillance-database \
    --test database_outage -- --ignored --test-threads=1 &
outage_test_pid=$!
for _ in $(seq 1 300); do
    [[ -f "$outage_coordination/ready" ]] && break
    sleep 0.05
done
[[ -f "$outage_coordination/ready" ]]
pg_ctl -D "$data_dir" -m fast -w stop >/dev/null
touch "$outage_coordination/database-down"
for _ in $(seq 1 300); do
    [[ -f "$outage_coordination/outage-observed" ]] && break
    sleep 0.05
done
[[ -f "$outage_coordination/outage-observed" ]]
pg_ctl -D "$data_dir" -l "$log_file" -o "-h 127.0.0.1 -k $socket_dir -p $port" -w start >/dev/null
touch "$outage_coordination/database-restored"
wait "$outage_test_pid"
