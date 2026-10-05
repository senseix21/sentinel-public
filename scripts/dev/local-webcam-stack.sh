#!/usr/bin/env bash
set -euo pipefail

# Local development stack without an RTSP proxy:
# webcam -> server-owned FFmpeg -> bounded H.264 distributor -> recorder/LiveKit.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
state_dir="${STATE_DIR:-${repo_root}/.local/webcam-run}"
log_dir="${state_dir}/logs"
pg_data="${state_dir}/pgdata"
socket_dir="${state_dir}/socket"
camera_id="${CAMERA_ID:-mac-webcam}"
webcam_device="${WEBCAM_DEVICE:-0}"
webcam_backend="${WEBCAM_BACKEND:-}"
video_width="${VIDEO_WIDTH:-1280}"
video_height="${VIDEO_HEIGHT:-720}"
video_fps="${VIDEO_FPS:-15}"
video_bitrate_kbps="${VIDEO_BITRATE_KBPS:-1500}"
recording_root="${RECORDING_ROOT:-${repo_root}/recordings}"
build_profile="${SENTINEL_BUILD_PROFILE:-dev}"
livekit_api_key="devkey"
livekit_api_secret="local-development-secret-not-for-deployment"
server_port="${SERVER_PORT:-3000}"
proxy_port="${PROXY_PORT:-3001}"
livekit_port="${LIVEKIT_PORT:-7880}"
pg_port="${PG_PORT:-55432}"
principal="${PRINCIPAL:-operator}"
role="${ROLE:-administrator}"
dev_username="${SENTINEL_DEV_USERNAME:-operator}"
dev_password="${SENTINEL_DEV_PASSWORD:-sentinel-local}"
proof_secret="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"

timestamp() { date '+%Y-%m-%d %H:%M:%S %z'; }
status() {
  local level="$1"
  shift
  printf '[%s] %-6s %s\n' "$(timestamp)" "${level}" "$*"
}
fail() {
  status "FAILED" "$*" >&2
  exit 1
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

case "${build_profile}" in
  dev)
    cargo_profile=()
    server_binary="${repo_root}/target/debug/surveillance-server"
    ;;
  release)
    cargo_profile=(--release)
    server_binary="${repo_root}/target/release/surveillance-server"
    ;;
  *)
    fail "SENTINEL_BUILD_PROFILE must be dev or release; got ${build_profile}"
    ;;
esac

if [[ -z "${webcam_backend}" ]]; then
  case "$(uname -s)" in
    Darwin) webcam_backend="avfoundation" ;;
    Linux) webcam_backend="v4l2" ;;
    *) webcam_backend="dshow" ;;
  esac
fi

for tool in cargo initdb pg_ctl psql ffmpeg ffprobe curl livekit-server lsof python3; do
  command -v "${tool}" >/dev/null || fail "missing required executable: ${tool}"
done

mkdir -p "${log_dir}" "${recording_root}" "${socket_dir}"

for port in "${server_port}" "${proxy_port}" "${livekit_port}" "${pg_port}"; do
  holder="$(lsof -nP -iTCP:"${port}" -sTCP:LISTEN -t 2>/dev/null | head -1 || true)"
  if [[ -n "${holder}" ]]; then
    fail "port ${port} is already in use by pid ${holder}; stop that process and retry"
  fi
done

livekit_pid=""
server_pid=""
proxy_pid=""

stop_and_join() {
  local pid="$1"
  local signal="$2"
  [[ -n "${pid}" ]] || return 0
  kill -0 "${pid}" 2>/dev/null || return 0
  kill "-${signal}" "${pid}" 2>/dev/null || true
  for _ in {1..300}; do
    kill -0 "${pid}" 2>/dev/null || {
      wait "${pid}" 2>/dev/null || true
      return 0
    }
    sleep 0.1
  done
  kill -KILL "${pid}" 2>/dev/null || true
  wait "${pid}" 2>/dev/null || true
}

