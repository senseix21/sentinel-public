Multi-Camera Security & Video Analytics Platform
System Architecture & Design Specification
Status: Architecture Proposal
Target deployment: Homes and malls
Initial deployment: 10 cameras
Target deployment: 20 cameras
Future scalability target: 200+ cameras
Primary media infrastructure: LiveKit
Implementation direction: Rust-based systems architecture
Recording retention: 30 days

1. Executive Summary
The system is a self-hosted video surveillance and analytics platform designed for home and mall security.
The system must simultaneously:
Ingest video from up to 20 cameras.
Continuously record camera feeds locally.
Retain approximately 30 days of footage.
Publish live camera feeds through LiveKit/WebRTC.
Detect humans in camera feeds.
Determine where people are concentrated.
Support configurable Regions of Interest (ROI).
Generate heatmap/occupancy information.
Allow users to retrieve recorded footage by camera and date/time range.
Continue critical operations independently when non-critical components fail.
The architecture should initially support 20 cameras efficiently while avoiding design decisions that prevent scaling toward 100–200+ cameras.
The system should therefore be designed as a modular media-processing platform, rather than as a conventional CRUD/web application.

2. Primary Business Problem
The business problem is:
Provide reliable real-time and historical visibility into physical spaces for security and monitoring.
The system should allow an operator to answer:
What is happening right now?
Which cameras are online?
Where are people concentrated?
Did someone enter a particular area?
What happened at a particular time?
Can I retrieve the corresponding footage?
The system therefore has two fundamental data paths:
Real-time path
Camera
   ↓
Ingestion
   ↓
Live Stream
   ↓
LiveKit
   ↓
Operator

Historical/analytics path
Camera
   ↓
Ingestion
   ↓
Recording
   ↓
Local Storage
   ↓
Time/Event Query
   ↓
Playback

Analytics operate alongside these paths.

3. Requirements
3.1 Functional Requirements
Camera ingestion
The system shall:
Support up to 10 cameras initially.
Support up to approximately 20 cameras in the target deployment.
Be capable of extending beyond 20 cameras.
Support heterogeneous camera specifications.
Support IP cameras using RTSP.
Automatically reconnect disconnected cameras.
Maintain camera health status.
Recording
The system shall:
Record continuously.
Store recordings locally.
Retain approximately 30 days of footage.
Automatically remove footage beyond the retention period.
Index recordings by camera and timestamp.
Allow historical footage to be retrieved by time range.
Live streaming
The system shall:
Publish camera streams through LiveKit.
Support multiple simultaneous viewers.
Provide low-latency live viewing.
Keep live streaming independent from recording.
Human detection
The system shall:
Detect humans in camera footage.
Produce timestamped detection information.
Associate detections with a camera.
Eventually support tracking/occupancy analytics where required.
ROI and heatmap
The system shall:
Support regions of interest.
Determine where people are concentrated.
Generate heatmap/occupancy information.
Associate analytics information with cameras and timestamps.
API
The API shall provide:
Camera information.
Camera health.
Recording search.
Time-range video retrieval.
Detection/event queries.
Heatmap/analytics information.

4. Non-Functional Requirements
Reliability
A failure in one subsystem should not unnecessarily stop another subsystem.
For example:
AI failure
    ≠
Recording failure

and:
LiveKit failure
    ≠
Recording failure

Recording is a critical data-preservation function and should be isolated from downstream consumers.

Performance
Performance should be measured using percentile-based metrics, especially p90, rather than averages alone.
Important measurements include:
Camera ingestion latency
Detection latency
Live-stream latency
API response latency
Playback startup latency
Event processing latency
Exact p90 targets remain an open requirement and must be agreed with Erik.

Availability
The system should operate continuously and automatically recover from:
Camera disconnection
Temporary network failure
Process failure
LiveKit failure
Analytics failure

Scalability
The initial target is:
10 cameras → 20 cameras

The architecture should not fundamentally prevent:
20 → 50 → 100 → 200+


5. Important Camera Requirements
The following camera characteristics must be confirmed before final hardware sizing:
Critical
Resolution
FPS
Codec
Average bitrate
Required protocol
RTSP availability
Optional
ONVIF
Audio
PTZ
Wired vs Wi-Fi
The most important four are:
Resolution
FPS
Codec
Bitrate

These determine much of the CPU, GPU, network, storage, and decoding requirements.
For example:
20 × 30 FPS
=
600 incoming frames/sec

