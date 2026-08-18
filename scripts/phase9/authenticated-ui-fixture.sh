#!/usr/bin/env bash
set -euo pipefail

# Phase 9 authenticated-dashboard fixture.
#
# Two dashboard screens could never be verified because the page cannot
# authenticate itself: it sends no credential by design (ADR 009 Decision 6),
# so with any database configured every /api call answers 401. Playback with
# real segments and the settings write path both need a session, and a session
# only exists behind a reverse proxy that proves its origin and names a
# principal.
#
# This stands that up locally:
#
#   PostgreSQL (temp)  identities, recordings, settings
#   MediaMTX + FFmpeg  synthetic RTSP sources, so segments are real files
#   livekit-server     so a camera added through the UI can actually publish
#   surveillance-server  loopback, with a trust boundary configured
#   login proxy        the reverse proxy the trust boundary expects
#
# The proxy is a *fixture*, not a deployment artifact: it checks one generated
# password and keeps bounded sessions only in memory. It exists so the screens
# behind authentication can be driven without inventing a production provider.
#
# Usage:  scripts/phase9/authenticated-ui-fixture.sh
# Vars:   WINDOW (seconds to hold, default 1800), CAMERAS (default 2),
#         PRINCIPAL (default admin), ROLE (default administrator).

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/surveillance-phase9-ui.XXXXXX")
LOG_DIR="${WORK_DIR}/logs"
mkdir -p "${LOG_DIR}" "${WORK_DIR}/socket"

WINDOW="${WINDOW:-1800}"
CAMERAS="${CAMERAS:-2}"
PRINCIPAL="${PRINCIPAL:-admin}"
ROLE="${ROLE:-administrator}"
DEV_PASSWORD=$(python3 -c 'import secrets; print(secrets.token_urlsafe(12))')
WIDTH="${WIDTH:-640}"
HEIGHT="${HEIGHT:-360}"
FPS="${FPS:-15}"
GOP="${GOP:-30}"

RTSP_PORT="${RTSP_PORT:-28554}"
SERVER_PORT="${SERVER_PORT:-3310}"
PROXY_PORT="${PROXY_PORT:-3311}"
LIVEKIT_PORT="${LIVEKIT_PORT:-7880}"
PG_PORT="${PG_PORT:-$((57500 + ($$ % 400)))}"

LIVEKIT_API_KEY="devkey"
LIVEKIT_API_SECRET="development-secret-of-at-least-32-bytes"
# A fixture value. A deployment reads this from the secret store, never from a
# script, and never from anything committed.
PROOF_SECRET="fixture-proof-secret-not-for-any-deployment"

RECORDING_ROOT="${WORK_DIR}/recordings"
SOCKET_DIR="${WORK_DIR}/socket"
PG_DATA="${WORK_DIR}/pgdata"

PIDS=()
note() { printf '\n== %s\n' "$*"; }
fail() { printf '\nFAILED: %s\n' "$*" >&2; exit 1; }

cleanup() {
  local status=$?
  note "tearing down"
  for pid in "${PIDS[@]:-}"; do
    [[ -n "${pid}" ]] && kill "${pid}" 2>/dev/null || true
  done
  sleep 2
  for pid in "${PIDS[@]:-}"; do
    [[ -n "${pid}" ]] && kill -9 "${pid}" 2>/dev/null || true
  done
  if [[ -f "${PG_DATA}/postmaster.pid" ]]; then
    pg_ctl -D "${PG_DATA}" -m fast -w stop >/dev/null 2>&1 || true
  fi
  note "artifacts in ${WORK_DIR}"
  exit "${status}"
}
trap cleanup EXIT INT TERM

for tool in initdb pg_ctl psql mediamtx ffmpeg curl python3 livekit-server; do
  command -v "${tool}" >/dev/null || fail "missing ${tool}"
done

# ------------------------------------------------------------------- database
note "starting PostgreSQL on port ${PG_PORT}"
initdb -D "${PG_DATA}" --auth=trust --no-locale --encoding=UTF8 >/dev/null
pg_ctl -D "${PG_DATA}" -l "${LOG_DIR}/postgres.log" \
  -o "-h 127.0.0.1 -k ${SOCKET_DIR} -p ${PG_PORT}" -w start >/dev/null

