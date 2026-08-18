#!/usr/bin/env bash
set -euo pipefail

# The whole local loop behind one command: `make run`.
#
#   PostgreSQL           identities, the recording index, persisted settings
#   MediaMTX             the loopback RTSP bridge
#   FFmpeg/AVFoundation  the Mac webcam, published into that bridge
#   livekit-server       so the camera's live tile carries actual video
#   surveillance-server  loopback, with a trust boundary configured
#   trust-boundary proxy the reverse proxy that boundary requires
#
# Why the last two are both here, and why this is not just `make server`:
# the dashboard holds no credential of any kind, by design (ADR 009 Decision 6).
# With a database configured an identity store exists, so every /api call the
# page makes answers 401 unless something in front of it proves the request's
# origin and names a principal. Without a database the page works but Playback
# has no index to search and Settings has nothing to persist to. Wanting every
# screen to work is what drags PostgreSQL and the proxy in.
#
# ---------------------------------------------------------------------------
# THE LOGIN IS A DEVELOPMENT FIXTURE. The proxy checks one per-run/environment
# password and owns bounded in-memory sessions before it asserts a principal.
# It has no account recovery, MFA, TLS, or durable session store. It binds to
# loopback, is not packaged, and does not replace ADR 009's production
# identity-aware reverse proxy.
# ---------------------------------------------------------------------------
#
# State persists between runs under STATE_DIR so recordings, settings and the
# identity survive a restart; that is the difference between this and
# scripts/phase9/authenticated-ui-fixture.sh, which is disposable by design.
#
# Usage:  make run          (or: scripts/dev/local-stack.sh)
# Vars:   WEBCAM_DEVICE VIDEO_WIDTH VIDEO_HEIGHT VIDEO_FPS VIDEO_BITRATE CAMERA_ID
#         RECORDING_ROOT STATE_DIR PRINCIPAL ROLE SENTINEL_BUILD_PROFILE,
#         SENTINEL_DEV_USERNAME, SENTINEL_DEV_PASSWORD, and the *_PORT values.

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

STATE_DIR="${STATE_DIR:-${REPO_ROOT}/.local/run}"
LOG_DIR="${STATE_DIR}/logs"
PG_DATA="${STATE_DIR}/pgdata"
SOCKET_DIR="${STATE_DIR}/socket"

WEBCAM_DEVICE="${WEBCAM_DEVICE:-0}"
VIDEO_WIDTH="${VIDEO_WIDTH:-1280}"
VIDEO_HEIGHT="${VIDEO_HEIGHT:-720}"
VIDEO_FPS="${VIDEO_FPS:-15}"
# The webcam delivers whatever rate it likes and libx264 will happily encode it
# unconstrained -- 3.91 Mbps measured here. `-r` forces the encoded rate and the
# cap keeps the fixture in the range a 720p IP camera occupies. Fixture values,
# not product configuration.
VIDEO_BITRATE="${VIDEO_BITRATE:-1500k}"
CAMERA_ID="${CAMERA_ID:-mac-webcam}"
RECORDING_ROOT="${RECORDING_ROOT:-${REPO_ROOT}/recordings}"
MEDIAMTX_CONFIG="${MEDIAMTX_CONFIG:-${REPO_ROOT}/config/mediamtx-webcam.yml}"
SENTINEL_BUILD_PROFILE="${SENTINEL_BUILD_PROFILE:-dev}"
case "${SENTINEL_BUILD_PROFILE}" in
  dev)
    CARGO_PROFILE_ARGS=()
    SERVER_BINARY="${REPO_ROOT}/target/debug/surveillance-server"
    ;;
  release)
    CARGO_PROFILE_ARGS=(--release)
    SERVER_BINARY="${REPO_ROOT}/target/release/surveillance-server"
    ;;
  *)
    printf 'SENTINEL_BUILD_PROFILE must be dev or release, got: %s\n' \
      "${SENTINEL_BUILD_PROFILE}" >&2
    exit 2
    ;;
esac

