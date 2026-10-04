#!/usr/bin/env bash

# Runs the real-room publication fixtures.
#
# This encodes the media and runs the tests; each test starts and stops its own
# server, because qualifying a restart means killing the server at a chosen
# moment and a wrapper cannot do that.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
livekit_bin="${LIVEKIT_SERVER_BIN:-livekit-server}"
port="${PHASE5_LIVEKIT_PORT:-7880}"
key="devkey"
secret="local-development-secret-not-for-deployment"

command -v "${livekit_bin}" >/dev/null || {
    echo "missing LiveKit server: ${livekit_bin}" >&2
    exit 1
}

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
# Serially: each test starts its own server, and two would collide on the RTC
# ports rather than only the signalling one.
# PHASE5_FIXTURE_FILTER is optional. When unset, all ignored tests run.
PHASE5_LIVEKIT_MEDIA="${media}" \
PHASE5_LIVEKIT_URL="ws://127.0.0.1:${port}" \
PHASE5_LIVEKIT_KEY="${key}" \
PHASE5_LIVEKIT_SECRET="${secret}" \
LIVEKIT_SERVER_BIN="${livekit_bin}" \
    cargo test -p surveillance-livekit --test room_fixture \
    -- --ignored --nocapture --test-threads=1 \
    ${PHASE5_FIXTURE_FILTER+"${PHASE5_FIXTURE_FILTER}"}

echo "Phase 5 real-room fixture passed"
