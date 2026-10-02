# Next actions — direct-camera qualification

## Locally complete

1. Final `make check` passed after the fixture shutdown correction.
2. Review and commit the direct-camera SPEC/PLAN, harness, metric gate/tests,
   evidence template, fixture correction, roadmap, runbook, and context together.

## External qualification sequence

1. Supply the Stage 0 inputs in `docs/direct-camera-rtsp/PLAN.md` without
   committing an endpoint or secret.
2. Build the release server and run `make direct-camera-qualification` against
   one authorized H.264 physical camera.
3. Complete recording, reconnect, credential, graceful/forced shutdown,
   detection, and LiveKit isolation stages for every supported profile.
4. Execute the approved sustained soak on target hardware.
5. Run the exact 10-camera workload, then measure 20 cameras or document the
   limiting resource.

Do not mark the Phase 9 direct-camera item complete from the synthetic fixtures
or from the existence of the harness.