At 200 cameras:
200 × 30 FPS
=
6,000 incoming frames/sec

However, human detection should not necessarily run at the full camera frame rate.

6. Proposed High-Level Architecture
                        ┌─────────────────────┐
                         │     IP Cameras      │
                         │      RTSP/ONVIF     │
                         └──────────┬──────────┘
                                    │
                                    ▼
                         ┌─────────────────────┐
                         │ Camera Ingestion    │
                         │                     │
                         │ RTSP connection     │
                         │ Reconnection         │
                         │ Timestamping         │
                         │ Health monitoring   │
                         └──────────┬──────────┘
                                    │
                                    ▼
                         ┌─────────────────────┐
                         │ Media Distribution  │
                         │ / Frame Pipeline    │
                         └──────┬────┬────┬─────┘
                                │    │    │
                    ┌───────────┘    │    └─────────────┐
                    ▼                ▼                  ▼
             ┌────────────┐  ┌─────────────┐   ┌─────────────┐
             │ Recorder   │  │ LiveKit     │   │ Detection   │
             │            │  │ Publisher   │   │ Pipeline    │
             └─────┬──────┘  └──────┬──────┘   └──────┬──────┘
                   │                │                  │
                   ▼                ▼                  ▼
             ┌────────────┐   ┌────────────┐    ┌─────────────┐
             │ Local     │   │ LiveKit    │    │ Analytics   │
             │ Storage   │   │ Server     │    │ Engine      │
             └─────┬──────┘   └──────┬─────┘    └──────┬──────┘
                   │                │                  │
                   │                │                  ▼
                   │                │          ┌─────────────┐
                   │                │          │ ROI /       │
                   │                │          │ Heatmaps    │
                   │                │          └──────┬──────┘
                   │                │                 │
                   ▼                ▼                 ▼
             ┌────────────────────────────────────────────┐
             │              Metadata Database             │
             │                                             │
             │ Cameras / Recordings / Events / Analytics  │
             └─────────────────────┬───────────────────────┘
                                   │
                                   ▼
                            ┌──────────────┐
                            │ API Service  │
                            └──────┬───────┘
                                   │
                                   ▼
                            ┌──────────────┐
                            │  Dashboard   │
                            └──────────────┘


7. Core Architectural Principle
Decode once, distribute many
The architecture should avoid every subsystem independently connecting to every camera.
Bad:
Camera
 ├── Recorder → RTSP connection
 ├── AI → RTSP connection
 └── LiveKit → RTSP connection

This creates unnecessary:
network traffic
decoding
connection management
synchronization problems
Preferred:
Camera
   ↓
Ingestion
   ↓
Media Distribution
 ├── Recorder
 ├── LiveKit
 └── Detection

The ingestion layer owns the camera connection.
Consumers subscribe to the media pipeline.





8. Camera Ingestion
The ingestion subsystem owns camera connectivity.
Responsibilities:
Establish RTSP connection.
Authenticate with camera.
Receive packets.
Detect connection failures.
Reconnect.
Maintain camera state.
Timestamp media.
Expose media to downstream consumers.
Possible camera states:
DISCONNECTED
CONNECTING
CONNECTED
RECONNECTING
ERROR

The ingestion subsystem should not own:
AI inference
recording policy
user authentication
dashboard logic
This keeps its responsibility narrow.

9. Media Distribution Layer
This is an important architectural boundary.
The ingestion system receives the camera stream and distributes it to consumers.
Potential consumers:
               Media Distribution
                 /       |       \
                /        |        \
               ▼         ▼         ▼
          Recorder    LiveKit    Detection

This allows additional consumers to be added later:
Thumbnail Generator
Cloud Backup
Object Detection
Face Detection
License Plate Detection
Motion Detection
Audio Analysis

without changing camera connectivity.

10. Recording Architecture
Recording is a critical subsystem.
The recorder should create segmented video files rather than enormous continuous files.
Example:
camera-01/
    2026/
        08/
            08/
                10-00-00.mp4
                10-01-00.mp4
                10-02-00.mp4

Segment duration can be configurable.
A practical starting point is approximately:
30–60 seconds

The exact value should be validated based on playback requirements and filesystem performance.

11. Recording Metadata
The database should index recordings.
Example:
recordings
--------------------------------
id
camera_id
start_time
end_time
duration
file_path
codec
resolution
size_bytes
created_at

