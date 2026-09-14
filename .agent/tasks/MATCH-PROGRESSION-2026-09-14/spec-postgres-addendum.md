# PostgreSQL and regional multiplayer — design addendum

Frozen before documentation edits on 2026-09-14. Extends, rather than replaces,
the original match-progression specification following the user's request to
choose storage for a worldwide MOBA with thousands of players.

- DB1: Choose the primary database from the actual persistence/concurrency needs,
  distinguish CCU from database workload, and use official sources.
- DB2: Describe a practical beta architecture with shared accounts/history and
  regional authoritative game servers. Keep storage/network I/O outside ticks.
- DB3: Specify multi-server allocation, account-wide active-seat uniqueness,
  atomic idempotent settlement and owner-aware recovery. Do not mistake a backend
  restart for the death of every active game server.
- DB4: Identify reusable prototype components and PostgreSQL-specific work.
  Explicitly supersede the SQLite choice and its validation claims for production.
- DB5: Record evidence and remaining work. This turn selects/designs the backend;
  it does not install dependencies, deploy infrastructure, change credentials or
  claim that persistence and rated matchmaking already work.

The original feature AC1–AC7 remain unfinished. PostgreSQL is the selected
production engine; the prior rusqlite/SQLite dependency proposal is superseded.
An exact Rust dependency change still follows the user's dependency-approval rule.
