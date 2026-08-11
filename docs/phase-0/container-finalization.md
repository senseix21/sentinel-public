# Container and Crash-Finalization Evidence

Status: MVP decision evidence captured on 2026-08-10

## Experiment

Run:

```bash
scripts/phase0/container-crash-fixture.sh
```

The harness creates normally finalized and forcibly interrupted files for:

- H.264 and H.265/HEVC.
- Matroska.
- Fragmented MP4 using `frag_keyframe`, `empty_moov`, and `default_base_moof`.
- Classic MP4 as a negative control.

Normal files contain four seconds of synthetic 640×360, 15 FPS media. Interrupted H.264 encoders are killed after three seconds; H.265 encoders are killed after six seconds to account for observed startup/output buffering. The process receives `SIGKILL`, so no trailer or graceful encoder/muxer flush can occur. Every surviving output is probed and fully decoded.

## Results

| Codec | Container | Normal finalize | Abrupt interruption | Interrupted duration |
|---|---|---|---|---:|
| H.264 | Matroska | Probe/decode passed | Probe/decode passed | Unknown |
| H.265 | Matroska | Probe/decode passed | Probe/decode passed | Unknown |
| H.264 | Fragmented MP4 | Probe/decode passed | Probe/decode passed | 2.0 seconds |
| H.265 | Fragmented MP4 | Probe/decode passed | Probe/decode passed | 5.0 seconds |
| H.264 | Classic MP4 | Probe/decode passed | Probe failed | Unavailable |
| H.265 | Classic MP4 | Probe/decode passed | Probe failed | Unavailable |

The observed interrupted file sizes aligned with flushed chunks rather than requested process duration. This experiment demonstrates recoverability of already written media, not immunity from losing buffered media immediately before a crash.

## MVP decision

Use fragmented MP4 for the H.264 MVP recording path:

- Initialize the file for fragmented operation.
- Cut fragments only at random-access/keyframe boundaries.
- Write active segments with a `.partial.mp4` suffix.
- On normal completion, flush/finalize, sync according to the selected durability policy, probe/validate essential metadata, then atomically rename to `.mp4` within the same filesystem.
- Insert completed metadata only after the durable rename succeeds.
- On crash/restart, reconcile `.partial.mp4` files. Probeable files may be retained as explicitly incomplete/recovered segments; invalid files are quarantined or removed according to a recorded policy.
- Never expose a partial file as a completed recording.

Classic MP4 is rejected for continuous recording because abrupt termination leaves the file without required final metadata. Matroska remains a technically viable recovery-oriented alternative, but its interrupted output lacked duration and browser delivery generally benefits from the MP4 family. Any browser/playback claim still requires actual client qualification.

## Limitations

- Synthetic local encoding is not RTSP stream-copy behavior from a representative camera.
- Filesystem power-loss durability was not tested; `SIGKILL` does not model storage caches or torn writes.
- Fragment length, GOP, and muxer buffering affect recoverable duration.
- H.265 recording remains experimental and is not the MVP browser/live-view codec.
- Audio interleaving and A/V synchronization were not tested.
- PostgreSQL/file atomicity requires reconciliation because no cross-resource transaction exists.

## Required follow-up

Repeat the chosen fragmented-MP4 path with representative H.264 camera packets, abrupt process termination, machine restart/power-loss testing on production storage, and browser/playback clients.