The API should query metadata rather than scan the filesystem.

12. Time-Range Playback
Example:
GET /cameras/{camera_id}/recordings
    ?start=...
    &end=...

Processing:
API request
     ↓
Query recording index
     ↓
Find matching segments
     ↓
Determine required portions
     ↓
Return/stream requested footage

For example:
Requested:

10:03:15 → 10:05:20

Existing segments:

10:03:00 → 10:04:00
10:04:00 → 10:05:00
10:05:00 → 10:06:00

The system identifies exactly which files are required instead of searching the entire disk.

13. Retention
Retention target:
30 days

The recorder/storage manager should continuously enforce:
if recording_age > retention_period:
    delete_recording()

However, deletion should preferably be based on indexed metadata and storage state rather than blindly scanning directories.
Storage pressure should also be monitored.
Example policy:
Normal
    ↓
Storage warning
    ↓
Storage critical
    ↓
Aggressive retention cleanup


14. Human Detection Pipeline
Human detection should be treated as an independent workload.
Camera Stream
      ↓
Frame Sampling
      ↓
Preprocessing
      ↓
Human Detector
      ↓
Detection Results
      ↓
Tracking / Aggregation
      ↓
Analytics

Do not automatically perform inference on every frame.
For example:
Camera:
30 FPS

Detection:
5 FPS

This is an architectural parameter that can later be tuned.
The detector should produce structured results rather than directly manipulating recordings or dashboards.
Example:
{
  "camera_id": "camera-03",
  "timestamp": "2026-08-08T10:15:32Z",
  "objects": [
    {
      "type": "person",
      "confidence": 0.94,
      "bounding_box": {
        "x": 0.42,
        "y": 0.21,
        "width": 0.12,
        "height": 0.31
      }
    }
  ]
}


15. Detection vs Tracking
Detection answers:
Is there a person in this frame?
Tracking answers:
Is this the same person we saw previously?
For basic MVP human detection, detection can be implemented first.
Tracking should be introduced when requirements demand:
occupancy duration
people entering/exiting areas
movement paths
unique-person counting
This follows the agreed principle:
Detection first, then the rest.

16. ROI
A Region of Interest is an area within a camera's field of view.
Example:
+---------------------------------------+
|                                       |
|          Camera Image                 |
|                                       |
|      +-------------------+            |
|      |     ROI: Door     |            |
|      |                   |            |
|      +-------------------+            |
|                                       |
+---------------------------------------+

An ROI should be represented independently from the video.
Example:
roi
----------------------
id
camera_id
name
polygon
created_at
updated_at

A camera may eventually have multiple ROIs.

17. Heatmap
The heatmap should be derived from human detections.
Conceptually:
Human Detection
       ↓
Position
       ↓
Spatial Aggregation
       ↓
Temporal Aggregation
       ↓
Heatmap

The heatmap can represent:
people density
occupancy
frequently occupied areas
The exact semantic definition must still be confirmed with Erik.

18. LiveKit Architecture
LiveKit is responsible for real-time distribution, not archival storage.
Camera
   ↓
Ingestion
   ↓
LiveKit Publisher
   ↓
LiveKit
   ↓
WebRTC
   ↓
Dashboard

The recording system should not depend on LiveKit being available.
If:
LiveKit DOWN

the expected behavior is:
Recording → CONTINUES
Detection → CONTINUES
Live Viewing → UNAVAILABLE

This is deliberate fault isolation.

19. Failure Isolation
The system should be designed around failure domains.
Camera fails
Camera 3 DOWN

Camera 1 → continues
Camera 2 → continues
Camera 4 → continues
...

Camera 3 enters reconnecting state.

AI fails
AI DOWN

Recording → continues
LiveKit → continues
API → continues

Detection temporarily becomes unavailable.

LiveKit fails
LiveKit DOWN

Recording → continues
Detection → continues
Playback → continues
Live viewing → unavailable


Database fails
This requires careful design.
The media pipeline should not immediately stop recording simply because metadata storage is temporarily unavailable.
The system should have a bounded strategy for handling temporary metadata failures.

Disk fills
The system should:
Detect storage pressure.
Emit an alert.
Apply retention cleanup.
Protect the most recent footage.
Report failure if storage cannot be recovered.

20. Scalability Analysis
The key architectural question is:
If the system grows from 20 cameras to 200 cameras, what breaks first?
We should reason from resource consumption.