PSQL=(psql -X -q -v ON_ERROR_STOP=1 -h 127.0.0.1 -p "${PG_PORT}" -d postgres)
DATABASE_URL="postgres://127.0.0.1:${PG_PORT}/postgres"

# The migrations are deliberately not applied here. The server owns them --
# every repository holds a `sqlx::migrate!("../../migrations")` and runs it at
# startup, recorded in `_sqlx_migrations`.
#
# This script used to apply them with psql first. The objects then existed while
# `_sqlx_migrations` was empty, so the server's first migration failed on a
# table that was already there, and because `run_pass` migrates before every
# pass, startup reconciliation was deferred and every later pass failed too.
# Recording and search kept working, which is why nobody noticed: the 2026-08-16
# run in docs/phase-9/ was taken with reconciliation silently degraded. Nothing
# it claims depended on reconciliation, but the run was not the clean posture it
# read as.
#
# So the principal is created after the server is up, below, because the table
# to put it in does not exist until then.

# ------------------------------------------------------------------------ SFU
note "starting livekit-server on ws://127.0.0.1:${LIVEKIT_PORT}"
cat > "${WORK_DIR}/livekit.yaml" <<EOF
port: ${LIVEKIT_PORT}
bind_addresses: ["127.0.0.1"]
rtc:
  tcp_port: 7881
  port_range_start: 51000
  port_range_end: 51200
  use_external_ip: false
keys:
  ${LIVEKIT_API_KEY}: ${LIVEKIT_API_SECRET}
logging:
  level: info
EOF
livekit-server --config "${WORK_DIR}/livekit.yaml" > "${LOG_DIR}/livekit.log" 2>&1 &
PIDS+=($!)

# -------------------------------------------------------------- video sources
# Two path families, because the two ways a camera is created carry different
# credentials. The bootstrap camera comes from the environment and reads
# `phase3-camera-N` with a username and password, so credentialed RTSP stays
# exercised. A camera added through the dashboard form carries none — the page
# holds no credential of any kind (ADR 009 Decision 6) — so it needs a source
# readable without one, and that is `open-camera-N`. Publishing stays
# authenticated on both: this is a fixture, not an open relay.
note "starting MediaMTX and $((CAMERAS + 1)) synthetic sources"
cat > "${WORK_DIR}/mediamtx.yml" <<EOF
logLevel: info
logDestinations: [stdout]

authMethod: internal
authInternalUsers:
  - user: fixture-publisher
    pass: fixture-publisher-pass
    ips: ["127.0.0.1"]
    permissions:
      - action: publish
        path: "~^(phase3|open)-camera-[0-9]+\$"
  - user: fixture-reader
    pass: fixture-reader-pass
    ips: ["127.0.0.1"]
    permissions:
      - action: read
        path: "~^phase3-camera-[0-9]+\$"
  - user: any
    ips: ["127.0.0.1"]
    permissions:
      - action: read
        path: "~^open-camera-[0-9]+\$"

rtsp: true
rtspTransports: [tcp]
rtspAddress: 127.0.0.1:${RTSP_PORT}

rtmp: false
hls: false
webrtc: false
srt: false
moq: false
api: false
metrics: false
pprof: false
playback: false

paths:
  "~^(phase3|open)-camera-[0-9]+\$":
EOF
mediamtx "${WORK_DIR}/mediamtx.yml" > "${LOG_DIR}/mediamtx.log" 2>&1 &
PIDS+=($!)
sleep 2

publish_source() {
  ffmpeg -hide_banner -loglevel error \
    -re -f lavfi -i "testsrc2=size=${WIDTH}x${HEIGHT}:rate=${FPS}" \
    -c:v libx264 -profile:v baseline -preset ultrafast -tune zerolatency \
    -g "${GOP}" -keyint_min "${GOP}" -sc_threshold 0 -bf 0 -an \
    -f rtsp -rtsp_transport tcp \
    "rtsp://fixture-publisher:fixture-publisher-pass@127.0.0.1:${RTSP_PORT}/$1" \
    > "${LOG_DIR}/source-$2.log" 2>&1 &
  PIDS+=($!)
}

