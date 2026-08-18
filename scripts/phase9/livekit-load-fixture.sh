#!/usr/bin/env bash
# Phase 9 (scale from measurements): publish N synthetic cameras to LiveKit while
# M viewers subscribe, and record what it costs.
#
# This exists because `AGENTS.md` forbids claiming a camera count from
# architecture. It produces a documented workload and a repeatable measurement,
# not a verdict. Read `docs/phase-9/livekit-load-*.md` for what a run meant.
#
# What is deliberately NOT part of the measured system:
#   - MediaMTX and the FFmpeg `testsrc2` publishers are the *simulated cameras*.
#     They run on the same machine and their cost is reported separately so it is
#     never mistaken for the platform's.
#   - `livekit-server` is the SFU. In a real deployment it may sit elsewhere, so
#     its cost is also reported on its own line.
#
# Requires: livekit-server, lk, mediamtx, ffmpeg, curl, python3.

set -Eeuo pipefail

CAMERAS="${CAMERAS:-10}"
VIEWERS="${VIEWERS:-10}"
# Steady-state measurement window, in seconds, after every camera is publishing
# and every viewer has subscribed. Warm-up is excluded by construction.
WINDOW="${WINDOW:-180}"
SAMPLE_EVERY="${SAMPLE_EVERY:-5}"

WIDTH="${WIDTH:-640}"
HEIGHT="${HEIGHT:-360}"
FPS="${FPS:-15}"
GOP="${GOP:-30}"

RTSP_PORT="${RTSP_PORT:-28554}"
HEALTH_BIND="${HEALTH_BIND:-127.0.0.1:3300}"
LIVEKIT_BIND_PORT="${LIVEKIT_BIND_PORT:-7880}"
LIVEKIT_URL="ws://127.0.0.1:${LIVEKIT_BIND_PORT}"
LIVEKIT_API_KEY="${LIVEKIT_API_KEY:-devkey}"
LIVEKIT_API_SECRET="${LIVEKIT_API_SECRET:-development-secret-of-at-least-32-bytes}"
LIVEKIT_NAMESPACE="${LIVEKIT_NAMESPACE:-loadtest}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/livekit-load.XXXXXX")"
LOG_DIR="${WORK_DIR}/logs"
RECORDING_ROOT="${WORK_DIR}/recordings"
mkdir -p "${LOG_DIR}" "${RECORDING_ROOT}"

PIDS=()
note() { printf '[%s] %s\n' "$(date -u +%H:%M:%S)" "$*"; }
fail() { printf '[%s] FAILED: %s\n' "$(date -u +%H:%M:%S)" "$*" >&2; exit 1; }

cleanup() {
  local status=$?
  note "stopping ${#PIDS[@]} process(es)"
  for pid in "${PIDS[@]:-}"; do
    [[ -n "${pid}" ]] && kill "${pid}" 2>/dev/null || true
  done
  sleep 2
  for pid in "${PIDS[@]:-}"; do
    [[ -n "${pid}" ]] && kill -9 "${pid}" 2>/dev/null || true
  done
  note "artifacts in ${WORK_DIR}"
  exit "${status}"
}
trap cleanup EXIT INT TERM

for tool in livekit-server lk mediamtx ffmpeg curl python3; do
  command -v "${tool}" >/dev/null || fail "missing ${tool}"
done

# ---------------------------------------------------------------- SFU
cat > "${WORK_DIR}/livekit.yaml" <<EOF
port: ${LIVEKIT_BIND_PORT}
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

note "starting livekit-server on ${LIVEKIT_URL}"
livekit-server --config "${WORK_DIR}/livekit.yaml" > "${LOG_DIR}/livekit-server.log" 2>&1 &
LIVEKIT_PID=$!
PIDS+=("${LIVEKIT_PID}")

for _ in {1..50}; do
  curl -fsS "http://127.0.0.1:${LIVEKIT_BIND_PORT}/" >/dev/null 2>&1 && break
  sleep 0.2
done

# ---------------------------------------------------------------- simulated cameras
note "starting MediaMTX on 127.0.0.1:${RTSP_PORT}"
mediamtx "${REPO_ROOT}/tests/fixtures/mediamtx-phase3.yml" > "${LOG_DIR}/mediamtx.log" 2>&1 &
MEDIAMTX_PID=$!
PIDS+=("${MEDIAMTX_PID}")
sleep 2