20.1 Human Detection
This is likely one of the first major bottlenecks.
At:
30 FPS × 20 cameras
=
600 frames/sec

At:
30 FPS × 200 cameras
=
6,000 frames/sec

Running expensive inference on every frame is not practical.
Therefore:
Camera FPS
     ↓
Sampling
     ↓
Detection FPS

The detection workload must be independently scalable.
Possible future architecture:
                   Detection Queue
                          │
            ┌─────────────┼─────────────┐
            ▼             ▼             ▼
        Detector 1    Detector 2    Detector N

This allows horizontal scaling.

21. GPU Scaling
If detection uses GPU acceleration, GPU capacity eventually becomes finite.
The failure pattern is:
More cameras
     ↓
More inference jobs
     ↓
GPU queue increases
     ↓
Inference latency increases
     ↓
Detection becomes stale

Therefore the system should monitor:
inference FPS
queue depth
inference latency
GPU utilization
dropped/skipped frames
A detector should be able to apply backpressure or frame dropping rather than allowing unbounded queues.

22. Storage Scaling
Storage can become a major bottleneck.
For example, assume:
4 Mbps per camera

At 20 cameras:
4 Mbps × 20
=
80 Mbps

At 200 cameras:
4 Mbps × 200
=
800 Mbps

800 Mbps is approximately:
100 MB/s

Continuous storage:
100 MB/s × 86,400 sec/day
≈ 8.64 TB/day

30 days:
≈ 259 TB

This is a rough estimate using decimal units and assumes the full 4 Mbps is continuously recorded.
Actual storage requirements depend on:
bitrate
codec
VBR/CBR
camera activity
recording policy
audio
metadata
filesystem overhead
The important architectural conclusion is:
At 200 cameras, storage becomes a first-class subsystem rather than simply "a directory on the server."

23. Network Scaling
Network consumption has two major sources.
Camera → system
20 cameras × bitrate

and eventually:
200 cameras × bitrate

System → viewers
Viewer traffic can become much larger.
For example:
1 camera
   ↓
100 viewers

can produce substantially more egress traffic than the camera's original ingress stream.
This is one reason LiveKit exists as a media distribution layer.
At larger scale, LiveKit itself may need to become distributed.

24. LiveKit Scaling
At 20 cameras:
Single LiveKit deployment

may be sufficient depending on viewer count and hardware.
At larger scale:
                Load Balancer
                       │
              ┌────────┴────────┐
              ▼                 ▼
          LiveKit A         LiveKit B
              │                 │
              ▼                 ▼
           Cameras           Cameras

The exact topology depends on LiveKit's deployment model and required scale.
The important architectural principle is:
Do not make the rest of the system dependent on a single LiveKit process.

25. Database Scaling
The database primarily stores metadata:
Cameras
Recordings
Detections
Events
ROIs
Analytics
Users

It should not store raw video.
For example:
BAD:

PostgreSQL
   ↓
Video blobs

Preferred:
Local filesystem/object storage
   ↓
Video

PostgreSQL
   ↓
Metadata + indexes

Detection/event metadata is significantly smaller than video data and should scale much more easily.

26. Memory
Every camera can create:
packet buffers
decoded frame buffers
detection queues
encoding buffers
LiveKit buffers
At 200 cameras, careless buffering can consume significant memory.
Therefore queues should be:
bounded
observable
backpressure-aware
independently configurable
Never allow:
camera → infinite queue

A slow consumer should not cause unlimited memory growth.

27. Resource Bottleneck Ranking
Initial expected ranking:
Resource
Expected risk
AI inference
Very High
GPU inference capacity
Very High
Storage capacity/write throughput
High
Network
High
LiveKit resources
Medium–High
Memory/buffering
Medium
Database
Low–Medium
REST API
Low

This is a hypothesis and should eventually be validated through benchmarking.

28. Architectural Response to Scaling
The architecture should make expensive components independently scalable.
Instead of:
One giant application

conceptually:
                   Camera Ingestion
                           │
                    Media Distribution
                    /        |        \
                   /         |         \
                  ▼          ▼          ▼
             Recording    LiveKit    Detection
                                          │
                                   Detection Queue
                                          │
                              ┌───────────┼───────────┐
                              ▼           ▼           ▼
                            GPU 1       GPU 2       GPU N

