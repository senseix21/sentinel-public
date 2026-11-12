# Status — 2026-10-09 17:38 +06

Verified: documentation-only follow-up specification added for continuous person
presence with face-only overlays. No runtime behavior, launcher default or model
changed. Prior implemented face slice and verification remain in
[the previous snapshot](../2026-10-09-1503/repository-status.md).

User reported that turning away loses detection. Source inspection confirms
face-only default, frame-count tracker coasting, motion heartbeat, model-sized
decoder and unbounded internal tracker collection despite capped output.
The proposed combined mode must address those contracts explicitly.
