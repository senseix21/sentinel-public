

# RTSP Stream Recorder & Clip Extractor with Stream Forwarding

This service connects to an RTSP stream and records it to disk with timestamped filenames. It also provides an API to extract exact time-based clips from the recorded content and supports real-time stream forwarding to external MediaMTX servers.

## Stream Forwarding Overview

The service includes a robust stream forwarding feature that uses FFmpeg to transcode and forward the RTSP stream to an external MediaMTX server. This enables you to:

- **Distribute streams** to multiple MediaMTX servers
- **Transcode streams** with optimized settings for low latency
- **Monitor forwarding** status and control it via API
- **Configure targets** dynamically without restarting the service

## Stream Forwarding Technology

The forwarder uses H.265 (HEVC) encoding for efficient stream delivery:

```bash
ffmpeg -rtsp_transport tcp -i rtsp://139.162.13.249:8554/mystream \
        -c:v libx265 -preset ultrafast -x265-params tune=zerolatency \
        -c:a aac \
        -f rtsp rtsp://192.168.1.157:8554/mystream
```

**Key Features:**
- **H.265 Encoding**: More efficient compression than H.264 (HEVC)
- **Low Latency**: `ultrafast` preset with `tune=zerolatency`
- **TCP Transport**: Ensures reliable RTSP streaming
- **AAC Audio**: High-quality audio compression
- **Process Management**: Automatic FFmpeg process lifecycle control

**Note**: Reconnection options are omitted for FFmpeg compatibility. The service handles reconnection automatically.

### H.265 vs H.264
- **H.265 (HEVC)**: ~50% better compression, better quality at same bitrate
- **H.264**: Wider compatibility, faster encoding, lower CPU usage
- Use H.265 when bandwidth efficiency is critical, H.264 for broader compatibility

## Setup

### Prerequisites (macOS)
```bash
brew install gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-libav ffmpeg
```

### Starting the Service
Run the command below in the workspace root to start the cam_rtsp server:
```bash
cargo run --bin cam_rtsp -- --stream-url "rtsp://139.162.13.249:8554/mystream" --recordings-dir "./recordings"
```

### Starting with Stream Forwarding
To enable stream forwarding to an external MediaMTX server:
```bash
# Basic forwarding (start via API)
cargo run --bin cam_rtsp -- \
  --stream-url "rtsp://139.162.13.249:8554/mystream" \
  --recordings-dir "./recordings" \
  --enable-forwarding \
  --forward-target-url "rtsp://target-server:8554/forwarded_stream"

# Auto-start forwarding on server startup
cargo run --bin cam_rtsp -- \
  --stream-url "rtsp://139.162.13.249:8554/mystream" \
  --recordings-dir "./recordings" \
  --enable-forwarding \
  --auto-start-forwarding \
  --forward-target-url "rtsp://target-server:8554/forwarded_stream"
```

**Stream Forwarding Configuration Options:**
- `--enable-forwarding`: Enable stream forwarding (default: false)
- `--auto-start-forwarding`: Automatically start stream forwarding on server startup (requires `--enable-forwarding`)
- `--forward-target-url`: Target MediaMTX RTSP URL for forwarding
- `--forward-auth`: Authentication for target server (optional)

### Configuration Examples

#### Basic Forwarding
```bash
cargo run --bin cam_rtsp -- \
  --stream-url "rtsp://source-server:8554/camera1" \
  --recordings-dir "./recordings" \
  --enable-forwarding \
  --forward-target-url "rtsp://192.168.1.100:8554/forwarded_camera"
```

#### Auto-Start Forwarding on Server Startup
```bash
cargo run --bin cam_rtsp -- \
  --stream-url "rtsp://139.162.13.249:8554/mystream" \
  --recordings-dir "./recordings" \
  --enable-forwarding \
  --auto-start-forwarding \
  --forward-target-url "rtsp://192.168.1.157:8554/forwarded_stream"
```