This does not necessarily mean deploying dozens of microservices initially.
For 10–20 cameras, a modular monolith is likely the better starting point.
The architecture should create boundaries that can later be extracted when scale requires it.

29. Modular Monolith vs Microservices
Recommended MVP
Use a modular architecture in one deployable system.
Conceptually:
surveillance/
├── camera/
├── media/
├── recorder/
├── detector/
├── analytics/
├── livekit/
├── storage/
├── api/
├── config/
└── common/

This avoids premature distributed-system complexity.
At larger scale, modules can become independent services:
camera-service
recording-service
detection-service
analytics-service
api-service

The architecture should therefore optimize for:
Strong internal boundaries first, distributed deployment later.

30. Rust Architecture
Rust is particularly appropriate for the core media/control plane because the system requires:
concurrency
predictable resource usage
networking
long-running processes
failure isolation
bounded memory
high I/O throughput
A conceptual Rust workspace:
surveillance/
│
├── crates/
│   ├── camera/
│   ├── media/
│   ├── recorder/
│   ├── detector/
│   ├── analytics/
│   ├── livekit/
│   ├── storage/
│   ├── api/
│   ├── config/
│   └── common/
│
└── apps/
    └── surveillance-server/


31. Core Rust Interfaces
The architecture should be driven by interfaces rather than concrete implementations.
Conceptually:
trait CameraSource {
    async fn connect(&mut self) -> Result<()>;
    async fn receive(&mut self) -> Result<MediaPacket>;
}

trait Recorder {
    async fn write(&mut self, packet: MediaPacket) -> Result<()>;
}

trait Detector {
    async fn detect(&self, frame: Frame) -> Result<DetectionResult>;
}

trait StreamPublisher {
    async fn publish(&self, stream: MediaStream) -> Result<()>;
}

The exact interfaces should be designed after the media representation is finalized.

32. Important Design Decision: Packet vs Frame
The architecture should distinguish between:
Encoded packet

and:
Decoded frame

This is important.
Recording generally benefits from retaining the original encoded stream.
AI generally requires decoded frames.
Therefore the pipeline should avoid unnecessary decode/re-encode cycles.
Conceptually:
RTSP
 │
 ├── Encoded packets ──────► Recorder
 │
 └── Decode ───────────────► Detection

LiveKit's exact requirements will determine whether and where additional encoding is necessary.
This decision can significantly reduce CPU consumption.

33. Backpressure
Every pipeline needs explicit backpressure.
Example:
Camera
  ↓
100 FPS
  ↓
Detector
  ↓
only handles 20 FPS

Do not allow:
100 FPS → unlimited queue

Instead:
bounded queue
      ↓
sampling/drop policy
      ↓
detector

For surveillance analytics, dropping stale frames is often preferable to processing old frames after their usefulness has passed.

34. Observability
Every camera and subsystem should expose metrics.
Camera metrics
connection_state
input_fps
bitrate
packet_loss
reconnect_count

Recording
recording_fps
write_rate
disk_usage
segments_written
recording_errors

Detection
inference_fps
inference_latency
queue_depth
GPU_utilization
detections_per_second
dropped_frames

LiveKit
publish_state
subscriber_count
stream_latency
errors

API
request_count
p50
p90
p99
error_rate


35. P90 Performance Model
Average latency is insufficient.
For example:
Average = 200 ms
P90 = 1.8 seconds

The average looks good, but 10% of requests are very slow.
Therefore the system should establish p90 targets for:
Live streaming
Define:
P90 live latency < agreed target

Detection
P90 detection latency < agreed target

API
P90 recording-search latency < agreed target

Playback
P90 playback-start latency < agreed target

Exact numbers should be agreed with Erik rather than invented as requirements.

36. Security
The system should eventually include:
authenticated users
authorization
TLS
secure LiveKit credentials
protected camera credentials
secure API endpoints
audit logging
Camera credentials should never be exposed to dashboard clients.
The browser should receive authorized LiveKit access rather than direct camera credentials.

37. Dashboard
The dashboard is a consumer of the platform.
Initial capabilities:
Camera Grid
    ↓
LiveKit streams

Camera Selection
    ↓
Timeline

Time Range
    ↓
Playback

Analytics
    ↓
Heatmap / People Density

The dashboard should not directly interact with cameras.

38. API Design
Potential API surface:
GET /api/cameras

GET /api/cameras/{id}

GET /api/cameras/{id}/health

GET /api/cameras/{id}/recordings

