# Database migrations

Migrations are forward-only and owned by `crates/database`. Apply them in
lexicographic order. `scripts/phase4/postgres-fixture.sh` proves application from
an empty PostgreSQL database and idempotent migration-runner bookkeeping without
requiring a checked-in credential.
