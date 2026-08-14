#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
mediamtx_bin="${MEDIAMTX_BIN:-/usr/local/bin/mediamtx}"
ffmpeg_bin="${FFMPEG_BIN:-ffmpeg}"
fixture_config="${repo_root}/tests/fixtures/mediamtx-phase3.yml"
fixture_root="$(mktemp -d /private/tmp/surveillance-phase3.XXXXXX)"
server_pid=""
mediamtx_pid=""
declare -a publisher_pids=()

cleanup() {
    if [[ -n "${server_pid}" ]] && kill -0 "${server_pid}" 2>/dev/null; then
        kill -INT "${server_pid}" 2>/dev/null || true
        wait "${server_pid}" 2>/dev/null || true
    fi
    for pid in "${publisher_pids[@]:-}"; do
        if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
            kill -INT "${pid}" 2>/dev/null || true
            wait "${pid}" 2>/dev/null || true
        fi
    done
    if [[ -n "${mediamtx_pid}" ]] && kill -0 "${mediamtx_pid}" 2>/dev/null; then
        kill -INT "${mediamtx_pid}" 2>/dev/null || true
        wait "${mediamtx_pid}" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

for executable in "${mediamtx_bin}" "${ffmpeg_bin}" curl lsof; do
    if ! command -v "${executable}" >/dev/null 2>&1; then
        echo "missing fixture executable: ${executable}" >&2
        exit 1
    fi
done

start_publisher() {
    local index="$1"
    local path="phase3-camera-${index}"
    "${ffmpeg_bin}" -hide_banner -loglevel warning -re \
        -f lavfi -i "testsrc2=size=640x360:rate=15" \
        -an -c:v libx264 -profile:v high -pix_fmt yuv420p \
        -preset ultrafast -tune zerolatency -g 30 -keyint_min 30 -sc_threshold 0 \
        -f rtsp -rtsp_transport tcp \
        "rtsp://fixture-publisher:fixture-publisher-pass@127.0.0.1:28554/${path}" \
        >"${fixture_root}/publisher-${index}.log" 2>&1 &
    LAST_PUBLISHER_PID="$!"
    publisher_pids+=("${LAST_PUBLISHER_PID}")
}

wait_for_count() {
    local camera="$1"
    local wanted="$2"
    for _ in {1..900}; do
        local count
        count="$(find "${fixture_root}/recordings/${camera}" -type f -name '*.mp4' ! -name '*.partial.mp4' 2>/dev/null | wc -l | tr -d ' ')"
        if (( count >= wanted )); then
            return 0
        fi
        if ! kill -0 "${server_pid}" 2>/dev/null; then
            echo "server exited while waiting for ${camera}; log: ${fixture_root}/server.log" >&2
            return 1
        fi
        sleep 0.1
    done
    echo "timed out waiting for ${wanted} segments from ${camera}" >&2
    return 1
}

"${mediamtx_bin}" "${fixture_config}" >"${fixture_root}/mediamtx.log" 2>&1 &
mediamtx_pid="$!"
for _ in {1..100}; do
    grep -q "started with listeners on 127.0.0.1:28554" "${fixture_root}/mediamtx.log" 2>/dev/null && break
    kill -0 "${mediamtx_pid}" 2>/dev/null || { echo "MediaMTX failed: ${fixture_root}/mediamtx.log" >&2; exit 1; }
    sleep 0.05
done
grep -q "started with listeners on 127.0.0.1:28554" "${fixture_root}/mediamtx.log"

for index in {1..10}; do
    start_publisher "${index}"
done

for index in {1..10}; do
    for _ in {1..100}; do
        grep -q "is publishing to path 'phase3-camera-${index}'" "${fixture_root}/mediamtx.log" 2>/dev/null && break
        sleep 0.05
    done
    grep -q "is publishing to path 'phase3-camera-${index}'" "${fixture_root}/mediamtx.log"
done

for index in {1..10}; do
    mkdir -p "${fixture_root}/recordings/camera-$(printf '%02d' "${index}")"
done
(
    cd "${repo_root}"
    cargo build -p surveillance-server
)
(
    cd "${repo_root}"
    export SURVEILLANCE_CAMERA_ID="camera-01"
    export SURVEILLANCE_RTSP_URL="rtsp://127.0.0.1:28554/phase3-camera-1"
    export SURVEILLANCE_RTSP_USERNAME="fixture-reader"
    export SURVEILLANCE_RTSP_PASSWORD="fixture-reader-pass"
    export SURVEILLANCE_VIDEO_WIDTH="640"
    export SURVEILLANCE_VIDEO_HEIGHT="360"
    export SURVEILLANCE_RECORDING_ROOT="${fixture_root}/recordings"
    export SURVEILLANCE_HEALTH_BIND="127.0.0.1:3300"
    # This Phase 3 regression deliberately accelerates ten streams and measures
    # camera/recorder isolation. Phase 6 owns a separate real-time decode gate.
    export SURVEILLANCE_DETECTION_ENABLED="false"
    export RUST_LOG="info"
    exec "${repo_root}/target/debug/surveillance-server" >"${fixture_root}/server.log" 2>&1
) &
server_pid="$!"

for _ in {1..200}; do
    curl -fsS "http://127.0.0.1:3300/api/cameras/camera-01" >/dev/null 2>&1 && break
    kill -0 "${server_pid}" 2>/dev/null || { echo "server startup failed: ${fixture_root}/server.log" >&2; exit 1; }
    sleep 0.05
done

for index in {2..10}; do
    camera="camera-$(printf '%02d' "${index}")"
    curl -fsS -X POST "http://127.0.0.1:3300/api/cameras" \
        -H 'Content-Type: application/json' \
        --data-binary "{\"camera_id\":\"${camera}\",\"rtsp_endpoint\":\"rtsp://127.0.0.1:28554/phase3-camera-${index}\",\"username\":\"fixture-reader\",\"password\":\"fixture-reader-pass\",\"width\":640,\"height\":360,\"detection_enabled\":false}" \
        >/dev/null
    curl -fsS -X POST "http://127.0.0.1:3300/api/cameras/${camera}/start" >/dev/null
done

for index in {1..10}; do
    wait_for_count "camera-$(printf '%02d' "${index}")" 1
done

curl -fsS "http://127.0.0.1:3300/metrics" >"${fixture_root}/metrics-before-outage.txt"
lsof -p "${server_pid}" >"${fixture_root}/fds-before-outage.txt"
for index in {1..10}; do
    camera="camera-$(printf '%02d' "${index}")"
    grep -q "surveillance_camera_connected{camera_id=\"${camera}\"} 1" "${fixture_root}/metrics-before-outage.txt"
done

before_b="$(find "${fixture_root}/recordings/camera-02" -type f -name '*.mp4' ! -name '*.partial.mp4' | wc -l | tr -d ' ')"
publisher_a="${publisher_pids[0]}"
kill -INT "${publisher_a}"
wait "${publisher_a}" || true
wait_for_count "camera-02" "$((before_b + 1))"
camera_b_reconnects="$(curl -fsS "http://127.0.0.1:3300/api/cameras/camera-02/health" | sed -n 's/.*"reconnects":\([0-9][0-9]*\).*/\1/p')"
if [[ "${camera_b_reconnects}" != "0" ]]; then
    echo "camera B reconnected during camera A outage" >&2
    exit 1
fi

start_publisher 1
wait_for_count "camera-01" 2
camera_a_health="$(curl -fsS "http://127.0.0.1:3300/api/cameras/camera-01/health")"
camera_a_epochs="$(sed -n 's/.*"connection_epochs":\([0-9][0-9]*\).*/\1/p' <<<"${camera_a_health}")"
if (( camera_a_epochs < 2 )); then
    echo "camera A did not establish a fresh connection epoch" >&2
    exit 1
fi

# Remove camera A while it is reconnecting; the joined removal must not disturb B.
recovered_publisher_a="${LAST_PUBLISHER_PID}"
kill -INT "${recovered_publisher_a}"
wait "${recovered_publisher_a}" || true
for _ in {1..100}; do
    camera_a_health="$(curl -fsS "http://127.0.0.1:3300/api/cameras/camera-01/health")"
    grep -q '"connection_state":"reconnecting"' <<<"${camera_a_health}" && break
    sleep 0.05
done
grep -q '"connection_state":"reconnecting"' <<<"${camera_a_health}"
curl -fsS -X DELETE "http://127.0.0.1:3300/api/cameras/camera-01" >/dev/null
if curl -fsS "http://127.0.0.1:3300/api/cameras/camera-01" >/dev/null 2>&1; then
    echo "removed reconnecting camera A remained visible" >&2
    exit 1
fi

# Remove a connected camera and prove an unrelated camera remains connected.
curl -fsS -X DELETE "http://127.0.0.1:3300/api/cameras/camera-10" >/dev/null
if curl -fsS "http://127.0.0.1:3300/api/cameras/camera-10" >/dev/null 2>&1; then
    echo "removed connected camera remained visible" >&2
    exit 1
fi
curl -fsS "http://127.0.0.1:3300/api/cameras/camera-02/health" | grep -q '"connection_state":"connected"'

# Terminal authentication and capability failures remain camera-local.
for failure_case in credentials dimensions; do
    failure_id="camera-bad-${failure_case}"
    password="fixture-reader-pass"
    width=640
    if [[ "${failure_case}" == "credentials" ]]; then
        password="wrong-password"
    else
        width=800
    fi
    curl -fsS -X POST "http://127.0.0.1:3300/api/cameras" \
        -H 'Content-Type: application/json' \
        --data-binary "{\"camera_id\":\"${failure_id}\",\"rtsp_endpoint\":\"rtsp://127.0.0.1:28554/phase3-camera-9\",\"username\":\"fixture-reader\",\"password\":\"${password}\",\"width\":${width},\"height\":360,\"detection_enabled\":false}" \
        >/dev/null
    curl -fsS -X POST "http://127.0.0.1:3300/api/cameras/${failure_id}/start" >/dev/null
    for _ in {1..100}; do
        failure_health="$(curl -fsS "http://127.0.0.1:3300/api/cameras/${failure_id}/health")"
        grep -q '"pipeline_state":"terminal"' <<<"${failure_health}" && break
        sleep 0.05
    done
    grep -q '"pipeline_state":"terminal"' <<<"${failure_health}"
    curl -fsS "http://127.0.0.1:3300/api/cameras/camera-02/health" | grep -q '"connection_state":"connected"'
    curl -fsS -X DELETE "http://127.0.0.1:3300/api/cameras/${failure_id}" >/dev/null
done

ps -o pid=,%cpu=,rss= -p "${server_pid}" >"${fixture_root}/server-resources.txt"
lsof -p "${server_pid}" >"${fixture_root}/fds-final.txt"
fds_before="$(wc -l <"${fixture_root}/fds-before-outage.txt" | tr -d ' ')"
fds_final="$(wc -l <"${fixture_root}/fds-final.txt" | tr -d ' ')"
if (( fds_final > fds_before + 2 )); then
    echo "file descriptors grew across reconnect/removal: before=${fds_before}, final=${fds_final}" >&2
    exit 1
fi
curl -fsS "http://127.0.0.1:3300/metrics" >"${fixture_root}/metrics-final.txt"
if grep -E 'surveillance_media_queue_(rejected_full_total|dropped_on_close_total)\{[^}]*consumer="recorder"\} [1-9]' "${fixture_root}/metrics-final.txt"; then
    echo "recorder queue loss observed during synthetic qualification" >&2
    exit 1
fi

for index in {1..10}; do
    segment="$(find "${fixture_root}/recordings/camera-$(printf '%02d' "${index}")" -type f -name '*.mp4' ! -name '*.partial.mp4' -print -quit)"
    "${ffmpeg_bin}" -v error -i "${segment}" -map 0:v:0 -f null -
done

kill -INT "${server_pid}"
wait "${server_pid}"
server_pid=""

read_connections="$(grep -c "is reading from path 'phase3-camera-" "${fixture_root}/mediamtx.log" || true)"
if (( read_connections != 12 )); then
    echo "expected ten initial readers, one camera-A reconnect, and one dimension-validation reader; observed ${read_connections}" >&2
    exit 1
fi

find "${fixture_root}/recordings" -type f -name '*.partial.mp4' -print >"${fixture_root}/remaining-partials.txt"
partial_count="$(wc -l <"${fixture_root}/remaining-partials.txt" | tr -d ' ')"
shutdown_failure_count="$(grep -c 'recording lifecycle failure delivered failure=Shutdown' "${fixture_root}/server.log" || true)"
if (( partial_count != shutdown_failure_count )); then
    echo "shutdown partials were not matched by explicit failure facts: partials=${partial_count}, failures=${shutdown_failure_count}" >&2
    exit 1
fi
while IFS= read -r partial; do
    [[ -z "${partial}" ]] || "${ffmpeg_bin}" -v error -i "${partial}" -map 0:v:0 -f null -
done <"${fixture_root}/remaining-partials.txt"

echo "Phase 3 synthetic ten-camera and two-camera isolation fixture passed; evidence: ${fixture_root}"