publish_source "phase3-camera-1" "bootstrap"
for index in $(seq 1 "${CAMERAS}"); do
  publish_source "open-camera-${index}" "open-${index}"
done
sleep 5

# --------------------------------------------------------- system under test
note "building the server (release)"
( cd "${REPO_ROOT}" && cargo build --release -p surveillance-server ) \
  > "${LOG_DIR}/build.log" 2>&1 || fail "release build failed; see ${LOG_DIR}/build.log"

# SURVEILLANCE_API_MUTATING_SURFACE_ENABLED is set below because the Settings
# screen's write path is one of the things this fixture exists to drive. ADR 008
# ships it disabled, and while it is disabled `PUT /api/settings` answers 404 --
# a disabled surface does not advertise itself -- so without this the screen
# cannot be exercised at all. It is opt-in per deployment; a fixture opting in
# says nothing about what a deployment should do.
note "starting surveillance-server on 127.0.0.1:${SERVER_PORT}"
SURVEILLANCE_CAMERA_ID="camera-01" \
SURVEILLANCE_RTSP_URL="rtsp://127.0.0.1:${RTSP_PORT}/phase3-camera-1" \
SURVEILLANCE_RTSP_USERNAME="fixture-reader" \
SURVEILLANCE_RTSP_PASSWORD="fixture-reader-pass" \
SURVEILLANCE_VIDEO_WIDTH="${WIDTH}" \
SURVEILLANCE_VIDEO_HEIGHT="${HEIGHT}" \
SURVEILLANCE_RECORDING_ROOT="${RECORDING_ROOT}" \
SURVEILLANCE_HEALTH_BIND="127.0.0.1:${SERVER_PORT}" \
SURVEILLANCE_DATABASE_URL="${DATABASE_URL}" \
SURVEILLANCE_MAXIMUM_CAMERAS="$((CAMERAS + 4))" \
SURVEILLANCE_DETECTION_ENABLED="false" \
SURVEILLANCE_LIVEKIT_URL="ws://127.0.0.1:${LIVEKIT_PORT}" \
SURVEILLANCE_LIVEKIT_API_KEY="${LIVEKIT_API_KEY}" \
SURVEILLANCE_LIVEKIT_API_SECRET="${LIVEKIT_API_SECRET}" \
SURVEILLANCE_LIVEKIT_NAMESPACE="uifixture" \
SURVEILLANCE_TRUSTED_PROXY_PEERS="127.0.0.1" \
SURVEILLANCE_PROXY_PROOF_SECRET="${PROOF_SECRET}" \
SURVEILLANCE_API_MUTATING_SURFACE_ENABLED="true" \
RUST_LOG="info" \
"${REPO_ROOT}/target/release/surveillance-server" > "${LOG_DIR}/server.log" 2>&1 &
PIDS+=($!)

for _ in {1..60}; do
  curl -fsS -m 1 "http://127.0.0.1:${SERVER_PORT}/live" >/dev/null 2>&1 && break
  sleep 1
done
curl -fsS -m 2 "http://127.0.0.1:${SERVER_PORT}/live" >/dev/null \
  || fail "the server did not come up; see ${LOG_DIR}/server.log"
echo "  ok: server is live"

# The server's startup migrations have run by now, so `principals` exists.
#
# scope_all, because the point is to reach the settings and playback screens.
# A scoped principal is the interesting security case and is covered by unit
# tests; here it would only obscure which refusals are about scope.
"${PSQL[@]}" >/dev/null <<SQL
INSERT INTO principals (principal_id, kind, display_name, role, scope_all, enabled)
VALUES ('${PRINCIPAL}', 'human', 'Fixture ${ROLE}', '${ROLE}', true, true)
ON CONFLICT (principal_id) DO UPDATE SET role = EXCLUDED.role, enabled = true;
SQL
echo "  ok: principal ${PRINCIPAL} (${ROLE}, all cameras) is enabled"

