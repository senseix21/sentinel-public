#!/usr/bin/env bash
# Qualify one physical RTSP camera against the production Rust ingest path.
#
# This harness deliberately starts no RTSP proxy, synthetic publisher,
# PostgreSQL, LiveKit, or browser. It closes the direct-ingest and recording
# portion of docs/direct-camera-rtsp/SPEC.md while leaving optional dependency
# stages to their dedicated fixtures. All output is written beneath EVIDENCE_DIR.
#
# Required environment:
#   QUALIFICATION_ACK=direct-physical-camera
#   SURVEILLANCE_CAMERA_ID
#   SURVEILLANCE_RTSP_URL       credential-free rtsp:// endpoint
#   SURVEILLANCE_VIDEO_WIDTH
#   SURVEILLANCE_VIDEO_HEIGHT
#   SURVEILLANCE_RTSP_USERNAME and SURVEILLANCE_RTSP_PASSWORD, or neither
#
# Optional environment:
#   SERVER_BINARY              default target/release/surveillance-server
#   EVIDENCE_DIR               default .local/direct-camera/<UTC timestamp>
#   QUALIFICATION_SECONDS      default 180; minimum 90
#   SAMPLE_EVERY_SECONDS       default 10
#   CONNECT_TIMEOUT_SECONDS    default 60
#   SURVEILLANCE_HEALTH_BIND   default 127.0.0.1:3000
#   QUALIFICATION_HTTP_URL     default derived from the health bind
#   SURVEILLANCE_RECORDING_ROOT default <evidence>/recordings

set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
server_binary="${SERVER_BINARY:-${repo_root}/target/release/surveillance-server}"
qualification_seconds="${QUALIFICATION_SECONDS:-180}"
sample_every_seconds="${SAMPLE_EVERY_SECONDS:-10}"
connect_timeout_seconds="${CONNECT_TIMEOUT_SECONDS:-60}"
health_bind="${SURVEILLANCE_HEALTH_BIND:-127.0.0.1:3000}"
http_url="${QUALIFICATION_HTTP_URL:-http://${health_bind}}"
utc_stamp="$(date -u '+%Y%m%dT%H%M%SZ')"
evidence_dir="${EVIDENCE_DIR:-${repo_root}/.local/direct-camera/${utc_stamp}}"
recording_root="${SURVEILLANCE_RECORDING_ROOT:-${evidence_dir}/recordings}"
camera_id="${SURVEILLANCE_CAMERA_ID:-}"
rtsp_url="${SURVEILLANCE_RTSP_URL:-}"
server_pid=""
sample_pid=""

fail() {
    printf 'direct-camera qualification: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "missing required executable: $1"
}

require_positive_integer() {
    local name="$1"
    local value="$2"
    [[ "${value}" =~ ^[1-9][0-9]*$ ]] || fail "${name} must be a positive integer"
}

cleanup() {
    local exit_status=$?
    trap - EXIT INT TERM
    if [[ -n "${sample_pid}" ]] && kill -0 "${sample_pid}" 2>/dev/null; then
        kill -TERM "${sample_pid}" 2>/dev/null || true
        wait "${sample_pid}" 2>/dev/null || true
    fi
    if [[ -n "${server_pid}" ]] && kill -0 "${server_pid}" 2>/dev/null; then
        kill -TERM "${server_pid}" 2>/dev/null || true
        for _ in {1..100}; do
            kill -0 "${server_pid}" 2>/dev/null || break
            sleep 0.1
        done
        if kill -0 "${server_pid}" 2>/dev/null; then
            kill -KILL "${server_pid}" 2>/dev/null || true
        fi
        wait "${server_pid}" 2>/dev/null || true
    fi
    exit "${exit_status}"
}
trap cleanup EXIT INT TERM

[[ "${QUALIFICATION_ACK:-}" == "direct-physical-camera" ]] ||
    fail "set QUALIFICATION_ACK=direct-physical-camera after confirming the SPEC entry gate"
