Prepared for the client meeting · 29 October 2026

# One camera feed. Three useful paths.

A surveillance system that saves camera video, lets operators watch live, and analyzes selected images for people and faces.

SENTINEL runs on infrastructure controlled by the client. It brings camera connections, recordings, live viewing, and detection together behind one browser dashboard.

**The key idea:** connect to each camera once, then share that incoming video inside the system. Recording, live viewing, and detection each have their own job.

### How the video moves

```text
IP camera
    | one camera connection
    v
SENTINEL receives and shares video
    |
    +--> Recording --> video files + searchable database index
    |
    +--> Live viewing --> LiveKit --> browser
    |
    `--> Sampled detection --> person presence / face results --> dashboard
```

Each camera follows this same pattern. Video files are stored separately from the database. The diagram describes the implemented local flow; production qualification remains open.

### What operators can do

Watch live cameras, check camera health, find saved recordings by camera and time, and see current detection results.

### Where the product stands

The core flow works in local development tests. Real-site camera compatibility, accuracy, sustained capacity, and deployment readiness still need validation.

---

Video journey

## From a camera to live and saved video

### 1. Connect and monitor

SENTINEL connects to an IP camera using RTSP, the camera's video-streaming protocol. It monitors that connection and retries if the camera disconnects. Other cameras have independent connections.

### 2. Share the incoming video

The server distributes the feed to recording, live viewing, and detection. Opening a second browser viewer does not create a second connection to the camera.

### 3. Save manageable recording files

The recorder saves short MP4 files, typically around one minute in the local setup. The duration is configurable. For the supported H.264 path, it keeps the original compressed video rather than processing every image again.

### 4. Keep a searchable catalogue

The video files live on storage. A PostgreSQL database acts like a library catalogue: it records which camera each file belongs to, when it starts and ends, and where the server can find it. The database does not hold the video itself.

### 5. Deliver live video to a browser

LiveKit is the video delivery service between SENTINEL and viewers. It uses WebRTC, the technology browsers use for real-time video. The current live path converts video into a browser-compatible stream. This extra work starts when a viewer is present and stops after viewers leave and a short grace period.

### Later: “Show me the entrance at 10:15”

The operator selects the entrance camera and a time range. SENTINEL checks the recording catalogue, finds the matching files, and returns the saved video for playback or retrieval. It does not need to search every folder.

**Timing expectation:** current retrieval follows recording segments and video packet/keyframe boundaries. Exact frame-by-frame clipping and unrestricted seeking are not promised.

**Retention:** older footage is removed according to configured age and storage-pressure policies. The client must confirm the retention period and usable storage. A 30-day goal requires sizing against actual camera bitrates.

---

Detection explained

## Seeing a person is different from seeing a face

Detection examines selected images from the same incoming video. It runs alongside recording.

### Person presence

The person model looks for people, including people whose faces are not visible. Short-lived tracking helps maintain a current presence estimate between observations. This is a local estimate with expiry rules; it is not a count of unique visitors.

### Visible faces

A separate face model locates visible faces. Face results drive the rectangles drawn on live video. A person can be present with no face rectangle, for example when facing away.

**Example:** someone walks into view and then turns away. Person detection can still report their presence, while the face rectangle disappears when there is no fresh visible-face result. Actual results depend on the scene and model accuracy.

### Why analyze selected images?

Recording saves the incoming video, but detection does not need to inspect every frame. Sampling reduces processing work. If detection falls behind, it prefers recent images over a growing backlog of old ones.

### How results reach the dashboard

The detector produces small results: camera, time, presence information, and detected locations. An internal message channel passes these facts to interested components, and live detection updates reach the dashboard through LiveKit. Video remains on its separate media path.

### What is implemented locally

- Real person and face models.

- Independent person and face processing lanes.

- Current presence information and face overlays.

- Expiry of stale results.

### What this does not establish

- A person's identity or face recognition.

- Accurate unique-visitor counts.

- Production accuracy in all lighting and camera positions.

- A searchable, permanent detection-event history.

**Saved evidence:** recordings keep the original camera video. Dashboard face rectangles are not burned into the recorded files. Live boxes are approximate overlays, not a promise of exact frame synchronization.

Heatmaps, zone-based occupancy, dwell time, and automatic alert rules remain future product work. Their meaning and acceptance criteria must be agreed before implementation.

---

Reliability and access

## Keep problems contained and visible

The system separates camera, recording, live-viewing, and detection responsibilities. A problem in a less critical feature should not unnecessarily interrupt recording.

### A camera disconnects

That camera retries; the other cameras continue. Footage cannot be recovered for the interval when no video was received. Controlled local source-outage tests have demonstrated recording, detection, and existing live viewers recovering.

### Detection or live delivery fails

Recording has its own consumer and is designed to continue. Local model-startup failure tests demonstrated recording and live viewing continuing. LiveKit-server restart recovery still needs separate qualification.

### The database is temporarily unavailable

Video can continue to be written to storage while search and indexing degrade. Reconciliation compares stored files with the catalogue when the database returns. This is recovery support, not unlimited outage tolerance.

### Storage is running out

The system monitors space and applies retention cleanup. If it cannot free enough space or storage cannot keep up, recording is at risk and the failure is reported. No software can promise continued recording on a full or failed disk.

### Access to cameras and footage

Protected requests require authentication. Permissions restrict actions and camera access. Sensitive actions are audited, and camera credentials are not returned to ordinary clients. Production login, secure HTTPS access, network exposure, and identity-provider integration must be finalized for deployment.

**Self-hosted does not automatically mean backed up.** Video lives on client-controlled storage. Backup, disk redundancy, recovery procedures, and operational ownership remain deployment decisions.

### What still needs hardening

Open issues include sustained recording backpressure, recording-index conflicts after long outages, variable full-stack shutdown time, and dependency-security/build checks. Local recovery success does not yet establish production reliability.

The initial target is 10 cameras on one machine, followed by 20. Those capacities require sustained tests on the selected hardware. A route to 100–200+ cameras is an architectural direction, not a supported-capacity claim.

---

Meeting guide

## What to say and what to agree

**A one-minute explanation**

“SENTINEL connects to each camera once. It shares the video with three independent functions: saving recordings, delivering live video to a browser, and checking selected images for people and visible faces. Recordings stay on our server, with a database catalogue so we can find footage by camera and time. The core flow works in local tests. The next step is to qualify your cameras, hardware, detection accuracy, and operating requirements before production rollout.”

**Implemented and demonstrated locally**

Camera ingest and reconnect; segmented recording and indexed retrieval; browser live viewing; real person/face detection; access controls, settings, health, and operational metrics.

**Requires site and production qualification**

Actual camera models and formats; low-light and real-scene accuracy; sustained 10/20-camera load; viewer concurrency and latency; storage reliability; production security, packaging, backups, and upgrades.

**Future scope to define**

Heatmaps, zone-based analytics, dwell time, alerts, permanent detection history, and any identity or unique-person requirement.

### Decisions to confirm with the client

- **Cameras:** how many initially, which models, resolution and frame rate; is audio or remote camera movement required?

- **Recording:** how many days, who can retrieve footage, and how precise must exported clips be?

- **Live viewing:** how many simultaneous users and cameras per screen, local or remote access, and acceptable delay?

- **Detection:** which situations matter, what should happen after detection, and what level of missed or false detections is acceptable?

- **Operations:** server and storage location, user roles, backup responsibility, outage tolerance, and support ownership?

**Recommended next milestone:** agree a pilot acceptance checklist, test representative cameras on the proposed server, resolve reliability/security gaps, then run a sustained 10-camera trial before increasing to 20.

**Basis:** prepared 29 October 2026 from the checked-out implementation, README, ROADMAP, accepted ADRs 014–017, person/face verification notes, and the latest recorded local qualification (23 October 2026). No new runtime qualification was performed for this document. The architecture proposal describes the target; it is not proof of delivered capability.

## Internal source references

- [Current project overview](../../README.md)
- [Delivery roadmap](../../ROADMAP.md)
- [Latest recorded local qualification](../../context/2026-10-23/repository-status.md)
- [Person-presence design decision](../adr/017-person-presence-with-face-overlays.md)
- [Person-presence verification](../person-presence/VERIFICATION.md)
- [Face-detection verification](../face-detection/VERIFICATION.md)
- [Recording retrieval contract](../adr/014-recording-video-subresource.md)