#### H.265 Encoding (High Efficiency)
```bash
cargo run --bin cam_rtsp -- \
  --stream-url "rtsp://139.162.13.249:8554/mystream" \
  --recordings-dir "./recordings" \
  --enable-forwarding \
  --auto-start-forwarding \
  --forward-target-url "rtsp://192.168.1.157:8554/h265_stream"
```

#### Environment Variable Configuration
```bash
export RECORDINGS_DIR="./recordings"
export STREAM_URL="rtsp://139.162.13.249:8554/mystream"
export ENABLE_FORWARDING="true"
export FORWARD_TARGET_URL="rtsp://192.168.1.157:8554/mystream"

cargo run --bin cam_rtsp --
```

#### Dynamic Configuration via API
Start with basic configuration, then use API endpoints to update forwarding targets:
```bash
# Start service
cargo run --bin cam_rtsp -- --stream-url "rtsp://source:8554/stream"

# Set forwarding target via API
curl -X POST "http://localhost:3000/forwarding/config" \
  -H "Content-Type: application/json" \
  -d '{
    "target_url": "rtsp://server1:8554/stream",
    "source_stream": "rtsp://source:8554/stream",
    "enabled": true
  }'
```

### Environment Variables
You can also configure using environment variables:
```bash
export RECORDINGS_DIR="../recordings"
export FFMPEG_PATH="ffmpeg"
export FFPROBE_PATH="ffprobe"
export ENABLE_FORWARDING="true"
export FORWARD_TARGET_URL="rtsp://target-server:8554/stream"
export FORWARD_AUTH=""
```

## API Endpoints

### Stream Forwarding Control

#### Get Forwarding Status
Check current stream forwarding status.

**Full curl request:**
```bash
curl -X GET "http://localhost:3000/forwarding/status" \
  -H "Content-Type: application/json"
```

**Response:**
```json
{
  "success": true,
  "forwarding": true,
  "target_url": "rtsp://target-server:8554/forwarded_stream",
  "timestamp": "2025-11-18T18:51:26.669028Z"
}
```

#### Start Forwarding
Start streaming to external MediaMTX server.

**Full curl request:**
```bash
curl -X POST "http://localhost:3000/forwarding/start" \
  -H "Content-Type: application/json"
```

**Response:**
```json
{
  "success": true,
  "message": "Stream forwarding started",
  "timestamp": "2025-11-18T18:51:26.669028Z"
}
```

#### Stop Forwarding
Stop streaming to external MediaMTX server.

**Full curl request:**
```bash
curl -X POST "http://localhost:3000/forwarding/stop" \
  -H "Content-Type: application/json"
```

**Response:**
```json
{
  "success": true,
  "message": "Stream forwarding stopped",
  "timestamp": "2025-11-18T18:51:26.669028Z"
}
```

#### Get Forwarding Configuration
Get current forwarding configuration.

**Full curl request:**
```bash
curl -X GET "http://localhost:3000/forwarding/config" \
  -H "Content-Type: application/json"
```

**Response:**
```json
{
  "success": true,
  "config": {
    "target_url": "rtsp://target-server:8554/forwarded_stream",
    "target_auth": "",
    "source_stream": "rtsp://139.162.13.249:8554/mystream",
    "enabled": true
  }
}
```

#### Set Forwarding Configuration
Update forwarding configuration.

**Full curl request:**
```bash
curl -X POST "http://localhost:3000/forwarding/config" \
  -H "Content-Type: application/json" \
  -d '{
    "target_url": "rtsp://new-target:8554/new_stream",
    "target_auth": "",
    "source_stream": "rtsp://139.162.13.249:8554/mystream",
    "enabled": true
  }'
```

**Request Body:**
```json
{
  "target_url": "rtsp://new-target:8554/new_stream",
  "target_auth": "",
  "source_stream": "rtsp://139.162.13.249:8554/mystream",
  "enabled": true
}
```

**Response:**
```json
{
  "success": true,
  "message": "Stream forwarding configuration updated",
  "config": {
    "target_url": "rtsp://new-target:8554/new_stream",
    "target_auth": "",
    "source_stream": "rtsp://139.162.13.249:8554/mystream",
    "enabled": true
  },
  "timestamp": "2025-11-18T18:51:26.669028Z"
}
```

