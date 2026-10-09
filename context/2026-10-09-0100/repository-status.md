# Repository Status — 2026-10-09 01:00 +06

The user reported that video remained blank in Firefox after the Chromium-only
qualification in the previous snapshot. Do not describe Firefox as verified.

## Findings

- Firefox 157 viewer sessions join signalling, then stay connecting and leave
  without a connected media peer, even with the SFU advertising loopback.
- Mozilla documents that Firefox rejects loopback WebRTC by default. Local
  development requires `media.peerconnection.ice.loopback = true` in about:config.
  References: https://bugzilla.mozilla.org/show_bug.cgi?id=1659672 and
  https://bugzilla.mozilla.org/show_bug.cgi?id=1973521.
- The user has been given these steps and asked for the observed result.
  Successful Firefox decoding has not yet been confirmed.
- Both Playwright Firefox and system Firefox through geckodriver failed to
  launch automation on this host with "Could not find profile folder".
  This is a tooling limitation, not evidence that Firefox video works.

## Change

Local Firefox connection and retry notes now explain the exact browser setting.
Notes are retained by the LiveKit owner and restored after UI reconciliation;
previously replacing the tile subtree erased them at the next poll. Teardown
clears the notes. Guidance is limited to Firefox and loopback endpoints; remote
endpoints and other browsers do not receive the browser-setting suggestion.

Verified: 34 dashboard tests pass, including two regressions covering guidance
scope and persistence/cleanup. ONNX-enabled surveillance-server build passed;
git diff checks passed. No core Rust code changed and no full workspace gate
was rerun. The launcher is being restarted to serve the newly embedded UI.

A second issue was observed in logs: multiple development viewers share a token
identity and may evict each other (DUPLICATE_IDENTITY). That is not fixed in this
slice and must be addressed before claiming multi-viewer browser qualification.

Restart completed: the server serves the updated Firefox guidance. The repeated
Chromium browser check passed (1 to 464 decoded frames over 16 seconds).
Firefox success remains unconfirmed. The stack is left running.
