# Next Actions — 2026-10-09

The local `make run` default now starts real inference and opens the dashboard.
The stack was left running for the user; stop it before launching another instance.

Follow the still-open product and detection work in
`context/2026-10-05-0000/next-actions.md`: live thresholds, per-camera ROI editing,
representative accuracy evaluation, recovery qualification, and target hardware.

No core Rust architecture changed in this launcher slice. Prior architecture
and decisions remain in `context/2026-10-04-2021/`; the October 5 snapshot takes
precedence for threshold application classification.
