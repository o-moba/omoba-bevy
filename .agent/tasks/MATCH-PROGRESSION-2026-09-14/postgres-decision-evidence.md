# PostgreSQL decision evidence

The design decision criteria DB1–DB5 in spec-postgres-addendum.md pass. This is
completion of the database selection/design only; the original feature AC1–AC7
remain unfinished.

Reviewed official SQLite deployment guidance and PostgreSQL MVCC, row locking,
transaction isolation, conflict handling, replication and PITR documentation.
Sources are linked at the relevant claims in docs/match-progression.md. A separate
agent reviewed multi-server allocation, recovery and settlement without edits.

Updated docs/match-progression.md; marked server/migrations/README.md and
001_career.sql as the superseded SQLite prototype. The SQL change is comments
only; it is not a PostgreSQL migration. git diff --check passed. No Rust code,
Cargo manifests, credentials or infrastructure changed; no new test/build run
was needed for this documentation-only decision. No PostgreSQL verification or
capacity measurement has been performed.

The rusqlite proposal is withdrawn. A PostgreSQL Rust adapter and cryptographic
identity integration still need their concrete dependency change under the user's
existing approval rule. Main was not changed, and no commit or deployment was
made in this decision turn.
