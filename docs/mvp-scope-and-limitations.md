# MVP scope, limitations, and non-goals

This page states what the **current MVP build** is intended to cover. It complements the live feature list in [features.md](features.md).

## In scope for this MVP documentation

- Local and same-LAN-style testing via documented `Makefile` targets and env vars.
- Human playtest procedure ([playtest-script.md](playtest-script.md)) and comparable bug reports ([bug-report-template.md](bug-report-template.md)).
- Explicit separation of **MVP-blocking** versus **deferrable** gaps ([tasks/MVP-CHECKLIST.md](../tasks/MVP-CHECKLIST.md)).

## Known limitations (explicit)

- **Reconnect identity:** Bevy clients persist a local `client_session_id`, and the server can reclaim a timed-out player slot/id from a new UDP endpoint within the reclaim window. Legacy clients without a session id still reconnect as new players.
- **Server restart:** Clients that stay open after a server restart need to send `Join` again for team/character; behavior is documented, not seamless reconnect.
- **Player timeout:** Idle clients are dropped from snapshots after server `PLAYER_TIMEOUT` (~5s); this is intentional for this version.
- **Identity security:** The local session id is a reclaim token for playtest continuity, not account auth or cryptographic anti-cheat.
- **Operations:** No production SLOs, on-call runbooks, or hosted infrastructure; documentation targets developers and internal testers on their own machines.
- **Balance and content:** Tuning, full skill UX, and release-scale QA are ongoing; the checklist marks what blocks external MVP versus backlog.

## Non-goals (not promised in this MVP)

- Full production deployment, auth, matchmaking at scale, or anti-cheat.
- Exhaustive API or design documentation beyond what testers need to run and report.
- Replacing automated tests; scripts and `cargo test` complement manual playtest.

## Contradictions

If `README.md`, `RUNBOOK.md`, or this file disagrees with **observed behavior**, treat the **code and Makefile** as the temporary source of truth and file an **MVP-blocking** doc bug.
