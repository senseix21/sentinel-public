#!/usr/bin/env bash

# Compares normal and forced-exit finalization across codecs and containers.

set -euo pipefail

ffmpeg_bin="${FFMPEG_BIN:-ffmpeg}"
ffprobe_bin="${FFPROBE_BIN:-ffprobe}"
fixture_root="$(mktemp -d /private/tmp/surveillance-container-fixture.XXXXXX)"
result_file="${fixture_root}/results.tsv"
active_pid=""
codec_args=()
container_args=()

cleanup() {
    if [[ -n "${active_pid}" ]] && kill -0 "${active_pid}" 2>/dev/null; then
        kill -KILL "${active_pid}" 2>/dev/null || true
        wait "${active_pid}" 2>/dev/null || true
    fi
}

trap cleanup EXIT INT TERM

require_command() {
    local command_name="$1"

    if ! command -v "${command_name}" >/dev/null 2>&1; then
        echo "missing required command: ${command_name}" >&2
        exit 1
    fi
}

configure_codec_arguments() {
    local codec="$1"

    case "${codec}" in
        h264)
            codec_args=(
                -c:v libx264
                -preset ultrafast
                -tune zerolatency
                -g 15
                -keyint_min 15
                -sc_threshold 0
            )
            ;;
        h265)
            codec_args=(
                -c:v libx265
                -preset ultrafast
                -tune zerolatency
                -x265-params keyint=15:min-keyint=15:scenecut=0:bframes=0:log-level=error
            )
            ;;
        *)
            echo "unsupported codec: ${codec}" >&2
            return 1
            ;;
    esac
}

configure_container_arguments() {
    local container="$1"

    case "${container}" in
        matroska)
            container_args=(-f matroska)
            ;;
        fragmented-mp4)
            container_args=(
                -movflags +frag_keyframe+empty_moov+default_base_moof
                -frag_duration 1000000
                -f mp4
            )
            ;;
        classic-mp4)
            container_args=(-f mp4)
            ;;
        *)
            echo "unsupported container: ${container}" >&2
            return 1
            ;;
    esac
}

extension_for() {
    local container="$1"

    if [[ "${container}" == "matroska" ]]; then
        printf 'mkv'
    else
        printf 'mp4'
    fi
}

run_normal_case() {
    local codec="$1"
    local container="$2"
    local extension
    local output_file

    extension="$(extension_for "${container}")"
    output_file="${fixture_root}/${codec}-${container}-normal.${extension}"
    configure_codec_arguments "${codec}"
    configure_container_arguments "${container}"

    "${ffmpeg_bin}" -hide_banner -loglevel warning \
        -f lavfi -i "testsrc2=size=640x360:rate=15" \
        -t 4 -an -pix_fmt yuv420p \
        "${codec_args[@]}" "${container_args[@]}" \
        "${output_file}"
}

run_interrupted_case() {
    local codec="$1"
    local container="$2"
    local extension
    local interrupt_after_seconds="3"
    local output_file

    extension="$(extension_for "${container}")"
    output_file="${fixture_root}/${codec}-${container}-interrupted.${extension}"
    configure_codec_arguments "${codec}"
    configure_container_arguments "${container}"

    if [[ "${codec}" == "h265" ]]; then
        interrupt_after_seconds="6"
    fi

    "${ffmpeg_bin}" -hide_banner -loglevel warning -re \
        -f lavfi -i "testsrc2=size=640x360:rate=15" \
        -an -pix_fmt yuv420p \
        "${codec_args[@]}" "${container_args[@]}" \
        "${output_file}" \
        >"${fixture_root}/${codec}-${container}-interrupted.log" 2>&1 &
    active_pid="$!"

    sleep "${interrupt_after_seconds}"
    kill -KILL "${active_pid}"
    wait "${active_pid}" 2>/dev/null || true
    active_pid=""
}

record_result() {
    local file_path="$1"
    local file_name
    local probe_status="failed"
    local decode_status="not-run"
    local codec_name="unknown"
    local duration="unknown"
    local size_bytes="0"

    file_name="$(basename "${file_path}")"

    if [[ -f "${file_path}" ]]; then
        size_bytes="$(stat -f '%z' "${file_path}")"
    fi

    if "${ffprobe_bin}" -v error \
        -show_entries stream=codec_name \
        -show_entries format=duration \
        -of default=nw=1 "${file_path}" \
        >"${file_path}.probe" 2>"${file_path}.probe-error"; then
        probe_status="passed"
        codec_name="$(sed -n 's/^codec_name=//p' "${file_path}.probe" | head -n 1)"
        duration="$(sed -n 's/^duration=//p' "${file_path}.probe" | head -n 1)"

        if "${ffmpeg_bin}" -v error -i "${file_path}" -map 0:v:0 -f null - \
            >"${file_path}.decode" 2>"${file_path}.decode-error"; then
            decode_status="passed"
        else
            decode_status="failed"
        fi
    fi

    printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
        "${file_name}" "${probe_status}" "${decode_status}" \
        "${codec_name}" "${duration:-unknown}" "${size_bytes}" \
        >>"${result_file}"
}

require_command "${ffmpeg_bin}"
require_command "${ffprobe_bin}"

printf 'file\tprobe\tdecode\tcodec\tduration_seconds\tsize_bytes\n' >"${result_file}"

for codec in h264 h265; do
    for container in matroska fragmented-mp4 classic-mp4; do
        run_normal_case "${codec}" "${container}"
        run_interrupted_case "${codec}" "${container}"
    done
done

for media_file in "${fixture_root}"/*.mkv "${fixture_root}"/*.mp4; do
    record_result "${media_file}"
done

column -t -s $'\t' "${result_file}"
echo "fixture results: ${fixture_root}"
