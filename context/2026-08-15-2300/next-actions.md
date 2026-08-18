# Next Actions — 2026-08-15 23:00 +06

Ordered by what unblocks the most.

## 1. Get the deployment target confirmed

Single highest-value question outstanding. It blocks packaging (roadmap item 8),
four runbooks (item 9), the filesystem/process permission controls the threat
model lists as unenforced, and the secrets-file ownership check.

Needed: Linux distribution and version, container or systemd unit, storage
layout, and who performs upgrades.

## 2. Install `promtool` and run the alert tests

```
promtool test rules docs/phase-8/alerts_test.yml
```

Thirteen rules with a firing and a silent case each are written and
**unvalidated**. Until this runs, "critical alerts are actionable and tested"
stays `[!]`. Cheap, and it closes an exit-gate item.

## 3. Test concurrent settings updates

Carried from Phase 7 unchanged. Needs two concurrent writers against one
database. ADR 008 forbids inferring transactional safety from code structure,
and Phase 8 touched the same store.

## 4. Have someone else follow the runbooks

`disk-full.md`, `camera-offline.md`, `database-outage.md`, and
`livekit-outage.md` were written and (three of four) exercised by their author,
who knows what they meant. That is not the standard. A second person following
them literally is what makes the operator exit-gate claim honest.

## 5. Bootstrap path for the first administrator

Currently: with a database configured but no principals, the API grants nothing
and a routable bind is refused. There is no first-admin creation path — a
deliberate gap, since the alternatives are a loopback backdoor or a default
credential, both worse. Needs a decision: a CLI subcommand writing a token to a
`0600` file is the obvious shape.

## 6. Decide on fuzzing

`cargo-fuzz` needs nightly; the gate is stable 1.88 with
`unsafe_code = "forbid"`. Targets are named and prioritised in
`docs/phase-8/dependency-policy.md` — the RFC 6184 depayloader first, since a
compromised camera is a realistic position and that parser is the most exposed
surface in the tree.

## Not next

Phase 9 (10/20-camera qualification) still needs representative cameras and
target hardware. Nothing in Phase 8 changed that.