cleanup() {
  local exit_status=$?
  trap - EXIT INT TERM
  printf '\n'
  if [[ "${exit_status}" -eq 0 ]]; then
    status "STOP" "Shutting down SENTINEL"
  else
    status "STOP" "Shutting down after startup or runtime failure"
  fi
  if [[ -n "${server_pid}" ]] && kill -0 "${server_pid}" 2>/dev/null; then
    status "JOIN" "Rust server and owned FFmpeg source"
  fi
  stop_and_join "${server_pid}" INT
  if [[ -n "${proxy_pid}" ]] && kill -0 "${proxy_pid}" 2>/dev/null; then
    status "JOIN" "Development authentication proxy"
  fi
  stop_and_join "${proxy_pid}" TERM
  if [[ -n "${livekit_pid}" ]] && kill -0 "${livekit_pid}" 2>/dev/null; then
    status "JOIN" "LiveKit"
  fi
  stop_and_join "${livekit_pid}" TERM
  if [[ -f "${pg_data}/postmaster.pid" ]]; then
    status "JOIN" "PostgreSQL"
    pg_ctl -D "${pg_data}" -m fast -w stop >/dev/null 2>&1 || true
  fi
  status "SAVED" "Recordings · ${recording_root}"
  status "SAVED" "Logs       · ${log_dir}"
  exit "${exit_status}"
}
trap cleanup EXIT
trap 'exit 0' INT TERM

banner
status "BOOT" "Starting the MediaMTX-free local stack"
status "SOURCE" "${camera_id} · ${webcam_backend}:${webcam_device} · ${video_width}x${video_height} @ ${video_fps} FPS · ${video_bitrate_kbps} kbps"
if [[ -f "${pg_data}/postmaster.pid" ]] && pg_ctl -D "${pg_data}" status >/dev/null 2>&1; then
  fail "PostgreSQL is already running from ${pg_data}; stop it before retrying"
fi
if [[ ! -d "${pg_data}" ]]; then
  status "INIT" "PostgreSQL · ${pg_data}"
  initdb -D "${pg_data}" --auth=trust --no-locale --encoding=UTF8 >/dev/null
fi
status "START" "PostgreSQL · 127.0.0.1:${pg_port}"
pg_ctl -D "${pg_data}" -l "${log_dir}/postgres.log" \
  -o "-h 127.0.0.1 -k ${socket_dir} -p ${pg_port}" -w start >/dev/null \
  || fail "PostgreSQL did not start; see ${log_dir}/postgres.log"
status "READY" "PostgreSQL · 127.0.0.1:${pg_port}"
database_url="postgres://127.0.0.1:${pg_port}/postgres"
psql_command=(psql -X -q -v ON_ERROR_STOP=1 -h 127.0.0.1 -p "${pg_port}" -d postgres)

status "START" "LiveKit · ws://127.0.0.1:${livekit_port}"
livekit-server --dev --bind 127.0.0.1 --config-body "port: ${livekit_port}" \
  --keys "${livekit_api_key}: ${livekit_api_secret}" \
  >"${log_dir}/livekit.log" 2>&1 &
livekit_pid=$!
for _ in {1..100}; do
  if lsof -nP -iTCP:"${livekit_port}" -sTCP:LISTEN -t 2>/dev/null | grep -q .; then
    break
  fi
  kill -0 "${livekit_pid}" 2>/dev/null || {
    fail "LiveKit exited during startup; see ${log_dir}/livekit.log"
  }
  sleep 0.1
done
lsof -nP -iTCP:"${livekit_port}" -sTCP:LISTEN -t 2>/dev/null | grep -q . \
  || fail "LiveKit readiness timed out; see ${log_dir}/livekit.log"
status "READY" "LiveKit · ws://127.0.0.1:${livekit_port}"

status "BUILD" "surveillance-server · ${build_profile}"
cargo_features=()
if [[ -n "${SURVEILLANCE_CARGO_FEATURES:-}" ]]; then
  cargo_features=(--features "${SURVEILLANCE_CARGO_FEATURES}")
fi
(cd "${repo_root}" && cargo build -p surveillance-server "${cargo_profile[@]}" "${cargo_features[@]}") \
  >"${log_dir}/build.log" 2>&1 \
  || fail "server build failed; see ${log_dir}/build.log"
status "READY" "Binary · ${server_binary}"

