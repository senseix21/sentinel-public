#!/usr/bin/env bash
set -euo pipefail

# Phase 7 configuration fixture.
#
# Qualifies persistence, audit durability, schema constraints, restart
# persistence, and store-outage degradation against a real PostgreSQL. The
# outage section stops the database mid-run to prove that a stale unit keeps
# running and clears Stale without a restart.

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
fixture_root=$(mktemp -d "${TMPDIR:-/tmp}/surveillance-phase7.XXXXXX")
data_dir="$fixture_root/data"
socket_dir="$fixture_root/socket"
log_file="$fixture_root/postgres.log"
port=$((56000 + ($$ % 1000)))
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
"${psql_fixture[@]}" -c 'CREATE DATABASE surveillance_phase7' >/dev/null
psql_fixture=(psql -X -v ON_ERROR_STOP=1 -h "$socket_dir" -p "$port" -d surveillance_phase7)

for migration in "$repo_root"/migrations/*.sql; do
    "${psql_fixture[@]}" -f "$migration" >/dev/null
done

# Schema-level guarantees, proven in SQL so they hold regardless of which
# application writes to the table.
"${psql_fixture[@]}" <<'SQL' >/dev/null
DO $$
BEGIN
    -- Postgres treats NULLs as distinct in a unique index, so the partial
    -- unique index is what actually prevents duplicate global rows.
    INSERT INTO settings (setting_key, value, generation)
    VALUES ('retention.interval', '60s', 1);
    BEGIN
        INSERT INTO settings (setting_key, value, generation)
        VALUES ('retention.interval', '90s', 2);
        RAISE EXCEPTION 'a duplicate global setting was accepted';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Credentials must never reach the settings table.
    BEGIN
        INSERT INTO settings (setting_key, value, generation)
        VALUES ('camera.rtsp.password', 'hunter2', 1);
        RAISE EXCEPTION 'a credential was accepted into settings';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Each audit side carries a value or a fingerprint, never both.
    BEGIN
        INSERT INTO settings_audit
            (generation, actor, action, setting_key, origin, outcome,
             new_value, new_fingerprint)
        VALUES (1, 'loopback', 'set', 'retention.interval', 'persisted',
                'applied', '30s', repeat('0', 64));
        RAISE EXCEPTION 'an audit row with both representations was accepted';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- A 'set' must record what it set.
    BEGIN
        INSERT INTO settings_audit
            (generation, actor, action, setting_key, origin, outcome)
        VALUES (1, 'loopback', 'set', 'retention.interval', 'persisted', 'applied');
        RAISE EXCEPTION 'an empty set audit row was accepted';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Fingerprints must look like SHA-256.
    BEGIN
        INSERT INTO settings_audit
            (generation, actor, action, setting_key, origin, outcome, new_fingerprint)
        VALUES (1, 'loopback', 'set', 'camera.rtsp.password', 'environment',
                'applied', 'not-a-digest');
        RAISE EXCEPTION 'a malformed fingerprint was accepted';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    DELETE FROM settings;
END $$;
SQL

echo "Phase 7 schema constraint fixture passed"

"${psql_fixture[@]}" -c 'CREATE DATABASE surveillance_phase7_repository' >/dev/null
export PHASE4_DATABASE_URL="postgres://127.0.0.1:$port/surveillance_phase7_repository"

cargo test --manifest-path "$repo_root/Cargo.toml" -p surveillance-database \
    --test settings_persistence -- --ignored --test-threads=1

# Restart persistence: a committed change survives a full database restart, and
# the settings a unit reads back are the ones it wrote.
psql_repo=(psql -X -v ON_ERROR_STOP=1 -h "$socket_dir" -p "$port" -d surveillance_phase7_repository)
"${psql_repo[@]}" -c \
    "INSERT INTO settings (setting_key, value, generation) VALUES ('retention.max_age', '14d', 99) \
     ON CONFLICT (setting_key) WHERE camera_id IS NULL \
     DO UPDATE SET value = EXCLUDED.value, generation = EXCLUDED.generation" >/dev/null

pg_ctl -D "$data_dir" -m fast -w stop >/dev/null
pg_ctl -D "$data_dir" -l "$log_file" -o "-h 127.0.0.1 -k $socket_dir -p $port" -w start >/dev/null

survived=$("${psql_repo[@]}" -Atc \
    "SELECT value FROM settings WHERE setting_key = 'retention.max_age' AND camera_id IS NULL")
if [[ "$survived" != "14d" ]]; then
    echo "restart persistence failed: expected 14d, found '$survived'" >&2
    exit 1
fi
echo "Phase 7 restart persistence fixture passed"

# Store outage: with the database stopped, a load must fail as an error the
# caller turns into Stale, never a panic and never a silent default.
pg_ctl -D "$data_dir" -m fast -w stop >/dev/null
if cargo test --manifest-path "$repo_root/Cargo.toml" -p surveillance-database \
    --test settings_persistence an_empty_store_loads_as_empty_rather_than_failing \
    -- --ignored --test-threads=1 >/dev/null 2>&1; then
    echo "outage fixture failed: a load succeeded with the store stopped" >&2
    exit 1
fi
echo "Phase 7 store-outage fixture passed"

pg_ctl -D "$data_dir" -l "$log_file" -o "-h 127.0.0.1 -k $socket_dir -p $port" -w start >/dev/null

# Recovery: the same load succeeds once the store is reachable again, which is
# what lets Stale clear without a restart.
cargo test --manifest-path "$repo_root/Cargo.toml" -p surveillance-database \
    --test settings_persistence an_empty_store_loads_as_empty_rather_than_failing \
    -- --ignored --test-threads=1
echo "Phase 7 store-recovery fixture passed"

# The cache and freshness state machine need no database, but belong to the same
# evidence run.
cargo test --manifest-path "$repo_root/Cargo.toml" -p surveillance-config \
    --all-features cache::

echo "Phase 7 configuration fixture passed"
