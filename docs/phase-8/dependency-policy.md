# Dependency and System-Library Policy

Phase 8, P8.10. Roadmap instruction 10.

## Rust dependencies

Two tools, in `make check` via `make audit`:

| Tool | Reads | Catches |
|---|---|---|
| `cargo deny check` | the resolved dependency **graph**, `all-features = true` | advisories, licences, banned crates, unknown sources |
| `cargo audit` | `Cargo.lock` | advisories on any locked entry, built or not |

Both are run. They disagree usefully: `deny` is accurate about what ships,
`audit` is a broader net over what is pinned. A finding in `audit` that `deny`
does not see means the crate is locked but not built — worth knowing, not worth
failing on, provided the non-reachability is *verified* rather than assumed.

Policy is in `deny.toml` and `.cargo/audit.toml`. Both carry a reason per entry.

### Current exceptions

**RUSTSEC-2023-0071** — Marvin attack in `rsa` 0.9.10, no fixed release exists.

Reaches `Cargo.lock` through `sqlx-mysql`, an optional `sqlx` dependency. This
workspace pins `sqlx` with `default-features = false` and only the `postgres`
driver, so `sqlx-mysql` is never compiled.

Verified rather than asserted:

```
$ cargo tree -i rsa --target all
warning: nothing to print.

$ cargo deny check advisories
advisories ok
```

Deleted — not updated — when a fixed `rsa` ships. Re-examined immediately if
`cargo tree -i rsa` ever returns a path, or if any MySQL driver is enabled.

**RUSTSEC-2025-0119** — `number_prefix` unmaintained, via `indicatif` via
`muxide`. A warning, not a vulnerability. `unmaintained = "workspace"` in
`deny.toml` keeps it visible without failing the gate, because treating
"unmaintained" as fatal trains people to add blanket ignores.

### Adding a dependency

1. It must earn its place. `AGENTS.md` is explicit that speculative abstraction
   and unmeasured optimisation do not justify a dependency.
2. Pin it exactly (`=x.y.z`) in `[workspace.dependencies]`, as everything here
   is pinned.
3. `make audit` must pass without a new ignore. A new ignore needs a reason and
   a re-check condition in this document.
4. A new licence needs adding to `deny.toml`'s allowlist deliberately, with the
   same scrutiny as the dependency itself.

## System libraries

`cargo` has no visibility into these, and they are the highest-value attack
surface in the system: `docs/phase-8/threat-model.md` boundary B1 is a
compromised camera feeding arbitrary bytes into media parsing.

| Library | Used by | Exposure |
|---|---|---|
| FFmpeg (`ffmpeg`, `ffprobe`) | `crates/detector` decode, recorder metadata probe | **Highest.** Parses attacker-influenced media. Runs as an owned child process, so a crash is contained to one camera — but a memory-safety bug is still a memory-safety bug. |
| libx264 | test fixtures only | Not in the production path. |
| PostgreSQL | `crates/database` | Local to the unit; reached only by the server. |

### Patching

Because FFmpeg runs as a **separate process** rather than linked in-process, a
patch is a package upgrade and a restart of the decode children, not a rebuild
of the server. That is a deliberate property of the Phase 6 design and it is
what makes this policy cheap enough to actually follow.

1. **Weekly**: check the distribution's security feed for the target release
   and the FFmpeg security page.
2. **On a published FFmpeg advisory affecting a demuxer, decoder, or parser we
   reach** (H.264, MP4/fMP4, RTSP): patch within 7 days. Nothing else in this
   system has that exposure profile.
3. **Otherwise**: patch on the ordinary distribution cadence.
4. After patching, run `scripts/phase0/retina-rtsp-fixture.sh` and
   `cargo test -p surveillance-detector --test ffmpeg_decode -- --ignored`, and
   record the FFmpeg version in the evidence file. A patched library that
   changed probe output is a metadata-correctness problem, and this system
   derives metadata from actual media rather than defaults.

**Blocked**: the concrete distribution, package source, and upgrade mechanism
depend on the deployment target, which ADR 009 records as unconfirmed. The
cadence and trigger above are target-independent and stand; the commands to run
them are not writable yet. This is roadmap item 10's remaining gap and it is the
same blocker as P8.8.

## Fuzzing

Roadmap instruction 11. Targets, in threat-model priority order:

1. `crates/camera` RFC 6184 H.264 depayloading — parses bytes from a
   trusted-by-position peer that may be compromised. Highest value in the tree.
2. `apps/server` HTTP request-line and header parsing — the parser is
   hand-rolled and reachable pre-authentication.
3. `apps/server` recording time-range and query parsing.

Status: **not implemented.** `cargo-fuzz` requires a nightly toolchain, and this
workspace pins `rust-version = "1.88"` on stable with `unsafe_code = "forbid"`.
Introducing a nightly toolchain to the gate is a change to the build contract
that belongs in its own decision, not as a side effect of Phase 8.

What exists instead, and what it is worth: the depayloader has explicit
rejection tests for malformed, truncated, oversized, and discontinuous payloads
(Phase 0/2), and the HTTP layer has bounded-request and malformed-request tests.
Those are hand-chosen cases, not a fuzzer. The roadmap item is recorded `[~]`,
not `[x]`, and the gap is that nobody has run a coverage-guided fuzzer against
these parsers.
