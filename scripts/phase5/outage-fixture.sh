#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
livekit_bin="${LIVEKIT_SERVER_BIN:-/usr/local/bin/livekit-server}"

if [[ ! -x "${livekit_bin}" ]]; then
    echo "missing LiveKit server: ${livekit_bin}" >&2
    exit 1
fi
if lsof -nP -iTCP:7880 -sTCP:LISTEN >/dev/null 2>&1; then
    echo "port 7880 is already in use" >&2
    exit 1
fi

cd "${repo_root}"
PHASE5_LIVEKIT_URL="ws://127.0.0.1:17880" \
    cargo test -p surveillance-livekit --test publisher_fixture \
    livekit_outage_retries_without_blocking_recorder_queue -- --ignored --exact

echo "Phase 5 startup-outage isolation fixture passed"