GET /api/cameras/{id}/playback

GET /api/cameras/{id}/events

GET /api/cameras/{id}/heatmap

GET /api/cameras/{id}/rois

The API should operate on logical resources rather than exposing internal filesystem structure.

39. Data Ownership
Each subsystem should have clear ownership.
Data
Owner
Camera connection
Camera subsystem
Media stream
Media subsystem
Video files
Recording subsystem
Recording metadata
Storage subsystem
Detection
Detection subsystem
ROI
Analytics/configuration
Heatmap
Analytics subsystem
Users
Authentication/API subsystem
Live stream
LiveKit subsystem

This prevents multiple components from modifying the same state unpredictably.

40. Failure Philosophy
The architecture should follow:
Fail independently, recover automatically, preserve critical data.
Priority should roughly be:
Recording
   ↓
Camera connectivity
   ↓
Live streaming
   ↓
Detection
   ↓
Analytics

The exact business priority should be confirmed with Erik.

41. Testing Strategy
The system should be tested at several levels.
Unit tests
ROI calculations
retention logic
timestamp handling
event aggregation
configuration
state transitions
Integration tests
RTSP camera
recorder
LiveKit
detector
database
Failure tests
Simulate:
Camera disconnect
Network loss
Disk full
Database unavailable
AI process crash
LiveKit unavailable
Machine restart

Load tests
Test:
10 cameras
20 cameras
50 cameras
100 cameras
200 cameras

Measure:
CPU
GPU
RAM
network
disk
latency
dropped frames

42. Benchmarking Strategy
Before claiming that the architecture supports 20 or 200 cameras, benchmark the actual workload.
At minimum:
Camera count
Resolution
FPS
Codec
Bitrate
Detection FPS
Viewer count

Measure:
CPU %
GPU %
RAM
Disk MB/s
Network Mbps
P50
P90
P99
Dropped frames
Detection latency

The architecture should be driven by measured bottlenecks, not assumptions.

43. Architecture Evolution
Phase 1 — 10 cameras
Single server
    ↓
Modular Rust application
    ↓
Local storage
    ↓
Single LiveKit deployment

Phase 2 — 20 cameras
Optimize:
decoding
recording
AI sampling
GPU utilization
disk throughput
Phase 3 — 50–100 cameras
Extract heavy workloads:
Recording nodes
Detection workers
LiveKit nodes

Phase 4 — 200+ cameras
Introduce distributed scheduling and independent horizontal scaling where justified.

44. What We Should NOT Do Yet
Do not prematurely introduce:
Kubernetes
Kafka
microservices everywhere
distributed databases
cloud storage
complex service meshes
unless actual requirements justify them.
For 10–20 cameras, these technologies can add more operational complexity than value.
The first objective should be a reliable, measurable pipeline.

45. Open Questions for Erik
Before freezing the design, these questions remain:
Camera
What resolution/FPS will the cameras use?
H.264 or H.265?
Average bitrate?
RTSP confirmed?
Audio required?
PTZ required?
Detection
Which human-detection model?
What hardware/GPU is available?
What detection FPS is acceptable?
Is tracking required after detection?
Heatmap
Does "heatmap" mean person density, occupancy, movement, or something else?
Are ROIs configurable by users?
Playback
Stream only or allow downloads?
How quickly should playback begin?
Should playback include pre/post-event buffering?
LiveKit
Expected concurrent viewers?
Target live latency?
Single LiveKit node initially?
Deployment
What server hardware is available?
One physical server or multiple?
Linux distribution?
Is this an appliance-style deployment?
Reliability
What must never stop?
Required uptime?
What should happen during disk failure?
What should happen after power loss?

46. Senior Architect Decision Framework
The architecture should be evaluated using this sequence:
                   BUSINESS PROBLEM
                           ↓
                     REQUIREMENTS
                           ↓
                     CONSTRAINTS
                           ↓
                      DATA FLOW
                           ↓
                   RESPONSIBILITIES
                           ↓
                       BOUNDARIES
                           ↓
                      INTERFACES
                           ↓
                    FAILURE MODES
                           ↓
                   RESOURCE ANALYSIS
                           ↓
                      BOTTLENECKS
                           ↓
                     SCALABILITY
                           ↓
                  TECHNOLOGY CHOICES
                           ↓
                    IMPLEMENTATION

