#!/usr/bin/env bash

# Runs the real-room publication fixture against a LiveKit server it owns.
#
# The server's lifetime belongs here rather than to the test, so a failing test
# cannot leave one listening and a second run cannot silently qualify against a
# server left over from the first.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
livekit_bin="${LIVEKIT_SERVER_BIN:-livekit-server}"
port="${PHASE5_LIVEKIT_PORT:-7880}"
key="devkey"
secret="local-development-secret-not-for-deployment"
log="${TMPDIR:-/tmp}/phase5-room-fixture-livekit.log"

command -v "${livekit_bin}" >/dev/null || {
    echo "missing LiveKit server: ${livekit_bin}" >&2
    exit 1
}

cleanup() {
    if [[ -n "${server_pid:-}" ]] && kill -0 "${server_pid}" 2>/dev/null; then
        kill "${server_pid}" 2>/dev/null || true
        wait "${server_pid}" 2>/dev/null || true
    fi
}
trap cleanup EXIT

"${livekit_bin}" --dev --bind 0.0.0.0 \
    --config-body "port: ${port}" \
    --keys "${key}: ${secret}" >"${log}" 2>&1 &
server_pid=$!

for _ in $(seq 1 100); do
    if (echo >"/dev/tcp/127.0.0.1/${port}") 2>/dev/null; then
        break
    fi
    kill -0 "${server_pid}" 2>/dev/null || {
        echo "LiveKit server exited during startup; see ${log}" >&2
        tail -20 "${log}" >&2
        exit 1
    }
    sleep 0.2
done

# Real encoded media, so the fixture proves pictures rather than a track.
# Generated here rather than committed: it is 2 seconds of colour bars, it
# regenerates in under a second, and a binary blob in the tree would have to be
# trusted rather than read.
#
# Baseline-ish High Profile at a small geometry with a keyframe every fifteen
# frames and one slice per picture, which is what lets the fixture group NAL
# units into access units by looking for the next slice.
media="${TMPDIR:-/tmp}/phase5-room-fixture.h264"
ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i "testsrc=size=320x240:rate=15" -t 2 \
    -c:v libx264 -profile:v high -g 15 -x264-params "slices=1" \
    -pix_fmt yuv420p -f h264 "${media}"
test -s "${media}" || {
    echo "ffmpeg produced no media at ${media}" >&2
    exit 1
}

cd "${repo_root}"
PHASE5_LIVEKIT_MEDIA="${media}" \
PHASE5_LIVEKIT_URL="ws://127.0.0.1:${port}" \
PHASE5_LIVEKIT_KEY="${key}" \
PHASE5_LIVEKIT_SECRET="${secret}" \
    cargo test -p surveillance-livekit --test room_fixture -- --ignored --nocapture

echo "Phase 5 real-room publication fixture passed"
