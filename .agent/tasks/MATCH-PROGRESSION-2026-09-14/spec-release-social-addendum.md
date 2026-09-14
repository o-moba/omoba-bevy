# Release career and social implementation — frozen 2026-09-14

The user explicitly approved implementing the PostgreSQL direction and requested end-of-match statistics, persistent profiles and friends now. This replaces the abandoned SQLite proposal and its pending dependency question. Necessary PostgreSQL and device-signature implementation libraries are within this accepted scope. No production infrastructure, deployment or secrets are changed.

- RS1: Connect authoritative round statistics to a durable PostgreSQL allocation and idempotent settlement. Show pending/failure honestly; retry outbox after restart, recover only expired owners, never double-credit.
- RS2: Persist device identity, bind accounts to verified signatures, authenticate profile mutations and social requests, reject replay/forgery, preserve Unicode names and historical loadouts.
- RS3: Implement own profile, match history/detail, rating and progression UI on desktop/mobile. Separate gameplay identity from account identity.
- RS4: Implement persistent friend requests, incoming/outgoing lists, accept/reject/cancel/remove, exact-ID addressing, friend profile inspection and online/in-game presence. Enforce self/duplicate/direction/authorization constraints.
- RS5: Integrate saved-rating queue and frozen rated rosters; preserve explicit guest/development unranked compatibility. Finished results remain until explicit action.
- RS6: Run fresh tests including a real isolated PostgreSQL database, signed protocol and native client verification. Update version/changelog/features/runbook and evidence. Report physical-device/global-load/recovery limits accurately.

Replays are expressly deferred. Basic social scope is friendship and presence; party matchmaking, chat, voice, email/password account recovery and production deployment are not implied complete. Existing acceptance criteria remain, with PostgreSQL replacing SQLite.
