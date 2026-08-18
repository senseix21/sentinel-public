#!/usr/bin/env bash
#
# Checks the SPS dimension parser against every recorded segment on this
# machine, using ffprobe's reported coded size as the answer.
#
# A parser validated only against byte strings chosen by its author proves
# little. This runs it over whatever real media the recorder has actually
# written and fails on the first disagreement.
#
# Requires: ffmpeg, ffprobe, python3, and a populated recordings root.
#
#   scripts/phase5/sps-corpus-fixture.sh [recordings-root]
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RECORDINGS="${1:-${RECORDINGS_ROOT:-${REPO_ROOT}/recordings}}"
CORPUS="${CORPUS_FILE:-$(mktemp -t sps-corpus)}"

for tool in ffmpeg ffprobe python3; do
  command -v "${tool}" >/dev/null 2>&1 || {
    echo "missing required tool: ${tool}" >&2
    exit 127
  }
done

[[ -d "${RECORDINGS}" ]] || {
  echo "recordings root not found: ${RECORDINGS}" >&2
  exit 66
}

echo "collecting sequence parameter sets from ${RECORDINGS}" >&2

RECORDINGS="${RECORDINGS}" CORPUS="${CORPUS}" python3 - <<'PY'
import json, os, re, subprocess, sys

recordings = os.environ["RECORDINGS"]
corpus = os.environ["CORPUS"]


def nal_units(buf):
    starts = [m.start() for m in re.finditer(b"\x00\x00\x01", buf)]
    for index, start in enumerate(starts):
        end = starts[index + 1] if index + 1 < len(starts) else len(buf)
        nal = buf[start + 3:end]
        while nal.endswith(b"\x00"):
            nal = nal[:-1]
        yield nal


def coded_size(path):
    probe = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
         "stream=coded_width,coded_height", "-of", "json", path],
        capture_output=True, text=True).stdout
    streams = json.loads(probe or "{}").get("streams") or []
    if not streams:
        return None
    width, height = streams[0].get("coded_width"), streams[0].get("coded_height")
    return None if width is None or height is None else (width, height)


paths = sorted(
    os.path.join(root, name)
    for root, _dirs, files in os.walk(recordings)
    for name in files
    if name.endswith(".mp4")
)

lines, skipped = [], 0
for path in paths:
    size = coded_size(path)
    if size is None:
        skipped += 1
        continue
    annexb = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", path, "-c:v", "copy", "-bsf:v",
         "h264_mp4toannexb", "-f", "h264", "-"],
        capture_output=True).stdout
    sps = next((n for n in nal_units(annexb) if n and (n[0] & 0x1f) == 7), None)
    if sps is None:
        skipped += 1
        continue
    lines.append(f"{sps.hex()} {size[0]} {size[1]} {path}")

with open(corpus, "w") as handle:
    handle.write("\n".join(lines) + "\n")

print(f"segments={len(paths)} usable={len(lines)} unreadable={skipped}", file=sys.stderr)
if not lines:
    print("no usable segments; nothing was verified", file=sys.stderr)
    sys.exit(65)
PY

echo "checking the parser against ${CORPUS}" >&2
SPS_CORPUS="${CORPUS}" cargo test -p surveillance-media --all-features \
  --lib sps::tests::parses_every_recorded_segment -- --ignored --nocapture
