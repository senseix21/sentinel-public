# Repository Status — 2026-10-09 00:49 +06

Base HEAD remains `015bc4e`; development-launcher changes are uncommitted.
Continues `context/2026-10-09/` after the user reported blank live video.

## Reproduction and fix

Verified before the fix in headless Chromium: authenticated viewer-token issuance
and room signalling worked, but ICE stayed checking, all candidate pairs received
zero bytes, and the video remained hidden with readyState 0. The user's Firefox
sessions likewise repeatedly disconnected without establishing their media peer.
LiveKit selected a non-loopback interface automatically despite --bind 127.0.0.1.

The launcher now supplies explicit local RTC configuration: node_ip 127.0.0.1,
enable_loopback_candidate true, use_external_ip false, and IP inclusion 127/32.
UDP uses LIVEKIT_PORT + 2. TCP fallback is disabled because LiveKit's TCP media
listener binds all interfaces, independently of the signalling bind. Observed
UDP socket is 127.0.0.1:7882, with no listener on 7881.

## Verification

- Same browser reproduction after the loopback change: video readyState 4,
  visible 1280x720, currentTime advancing, ICE connected, framesDecoded growing.
- `make verify-live-browser` supplies a separate development viewer identity
  and asserts actual decoding across multiple five-second UI refreshes. A run
  advanced from 7 to 485 decoded frames over 16 seconds.
- `test_livekit_loopback.py` launches the real LiveKit binary with the actual
  launcher config and checks advertised node IP, ports and TCP listening sockets.
  It skips explicitly if LiveKit/lsof are unavailable.
- 19 Python tests and 32 dashboard tests passed during this slice; shell syntax,
  browser script syntax, and git diff checks passed.
- Restart finalized the in-progress segment. The stack remains running for the user.

## Limits

Headless Chromium decoding was exercised. Firefox's original failure is evidenced
by LiveKit logs; Firefox automation could not launch on this host, so this is not
an automated Firefox decoding qualification. Accuracy, multi-camera capacity,
production network reachability and restart qualification gates remain open.
