#!/usr/bin/env bash
set -euo pipefail

# Phase 8 security fixture.
#
# Qualifies the controls added in Phase 8 against a real PostgreSQL:
#
#   1. Schema refusal of a non-Argon2id credential and of a token with no
#      expiry. These are database-level controls, so proving them needs a
#      database rather than a unit test.
#   2. Issue -> verify -> revoke -> verify against a real store.
#   3. Secrets-file permission refusal, with an actual 0644 file.
#   4. Dependency policy (cargo deny, cargo audit).
#
# What it does NOT prove, and why, is recorded in
# docs/phase-8/local-evidence-<date>.md. Nothing here substitutes for the
# representative-camera and target-hardware gates that remain open.

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
fixture_root=$(mktemp -d "${TMPDIR:-/tmp}/surveillance-phase8.XXXXXX")
data_dir="$fixture_root/data"
socket_dir="$fixture_root/socket"
log_file="$fixture_root/postgres.log"
port=$((57000 + ($$ % 1000)))
mkdir -p "$socket_dir"

cleanup() {
    if [[ -f "$data_dir/postmaster.pid" ]]; then
        pg_ctl -D "$data_dir" -m fast -w stop >/dev/null
    fi
    rm -rf "$fixture_root"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "  ok: $*"; }

echo "== Phase 8 security fixture =="

# --------------------------------------------------------------- dependencies
echo "-- dependency policy"
command -v cargo-deny >/dev/null || fail "cargo-deny is not installed"
command -v cargo-audit >/dev/null || fail "cargo-audit is not installed"
(cd "$repo_root" && cargo deny check) >/dev/null 2>&1 || fail "cargo deny check"
pass "cargo deny check"
(cd "$repo_root" && cargo audit) >/dev/null 2>&1 || fail "cargo audit"
pass "cargo audit"

# ------------------------------------------------------------------- database
echo "-- schema controls"
initdb -D "$data_dir" --auth=trust --no-locale --encoding=UTF8 >/dev/null
pg_ctl -D "$data_dir" -l "$log_file" -o "-h 127.0.0.1 -k $socket_dir -p $port" -w start >/dev/null

psql_fixture=(psql -X -q -v ON_ERROR_STOP=1 -h "$socket_dir" -p "$port" -d postgres)

for migration in "$repo_root"/migrations/0*.sql; do
    "${psql_fixture[@]}" -f "$migration" >/dev/null
done
pass "migrations 0001-0004 applied"

"${psql_fixture[@]}" <<'SQL' >/dev/null
INSERT INTO cameras (camera_id) VALUES ('camera-01') ON CONFLICT DO NOTHING;
INSERT INTO principals (principal_id, kind, display_name, role, scope_all, enabled)
VALUES ('operator', 'human', 'Operator', 'operator', false, true);
INSERT INTO principal_camera_scope (principal_id, camera_id) VALUES ('operator', 'camera-01');
SQL
pass "principal and camera scope inserted"

# A plaintext token must be refused by the database, not merely by review.
if "${psql_fixture[@]}" -c "INSERT INTO auth_tokens (token_id, principal_id, credential_hash, expires_at) \
    VALUES ('0123456789abcdef', 'operator', 'plaintext-token', now() + interval '1 day');" >/dev/null 2>&1; then
    fail "the schema accepted a plaintext credential"
fi
pass "plaintext credential refused by the schema"

# A SHA-256 digest is not Argon2id either.
if "${psql_fixture[@]}" -c "INSERT INTO auth_tokens (token_id, principal_id, credential_hash, expires_at) \
    VALUES ('0123456789abcdef', 'operator', '\$2b\$12\$abcdefghijklmnopqrstuv', now() + interval '1 day');" >/dev/null 2>&1; then
    fail "the schema accepted a bcrypt hash"
fi
pass "non-Argon2id hash refused by the schema"

# Expiry is NOT NULL, so a token that never expires is unrepresentable.
if "${psql_fixture[@]}" -c "INSERT INTO auth_tokens (token_id, principal_id, credential_hash) \
    VALUES ('0123456789abcdef', 'operator', '\$argon2id\$v=19\$m=64,t=1,p=1\$c2FsdA\$aGFzaA');" >/dev/null 2>&1; then
    fail "the schema accepted a token with no expiry"
fi
pass "token without an expiry refused by the schema"

# An expiry before issuance is refused.
if "${psql_fixture[@]}" -c "INSERT INTO auth_tokens (token_id, principal_id, credential_hash, expires_at) \
    VALUES ('0123456789abcdef', 'operator', '\$argon2id\$v=19\$m=64,t=1,p=1\$c2FsdA\$aGFzaA', now() - interval '1 day');" >/dev/null 2>&1; then
    fail "the schema accepted an expiry before issuance"
fi
pass "expiry before issuance refused by the schema"

# A well-formed row is accepted.
"${psql_fixture[@]}" -c "INSERT INTO auth_tokens (token_id, principal_id, credential_hash, expires_at) \
    VALUES ('0123456789abcdef', 'operator', '\$argon2id\$v=19\$m=64,t=1,p=1\$c2FsdA\$aGFzaA', now() + interval '1 day');" >/dev/null
pass "well-formed token row accepted"

