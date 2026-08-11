# Resource-Sizing Worksheet

Status: planning model; no production capacity is proven

## Storage formula

For continuous recording with decimal network/storage units:

```text
TB = bitrate_Mbps × camera_count × retention_days × 86,400 / 8 / 1,000,000
```

At 30 days this simplifies to:

```text
TB = bitrate_Mbps × camera_count × 0.324
```

Add filesystem/container overhead, VBR peaks, incomplete cleanup, operational free-space reserve, and growth margin separately. A provisional planning multiplier of 1.2 is shown only as a sensitivity value, not a requirement.

| Per-camera bitrate | 10 cameras / 30d | With 1.2× | 20 cameras / 30d | With 1.2× | 200 cameras / 30d | With 1.2× |
|---:|---:|---:|---:|---:|---:|---:|
| 2 Mbps | 6.48 TB | 7.78 TB | 12.96 TB | 15.55 TB | 129.60 TB | 155.52 TB |
| 4 Mbps | 12.96 TB | 15.55 TB | 25.92 TB | 31.10 TB | 259.20 TB | 311.04 TB |
| 8 Mbps | 25.92 TB | 31.10 TB | 51.84 TB | 62.21 TB | 518.40 TB | 622.08 TB |
| 12 Mbps | 38.88 TB | 46.66 TB | 77.76 TB | 93.31 TB | 777.60 TB | 933.12 TB |
| 20 Mbps | 64.80 TB | 77.76 TB | 129.60 TB | 155.52 TB | 1,296.00 TB | 1,555.20 TB |

The `analysis.md` 259 TB example is mathematically correct for exactly 200 continuously recorded 4 Mbps streams, but it is only a scenario. Resolution and codec do not determine storage independently of measured bitrate.

## Network formula

```text
camera_ingress_Mbps = sum(measured_camera_bitrates)
viewer_egress_Mbps = sum(published_stream_bitrate × concurrent_subscribers)
```

Include protocol overhead, retransmission, peak VBR, management traffic, and safety margin. LiveKit viewer egress can exceed camera ingress substantially.

## Memory model

For each bounded queue:

```text
queue_memory ≈ capacity_items × p99_item_size
```

For decoded frames before alignment/pool overhead:

```text
YUV420_bytes ≈ width × height × 1.5
```

Examples: one 1920×1080 frame is about 3.11 MB; one 3840×2160 frame is about 12.44 MB. A queue of 30 uncompressed 4K frames is therefore roughly 373 MB before overhead. This is why detector queues must remain shallow and freshness-oriented.

## Compute model

Record separately:

- Encoded packet handling/remux cost.
- Each required decode path by codec/resolution/FPS.
- Detection preprocessing and inference cost by model/input size/batch.
- Required transcodes for LiveKit or playback compatibility.

Do not multiply a one-camera average and call it capacity. Run simultaneous-source tests and report percentile latency, drops, thermal throttling, disk contention, and sustained duration.

## Observed development host

On 2026-08-10 the local development machine reported an Intel MacBook Pro with 8 CPU cores, 32 GB RAM, Intel UHD 630 plus Radeon Pro 560X 4 GB, and approximately 105 GiB free on the working filesystem. This host is suitable for development spikes; it is not the confirmed production server and cannot hold the 30-day target even at the lowest table scenario.

