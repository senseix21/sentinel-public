# H.265 Encoding Configuration Guide

This guide explains how to configure H.265 encoding parameters for optimal streaming performance, file size, and quality.

## Overview

The H.265 encoder provides significant file size reduction (~50% compared to H.264) while maintaining quality. Use these parameters to fine-tune your streaming configuration:

```bash
cargo run --bin cam_rtsp -- \
    --enable-h265 \
    --h265-preset <preset> \
    --h265-crf <crf> \
    --frame-rate <fps> \
    --force-frame-rate [true|false]
```

## Parameter Effects

### Frame Rate (`--frame-rate` and `--force-frame-rate`)

**What it controls**: Number of frames per second in the output stream.

**Impact**:
- **Speed**: Higher FPS = more data to process = higher CPU usage
- **File Size**: Higher FPS = larger files (60fps file is ~2x larger than 30fps)
- **Quality**: Higher FPS = smoother motion, better for fast action

**Format**: FFprobe displays FPS as fraction (e.g., `60/1` = 60 FPS, `30/1` = 30 FPS)

**Example Usage**:
```bash
# Standard smooth video
--frame-rate 60

# Balanced motion
--frame-rate 30

# Basic motion
--frame-rate 15

# Minimal motion
--frame-rate 10
```

### H.265 Preset (`--h265-preset`)

**What it controls**: Encoding speed vs file size tradeoff. Lower presets = faster encoding but larger files.

**Impact**:
- **Encoding Speed**: ultrafast > veryfast > slow > veryslow
- **File Size**: ultrafast > veryfast > slow > veryslow
- **Quality**: Generally consistent, but faster presets may have minor quality loss

**Available Presets** (fastest to slowest):
- **ultrafast**: Fastest encoding, largest files, lowest quality
- **superfast**: Fast encoding, large files
- **veryfast**: Good balance (recommended default)
- **faster**: Slower encoding, smaller files
- **fast**: Slow encoding, smaller files
- **medium**: Very slow encoding, small files
- **slow**: Extremely slow encoding, tiny files
- **slower**: Insanely slow encoding, minuscule files
- **veryslow**: Pathologically slow encoding, microscopic files

**Example Usage**:
```bash
# Best for low-end systems
--h265-preset ultrafast

# Good balance (default)
--h265-preset veryfast

# Best quality/size ratio
--h265-preset slow
```

### CRF (`--h265-crf`)

**What it controls**: Constant Rate Factor - quality vs file size tradeoff. Lower CRF = better quality but larger files.

**Impact**:
- **Quality**: Lower CRF = better quality
- **File Size**: Lower CRF = larger files
- **Encoding Speed**: Minimal impact compared to preset

**Range**: 0-51 (practical range: 18-35)

**Quality Guide**:
- **18-22**: Excellent quality (large files)
- **23-27**: Good quality (recommended range)
- **28-32**: Acceptable quality (default: 28)
- **33-35**: Lower quality (small files)
- **35+**: Poor quality (very small files)

**Example Usage**:
```bash
# Maximum quality
--h265-crf 22

# High quality
--h265-crf 25

# Good quality (default)
--h265-crf 28

# Smaller files
--h265-crf 32

# Minimum acceptable quality
--h265-crf 35
```

## Configuration Scenarios

### Scenario 1: Low Quality/Small Size
```bash
--h265-preset ultrafast --h265-crf 35 --frame-rate 15
```
**Use Case**: Bandwidth-constrained environments
**Pros**: Fast encoding, small files
**Cons**: Lower quality, choppy motion

### Scenario 2: Balanced/Default (Recommended)
```bash
--h265-preset veryfast --h265-crf 28 --frame-rate 30
```
**Use Case**: General purpose streaming
**Pros**: Good speed/quality balance, moderate file sizes
**Cons**: Neither the fastest nor smallest files

### Scenario 3: High Quality/Large Size
```bash
--h265-preset slow --h265-crf 22 --frame-rate 60
```
**Use Case**: High-quality recording, fast action content
**Pros**: Excellent quality, smooth motion
**Cons**: Slow encoding, large files

## Working Example: Production Startup Command

### Full Production Command with H.265 Encoding
```bash
cargo run --bin cam_rtsp -- \
    --stream-url "rtsp://139.162.13.249:8554/mystream" \
    --recordings-dir "./recordings" \
    --enable-forwarding \
    --auto-start-forwarding \
    --forward-target-url "rtsp://192.168.1.157:8554/mystream" \
    --enable-h265 \
    --h265-preset ultrafast \
    --h265-crf 30 \
    --frame-rate 60.0 \
    --force-frame-rate \
    --log-level debug
```

