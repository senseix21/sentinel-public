#!/usr/bin/env bash

set -euo pipefail

mediamtx_bin="${MEDIAMTX_BIN:-/usr/local/opt/mediamtx/bin/mediamtx}"
mediamtx_config="${MEDIAMTX_CONFIG:-/usr/local/etc/mediamtx/mediamtx.yml}"
ffmpeg_bin="${FFMPEG_BIN:-ffmpeg}"
ffprobe_bin="${FFPROBE_BIN:-ffprobe}"
fixture_url="rtsp://127.0.0.1:8554/reconnect-fixture"
fixture_root="$(mktemp -d /private/tmp/surveillance-rtsp-fixture.XXXXXX)"
server_work_dir="${fixture_root}/server"
server_pid=""
publisher_pid=""

mkdir -p "${server_work_dir}"

cleanup() {
    if [[ -n "${publisher_pid}" ]] && kill -0 "${publisher_pid}" 2>/dev/null; then
        kill -INT "${publisher_pid}" 2>/dev/null || true
        wait "${publisher_pid}" 2>/dev/null || true
    fi

    if [[ -n "${server_pid}" ]] && kill -0 "${server_pid}" 2>/dev/null; then
        kill -INT "${server_pid}" 2>/dev/null || true
        wait "${server_pid}" 2>/dev/null || true
    fi
}

trap cleanup EXIT INT TERM

require_executable() {
    local executable="$1"
    local label="$2"

    if [[ ! -x "${executable}" ]] && ! command -v "${executable}" >/dev/null 2>&1; then
        echo "missing ${label}: ${executable}" >&2
        exit 1
    fi
}

start_server() {
    (
        cd "${server_work_dir}"
        exec "${mediamtx_bin}" "${mediamtx_config}"
    ) >"${fixture_root}/mediamtx.log" 2>&1 &
    server_pid="$!"

    for _ in {1..50}; do
        if grep -q "\[RTSP\] started" "${fixture_root}/mediamtx.log" 2>/dev/null; then
            return
        fi

        if ! kill -0 "${server_pid}" 2>/dev/null; then
            echo "MediaMTX exited before becoming ready" >&2
            return 1
        fi

        sleep 0.1
    done

    echo "MediaMTX did not become ready" >&2
    return 1
}

start_publisher() {
    local generation="$1"

    "${ffmpeg_bin}" -hide_banner -loglevel warning -re \
        -f lavfi -i "testsrc2=size=640x360:rate=15" \
        -an -c:v libx264 -preset ultrafast -tune zerolatency \
        -g 30 -keyint_min 30 -sc_threshold 0 \
        -metadata "fixture_generation=${generation}" \
        -f rtsp -rtsp_transport tcp "${fixture_url}" \
        >"${fixture_root}/publisher-${generation}.log" 2>&1 &
    publisher_pid="$!"

    for _ in {1..50}; do
        if "${ffprobe_bin}" -v error -rtsp_transport tcp \
            -show_entries stream=codec_name -of default=nw=1 \
            "${fixture_url}" >"${fixture_root}/probe-ready-${generation}.txt" 2>/dev/null; then
            return
        fi

        if ! kill -0 "${publisher_pid}" 2>/dev/null; then
            echo "publisher generation ${generation} exited before becoming ready" >&2
            return 1
        fi

        sleep 0.1
    done

    echo "publisher generation ${generation} did not become ready" >&2
    return 1
}

stop_publisher() {
    kill -INT "${publisher_pid}"
    wait "${publisher_pid}" 2>/dev/null || true
    publisher_pid=""
}

capture_generation() {
    local generation="$1"
    local output_file="${fixture_root}/generation-${generation}.mkv"

    "${ffmpeg_bin}" -hide_banner -loglevel warning \
        -rtsp_transport tcp -i "${fixture_url}" \
        -t 4 -an -c:v copy "${output_file}"

    "${ffprobe_bin}" -v error \
        -show_entries format=filename,duration,size \
        -show_entries stream=codec_name,width,height,avg_frame_rate,start_time \
        -of json "${output_file}" >"${fixture_root}/generation-${generation}-summary.json"

    "${ffprobe_bin}" -v error -select_streams v:0 \
        -show_packets \
        -show_entries packet=pts,dts,pts_time,dts_time,duration_time,flags \
        -of json "${output_file}" >"${fixture_root}/generation-${generation}-packets.json"

    "${ffmpeg_bin}" -v error -i "${output_file}" -map 0:v:0 -f null -
}

require_executable "${mediamtx_bin}" "MediaMTX"
require_executable "${ffmpeg_bin}" "FFmpeg"
require_executable "${ffprobe_bin}" "FFprobe"

start_server
start_publisher 1
capture_generation 1
stop_publisher

if "${ffprobe_bin}" -v error -rtsp_transport tcp \
    -show_entries stream=codec_name "${fixture_url}" >/dev/null 2>&1; then
    echo "fixture unexpectedly remained available after publisher shutdown" >&2
    exit 1
fi

echo "offline boundary observed" >"${fixture_root}/offline-boundary.txt"

start_publisher 2
capture_generation 2
stop_publisher

echo "fixture results: ${fixture_root}"