PRINCIPAL="${PRINCIPAL:-operator}"
ROLE="${ROLE:-administrator}"
SENTINEL_DEV_USERNAME="${SENTINEL_DEV_USERNAME:-operator}"
SENTINEL_DEV_SESSION_TTL_SECONDS="${SENTINEL_DEV_SESSION_TTL_SECONDS:-28800}"
if [[ -n "${SENTINEL_DEV_PASSWORD:-}" ]]; then
  DEV_PASSWORD="${SENTINEL_DEV_PASSWORD}"
  DEV_PASSWORD_SOURCE="environment"
else
  DEV_PASSWORD=$(python3 -c 'import secrets; print(secrets.token_urlsafe(12))')
  DEV_PASSWORD_SOURCE="generated"
fi

RTSP_PORT="${RTSP_PORT:-8554}"
RTSP_PATH="${RTSP_PATH:-webcam}"
SERVER_PORT="${SERVER_PORT:-3000}"
PROXY_PORT="${PROXY_PORT:-3001}"
LIVEKIT_PORT="${LIVEKIT_PORT:-7880}"
PG_PORT="${PG_PORT:-55432}"

RTSP_URL="rtsp://127.0.0.1:${RTSP_PORT}/${RTSP_PATH}"

# Generated per run and never written to disk. The origin proof is the only
# thing standing between a loopback caller and an asserted identity, so a value
# committed to a script would be a value everyone knows.
PROOF_SECRET=$(python3 -c 'import secrets; print(secrets.token_hex(32))')
LIVEKIT_API_KEY="devkey"
LIVEKIT_API_SECRET=$(python3 -c 'import secrets; print(secrets.token_hex(24))')

PIDS=()
timestamp() { date '+%Y-%m-%d %H:%M:%S %z'; }
status() {
  local level="$1"
  shift
  printf '[%s] %-6s %s\n' "$(timestamp)" "${level}" "$*"
}
banner() {
  cat <<'EOF'

 ███████╗███████╗███╗   ██╗████████╗██╗███╗   ██╗███████╗██╗
 ██╔════╝██╔════╝████╗  ██║╚══██╔══╝██║████╗  ██║██╔════╝██║
 ███████╗█████╗  ██╔██╗ ██║   ██║   ██║██╔██╗ ██║█████╗  ██║
 ╚════██║██╔══╝  ██║╚██╗██║   ██║   ██║██║╚██╗██║██╔══╝  ██║
 ███████║███████╗██║ ╚████║   ██║   ██║██║ ╚████║███████╗███████╗
 ╚══════╝╚══════╝╚═╝  ╚═══╝   ╚═╝   ╚═╝╚═╝  ╚═══╝╚══════╝╚══════╝

                 SURVEILLANCE CONTROL SYSTEM
                      LOCAL DEVELOPMENT

EOF
}
fail() { status "FAILED" "$*" >&2; exit 1; }

cleanup() {
  local status=$?
  printf '\n'
  if [[ "${status}" -eq 0 ]]; then
    status "STOP" "Shutting down SENTINEL"
  else
    status "STOP" "Shutting down SENTINEL after startup/runtime failure"
  fi
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
  status "SAVED" "Recordings · ${RECORDING_ROOT}"
  status "SAVED" "Logs       · ${LOG_DIR}"
  exit "${status}"
}
trap cleanup EXIT
# An operator's Ctrl-C or service-manager TERM is the requested shutdown path,
# not a runtime failure. EXIT retains real startup/child failures for cleanup.
trap 'exit 0' INT TERM

banner
status "BOOT" "Starting the SENTINEL local stack"

for tool in cargo initdb pg_ctl psql mediamtx ffmpeg ffprobe curl python3 livekit-server lsof; do
  command -v "${tool}" >/dev/null || fail "missing ${tool}"
done
# `lk` used to be required here because the publisher drove it as a child
# process. ADR 011 moved publication in-process, so the CLI is no longer part of
# the running stack; it remains useful as an independent subscriber when
# verifying what a viewer actually receives (scripts/phase5/livekit-fixture.sh).

