# Decisions and open questions

Accepted: isolate coverage-guided fuzz tooling rather than change the normal
production compiler or duplicate parsers. Preserve generated corpora locally and
retain curated crash seeds and stable regression tests in source.

Verified limits: bounded runs are not proof of bug absence. Socket framing,
authentication, configuration, H.264 depayloading, foreign decoder/media parsing,
and concurrent workloads still need separate targets. Phase 8's fuzz item remains
partial. Normal macOS release and the other production gates remain unresolved.

User follow-up: asked whether RTSP cameras can play through this system. The
implemented direct-camera path is H.264 RTSP/TCP with browser delivery through
LiveKit/WebRTC. A physical endpoint and credentials in local configuration,
network access, and camera-specific compatibility qualification are required.