SOURCE_PIDS=()
for index in $(seq 1 "${CAMERAS}"); do
  path="phase3-camera-${index}"
  ffmpeg -hide_banner -loglevel error \
    -re -f lavfi -i "testsrc2=size=${WIDTH}x${HEIGHT}:rate=${FPS}" \
    -c:v libx264 -profile:v baseline -preset ultrafast -tune zerolatency \
    -g "${GOP}" -keyint_min "${GOP}" -sc_threshold 0 -bf 0 -an \
    -f rtsp -rtsp_transport tcp \
    "rtsp://fixture-publisher:fixture-publisher-pass@127.0.0.1:${RTSP_PORT}/${path}" \
    > "${LOG_DIR}/source-${index}.log" 2>&1 &
  SOURCE_PIDS+=($!)
  PIDS+=($!)
done
note "started ${CAMERAS} synthetic RTSP sources at ${WIDTH}x${HEIGHT}@${FPS}"
sleep 5

# ---------------------------------------------------------------- the system under test
note "building the server (release)"
( cd "${REPO_ROOT}" && cargo build --release -p surveillance-server ) \
  > "${LOG_DIR}/build.log" 2>&1 || fail "release build failed; see ${LOG_DIR}/build.log"

note "starting the server"
SURVEILLANCE_CAMERA_ID="camera-01" \
SURVEILLANCE_RTSP_URL="rtsp://127.0.0.1:${RTSP_PORT}/phase3-camera-1" \
SURVEILLANCE_RTSP_USERNAME="fixture-reader" \
SURVEILLANCE_RTSP_PASSWORD="fixture-reader-pass" \
SURVEILLANCE_VIDEO_WIDTH="${WIDTH}" \
SURVEILLANCE_VIDEO_HEIGHT="${HEIGHT}" \
SURVEILLANCE_RECORDING_ROOT="${RECORDING_ROOT}" \
SURVEILLANCE_HEALTH_BIND="${HEALTH_BIND}" \
SURVEILLANCE_MAXIMUM_CAMERAS="$((CAMERAS + 2))" \
SURVEILLANCE_DETECTION_ENABLED="false" \
SURVEILLANCE_LIVEKIT_URL="${LIVEKIT_URL}" \
SURVEILLANCE_LIVEKIT_API_KEY="${LIVEKIT_API_KEY}" \
SURVEILLANCE_LIVEKIT_API_SECRET="${LIVEKIT_API_SECRET}" \
SURVEILLANCE_LIVEKIT_NAMESPACE="${LIVEKIT_NAMESPACE}" \
RUST_LOG="info" \
"${REPO_ROOT}/target/release/surveillance-server" > "${LOG_DIR}/server.log" 2>&1 &
SERVER_PID=$!
PIDS+=("${SERVER_PID}")

for _ in {1..100}; do
  curl -fsS "http://${HEALTH_BIND}/live" >/dev/null 2>&1 && break
  sleep 0.3
done
curl -fsS "http://${HEALTH_BIND}/live" >/dev/null || fail "server never became live"

for ((index = 2; index <= CAMERAS; index++)); do
  id="$(printf 'camera-%02d' "${index}")"
  curl -fsS -X POST "http://${HEALTH_BIND}/api/cameras" \
    -H 'content-type: application/json' \
    -d "{\"camera_id\":\"${id}\",\"rtsp_endpoint\":\"rtsp://127.0.0.1:${RTSP_PORT}/phase3-camera-${index}\",\"username\":\"fixture-reader\",\"password\":\"fixture-reader-pass\",\"width\":${WIDTH},\"height\":${HEIGHT},\"detection_enabled\":false}" \
    > "${LOG_DIR}/add-${id}.log" 2>&1 || fail "could not add ${id}"
done
note "added ${CAMERAS} cameras"

# Adding a camera registers it; it does not run it. Without this the added
# cameras never open an RTSP session and never reach the distributor, which is
# why they were invisible in the server log.
for ((index = 2; index <= CAMERAS; index++)); do
  id="$(printf 'camera-%02d' "${index}")"
  curl -fsS -X POST "http://${HEALTH_BIND}/api/cameras/${id}/start" \
    > "${LOG_DIR}/start-${id}.log" 2>&1 || fail "could not start ${id}"
done
note "started ${CAMERAS} cameras"