This is the reusable formula.
Do not start with:
"Which Rust crate should I use?"
Start with:
"What is the system responsible for?"
Then:
"What data moves through it?"
Then:
"Who owns that data?"
Then:
"What can fail?"
Then:
"What resource becomes the bottleneck?"
Then:
"Can that bottleneck scale independently?"
Only after answering those questions should the Rust implementation be designed.

47. Final Architectural Principle
The most important design decision is not LiveKit, Rust, PostgreSQL, FFmpeg, GStreamer, or the AI model.
It is this:
               CAMERA
                   │
                   ▼
             INGEST ONCE
                   │
                   ▼
          DISTRIBUTE MEDIA
           /       |       \
          /        |        \
         ▼         ▼         ▼
    RECORD      STREAM      DETECT
      │           │           │
      ▼           ▼           ▼
   STORAGE      LIVEKIT    ANALYTICS
                              │
                              ▼
                         HEATMAP/ROI

Each branch should be independently observable, independently recoverable, and eventually independently scalable.
That gives us a system that is simple enough for 10–20 cameras today, but has an architectural path toward 100–200+ cameras tomorrow without rewriting the entire platform.
The next engineering step should therefore not be coding.
It should be to confirm the remaining camera specifications, AI/GPU requirements, concurrency targets, storage hardware, and exact p90 performance objectives with Erik, then turn those into a concrete resource-sizing and component design.



Absolutely. That changes the design in an important way: 30 days, detection rate, ROI behavior, event rules, retention, etc. should be configuration—not hardcoded constants.
I’d add this to the spec:
Configurable System Settings
All operational values should be centrally configurable and exposed through an API.
Examples:
recording.retention_days = 30
detection.sample_fps = 5
detection.confidence_threshold = 0.70
camera.reconnect_interval = 5s
recording.segment_duration = 60s

The API should support:
GET /api/settings
GET /api/settings/{key}

PUT /api/settings/{key}

Changes should be validated before being applied, and settings should persist across restarts.

Event Pub/Sub
Events should use a pub/sub architecture rather than tightly coupling producers to consumers.
                   Event Bus
                       │
        ┌──────────────┼──────────────┐
        │              │              │
        ▼              ▼              ▼
  Recording       Detection       Camera
    Events          Events         Events
        │              │              │
        └──────────────┼──────────────┘
                       │
                 Event Subscribers
              ┌────────┼────────┐
              ▼        ▼        ▼
           Analytics  Alerts   API/UI

For example:
person.detected
person.entered_roi
person.exited_roi
camera.online
camera.offline
recording.started
recording.completed
storage.warning
storage.critical

The important part is that events are configurable.
For example, an administrator could configure:
WHEN:
    person.entered_roi

WHERE:
    camera = camera-03
    roi = restricted-area

CONDITION:
    confidence > 0.80

THEN:
    publish event
    → dashboard notification
    → webhook
    → future alert system

This means we don't hardcode business logic such as:
if person enters ROI:
    send_alert()

Instead:
Detection
    ↓
Event
    ↓
Event Bus
    ↓
Configurable Rules
    ↓
Actions/Subscribers

That's a much better long-term architecture because new event consumers can be added without modifying the detection pipeline.
One architectural addition I'd make
I'd introduce a dedicated Configuration + Event layer:
                        ┌─────────────────┐
                         │ Configuration   │
                         │ API             │
                         └────────┬────────┘
                                  │
                                  ▼
                         ┌─────────────────┐
                         │ Config Store    │
                         └────────┬────────┘
                                  │
                                  ▼
Camera → Ingestion → Media Distribution
                         │
             ┌───────────┼────────────┐
             ▼           ▼            ▼
         Recorder     LiveKit      Detection
                                      │
                                      ▼
                                  Event Bus
                                      │
                         ┌────────────┼────────────┐
                         ▼            ▼            ▼
                     Analytics      Rules       Dashboard
                         │            │
                         ▼            ▼
                     Heatmaps      Actions

And I'd explicitly state:
No operational value that is expected to change should be hardcoded into the core pipeline.
The code should provide defaults, but deployment-specific values should come from configuration.
That includes the 30-day retention period. It should be a default such as 30, but Erik/admin should be able to change it through the API without modifying or recompiling the application.
For the event bus itself, we can keep the MVP simple—an in-process bounded pub/sub system inside the Rust application for 10–20 cameras. If we later scale to multiple nodes, the same event interface can be backed by an external broker without redesigning every producer/consumer.

