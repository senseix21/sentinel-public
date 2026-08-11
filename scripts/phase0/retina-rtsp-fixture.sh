#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
mediamtx_bin="${MEDIAMTX_BIN:-/usr/local/bin/mediamtx}"
ffmpeg_bin="${FFMPEG_BIN:-ffmpeg}"
fixture_config="${repo_root}/tests/fixtures/mediamtx-retina.yml"
fixture_url="rtsp://fixture-publisher:fixture-publisher-pass@127.0.0.1:18554/retina-fixture"
fixture_root="$(mktemp -d /private/tmp/surveillance-retina-fixture.XXXXXX)"
server_pid=""
publisher_pid=""
controller_pid=""
app_pid=""
publisher_pid_file="${fixture_root}/publisher.pid"

cleanup() {
    if [[ -n "${app_pid}" ]] && kill -0 "${app_pid}" 2>/dev/null; then
        kill -INT "${app_pid}" 2>/dev/null || true
        wait "${app_pid}" 2>/dev/null || true
    fi
    if [[ -n "${publisher_pid}" ]] && kill -0 "${publisher_pid}" 2>/dev/null; then
        kill -INT "${publisher_pid}" 2>/dev/null || true
        wait "${publisher_pid}" 2>/dev/null || true
    fi
    if [[ -n "${controller_pid}" ]] && kill -0 "${controller_pid}" 2>/dev/null; then
        kill -INT "${controller_pid}" 2>/dev/null || true
        wait "${controller_pid}" 2>/dev/null || true
    fi
    if [[ -f "${publisher_pid_file}" ]]; then
        recovery_publisher_pid="$(<"${publisher_pid_file}")"
        if [[ -n "${recovery_publisher_pid}" ]] && kill -0 "${recovery_publisher_pid}" 2>/dev/null; then
            kill -INT "${recovery_publisher_pid}" 2>/dev/null || true
        fi
    fi
    if [[ -n "${server_pid}" ]] && kill -0 "${server_pid}" 2>/dev/null; then
        kill -INT "${server_pid}" 2>/dev/null || true
        wait "${server_pid}" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

if [[ ! -x "${mediamtx_bin}" ]]; then
    echo "missing MediaMTX executable: ${mediamtx_bin}" >&2
    exit 1
fi
if ! command -v "${ffmpeg_bin}" >/dev/null 2>&1; then
    echo "missing FFmpeg executable: ${ffmpeg_bin}" >&2
    exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
    echo "missing curl executable" >&2
    exit 1
fi

"${mediamtx_bin}" "${fixture_config}" >"${fixture_root}/mediamtx.log" 2>&1 &
server_pid="$!"
for _ in {1..50}; do
    if grep -q "started with listeners on 127.0.0.1:18554" "${fixture_root}/mediamtx.log" 2>/dev/null; then
        break
    fi
    if ! kill -0 "${server_pid}" 2>/dev/null; then
        echo "MediaMTX exited before readiness; log: ${fixture_root}/mediamtx.log" >&2
        exit 1
    fi
    sleep 0.1
done
if ! grep -q "started with listeners on 127.0.0.1:18554" "${fixture_root}/mediamtx.log"; then
    echo "MediaMTX readiness timed out; log: ${fixture_root}/mediamtx.log" >&2
    exit 1
fi

"${ffmpeg_bin}" -hide_banner -loglevel warning -re \
    -f lavfi -i "testsrc2=size=640x360:rate=15" \
    -an -c:v libx264 -profile:v high -pix_fmt yuv420p \
    -preset ultrafast -tune zerolatency -g 15 -keyint_min 15 -sc_threshold 0 \
    -f rtsp -rtsp_transport tcp "${fixture_url}" \
    >"${fixture_root}/publisher.log" 2>&1 &
publisher_pid="$!"
echo "${publisher_pid}" >"${publisher_pid_file}"

for _ in {1..50}; do
    if grep -q "is publishing to path 'retina-fixture'" "${fixture_root}/mediamtx.log" 2>/dev/null; then
        break
    fi
    if ! kill -0 "${publisher_pid}" 2>/dev/null; then
        echo "FFmpeg publisher exited before readiness; log: ${fixture_root}/publisher.log" >&2
        exit 1
    fi
    sleep 0.1
done
if ! grep -q "is publishing to path 'retina-fixture'" "${fixture_root}/mediamtx.log"; then
    echo "publisher readiness timed out; logs: ${fixture_root}" >&2
    exit 1
fi

(
    for _ in {1..1200}; do
        if [[ -f "${fixture_root}/ready-for-outage" ]]; then
            break
        fi
        sleep 0.05
    done
    if [[ ! -f "${fixture_root}/ready-for-outage" ]]; then
        exit 1
    fi

    active_publisher_pid="$(<"${publisher_pid_file}")"
    kill -INT "${active_publisher_pid}"
    for _ in {1..100}; do
        if ! kill -0 "${active_publisher_pid}" 2>/dev/null; then
            break
        fi
        sleep 0.01
    done
    sleep 0.75

    "${ffmpeg_bin}" -hide_banner -loglevel warning -re \
        -f lavfi -i "testsrc2=size=640x360:rate=15" \
        -an -c:v libx264 -profile:v high -pix_fmt yuv420p \
        -preset ultrafast -tune zerolatency -g 15 -keyint_min 15 -sc_threshold 0 \
        -f rtsp -rtsp_transport tcp "${fixture_url}" \
        >"${fixture_root}/publisher-recovery.log" 2>&1 &
    recovery_publisher_pid="$!"
    echo "${recovery_publisher_pid}" >"${publisher_pid_file}"
    for _ in {1..100}; do
        publish_count="$(grep -c "is publishing to path 'retina-fixture'" "${fixture_root}/mediamtx.log" || true)"
        if (( publish_count >= 2 )); then
            touch "${fixture_root}/publisher-restarted"
            exit 0
        fi
        if ! kill -0 "${recovery_publisher_pid}" 2>/dev/null; then
            exit 1
        fi
        sleep 0.05
    done
    exit 1
) &
controller_pid="$!"

(
    cd "${repo_root}"
    RETINA_FIXTURE_CONTROL_DIR="${fixture_root}" \
        cargo test -p surveillance-camera --test retina_mediamtx -- --ignored --test-threads=1
)

wait "${controller_pid}"
controller_pid=""

recovery_publisher_pid="$(<"${publisher_pid_file}")"
kill -INT "${recovery_publisher_pid}"
for _ in {1..100}; do
    if ! kill -0 "${recovery_publisher_pid}" 2>/dev/null; then
        break
    fi
    sleep 0.01
done

"${ffmpeg_bin}" -hide_banner -loglevel warning -readrate 10 \
    -f lavfi -i "testsrc2=size=640x360:rate=15" \
    -an -c:v libx264 -profile:v high -pix_fmt yuv420p \
    -preset ultrafast -tune zerolatency -g 15 -keyint_min 15 -sc_threshold 0 \
    -f rtsp -rtsp_transport tcp "${fixture_url}" \
    >"${fixture_root}/publisher-accelerated.log" 2>&1 &
publisher_pid="$!"
echo "${publisher_pid}" >"${publisher_pid_file}"
for _ in {1..100}; do
    publish_count="$(grep -c "is publishing to path 'retina-fixture'" "${fixture_root}/mediamtx.log" || true)"
    if (( publish_count >= 3 )); then
        break
    fi
    if ! kill -0 "${publisher_pid}" 2>/dev/null; then
        echo "accelerated publisher exited; log: ${fixture_root}/publisher-accelerated.log" >&2
        exit 1
    fi
    sleep 0.05
done

(
    cd "${repo_root}"
    RETINA_RECORDING_OUTPUT_DIR="${fixture_root}" \
        cargo test -p surveillance-recorder --test retina_fmp4 -- --ignored
)

kill -INT "${publisher_pid}"
for _ in {1..100}; do
    if ! kill -0 "${publisher_pid}" 2>/dev/null; then
        break
    fi
    sleep 0.01
done

"${ffmpeg_bin}" -hide_banner -loglevel warning -readrate 4 \
    -f lavfi -i "testsrc2=size=640x360:rate=15" \
    -an -c:v libx264 -profile:v high -pix_fmt yuv420p \
    -preset ultrafast -tune zerolatency -g 15 -keyint_min 15 -sc_threshold 0 \
    -f rtsp -rtsp_transport tcp "${fixture_url}" \
    >"${fixture_root}/publisher-server.log" 2>&1 &
publisher_pid="$!"
echo "${publisher_pid}" >"${publisher_pid_file}"
for _ in {1..100}; do
    publish_count="$(grep -c "is publishing to path 'retina-fixture'" "${fixture_root}/mediamtx.log" || true)"
    if (( publish_count >= 4 )); then
        break
    fi
    if ! kill -0 "${publisher_pid}" 2>/dev/null; then
        echo "server publisher exited; log: ${fixture_root}/publisher-server.log" >&2
        exit 1
    fi
    sleep 0.05
done

server_output="${fixture_root}/server-output"
mkdir -p "${server_output}"
(
    cd "${repo_root}"
    cargo build -p surveillance-server
)
(
    cd "${repo_root}"
    export SURVEILLANCE_CAMERA_ID="fixture-camera"
    export SURVEILLANCE_RTSP_URL="rtsp://127.0.0.1:18554/retina-fixture"
    export SURVEILLANCE_RTSP_USERNAME="fixture-reader"
    export SURVEILLANCE_RTSP_PASSWORD="fixture-reader-pass"
    export SURVEILLANCE_VIDEO_WIDTH="640"
    export SURVEILLANCE_VIDEO_HEIGHT="360"
    export SURVEILLANCE_RECORDING_ROOT="${server_output}"
    export RUST_LOG="info"
    exec "${repo_root}/target/debug/surveillance-server" >"${fixture_root}/server.log" 2>&1
) &
app_pid="$!"

for _ in {1..100}; do
    if curl -fsS "http://127.0.0.1:3000/ready" >/dev/null 2>&1; then
        break
    fi
    if ! kill -0 "${app_pid}" 2>/dev/null; then
        echo "surveillance server exited before readiness; log: ${fixture_root}/server.log" >&2
        exit 1
    fi
    sleep 0.05
done
curl -fsS "http://127.0.0.1:3000/live" | grep -q '^live$'
curl -fsS "http://127.0.0.1:3000/ready" | grep -q '^ready$'
curl -fsS "http://127.0.0.1:3000/metrics" | grep -q 'surveillance_camera_connected{camera_id="fixture-camera"} 1'

for _ in {1..600}; do
    segment_count="$(find "${server_output}" -type f -name '*.mp4' ! -name '*.partial.mp4' | wc -l | tr -d ' ')"
    if (( segment_count >= 3 )); then
        break
    fi
    if ! kill -0 "${app_pid}" 2>/dev/null; then
        echo "surveillance server exited before three segments; log: ${fixture_root}/server.log" >&2
        exit 1
    fi
    sleep 0.1
done
segment_count="$(find "${server_output}" -type f -name '*.mp4' ! -name '*.partial.mp4' | wc -l | tr -d ' ')"
if (( segment_count < 3 )); then
    echo "surveillance server did not produce three segments; log: ${fixture_root}/server.log" >&2
    exit 1
fi

kill -INT "${app_pid}"
for _ in {1..100}; do
    if ! kill -0 "${app_pid}" 2>/dev/null; then
        break
    fi
    sleep 0.05
done
if kill -0 "${app_pid}" 2>/dev/null; then
    echo "surveillance server did not exit after SIGINT; log: ${fixture_root}/server.log" >&2
    exit 1
fi
wait "${app_pid}"
app_pid=""

while IFS= read -r segment; do
    "${ffmpeg_bin}" -v error -i "${segment}" -map 0:v:0 -f null -
done < <(find "${server_output}" -type f -name '*.mp4' ! -name '*.partial.mp4' -print)

crash_output="${fixture_root}/crash-output"
mkdir -p "${crash_output}"
(
    cd "${repo_root}"
    export SURVEILLANCE_CAMERA_ID="crash-camera"
    export SURVEILLANCE_RTSP_URL="rtsp://127.0.0.1:18554/retina-fixture"
    export SURVEILLANCE_RTSP_USERNAME="fixture-reader"
    export SURVEILLANCE_RTSP_PASSWORD="fixture-reader-pass"
    export SURVEILLANCE_VIDEO_WIDTH="640"
    export SURVEILLANCE_VIDEO_HEIGHT="360"
    export SURVEILLANCE_RECORDING_ROOT="${crash_output}"
    export SURVEILLANCE_HEALTH_BIND="127.0.0.1:3001"
    export RUST_LOG="info"
    exec "${repo_root}/target/debug/surveillance-server" >"${fixture_root}/server-crash.log" 2>&1
) &
app_pid="$!"

partial=""
for _ in {1..300}; do
    partial="$(find "${crash_output}" -type f -name '*.partial.mp4' -size +10000c -print -quit)"
    if [[ -n "${partial}" ]]; then
        break
    fi
    if ! kill -0 "${app_pid}" 2>/dev/null; then
        echo "crash-test server exited before persisting a fragment; log: ${fixture_root}/server-crash.log" >&2
        exit 1
    fi
    sleep 0.05
done
if [[ -z "${partial}" ]]; then
    echo "crash-test server did not persist a bounded partial fragment; log: ${fixture_root}/server-crash.log" >&2
    exit 1
fi

kill -KILL "${app_pid}"
wait "${app_pid}" 2>/dev/null || true
app_pid=""
if [[ ! -f "${partial}" ]]; then
    echo "active partial disappeared after SIGKILL: ${partial}" >&2
    exit 1
fi
"${ffmpeg_bin}" -v error -i "${partial}" -map 0:v:0 -f null -

(
    cd "${repo_root}"
    export SURVEILLANCE_CAMERA_ID="crash-camera"
    export SURVEILLANCE_RTSP_URL="rtsp://127.0.0.1:18554/retina-fixture"
    export SURVEILLANCE_RTSP_USERNAME="fixture-reader"
    export SURVEILLANCE_RTSP_PASSWORD="intentionally-wrong"
    export SURVEILLANCE_VIDEO_WIDTH="640"
    export SURVEILLANCE_VIDEO_HEIGHT="360"
    export SURVEILLANCE_RECORDING_ROOT="${crash_output}"
    export SURVEILLANCE_HEALTH_BIND="127.0.0.1:3001"
    export RUST_LOG="info"
    exec "${repo_root}/target/debug/surveillance-server" >"${fixture_root}/server-recovery.log" 2>&1
) &
app_pid="$!"
for _ in {1..200}; do
    if curl -fsS "http://127.0.0.1:3001/api/cameras/crash-camera" 2>/dev/null | grep -q '"pipeline_state":"terminal"'; then
        break
    fi
    if ! kill -0 "${app_pid}" 2>/dev/null; then
        echo "reconciliation server exited unexpectedly; log: ${fixture_root}/server-recovery.log" >&2
        exit 1
    fi
    sleep 0.05
done
if ! curl -fsS "http://127.0.0.1:3001/api/cameras/crash-camera" | grep -q '"pipeline_state":"terminal"'; then
    echo "invalid credentials did not produce terminal camera state" >&2
    exit 1
fi
kill -INT "${app_pid}"
wait "${app_pid}"
app_pid=""
if ! grep -q "recording files require metadata reconciliation" "${fixture_root}/server-recovery.log"; then
    echo "restart did not report crash reconciliation evidence; log: ${fixture_root}/server-recovery.log" >&2
    exit 1
fi
if [[ ! -f "${partial}" ]]; then
    echo "restart incorrectly promoted or removed the incomplete partial: ${partial}" >&2
    exit 1
fi

echo "Retina RTSP fixture passed; logs: ${fixture_root}"