export SURVEILLANCE_CAMERA_ID="${camera_id}"
# Required by the legacy bootstrap settings shape and replaced before pipeline
# construction when SURVEILLANCE_WEBCAM_DEVICE is present.
export SURVEILLANCE_RTSP_URL="rtsp://127.0.0.1/unused-webcam-source"
export SURVEILLANCE_WEBCAM_DEVICE="${webcam_device}"
export SURVEILLANCE_WEBCAM_BACKEND="${webcam_backend}"
export SURVEILLANCE_WEBCAM_FPS="${video_fps}"
export SURVEILLANCE_WEBCAM_BITRATE_KBPS="${video_bitrate_kbps}"
export SURVEILLANCE_WEBCAM_KEYFRAME_INTERVAL="${video_fps}"
export SURVEILLANCE_VIDEO_WIDTH="${video_width}"
export SURVEILLANCE_VIDEO_HEIGHT="${video_height}"
export SURVEILLANCE_RECORDING_ROOT="${recording_root}"
export SURVEILLANCE_HEALTH_BIND="127.0.0.1:${server_port}"
export SURVEILLANCE_DATABASE_URL="${database_url}"
export SURVEILLANCE_DETECTION_ENABLED="${SURVEILLANCE_DETECTION_ENABLED:-false}"
export SURVEILLANCE_TRUSTED_PROXY_PEERS="127.0.0.1"
export SURVEILLANCE_PROXY_PROOF_SECRET="${proof_secret}"
export SURVEILLANCE_API_MUTATING_SURFACE_ENABLED="true"
export SURVEILLANCE_LIVEKIT_URL="ws://127.0.0.1:${livekit_port}"
export SURVEILLANCE_LIVEKIT_API_KEY="${livekit_api_key}"
export SURVEILLANCE_LIVEKIT_API_SECRET="${livekit_api_secret}"
export SURVEILLANCE_LIVEKIT_NAMESPACE="local"
export RUST_LOG="${RUST_LOG:-info}"

status "START" "Rust server + owned FFmpeg source · http://127.0.0.1:${server_port}"
"${server_binary}" >"${log_dir}/server.log" 2>&1 &
server_pid=$!
pipeline_ready=""
for _ in {1..300}; do
  if curl -fsS "http://127.0.0.1:${server_port}/ready" >/dev/null 2>&1; then
    pipeline_ready="yes"
    break
  fi
  kill -0 "${server_pid}" 2>/dev/null || fail "server exited during startup; see ${log_dir}/server.log"
  sleep 0.1
done
[[ -n "${pipeline_ready}" ]] \
  || fail "camera pipeline readiness timed out; check camera permission/device and ${log_dir}/server.log"
status "READY" "Camera pipeline · connected and recording"

"${psql_command[@]}" >/dev/null <<SQL \
  || fail "could not create the local principal; see ${log_dir}/server.log"
INSERT INTO principals (principal_id, kind, display_name, role, scope_all, enabled)
VALUES ('${principal}', 'human', 'Local ${role}', '${role}', true, true)
ON CONFLICT (principal_id) DO UPDATE SET role = EXCLUDED.role, enabled = true;
SQL
status "READY" "Recording index and operator identity · PostgreSQL"

status "START" "Development login proxy · http://127.0.0.1:${proxy_port}"
UPSTREAM_PORT="${server_port}" LISTEN_PORT="${proxy_port}" \
PROOF_SECRET="${proof_secret}" PRINCIPAL="${principal}" \
DEV_USERNAME="${dev_username}" DEV_PASSWORD="${dev_password}" \
python3 "${repo_root}/scripts/dev/trust-boundary-proxy.py" \
  >"${log_dir}/proxy.log" 2>&1 &
proxy_pid=$!
for _ in {1..100}; do
  if lsof -nP -iTCP:"${proxy_port}" -sTCP:LISTEN -t 2>/dev/null | grep -q .; then
    break
  fi
  kill -0 "${proxy_pid}" 2>/dev/null || fail "proxy exited during startup; see ${log_dir}/proxy.log"
  sleep 0.1
done
lsof -nP -iTCP:"${proxy_port}" -sTCP:LISTEN -t 2>/dev/null | grep -q . \
  || fail "proxy readiness timed out; see ${log_dir}/proxy.log"
status "READY" "Development authentication · ${dev_username} / supplied local password"

cat <<EOF

┌────────────────────── SENTINEL ONLINE ───────────────────────┐
│
│  DASHBOARD      http://127.0.0.1:${proxy_port}/
│  API / HEALTH   http://127.0.0.1:${server_port}/ready
│  LIVEKIT        ws://127.0.0.1:${livekit_port}
│  CAMERA         ${camera_id}
│  SOURCE         FFmpeg · ${webcam_backend}:${webcam_device}
│  VIDEO          ${video_width}x${video_height} @ ${video_fps} FPS · H.264
│  LOGIN          ${dev_username} / ${dev_password}
│  RECORDINGS     ${recording_root}
│  LOGS           ${log_dir}
│  BUILD          ${build_profile} · no performance claim
│
└──────────────────────────────────────────────────────────────┘

   MediaMTX is not running. FFmpeg is owned by the Rust camera session.
   Press Ctrl-C once for graceful recording finalization.
EOF

wait "${server_pid}"