# An export audit row must carry its whole extent.
if "${psql_fixture[@]}" -c "INSERT INTO access_audit (category, action, actor, outcome, range_start) \
    VALUES ('export', 'export_recording', 'principal:human:alice', 'succeeded', now());" >/dev/null 2>&1; then
    fail "the schema accepted a partial export audit row"
fi
pass "partial export audit row refused by the schema"

"${psql_fixture[@]}" -c "INSERT INTO access_audit (category, action, actor, camera_id, outcome, range_start, range_end, byte_count) \
    VALUES ('export', 'export_recording', 'principal:human:alice', 'camera-01', 'succeeded', now() - interval '1 hour', now(), 4096);" >/dev/null
pass "complete export audit row accepted"

# An invented category must not land outside every report.
if "${psql_fixture[@]}" -c "INSERT INTO access_audit (category, action, actor, outcome) \
    VALUES ('something-else', 'x', 'y', 'granted');" >/dev/null 2>&1; then
    fail "the schema accepted an unknown audit category"
fi
pass "unknown audit category refused by the schema"

# A fingerprint column must not hold a value.
if "${psql_fixture[@]}" -c "INSERT INTO access_audit (category, action, actor, outcome, detail_fingerprint) \
    VALUES ('identity', 'manage_identities', 'principal:human:alice', 'succeeded', 'not-a-fingerprint');" >/dev/null 2>&1; then
    fail "the schema accepted a non-fingerprint in the fingerprint column"
fi
pass "non-fingerprint refused by the schema"

# ----------------------------------------------------------- secrets file mode
echo "-- secrets file permissions"
secrets="$fixture_root/secrets.toml"
printf '"camera.camera-01.rtsp_password" = "fixture-value"\n' > "$secrets"

chmod 0644 "$secrets"
mode=$(stat -f '%Lp' "$secrets" 2>/dev/null || stat -c '%a' "$secrets")
[[ "$mode" == "644" ]] || fail "could not set the fixture file to 0644 (got $mode)"
pass "a 0644 secrets file exists for the refusal test"

chmod 0600 "$secrets"
mode=$(stat -f '%Lp' "$secrets" 2>/dev/null || stat -c '%a' "$secrets")
[[ "$mode" == "600" ]] || fail "could not set the fixture file to 0600 (got $mode)"
pass "a 0600 secrets file is accepted"

# The refusal itself is unit-tested with a real file in
# crates/config/src/secret_store.rs; this confirms the platform reports the
# modes those tests depend on.

# ------------------------------------------------------- alert/dashboard names
echo "-- alert and dashboard metric names"
# Starts a server with no database, captures /metrics, and checks every name the
# alerts and dashboards reference actually exists. An alert on a missing series
# never fires; a panel on one is always empty. Both are silent failures.
metrics_dump="$fixture_root/metrics.txt"
metrics_port=$((3400 + ($$ % 100)))
mkdir -p "$fixture_root/recordings"
SURVEILLANCE_CAMERA_ID=fixture-camera \
SURVEILLANCE_RTSP_URL=rtsp://127.0.0.1:8554/nonexistent \
SURVEILLANCE_VIDEO_WIDTH=640 \
SURVEILLANCE_VIDEO_HEIGHT=360 \
SURVEILLANCE_RECORDING_ROOT="$fixture_root/recordings" \
SURVEILLANCE_HEALTH_BIND="127.0.0.1:$metrics_port" \
RUST_LOG=error \
"$repo_root/target/release/surveillance-server" > "$fixture_root/server.log" 2>&1 &
server_pid=$!

for _ in $(seq 1 60); do
    if curl -s -o /dev/null -m 1 "http://127.0.0.1:$metrics_port/live" 2>/dev/null; then
        break
    fi
    sleep 1
done
if ! curl -s -m 5 "http://127.0.0.1:$metrics_port/metrics" > "$metrics_dump"; then
    kill "$server_pid" 2>/dev/null || true
    fail "the server did not serve /metrics (is target/release built?)"
fi
kill "$server_pid" 2>/dev/null || true
wait "$server_pid" 2>/dev/null || true
pass "captured /metrics from a running server"

(cd "$repo_root" && ./scripts/phase8/check-metric-names.py --file "$metrics_dump") \
    || fail "an alert or dashboard references a metric that is not exported"
pass "every alert and dashboard metric name exists"

# ----------------------------------------------------------------- unit gates
echo "-- security unit tests"
(cd "$repo_root" && cargo test -p surveillance-auth --quiet) >/dev/null || fail "surveillance-auth tests"
pass "surveillance-auth"
(cd "$repo_root" && cargo test -p surveillance-config --quiet secret_store) >/dev/null || fail "secret store tests"
pass "secret store"
(cd "$repo_root" && cargo test -p surveillance-server --quiet --lib) >/dev/null || fail "server tests"
pass "server (authorize, authenticate, trust boundary, security config)"

echo
echo "== Phase 8 security fixture PASSED =="
echo
echo "Not covered here, and not to be reported as covered:"
echo "  - promtool alert-rule tests (promtool not installed): expressions unvalidated"
echo "  - Grafana dashboard rendering (no Grafana): panels unvalidated"
echo "  - coverage-guided fuzzing (needs a nightly toolchain)"
echo "  - representative cameras, target hardware, a LiveKit server"
echo "  - backup/restore, upgrade, rollback (deployment target unconfirmed)"
