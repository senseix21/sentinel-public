# Decisions and Open Questions — 2026-08-10 23:04 +06

## Decisions

- Put stable camera/recording identities in `common`.
- Put path layout and partial/durable derivation in `storage`.
- Include recording identity in filenames to avoid timestamp-only collisions.
- Validate UTC calendar components before path formatting.

## Open questions

- Recording-ID generation and persistence strategy.
- Symlink-safe directory/file creation and Unix permissions.
- File and parent-directory synchronization policy before metadata completion.
- Conversion library from authoritative UTC instants to path components.