# A previous run that was killed hard -- SIGKILL, a closed terminal -- never
# runs the trap, and its children outlive it. An orphaned proxy is the worst of
# them: it still answers on ${PROXY_PORT} holding the *previous* run's proof
# secret, so this run's requests reach the server, fail the origin proof, and
# come back 401. That reads as an authentication defect and is not one. Refuse
# to start instead, the same way the PostgreSQL check below does.
for port_check in \
  "${SERVER_PORT}:surveillance-server" \
  "${PROXY_PORT}:trust-boundary proxy" \
  "${LIVEKIT_PORT}:livekit-server" \
  "${RTSP_PORT}:MediaMTX"; do
  port="${port_check%%:*}"
  owner="${port_check#*:}"
  holder=$(lsof -nP -iTCP:"${port}" -sTCP:LISTEN -t 2>/dev/null | head -1 || true)
  [[ -z "${holder}" ]] || fail "port ${port} (${owner}) is already in use by pid ${holder}, \
probably orphaned by an earlier run; stop it with: kill ${holder}"
done

mkdir -p "${LOG_DIR}" "${SOCKET_DIR}" "${RECORDING_ROOT}"

# ------------------------------------------------------------------- database
if [[ -f "${PG_DATA}/postmaster.pid" ]] && pg_ctl -D "${PG_DATA}" status >/dev/null 2>&1; then
  fail "a PostgreSQL is already running on ${PG_DATA}; stop it with: pg_ctl -D ${PG_DATA} -m fast stop"
fi
if [[ ! -d "${PG_DATA}" ]]; then
  status "INIT" "PostgreSQL · ${PG_DATA}"
  initdb -D "${PG_DATA}" --auth=trust --no-locale --encoding=UTF8 >/dev/null
fi
status "START" "PostgreSQL · 127.0.0.1:${PG_PORT}"
pg_ctl -D "${PG_DATA}" -l "${LOG_DIR}/postgres.log" \
  -o "-h 127.0.0.1 -k ${SOCKET_DIR} -p ${PG_PORT}" -w start >/dev/null \
  || fail "PostgreSQL did not start; see ${LOG_DIR}/postgres.log"
status "READY" "PostgreSQL · 127.0.0.1:${PG_PORT}"

PSQL=(psql -X -q -v ON_ERROR_STOP=1 -h 127.0.0.1 -p "${PG_PORT}" -d postgres)
DATABASE_URL="postgres://127.0.0.1:${PG_PORT}/postgres"

# Nothing applies the migrations here, deliberately. The server owns them: each
# repository holds a `sqlx::migrate!("../../migrations")` and runs it at
# startup, tracked in `_sqlx_migrations`.
#
# Applying them with psql first looks harmless and is not. The objects then
# exist while `_sqlx_migrations` is empty, so the server's first migration fails
# on a table that is already there -- and the visible symptom is not a migration
# error but `startup reconciliation deferred; recording will start degraded`,
# because `run_pass` migrates before every pass. Recording and search keep
# working, so the run looks fine while reconciliation never runs once. This
# script did exactly that until a run proved it.
#
# The consequence is the ordering below: the principal cannot be inserted until
# the server has created the table to insert it into, so it comes after the
# server is live and before the proxy that depends on it.

# ------------------------------------------------------------------------ SFU
status "START" "LiveKit · ws://127.0.0.1:${LIVEKIT_PORT}"
umask 077
cat > "${STATE_DIR}/livekit.yaml" <<EOF
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
umask 022
livekit-server --config "${STATE_DIR}/livekit.yaml" > "${LOG_DIR}/livekit.log" 2>&1 &
PIDS+=($!)

# -------------------------------------------------------------- video source
status "START" "MediaMTX · ${RTSP_URL}"
mediamtx "${MEDIAMTX_CONFIG}" > "${LOG_DIR}/mediamtx.log" 2>&1 &
PIDS+=($!)
sleep 2

