# Decisions and Open Questions — 2026-08-11 20:54 +06

## Accepted Phase 3 decisions

- Add creates a configured/stopped entry; start is explicit. Repeated start/stop
  is idempotent, duplicate add and unknown operations are typed failures.
- Recorder-first exit or preservation backpressure degrades/stops only its
  owning camera. Restart requires an explicit start until storage retry policy is
  designed.
- Reconnect backoff resets after a configured 30-second stable session, not on
  every brief successful connection.
- Stopped/terminal entries remain inspectable; removed entries disappear after
  cancellation and join.
- API/listener failure is an independently joined sibling and cannot signal
  camera shutdown. All Phase 3 HTTP remains loopback-only.

## External gates

- Ten representative 4 MP/15 FPS/H.264 High Profile cameras are unavailable.
- Target production CPU, storage, network, firmware, and the agreed soak duration
  remain unconfirmed.
- The synthetic run qualifies orchestration and isolation only; it is not a
  ten-camera production-capacity claim.

