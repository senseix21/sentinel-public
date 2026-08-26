# SENTINEL system overview — presentation outline

Companion notes for `SENTINEL-system-overview.pptx`, generated 2026-08-26.

## Audience and intent

This is a product-and-technical overview for stakeholders, operators, and engineers. It distinguishes implemented behavior from architecture targets and unresolved product or deployment gates.

## Slide outline

1. SENTINEL — title and technology frame.
2. What SENTINEL is — value proposition and scale targets.
3. End-to-end architecture — separate media and control planes.
4. Components and singular ownership — crate/system boundaries.
5. Camera ingest — lifecycle, one connection, reconnect and isolation.
6. Recording — fragmented MP4, actual metadata, PostgreSQL, playback and retention.
7. Live viewing — viewer-gated transcode through LiveKit/WebRTC and measured limits.
8. Detection and events — bounded freshness-first pipeline; fake inference limitation.
9. Operator surface and security — proxy trust, authorization, secrets, audit.
10. Failure model and observability — subsystem degradation behavior and metrics.
11. What works today — delivered capabilities only.
12. What is missing — product, deployment, qualification, and systems gaps.
13. Scale strategy — Phase 9 evidence-led qualification.
14. Recommended milestones — path from working implementation to deployable product.

## Primary repository sources

- `README.md` — current product description, architecture, capabilities, operational instructions.
- `AGENTS.md` — architecture invariants, ownership, evidence standards, and delivery rules.
- `ROADMAP.md` — authoritative item-by-item status and open gates.
- `analysis.md` — target architecture proposal.
- `docs/adr/001-media-pipeline.md` through `docs/adr/012-viewer-gated-transcode-publication.md` — accepted design decisions.
- `context/2026-08-19-2300/repository-status.md` — latest repository status snapshot found when the deck was created.
- `docs/phase-9/live-publication-qualification-2026-08-18.md` and `docs/phase-9/livekit-low-latency-2026-08-19.md` — live-view qualification evidence.

## Claim discipline

- “Working” means implemented in the workspace and supported by documented local tests or fixtures.
- The detection systems seam is working; production human detection is not.
- Architecture is designed for 10–20 cameras, but capacity is not claimed until the Phase 9 workload runs on target hardware.
- Live frame accounting is qualified for controlled local workloads; camera-to-glass latency is not.
- ROI, rules, analytics, packaging, and several deployment procedures are intentionally shown as gaps.

## Regeneration

From the repository root:

```bash
python3 docs/presentation/build_sentinel_deck.py
```

The generator imports `python-pptx` from `.local/presentation-tools`.