### Parameter Breakdown:
| Parameter | Value | Purpose |
|-----------|--------|---------|
| `--stream-url` | `rtsp://139.162.13.249:8554/mystream` | Source RTSP stream to receive |
| `--recordings-dir` | `./recordings` | Directory to store recorded clips |
| `--enable-forwarding` | (flag) | Enable stream forwarding functionality |
| `--auto-start-forwarding` | (flag) | Automatically start forwarding on startup |
| `--forward-target-url` | `rtsp://192.168.1.157:8554/mystream` | Target RTSP server to forward to |
| `--enable-h265` | (flag) | Enable H.265 re-encoding before forwarding |
| `--h265-preset` | `ultrafast` | Fastest encoding for real-time performance |
| `--h265-crf` | `30` | Good quality with moderate file size |
| `--frame-rate` | `60.0` | 60 FPS for smooth motion |
| `--force-frame-rate` | (flag) | Override source frame rate with 60 FPS |
| `--log-level` | `debug` | Detailed logging for troubleshooting |

### Expected Behavior:
1. **Stream Reception**: Connects to source RTSP server at 139.162.13.249:8554
2. **H.265 Re-encoding**: Converts incoming stream to H.265 with ultrafast preset
3. **Frame Rate Control**: Forces output to 60 FPS regardless of source frame rate
4. **Auto-Forwarding**: Immediately starts forwarding to target server at 192.168.1.157:8554
5. **Recording**: Saves clips to ./recordings directory
6. **Debug Logging**: Provides detailed operational logs

### Verification Commands:
```bash
# Check if H.265 encoding is working (should show "hevc")
ffprobe -rtsp_transport tcp -v error -print_format json -show_streams "rtsp://192.168.1.157:8554/mystream" | jq '.streams[] | select(.codec_type=="video") | {codec: .codec_name}'

# Check frame rate (should show "60/1")
ffprobe -rtsp_transport tcp -v error -print_format json -show_streams "rtsp://192.168.1.157:8554/mystream" | jq '.streams[] | select(.codec_type=="video") | {fps: .r_frame_rate}'

# Monitor logs for H.265 confirmation
tail -f /path/to/your/log/file.log | grep "H.265"
```

### Target Stream Access:
The forwarded stream will be available at:
```
rtsp://192.168.1.157:8554/mystream
```

This stream will be:
- **Encoded**: H.265 (HEVC) format
- **Framerate**: 60 FPS as configured
- **Quality**: CRF 30 (good quality with moderate file size)
- **Preset**: Ultrafast (optimized for real-time performance)

## Testing Configuration

### 1. Check Stream Codec and FPS
```bash
# Check codec and frame rate
ffprobe -rtsp_transport tcp -v error -print_format json -show_streams "rtsp://YOUR_TARGET_IP:8554/mystream" | jq '.streams[] | select(.codec_type=="video") | {fps: .r_frame_rate, codec: .codec_name}'

# Expected output for H.265 at 60 FPS:
{
  "fps": "60/1",
  "codec": "hevc"
}
```

### 2. Check Application Logs
Look for these confirmation messages:
```
✅ H.265 stream forwarding started successfully using ffmpeg to: rtsp://target:8554/mystream
Using H.265 encoder with configuration
H.265 Preset: veryfast, H.265 CRF: 28
```

### 3. Monitor Performance
```bash
# Monitor CPU usage during encoding
top -p $(pgrep cam_rtsp)

# Check memory usage
ps -p $(pgrep cam_rtsp) -o pid,rss,pcpu,pmem,command
```

## Recommended Starting Points

### For Live Surveillance
```bash
--enable-h265 \
--h265-preset veryfast \
--h265-crf 28 \
--frame-rate 30
```

### For High-Quality Recording
```bash
--enable-h265 \
--h265-preset slow \
--h265-crf 22 \
--frame-rate 60
```

### For Bandwidth-Constrained Networks
```bash
--enable-h265 \
--h265-preset ultrafast \
--h265-crf 32 \
--frame-rate 15
```

## Troubleshooting

### Issue: H.265 Not Working
- Check logs for "H.265 stream forwarding started" message
- Verify `--enable-h265` flag is set
- Confirm `ffmpeg` is installed and accessible

### Issue: Frame Rate Not Set Correctly
- Check `--frame-rate` value in logs
- Use `--force-frame-rate` to override source frame rate
- Verify with `ffprobe` output showing expected `60/1` format

### Issue: Poor Performance
- Try faster preset (`ultrafast`, `veryfast`)
- Increase CRF value (smaller files, less CPU)
- Reduce frame rate

### Issue: Large File Sizes
- Increase CRF value (higher number = smaller files)
- Use faster preset (smaller files)
- Reduce frame rate

## Key Insight

H.265 provides ~50% file size reduction compared to H.264 at the same quality level. This means you can afford slightly higher CRF values for even smaller files while maintaining acceptable quality levels.