# Camera connectivity is a precondition: without it there is nothing to measure.
# Publication state is deliberately NOT a precondition. Whether all ten cameras
# can hold a publication is one of the things this workload is measuring, so it
# is recorded rather than asserted -- a gate here would turn a measurement into
# a pass/fail and hide the answer.
note "waiting for all ${CAMERAS} cameras to connect"
for attempt in {1..120}; do
  connected="$(curl -fsS "http://${HEALTH_BIND}/metrics" | awk '/^surveillance_camera_connected\{/ && $NF == 1' | wc -l | tr -d ' ')"
  if [[ "${connected}" == "${CAMERAS}" ]]; then
    note "all ${CAMERAS} cameras connected after ${attempt} checks"
    break
  fi
  [[ "${attempt}" == 120 ]] && fail "only ${connected} of ${CAMERAS} cameras connected after 120 checks"
  sleep 1
done
publishing="$(curl -fsS "http://${HEALTH_BIND}/metrics" | awk '/^surveillance_livekit_state\{/ && $NF == 3' | wc -l | tr -d ' ')"
note "publishing at measurement start: ${publishing} of ${CAMERAS}"

# ---------------------------------------------------------------- viewers
note "starting ${VIEWERS} subscribers"
VIEWER_PIDS=()
for index in $(seq 1 "${VIEWERS}"); do
  camera="$(printf 'camera-%02d' "$(( (index - 1) % CAMERAS + 1 ))")"
  room="${LIVEKIT_NAMESPACE}-${camera}-live"
  LIVEKIT_URL="${LIVEKIT_URL}" LIVEKIT_API_KEY="${LIVEKIT_API_KEY}" LIVEKIT_API_SECRET="${LIVEKIT_API_SECRET}" \
  lk room join --identity "loadtest-viewer-${index}" --auto-subscribe "${room}" \
    > "${LOG_DIR}/viewer-${index}.log" 2>&1 &
  VIEWER_PIDS+=($!)
  PIDS+=($!)
done
sleep 10

subscribed=0
for index in $(seq 1 "${VIEWERS}"); do
  grep -qiE "subscribed|track" "${LOG_DIR}/viewer-${index}.log" 2>/dev/null && subscribed=$((subscribed + 1))
done
note "${subscribed}/${VIEWERS} subscriber logs show a track"

# ---------------------------------------------------------------- measure
# CPU is measured as consumed CPU-seconds over the window divided by wall
# seconds. `ps %cpu` on macOS is a lifetime-decayed average and would silently
# include warm-up; a cputime delta cannot.
collect_pids() {
  {
    echo "${SERVER_PID}"
    pgrep -P "${SERVER_PID}" 2>/dev/null || true          # lk publisher children
    echo "${LIVEKIT_PID}"
    echo "${MEDIAMTX_PID}"
    printf '%s\n' "${SOURCE_PIDS[@]}"
    printf '%s\n' "${VIEWER_PIDS[@]}"
  } | sort -u
}

note "measuring for ${WINDOW}s"
curl -fsS "http://${HEALTH_BIND}/metrics" > "${WORK_DIR}/metrics-start.txt"
python3 "${REPO_ROOT}/scripts/phase9/cpu-sample.py" start \
  "${WORK_DIR}/cpu-start.json" "${SERVER_PID}" "${LIVEKIT_PID}" "${MEDIAMTX_PID}" \
  "${SOURCE_PIDS[@]}" "${VIEWER_PIDS[@]}"

elapsed=0
while (( elapsed < WINDOW )); do
  sleep "${SAMPLE_EVERY}"
  elapsed=$((elapsed + SAMPLE_EVERY))
  {
    echo "t=${elapsed}"
    ps -o rss=,comm= -p $(collect_pids | tr '\n' ',' | sed 's/,$//') 2>/dev/null || true
  } >> "${WORK_DIR}/rss-samples.txt"
done

curl -fsS "http://${HEALTH_BIND}/metrics" > "${WORK_DIR}/metrics-end.txt"
python3 "${REPO_ROOT}/scripts/phase9/cpu-sample.py" stop \
  "${WORK_DIR}/cpu-start.json" "${SERVER_PID}" "${LIVEKIT_PID}" "${MEDIAMTX_PID}" \
  "${SOURCE_PIDS[@]}" "${VIEWER_PIDS[@]}" > "${WORK_DIR}/cpu-result.json"

# Final state: did anything degrade during the window?
curl -fsS "http://${HEALTH_BIND}/api/cameras" > "${WORK_DIR}/cameras-end.json" || true
du -sk "${RECORDING_ROOT}" | awk '{print $1}' > "${WORK_DIR}/recorded-kb.txt"

python3 "${REPO_ROOT}/scripts/phase9/report.py" "${WORK_DIR}" "${CAMERAS}" "${VIEWERS}" "${WINDOW}" \
  | tee "${WORK_DIR}/REPORT.txt"

note "run complete"
