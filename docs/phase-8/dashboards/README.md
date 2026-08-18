# Grafana Dashboards

P8.7. Three dashboards, importable as Grafana JSON.

| File | Answers |
|---|---|
| `fleet-overview.json` | "Is the system healthy right now, and if not, which camera?" |
| `camera-detail.json` | "What is wrong with *this* camera?" (templated by `camera_id`) |
| `security.json` | "Who is authenticating, what is being refused, is anyone probing?" |

## Status

**Written, not verified.** No Grafana is available in this environment, so no
panel here has been rendered. What *has* been checked is the part that silently
breaks: every metric name and label below was taken from the actual output of a
running server (`curl /metrics`), not from memory or from the source.

That distinction matters because the first draft of `../alerts.yml` invented
seven metric names, and a panel or alert on a series that does not exist is
indistinguishable from one that is quiet because everything is fine.

Treat "the panels render and are readable" as unproven until someone imports
them.

## Import

```bash
# Prometheus scrape target: the server's metrics bind, loopback by default.
#   - job_name: surveillance
#     static_configs: [{ targets: ["127.0.0.1:3000"] }]

# Grafana → Dashboards → Import → Upload JSON.
# Each dashboard expects a Prometheus datasource selected at import time.
```

## Gauge encodings

Several gauges are numeric rather than labelled. The dashboards use
`value_mappings` so a reader sees a word, not an integer.

| Metric | Values |
|---|---|
| `surveillance_camera_pipeline_state` | 0 Configured, 1 Starting, 2 Running, 3 Stopping, 4 Stopped, 5 Degraded, 6 Terminal |
| `surveillance_camera_connected` | 0, 1 |
| `surveillance_storage_pressure` | 0 Normal, 1 Warning, 2 Critical, 3 Emergency |
| `surveillance_configuration_freshness` | 0 Fresh, 1 Stale |
| `surveillance_livekit_state` | 0 Disabled, 1 Starting, 2 Connecting, 3 Publishing, 4 Backoff, 5 Degraded, 6 Terminal, 7 Stopping, 8 Stopped |
| `surveillance_configuration_store_available` | 0, 1 |

## What is deliberately not on a dashboard

- **Anything keyed by principal or token.** Those are not metric labels, by
  design — an identity in a metric is an identity in every scrape and every
  screenshot. Use the `access_audit` table for per-principal questions.
- **Detection accuracy.** `DeterministicFakeInference` is the only
  implementation; there is no accuracy to show and a panel would imply one.
- **Capacity headroom.** No camera-count claim is supported by measurement
  (`AGENTS.md`), so no panel projects one.