[[ -n "${camera_id}" ]] || fail "SURVEILLANCE_CAMERA_ID is required"
[[ -n "${rtsp_url}" ]] || fail "SURVEILLANCE_RTSP_URL is required"
[[ "${rtsp_url}" == rtsp://* ]] || fail "SURVEILLANCE_RTSP_URL must use rtsp://"
[[ "${rtsp_url}" != *'@'* && "${rtsp_url}" != *'?'* && "${rtsp_url}" != *'#'* ]] ||
    fail "SURVEILLANCE_RTSP_URL must not contain credentials, a query, or a fragment"
[[ -n "${SURVEILLANCE_VIDEO_WIDTH:-}" ]] || fail "SURVEILLANCE_VIDEO_WIDTH is required"
[[ -n "${SURVEILLANCE_VIDEO_HEIGHT:-}" ]] || fail "SURVEILLANCE_VIDEO_HEIGHT is required"
if [[ -n "${SURVEILLANCE_RTSP_USERNAME:-}" || -n "${SURVEILLANCE_RTSP_PASSWORD:-}" ]]; then
    [[ -n "${SURVEILLANCE_RTSP_USERNAME:-}" && -n "${SURVEILLANCE_RTSP_PASSWORD:-}" ]] ||
        fail "RTSP username and password must be supplied together"
fi

require_positive_integer QUALIFICATION_SECONDS "${qualification_seconds}"
require_positive_integer SAMPLE_EVERY_SECONDS "${sample_every_seconds}"
require_positive_integer CONNECT_TIMEOUT_SECONDS "${connect_timeout_seconds}"
require_positive_integer SURVEILLANCE_VIDEO_WIDTH "${SURVEILLANCE_VIDEO_WIDTH}"
require_positive_integer SURVEILLANCE_VIDEO_HEIGHT "${SURVEILLANCE_VIDEO_HEIGHT}"
(( qualification_seconds >= 90 )) || fail "QUALIFICATION_SECONDS must be at least 90"
(( sample_every_seconds <= qualification_seconds )) ||
    fail "SAMPLE_EVERY_SECONDS must not exceed QUALIFICATION_SECONDS"

require_command curl
require_command ffmpeg
require_command ffprobe
require_command lsof
require_command python3
require_command ps
[[ -x "${server_binary}" ]] || fail "release server is missing or not executable: ${server_binary}"

umask 077
mkdir -p "${evidence_dir}/metrics" "${evidence_dir}/probes" "${recording_root}"
chmod 700 "${evidence_dir}" "${evidence_dir}/metrics" "${evidence_dir}/probes"

cat > "${evidence_dir}/run.txt" <<EOF
status=running
started_at_utc=${utc_stamp}
camera_id=${camera_id}
expected_width=${SURVEILLANCE_VIDEO_WIDTH}
expected_height=${SURVEILLANCE_VIDEO_HEIGHT}
qualification_seconds=${qualification_seconds}
sample_every_seconds=${sample_every_seconds}
server_binary=${server_binary}
server_git_commit=$(git -C "${repo_root}" rev-parse --verify HEAD 2>/dev/null || printf 'unknown')
rtsp_endpoint=redacted-by-policy
rtsp_credentials=$([[ -n "${SURVEILLANCE_RTSP_USERNAME:-}" ]] && printf 'configured' || printf 'absent')
rtsp_proxy=absent
EOF

export SURVEILLANCE_RECORDING_ROOT="${recording_root}"
export SURVEILLANCE_HEALTH_BIND="${health_bind}"
export SURVEILLANCE_DETECTION_ENABLED="${SURVEILLANCE_DETECTION_ENABLED:-false}"
export RUST_LOG="${RUST_LOG:-info}"

"${server_binary}" > "${evidence_dir}/server.log" 2>&1 &
server_pid="$!"
printf '%s\n' "${server_pid}" > "${evidence_dir}/server.pid"

for ((second = 0; second < connect_timeout_seconds; second++)); do
    kill -0 "${server_pid}" 2>/dev/null ||
        fail "server exited before the camera became ready; inspect ${evidence_dir}/server.log"
    if curl --fail --silent --show-error --max-time 2 "${http_url}/ready" \
        > "${evidence_dir}/ready.json" 2>/dev/null; then
        break
    fi
    sleep 1
done
curl --fail --silent --show-error --max-time 2 "${http_url}/ready" \
    > "${evidence_dir}/ready.json" ||
    fail "camera did not become ready within ${connect_timeout_seconds}s"
curl --fail --silent --show-error --max-time 2 "${http_url}/live" \
    > "${evidence_dir}/live.json"

# This includes all established TCP peers owned by the server (for example
# PostgreSQL or LiveKit when the operator supplies them). The RTSP endpoint is
# intentionally not copied into the run manifest; the evidence directory is
# private and the operator may redact this file before publication.
lsof -nP -a -p "${server_pid}" -iTCP -sTCP:ESTABLISHED \
    > "${evidence_dir}/established-tcp.txt" || true
rtsp_port="$(python3 -c 'import sys, urllib.parse; parsed = urllib.parse.urlsplit(sys.argv[1]); print(parsed.port or 554)' "${rtsp_url}")"
rtsp_peer_count="$(
    grep -Ec -- "->[^ ]+:${rtsp_port}[[:space:]]+\\(ESTABLISHED\\)" \
        "${evidence_dir}/established-tcp.txt" || true
)"
[[ "${rtsp_peer_count}" == "1" ]] ||
    fail "expected one server-owned RTSP/TCP peer, observed ${rtsp_peer_count} on the configured port"

sample_metrics() {
    local sequence=0
    printf 'utc\tserver_cpu_percent\tserver_rss_kib\tchild_processes\topen_files\n' \
        > "${evidence_dir}/resource-samples.tsv"
    while kill -0 "${server_pid}" 2>/dev/null; do
        sequence=$((sequence + 1))
        curl --fail --silent --show-error --max-time 3 "${http_url}/metrics" \
            > "${evidence_dir}/metrics/$(printf '%05d' "${sequence}").prom" || return 1
        read -r cpu_percent rss_kib < <(
            ps -o '%cpu=' -o 'rss=' -p "${server_pid}" | awk '{$1=$1; print}'
        )
        child_processes="$(
            (pgrep -P "${server_pid}" 2>/dev/null || true) | wc -l | tr -d ' '
        )"
        open_files="$(lsof -p "${server_pid}" 2>/dev/null | wc -l | tr -d ' ')"
        printf '%s\t%s\t%s\t%s\t%s\n' \
            "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "${cpu_percent:-unavailable}" \
            "${rss_kib:-unavailable}" "${child_processes}" "${open_files}" \
            >> "${evidence_dir}/resource-samples.tsv"
        sleep "${sample_every_seconds}"
    done
}
sample_metrics &
sample_pid="$!"

