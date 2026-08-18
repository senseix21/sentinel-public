#!/usr/bin/env bash

# Qualifies LiveKit publication against a real server and subscribed observer.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
livekit_bin="${LIVEKIT_SERVER_BIN:-/usr/local/bin/livekit-server}"
lk_bin="${LIVEKIT_CLI_BIN:-/usr/local/bin/lk}"
fixture_root="$(mktemp -d /private/tmp/surveillance-livekit-phase5.XXXXXX)"
livekit_pid=""
subscriber_pid=""

cleanup() {
    if [[ -n "${subscriber_pid}" ]] && kill -0 "${subscriber_pid}" 2>/dev/null; then
        kill -INT "${subscriber_pid}" 2>/dev/null || true
        wait "${subscriber_pid}" 2>/dev/null || true
    fi
    if [[ -n "${livekit_pid}" ]] && kill -0 "${livekit_pid}" 2>/dev/null; then
        kill -INT "${livekit_pid}" 2>/dev/null || true
        wait "${livekit_pid}" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

for executable in "${livekit_bin}" "${lk_bin}"; do
    if [[ ! -x "${executable}" ]]; then
        echo "missing Phase 5 fixture executable: ${executable}" >&2
        exit 1
    fi
done
if [[ "$("${livekit_bin}" --version | awk '{print $3}')" != "1.13.5" ]]; then
    echo "Phase 5 fixture requires livekit-server 1.13.5" >&2
    exit 1
fi
if ! "${lk_bin}" --version | grep -q '2.18.2'; then
    echo "Phase 5 fixture requires lk 2.18.2" >&2
    exit 1
fi

"${livekit_bin}" --bind 127.0.0.1 --keys $'devkey: development-secret\n' \
    >"${fixture_root}/livekit.log" 2>&1 &
livekit_pid="$!"
for _ in {1..100}; do
    if grep -q 'server is listening' "${fixture_root}/livekit.log" 2>/dev/null; then
        break
    fi
    if ! kill -0 "${livekit_pid}" 2>/dev/null; then
        echo "LiveKit exited before readiness; log: ${fixture_root}/livekit.log" >&2
        exit 1
    fi
    sleep 0.05
done
if ! kill -0 "${livekit_pid}" 2>/dev/null; then
    echo "LiveKit readiness failed; log: ${fixture_root}/livekit.log" >&2
    exit 1
fi

LIVEKIT_URL="ws://127.0.0.1:7880" \
LIVEKIT_API_KEY="devkey" \
LIVEKIT_API_SECRET="development-secret" \
"${lk_bin}" room join --identity fixture-observer --auto-subscribe \
    surveillance-fixture-camera-live >"${fixture_root}/subscriber.log" 2>&1 &
subscriber_pid="$!"

export SURVEILLANCE_LIVEKIT_URL="ws://127.0.0.1:7880"
export SURVEILLANCE_LIVEKIT_API_KEY="devkey"
export SURVEILLANCE_LIVEKIT_API_SECRET="development-secret"
export SURVEILLANCE_LIVEKIT_NAMESPACE="surveillance"

"${repo_root}/scripts/phase0/retina-rtsp-fixture.sh" | tee "${fixture_root}/rtsp-fixture.log"

if ! grep -q 'surveillance-fixture-camera-publisher' "${fixture_root}/livekit.log"; then
    echo "LiveKit never observed the owned camera publisher; logs: ${fixture_root}" >&2
    exit 1
fi
if ! grep -q 'fixture-observer' "${fixture_root}/livekit.log"; then
    echo "LiveKit never observed the independent subscriber; logs: ${fixture_root}" >&2
    exit 1
fi
if pgrep -f 'surveillance-livekit-.*fixture-camera' >/dev/null 2>&1; then
    echo "owned LiveKit publisher process leaked after surveillance shutdown" >&2
    exit 1
fi

echo "Phase 5 LiveKit/RTSP/recording fixture passed; logs: ${fixture_root}"
