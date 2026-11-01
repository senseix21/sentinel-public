# ADR 019: Camera-independent startup and durable camera intent

Status: Accepted (2026-11-01; user requested implementation).

## Problem

Application startup requires a bootstrap camera. The development launcher waits
for that camera before exposing login. Dashboard registrations disappear on
restart, and readiness confuses camera connectivity with control-plane readiness.

## Decision

The process owns storage, HTTP, settings and optional LiveKit deployment settings
without constructing a camera. An explicitly configured legacy environment
camera remains an optional development source. Missing cameras are normal.
Readiness measures configured application dependencies; per-camera degradation
remains in camera resources and metrics.

The supervisor owns a bounded, versioned, local camera catalog and serializes its
writes with camera commands. New registrations are stopped. Start/stop persist
desired intent before applying it; shutdown does not rewrite desired intent.
Restart restores each valid registration independently and resumes running intent.
A pipeline start failure does not erase running intent. Remove deletes intent,
stops the pipeline, and preserves indexed footage. A timeout has an uncertain
outcome: clients must inspect camera state before retrying (existing API rule).
This extends ADR 006's process-local lifecycle decision.

The catalog is a **secret file**, not ordinary public settings: it contains RTSP
credentials, is bounded in size, requires mode 0600, uses exclusive process
locking and atomic replace with file/directory fsync, and is excluded from Git.
It is separate from the transactional operational-settings store in ADR 008;
there is no cross-store transaction or claim that registration is a settings
change. Existing access auditing still covers the camera command boundary.
Unreadable/corrupt catalogs fail startup rather than silently losing intent.
Invalid individual entries are reported without credential values and do not
prevent the dashboard or other cameras from starting.

Dashboard camera credentials are write-only inputs, held only until submission
and cleared after success; no local/session storage, read API, settings response,
or audit entry returns them. This narrowly extends ADR 009's browser credential
rule for camera onboarding. Bearer/platform credentials remain absent from the
browser. Existing authorization and proxy proof remain mandatory. The development
proxy remains loopback-only; remote operators use SSH forwarding. A production
proxy must supply HTTPS. This is not a public deployment or packaging decision.

Playback offers indexed archive identities independently of configured cameras.
Camera identity enumeration remains authenticated and scope-filtered.

## Trade-offs and reversal

One local catalog fits the single-process deployment without a new database
secret-encryption key lifecycle or cross-store credential transaction. Protect
and back up this file as a secret; losing it loses camera configuration, not
recordings. It supports one server owner, not concurrent distributed writers.
Move public registration metadata into PostgreSQL only alongside an explicit
secret-reference migration and recovery procedure. Extraction, remote workers,
or multiple control-plane writers require revisiting this decision.

## Acceptance

Zero-camera login/API succeeds; no source, decoder or model is started. Add,
restart, start, stop, remove, archive selection and credential redaction are
covered by deterministic tests. Unreachable cameras do not make `/ready` fail.
Catalog write failures reject mutations without changing runtime intent. Webcam
and deterministic RTSP fixtures remain explicit development commands.