deadline=$((SECONDS + qualification_seconds))
while (( SECONDS < deadline )); do
    kill -0 "${server_pid}" 2>/dev/null ||
        fail "server exited during qualification; inspect ${evidence_dir}/server.log"
    kill -0 "${sample_pid}" 2>/dev/null ||
        fail "metrics sampler exited during qualification"
    sleep 1
done

kill -TERM "${sample_pid}" 2>/dev/null || true
# TERM is the owned sampler shutdown path, so its signal status is expected.
wait "${sample_pid}" 2>/dev/null || true
sample_pid=""
first_metrics="$(find "${evidence_dir}/metrics" -type f -name '*.prom' -print | sort | head -1)"
[[ -n "${first_metrics}" ]] || fail "the metrics sampler produced no evidence"
curl --fail --silent --show-error --max-time 3 "${http_url}/metrics" \
    > "${evidence_dir}/metrics-final.prom"
python3 "${repo_root}/scripts/phase9/direct-camera-report.py" \
    "${first_metrics}" "${evidence_dir}/metrics-final.prom" "${camera_id}" \
    "${evidence_dir}/metrics-summary.json" ||
    fail "direct-camera metric gate failed; inspect ${evidence_dir}/metrics-summary.json"

kill -TERM "${server_pid}"
for _ in {1..300}; do
    kill -0 "${server_pid}" 2>/dev/null || break
    sleep 0.1
done
if kill -0 "${server_pid}" 2>/dev/null; then
    fail "server did not complete graceful shutdown within 30 seconds"
fi
wait "${server_pid}" || fail "server exited unsuccessfully"
server_pid=""

recording_count=0
while IFS= read -r -d '' recording; do
    recording_count=$((recording_count + 1))
    probe="${evidence_dir}/probes/$(printf '%05d' "${recording_count}").json"
    ffprobe -v error -count_frames -show_format -show_streams -of json \
        "${recording}" > "${probe}"
    ffmpeg -v error -i "${recording}" -map 0:v:0 -f null -
    printf '%05d\t%s\n' "${recording_count}" "${recording}" \
        >> "${evidence_dir}/recordings.tsv"
done < <(
    find "${recording_root}/${camera_id}" -type f -name '*.mp4' \
        ! -name '*.partial.mp4' -print0 2>/dev/null
)
(( recording_count >= 2 )) ||
    fail "expected at least two durable segments, found ${recording_count}"

partial_count="$(find "${recording_root}/${camera_id}" -type f \
    -name '*.partial.mp4' -print 2>/dev/null | wc -l | tr -d ' ')"
metric_samples="$(find "${evidence_dir}/metrics" -type f -name '*.prom' \
    -print 2>/dev/null | wc -l | tr -d ' ')"
completed_at="$(date -u '+%Y%m%dT%H%M%SZ')"

cat >> "${evidence_dir}/run.txt" <<EOF
status=completed
completed_at_utc=${completed_at}
metric_samples=${metric_samples}
durable_segments=${recording_count}
remaining_partials=${partial_count}
EOF

printf 'Direct-camera qualification completed. Evidence: %s\n' "${evidence_dir}"