status "START" "Webcam device ${WEBCAM_DEVICE} · ${VIDEO_WIDTH}x${VIDEO_HEIGHT} @ ${VIDEO_FPS} FPS"
ffmpeg -hide_banner -loglevel error \
  -f avfoundation -framerate "${VIDEO_FPS}" \
  -video_size "${VIDEO_WIDTH}x${VIDEO_HEIGHT}" \
  -i "${WEBCAM_DEVICE}:none" -an -vf format=yuv420p \
  -r "${VIDEO_FPS}" \
  -c:v libx264 -profile:v baseline -preset veryfast -tune zerolatency \
  -b:v "${VIDEO_BITRATE}" -maxrate "${VIDEO_BITRATE}" -bufsize "${VIDEO_BITRATE}" \
  -g "${VIDEO_FPS}" -keyint_min "${VIDEO_FPS}" -sc_threshold 0 -bf 0 \
  -f rtsp -rtsp_transport tcp "${RTSP_URL}" \
  > "${LOG_DIR}/webcam.log" 2>&1 &
PIDS+=($!)

# Prove the bridge carries video before starting anything that depends on it,
# so a denied camera permission surfaces here with an actionable message rather
# than as a camera stuck in `reconnecting` on a dashboard three steps later.
ready=""
for _ in $(seq 1 30); do
  if ffprobe -v error -rtsp_transport tcp -i "${RTSP_URL}" \
      -show_entries stream=codec_name -of csv=p=0 >/dev/null 2>&1; then
    ready="yes"
    break
  fi
  sleep 1
done
[[ -n "${ready}" ]] || fail "no video on ${RTSP_URL} after 30s.
  If this is the first run, macOS has to be told the terminal may use the
  camera: System Settings > Privacy & Security > Camera. List devices with
  'make webcam-devices' and pass WEBCAM_DEVICE=<n> if 0 is the wrong one.
  FFmpeg's own output is in ${LOG_DIR}/webcam.log"
status "READY" "${CAMERA_ID} · ${VIDEO_WIDTH}x${VIDEO_HEIGHT} @ ${VIDEO_FPS} FPS · H.264"

# --------------------------------------------------------------------- server
status "BUILD" "surveillance-server · ${SENTINEL_BUILD_PROFILE}"
( cd "${REPO_ROOT}" && cargo build "${CARGO_PROFILE_ARGS[@]}" -p surveillance-server ) \
  > "${LOG_DIR}/build.log" 2>&1 || fail "release build failed; see ${LOG_DIR}/build.log"

# Two opt-ins are set here that a deployment must decide for itself:
#
#   API_MUTATING_SURFACE_ENABLED  ADR 008 ships this false, and while it is
#       false PUT /api/settings answers 404 -- a disabled surface does not
#       advertise itself. The Settings screen cannot save anything without it.
#   DETECTION_ENABLED=false       detection has no production model; the only
#       PersonInference is DeterministicFakeInference. Running it here would
#       spend a decode budget on a fixture and put invented-looking events on
#       the dashboard.
status "START" "API server · 127.0.0.1:${SERVER_PORT}"
SURVEILLANCE_CAMERA_ID="${CAMERA_ID}" \
SURVEILLANCE_RTSP_URL="${RTSP_URL}" \
SURVEILLANCE_VIDEO_WIDTH="${VIDEO_WIDTH}" \
SURVEILLANCE_VIDEO_HEIGHT="${VIDEO_HEIGHT}" \
SURVEILLANCE_RECORDING_ROOT="${RECORDING_ROOT}" \
SURVEILLANCE_HEALTH_BIND="127.0.0.1:${SERVER_PORT}" \
SURVEILLANCE_DATABASE_URL="${DATABASE_URL}" \
SURVEILLANCE_DETECTION_ENABLED="false" \
SURVEILLANCE_LIVEKIT_URL="ws://127.0.0.1:${LIVEKIT_PORT}" \
SURVEILLANCE_LIVEKIT_API_KEY="${LIVEKIT_API_KEY}" \
SURVEILLANCE_LIVEKIT_API_SECRET="${LIVEKIT_API_SECRET}" \
SURVEILLANCE_LIVEKIT_NAMESPACE="local" \
SURVEILLANCE_TRUSTED_PROXY_PEERS="127.0.0.1" \
SURVEILLANCE_PROXY_PROOF_SECRET="${PROOF_SECRET}" \
SURVEILLANCE_API_MUTATING_SURFACE_ENABLED="true" \
RUST_LOG="${RUST_LOG:-info}" \
"${SERVER_BINARY}" > "${LOG_DIR}/server.log" 2>&1 &
PIDS+=($!)
server_wait_started=$(date +%s)
status "WAIT" "API startup · initializing control plane"

