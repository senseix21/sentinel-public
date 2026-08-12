# Next Actions — 2026-08-11 21:44 +06

1. Add SQLx with an explicitly bounded `PgPool` and embedded migration owner in
   `database`; do not leak SQLx types into `storage`.
2. Implement and real-PostgreSQL-test camera upsert plus completed
   insert-or-confirm, including identical replay, conflicting replay,
   transaction rollback, pool timeout, cancellation, and redacted errors.
3. Only then connect an application-owned bounded metadata writer after the
   recorder lifecycle channel; PostgreSQL must remain outside recorder/file I/O.

Playback byte delivery and retention thresholds remain decision gates. Indexed
search can proceed before playback once repository persistence is proven.