### Extract Clip
Extract a clip based on start and end time ranges.

**Endpoint:** `POST /clips?start_time=YYYY-MM-DDTHH:MM:SSZ&end_time=YYYY-MM-DDTHH:MM:SSZ`

**Parameters:**
- `start_time`: Start time in ISO 8601 format (UTC)
- `end_time`: End time in ISO 8601 format (UTC)
- `output_path` (optional): Custom output path for the clip

**Response:**
```json
{
  "success": true,
  "clip": {
    "clip_id": "uuid",
    "clip_path": "path/to/clip.mkv",
    "duration_seconds": 120.0,
    "source_recordings": ["direct_file_lookup"],
    "created_at": "2025-11-18T18:51:26.669028Z"
  }
}
```

## Usage Examples

### Stream Forwarding Examples

#### Example 1: Basic Stream Forwarding Setup
Start service with forwarding to a single external server:

```bash
cargo run --bin cam_rtsp -- \
  --stream-url "rtsp://139.162.13.249:8554/mystream" \
  --recordings-dir "./recordings" \
  --enable-forwarding \
  --forward-target-url "rtsp://192.168.1.157:8554/mystream"
```

#### Example 2: Control Forwarding via API
Start service, then control forwarding programmatically:

```bash
# Start service without forwarding
cargo run --bin cam_rtsp -- --stream-url "rtsp://source:8554/stream"

# Enable forwarding via API
curl -X POST "http://localhost:3000/forwarding/start"

# Check status
curl -s "http://localhost:3000/forwarding/status" | jq .

# Disable forwarding when needed
curl -X POST "http://localhost:3000/forwarding/stop"
```

#### Example 3: Dynamic Target Configuration
Update forwarding targets without restarting:

```bash
# Update forwarding target
curl -X POST "http://localhost:3000/forwarding/config" \
  -H "Content-Type: application/json" \
  -d '{
    "target_url": "rtsp://new-server:8554/backup_stream",
    "enabled": true
  }'

# Verify configuration
curl -s "http://localhost:3000/forwarding/config" | jq .
```

#### Example 4: Monitor Stream Forwarding
Set up monitoring and alerts:

```bash
#!/bin/bash
# monitoring_script.sh

# Check forwarding status every 30 seconds
while true; do
  STATUS=$(curl -s "http://localhost:3000/forwarding/status" | jq -r '.forwarding')
  TIMESTAMP=$(date)

  if [ "$STATUS" = "true" ]; then
    echo "[$TIMESTAMP] Stream forwarding is active"
  else
    echo "[$TIMESTAMP] Stream forwarding is STOPPED - alert!"
    # Send alert or take action
  fi

  sleep 30
done
```

### Clip Extraction Examples

#### Example 1: Extract last 3 seconds from a 24-second file
Extract from the 21st to 24th second of `stream_20251118_165456.mkv`:

```bash
curl -s -X POST "http://localhost:3000/clips?start_time=2025-11-18T16:55:17Z&end_time=2025-11-18T16:55:20Z" | jq .
```

**What this does:**
- Target file: `stream_20251118_165456.mkv` (starts at 16:54:56, 24 seconds long)
- Start time: 16:55:17 (21 seconds into the file)
- End time: 16:55:20 (24 seconds into the file)
- Result: Extracts exactly 3 seconds from the end of the file

#### Example 2: Extract middle 30 seconds from a file
Extract from the 10th to 40th second of `stream_20251119_140157.mkv`:

```bash
curl -s -X POST "http://localhost:3000/clips?start_time=2025-11-19T14:02:07Z&end_time=2025-11-19T14:02:37Z" | jq .
```

**What this does:**
- Target file: `stream_20251119_140157.mkv` (starts at 14:01:57)
- Start time: 14:02:07 (10 seconds into the file)
- End time: 14:02:37 (40 seconds into the file)
- Result: Extracts exactly 30 seconds from the middle of the file

#### Example 3: Extract first 50 seconds from a file
Extract the first 50 seconds from `stream_20251119_140157.mkv`:

```bash
curl -s -X POST "http://localhost:3000/clips?start_time=2025-11-19T14:01:57Z&end_time=2025-11-19T14:02:47Z" | jq .
```

**What this does:**
- Target file: `stream_20251119_140157.mkv` (starts at 14:01:57)
- Start time: 14:01:57 (beginning of the file)
- End time: 14:02:47 (50 seconds later)
- Result: Extracts exactly the first 50 seconds of the file

## How It Works

### Stream Forwarding Implementation

The stream forwarder uses a robust FFmpeg-based approach that ensures reliable stream distribution:

1. **Process Spawning**: Creates FFmpeg subprocess with proven command parameters
2. **Pipeline Management**: Handles FFmpeg process lifecycle (start/stop/monitor)
3. **Error Handling**: Graceful error handling and process recovery
4. **Configuration**: Supports dynamic target URL and authentication updates

**FFmpeg Command Structure:**
```bash
ffmpeg -rtsp_transport tcp -i source_stream \
        -c:v libx265 -preset ultrafast -x265-params tune=zerolatency \
        -c:a aac \
        -f rtsp target_stream
```

**Key Parameters Explained:**
- `-rtsp_transport tcp`: Use TCP transport for reliable RTSP streaming
- `-c:v libx265`: Use H.265 video codec
- `-preset ultrafast`: Fastest encoding for low latency
- `-x265-params tune=zerolatency`: Optimize for zero latency
- `-c:a aac`: AAC audio codec
- `-f rtsp`: Output to RTSP format

**Note**: Reconnection parameters are omitted for FFmpeg compatibility. The service handles process management and restarts internally.

### File Discovery Algorithm (for Clip Extraction)
1. **Find Closest Recording**: When extracting a clip, the system finds the most recent recording file that contains the start time (within 1 hour before the target time)
2. **Duration Detection**: Uses `ffprobe` to get the actual file duration
3. **Offset Calculation**: Calculates start and end offsets based on the file's timestamp
4. **Boundary Clamping**: Ensures extraction doesn't exceed the actual file duration
5. **Lossless Extraction**: Uses `ffmpeg` with `-c copy` for perfect quality without re-encoding

### File Naming Convention
**Recorded Files:** `stream_YYYYMMDD_HHMMSS.mkv`
- Example: `stream_20251118_165456.mkv` was recorded on November 18, 2025 at 16:54:56

**Clip Output:** `clip_YYYYMMDD_HHMMSS.mkv`
- Example: `clip_20251118_185126.mkv` was created on November 18, 2025 at 18:51:26

### Architecture Overview

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   RTSP Source   │───▶│  cam_rtsp Service │───▶│ External MediaMTX│
│ 139.162.13.249 │    │   (Recorder +   │    │   192.168.1.157  │
│     :8554       │    │  Forwarder)     │    │     :8554       │
└─────────────────┘    └─────────────────┘    └─────────────────┘
                               │
                               ▼
                        ┌─────────────────┐
                        │ Local Disk      │
                        │ Recordings     │
                        │ ./recordings/  │
                        └─────────────────┘
```

### Key Components

1. **Stream Receiver**: Handles incoming RTSP stream and recording
2. **Stream Forwarder**: Manages FFmpeg process for external distribution
3. **Clip Extractor**: Provides time-based clip extraction functionality
4. **API Server**: RESTful API for service control and monitoring

### Service Capabilities

✅ **Recording**: Continuous recording to disk with timestamps
✅ **Forwarding**: Real-time stream distribution to external servers
✅ **Clip Extraction**: Precise time-based clip generation
✅ **API Control**: Full HTTP API for programmatic control
✅ **Configuration**: Multiple configuration methods (CLI, env vars, API)
✅ **Monitoring**: Real-time status and health monitoring

## Other Endpoints

### List Recordings
Get a list of all recorded files:
```bash
curl -s "http://localhost:3000/recordings" | jq .
```

### Status
Check if the service is running:
```bash
curl -s "http://localhost:3000/status" | jq .
```