# Generous, because startup reconciliation walks the whole recording root before
# the listener binds, and that root persists between runs. A first run against
# an empty root binds in about a second; a root with a few hundred segments in
# it takes appreciably longer, and treating that as a failure would punish
# exactly the long-lived development loop this target exists to support.
for _ in $(seq 1 180); do
  curl -fsS -m 1 "http://127.0.0.1:${SERVER_PORT}/live" >/dev/null 2>&1 && break
  sleep 1
done
curl -fsS -m 2 "http://127.0.0.1:${SERVER_PORT}/live" >/dev/null \
  || fail "the server did not answer /live within 180s; see ${LOG_DIR}/server.log"
server_wait_seconds=$(( $(date +%s) - server_wait_started ))
status "READY" "API server · 127.0.0.1:${SERVER_PORT} · ${server_wait_seconds}s"

# The migrations ran as part of that startup, so `principals` exists now and not
# a moment earlier. Idempotent, because the cluster survives between runs.
"${PSQL[@]}" >/dev/null <<SQL || fail "could not create the principal; did the server's migrations run? see ${LOG_DIR}/server.log"
INSERT INTO principals (principal_id, kind, display_name, role, scope_all, enabled)
VALUES ('${PRINCIPAL}', 'human', 'Local ${ROLE}', '${ROLE}', true, true)
ON CONFLICT (principal_id) DO UPDATE SET role = EXCLUDED.role, enabled = true;
SQL
status "READY" "Operator · ${PRINCIPAL} · ${ROLE} · all cameras"

# A migration that failed does not stop the server, so ask directly rather than
# trusting that "server is live" meant the schema is whole.
pending=$("${PSQL[@]}" -tAc "SELECT count(*) FROM _sqlx_migrations WHERE success" 2>/dev/null || echo 0)
expected=$(ls "${REPO_ROOT}"/migrations/0*.sql | wc -l | tr -d ' ')
[[ "${pending}" == "${expected}" ]] \
  || fail "${pending} of ${expected} migrations applied; reconciliation would run degraded. See ${LOG_DIR}/server.log"
status "READY" "Database · ${pending}/${expected} migrations"

# ---------------------------------------------------------------------- proxy
status "START" "Development login proxy · http://127.0.0.1:${PROXY_PORT}"
UPSTREAM_PORT="${SERVER_PORT}" LISTEN_PORT="${PROXY_PORT}" \
PROOF_SECRET="${PROOF_SECRET}" PRINCIPAL="${PRINCIPAL}" \
DEV_USERNAME="${SENTINEL_DEV_USERNAME}" DEV_PASSWORD="${DEV_PASSWORD}" \
DEV_SESSION_TTL_SECONDS="${SENTINEL_DEV_SESSION_TTL_SECONDS}" \
python3 "${REPO_ROOT}/scripts/dev/trust-boundary-proxy.py" \
  > "${LOG_DIR}/proxy.log" 2>&1 &
PIDS+=($!)
sleep 2

# Prove all three sides of the boundary: the proxy refuses a request without a
# session, its own login can establish one, and the server refuses a direct
# request that lacks both bearer credentials and the proxy's origin proof.
unauthenticated=$(curl -s -o /dev/null -w '%{http_code}' -m 5 \
  "http://127.0.0.1:${PROXY_PORT}/api/cameras")
