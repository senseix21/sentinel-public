# Architecture Context — 2026-08-10 19:31 +06

The H.264 MVP recording container is fragmented MP4.

Active segments use `.partial.mp4`. On graceful completion the recorder flushes/finalizes, applies the selected durability sync, validates essential metadata, and atomically renames within the same filesystem to `.mp4`. Only then may completed metadata be persisted.

Startup reconciliation treats a probeable partial file as incomplete/recovered, never completed. A final file without metadata is an orphan eligible for metadata reconstruction. Completed metadata without a file is an explicit inconsistency.

Fragments and segment rotation align with safe random-access/keyframe boundaries. The fragment/GOP policy bounds likely crash loss, but only production storage/power-loss tests can establish durability.

Matroska remains a recovery-oriented fallback. Classic MP4 is rejected for continuous recording because abrupt interruption left both codec variants unprobeable.

