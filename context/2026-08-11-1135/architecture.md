# Architecture Context — 2026-08-11 11:35 +06

The recorder consumes complete H.264 access units already owned by the media
plane. `Fmp4SegmentSink` converts each NAL to MP4 length-prefix framing and uses
actual SPS/PPS plus normalized 90 kHz timestamps. It starts and rotates only at
keyframes, validates a 30–60 second target, and bounds samples, bytes, and
completed-output backlog.

`SegmentFileStore` owns one configured `RecordingLayout`. It rejects paths
outside that root, creates the partial file without overwrite, writes/flushes/
syncs, atomically renames to the durable path, syncs the parent directory, and
then invokes FFprobe asynchronously. Codec, dimensions, duration, and size come
from the durable file rather than configuration defaults.

The muxer currently receives PTS only and uses it as DTS. This is valid only for
the qualified no-B-frame synthetic source. Representative H.264 streams must
prove decode-order semantics or the media contract must gain explicit DTS.

`MediaWallClock` anchors media time to observed `SystemTime` and projects with
integer timebase arithmetic. Timestamp regression, timebase change, or excessive
wall-clock drift starts a numbered epoch. Storage converts mapped wall time to
UTC explicitly before constructing paths.