[[ "${unauthenticated}" == "401" ]] \
  || fail "the proxied API answered ${unauthenticated} without a session, expected 401; see ${LOG_DIR}"
login_headers=$(curl -sS -D - -o /dev/null -m 5 \
  -H "Origin: http://127.0.0.1:${PROXY_PORT}" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "username=${SENTINEL_DEV_USERNAME}" \
  --data-urlencode "password=${DEV_PASSWORD}" \
  "http://127.0.0.1:${PROXY_PORT}/login") \
  || fail "the development login request failed; see ${LOG_DIR}/proxy.log"
login_status=$(printf '%s\n' "${login_headers}" | awk 'NR == 1 {print $2}')
SESSION_COOKIE=$(printf '%s\n' "${login_headers}" \
  | awk 'BEGIN {IGNORECASE=1} /^Set-Cookie:/ {sub(/^[^:]*:[[:space:]]*/, ""); sub(/;.*/, ""); print; exit}' \
  | tr -d '\r')
[[ "${login_status}" == "303" && -n "${SESSION_COOKIE}" ]] \
  || fail "development login did not establish a session; see ${LOG_DIR}/proxy.log"
proxied=$(curl -s -o /dev/null -w '%{http_code}' -m 5 \
  -H "Cookie: ${SESSION_COOKIE}" "http://127.0.0.1:${PROXY_PORT}/api/cameras")
[[ "${proxied}" == "200" ]] || fail "the authenticated proxied API answered ${proxied}, not 200; see ${LOG_DIR}"
direct=$(curl -s -o /dev/null -w '%{http_code}' -m 5 "http://127.0.0.1:${SERVER_PORT}/api/cameras")
[[ "${direct}" == "401" ]] || fail "the direct API answered ${direct}, expected 401"
status "READY" "Security boundary · anonymous 401 · session 200 · direct 401"
status "AUTH" "Username · ${SENTINEL_DEV_USERNAME}"
if [[ "${DEV_PASSWORD_SOURCE}" == "generated" ]]; then
  status "AUTH" "Generated password · ${DEV_PASSWORD}"
else
  status "AUTH" "Password · supplied through SENTINEL_DEV_PASSWORD"
fi

cat <<EOF

┌────────────────────── SENTINEL ONLINE ───────────────────────┐
│
│  LIVE VIEW      http://127.0.0.1:${PROXY_PORT}/
│  API SERVER     http://127.0.0.1:${SERVER_PORT}/
│  CAMERA         ${CAMERA_ID}
│  RTSP SOURCE    ${RTSP_URL}
│  RECORDINGS     ${RECORDING_ROOT}
│  LOGS           ${LOG_DIR}
│  LOGIN          ${SENTINEL_DEV_USERNAME} · ${ROLE} · all cameras
│  BUILD          ${SENTINEL_BUILD_PROFILE} · no performance claim
│
└──────────────────────────────────────────────────────────────┘

⚠  DEVELOPMENT SECURITY MODE

   Port ${SERVER_PORT} is the server itself and answers 401 on protected API routes.
   Only port ${PROXY_PORT} carries the development session.

   The loopback proxy uses an in-memory development login and session.
   It must not be exposed or used as the production identity provider.

   Press Ctrl-C to stop SENTINEL.
   Recordings, settings, and database state will be preserved in:
   ${PG_DATA}
EOF

# Hold. Quiet while healthy, one line when that changes, so an unattended run
# does not scroll and a camera that drops is not silent.
healthy="yes"
while true; do
  sleep 15
  if curl -fsS -m 3 -H "Cookie: ${SESSION_COOKIE}" \
      "http://127.0.0.1:${PROXY_PORT}/api/cameras" >/dev/null 2>&1; then
    if [[ "${healthy}" != "yes" ]]; then
      status "READY" "API recovered"
      healthy="yes"
    fi
  elif [[ "${healthy}" == "yes" ]]; then
    status "WARN" "API stopped answering · see ${LOG_DIR}"
    healthy="no"
  fi
done
