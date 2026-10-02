# Direct-Camera RTSP Qualification Evidence

Status: not run

Date/time (UTC): TBD  
Operator: TBD  
Server commit/build: TBD

## Verdict

Pass, fail, or blocked: TBD.

Supported statement, if any: TBD. Name the exact vendor, model, firmware,
stream/profile, workload, hardware, and duration. Do not generalize to a vendor
family.

## Scope

- Requirements exercised: TBD.
- Requirements deliberately excluded: TBD.
- Authorized disruptions: TBD.
- Unavailable dependencies/equipment: TBD.

## Topology

```text
physical camera -> direct RTSP/TCP -> surveillance-server
                                      |-> recorder
                                      |-> detector (if enabled)
                                      `-> LiveKit (if enabled)
```

RTSP proxy absent from qualification topology: TBD.

## Inventory

### Camera

| Field | Measured value | Evidence source |
|---|---|---|
| Vendor/model | TBD | device label/admin page |
| Firmware | TBD | device admin page |
| Stream | redacted | configuration fingerprint |
| Codec/profile/level | TBD | SDP/SPS/segment probe |
| Resolution | TBD | SDP/SPS/segment probe |
| FPS | TBD | counted frames / measured duration |
| GOP | TBD | frame/keyframe probe |
| Average/peak bitrate | TBD | measured interval |
| Audio | TBD | SDP/probe |
| Authentication mode | TBD | redacted RTSP observation |

### Host and dependencies

| Field | Value |
|---|---|
| OS/kernel | TBD |
| CPU/GPU/RAM | TBD |
| Storage/filesystem | TBD |
| Network path | redacted summary |
| Rust/server version | TBD |
| FFmpeg/FFprobe | TBD |
| PostgreSQL | TBD/not used |
| LiveKit | TBD/not used |

## Workload

- Camera count and mix: TBD.
- Duration: TBD.
- Segment/fragment duration: TBD.
- Detection configuration: TBD.
- Viewer count/distribution: TBD.
- Metrics sampling interval: TBD.
- Approved objectives: TBD.

## Commands and artifacts

Direct-ingest harness invocation (do not include secrets or the private URL):

```bash
QUALIFICATION_ACK=direct-physical-camera \
QUALIFICATION_SECONDS=<seconds> \
scripts/phase9/direct-camera-qualification.sh
```

Private raw-artifact location and retention: TBD.

Committed artifacts contain no credentials, tokens, private RTSP URL, or
unredacted packet capture: TBD.

## Functional results

| Requirement | Result | Evidence |
|---|---|---|
| DC.R1 inventory | TBD | |
| DC.R2 direct connection/authentication | TBD | |
| DC.R3 truthful media | TBD | |
| DC.R4 continuous recording | TBD | |
| DC.R5 reconnect/recovery | TBD | |
| DC.R6 credential rejection/rotation | TBD | |
| DC.R7 LiveKit same-ingest publication | TBD | |
| DC.R8 detection isolation | TBD | |
| DC.R9 shutdown/restart | TBD | |
| DC.R10 security/network boundary | TBD | |
| DC.R11 realistic 10-camera workload | TBD | |

## Media and queue accounting

Record beginning/end/delta or rate as appropriate.

| Measurement | Value |
|---|---:|
| Camera access units | TBD |
| Packet-loss events/lost packets | TBD |
| Recorder received/rejected | TBD |
| Completed/failed segments | TBD |
| Segment gaps/partials | TBD |
| Detection sampled/dropped/inferred | TBD |
| Live received/evicted/stale | TBD |
| Live published/dropped/restarted | TBD |

## Resource and objective results

| Measurement | p50 | p90 | p99 | Maximum/total |
|---|---:|---:|---:|---:|
| CPU | TBD | TBD | TBD | TBD |
| RSS | TBD | TBD | TBD | TBD |
| Disk write latency/throughput | TBD | TBD | TBD | TBD |
| Network throughput/loss | TBD | TBD | TBD | TBD |
| Queue lag/depth | TBD | TBD | TBD | TBD |
| Connect/recovery time | TBD | TBD | TBD | TBD |
| Time to first picture | TBD | TBD | TBD | TBD |

## Failure and recovery timeline

| UTC time | Injection/observation | Expected | Actual | Gap/recovery |
|---|---|---|---|---|
| TBD | | | | |

## Segment truth

- Durable segments expected/observed: TBD.
- Decode result: TBD.
- Metadata/file agreement: TBD.
- Exact unexplained gaps, partials, or failures: TBD.

## Security review

- Endpoint contains no credentials/query/fragment: TBD.
- Logs, metrics, process arguments, evidence, and history are secret-free: TBD.
- Camera VLAN/firewall source/destination/protocol/port evidence: TBD.
- Browser receives no RTSP endpoint or camera credential: TBD.

## Anomalies and blockers

List every unexplained result and unavailable external input. “None” requires a
review of logs, metrics, files, and failure injections.

- TBD.

## Conclusion and next action

TBD. State the narrow claim supported, the requirements still open, and the
specific input or change needed next.
