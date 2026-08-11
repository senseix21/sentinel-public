# Repository Status — 2026-08-11 15:07 +06

- The repository-owned external gate passes all five camera fixture tests, the
  direct recorder fixture, and the composed `surveillance-server` binary.
- The server produced three 30-second segments from one authenticated RTSP
  source, handled SIGINT, exited zero, and every output fully decoded.
- Fixture readiness now tolerates Cargo package-lock delay instead of racing a
  ten-second outage-controller deadline.
- Verified limitation: current fMP4 bytes remain memory-resident until segment
  finalization. `SIGKILL` loses the active segment and normally leaves no
  content-bearing partial file to reconcile.