# A failed migration does not stop the server, and its only visible symptom is a
# deferred reconciliation buried in the log. Ask directly instead.
applied=$("${PSQL[@]}" -tAc "SELECT count(*) FROM _sqlx_migrations WHERE success" 2>/dev/null || echo 0)
expected=$(ls "${REPO_ROOT}"/migrations/0*.sql | wc -l | tr -d ' ')
[[ "${applied}" == "${expected}" ]] \
  || fail "${applied} of ${expected} migrations applied; see ${LOG_DIR}/server.log"
echo "  ok: ${applied} migrations applied by the server"

# ------------------------------------------------------ development auth proxy
note "starting the development login proxy on http://127.0.0.1:${PROXY_PORT}"
# The proxy lives in scripts/dev/trust-boundary-proxy.py rather than in a
# heredoc here, because scripts/dev/local-stack.sh needs the same thing and a
# second copy of its STRIP set is a hole waiting to open.

UPSTREAM_PORT="${SERVER_PORT}" LISTEN_PORT="${PROXY_PORT}" \
PROOF_SECRET="${PROOF_SECRET}" PRINCIPAL="${PRINCIPAL}" \
DEV_USERNAME="${PRINCIPAL}" DEV_PASSWORD="${DEV_PASSWORD}" \
DEV_SESSION_TTL_SECONDS="3600" \
python3 "${REPO_ROOT}/scripts/dev/trust-boundary-proxy.py" > "${LOG_DIR}/proxy.log" 2>&1 &
PIDS+=($!)
sleep 2

status=$(curl -s -o /dev/null -w '%{http_code}' -m 5 "http://127.0.0.1:${PROXY_PORT}/api/cameras")
[[ "${status}" == "401" ]] || fail "the proxied API answered ${status} without login, not 401; see ${LOG_DIR}"
login_headers=$(curl -sS -D - -o /dev/null -m 5 \
  -H "Origin: http://127.0.0.1:${PROXY_PORT}" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "username=${PRINCIPAL}" \
  --data-urlencode "password=${DEV_PASSWORD}" \
  "http://127.0.0.1:${PROXY_PORT}/login")
session_cookie=$(printf '%s\n' "${login_headers}" \
  | awk 'BEGIN {IGNORECASE=1} /^Set-Cookie:/ {sub(/^[^:]*:[[:space:]]*/, ""); sub(/;.*/, ""); print; exit}' \
  | tr -d '\r')
status=$(curl -s -o /dev/null -w '%{http_code}' -m 5 \
  -H "Cookie: ${session_cookie}" "http://127.0.0.1:${PROXY_PORT}/api/cameras")
[[ "${status}" == "200" ]] || fail "the proxied API answered ${status} after login, not 200; see ${LOG_DIR}"
echo "  ok: /api/cameras answers 401 before login and 200 with a session"

direct=$(curl -s -o /dev/null -w '%{http_code}' -m 5 "http://127.0.0.1:${SERVER_PORT}/api/cameras")
[[ "${direct}" == "401" ]] || fail "the direct API answered ${direct}, expected 401"
echo "  ok: the same route answers 401 without the proxy, so the proxy is what authenticates"

cat <<EOF

== ready ==

  Dashboard (authenticated): http://127.0.0.1:${PROXY_PORT}/
  Login username: ${PRINCIPAL}
  Login password: ${DEV_PASSWORD}
  Dashboard (unauthenticated, for comparison): http://127.0.0.1:${SERVER_PORT}/
  Principal: ${PRINCIPAL} (${ROLE}, all cameras)
  Spare RTSP sources for the add form (readable without a credential):
$(for index in $(seq 1 "${CAMERAS}"); do
    echo "    rtsp://127.0.0.1:${RTSP_PORT}/open-camera-${index}"
  done)
  Logs: ${LOG_DIR}

Holding for ${WINDOW}s. Ctrl-C tears everything down.
EOF

sleep "${WINDOW}"
