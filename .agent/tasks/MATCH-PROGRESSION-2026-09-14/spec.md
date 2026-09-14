# Persistent career, match results and skill-aware matchmaking

Frozen: 2026-09-14. Task: MATCH-PROGRESSION-2026-09-14. Base: e3913fc.

## User intent

Show a useful post-game dashboard with nicknames, chosen heroes, winners/losers and damage statistics. Persist history, progress and ratings in a database. Match players of similar strength and protect beginners from highly experienced opponents.

## Acceptance criteria

- AC1: A permanent authenticated profile is independent of nickname, transient entity ID and reconnect token. The client preserves its profile identity; Unicode nickname validation is shared and the server preserves each match-time name and loadout. No secret key is sent in game packets or logs. Guest/development compatibility is explicit and cannot silently receive ranked credit.
- AC2: Server-owned lifetime round totals count actual accepted damage by target category, damage received, kills/deaths/assists and last hits using typed identities. Totals survive disconnect, snapshot loss and cosmetic event-buffer expiry. A result freezes the roster, outcome, duration, ruleset and statistics once; later hits/reset cannot alter a finished result or its winner.
- AC3: Database migrations and transactions persist completed matches, participant statistics and profile progress. Repeated finalization cannot double-credit a match. Storage failure is visible and pending results survive until acknowledged; interrupted/abandoned games do not award ranked wins. Reopening the database preserves history and rating. New production dependencies require the user's pending approval before integration.
- AC4: Rated wins/losses change server-owned matchmaking rating; permanent progression is separate from rating. Eligible full human-controlled authenticated release rosters on the approved default ruleset may be rated; development/QA/legacy and custom tuning are explicitly unrated. The rating is outcome-based, not raw damage farming.
- AC5: An actual bounded waiting queue uses saved ratings and experience, preserves newcomer separation, selects compatible rosters and balances team rating. Incompatible/overflow players remain queued rather than filling an unfair match. Teams are frozen in Running, reconnects retain their seat, and explicit play-again returns to matchmaking. Small population waiting is shown honestly rather than widening into extreme skill mismatch.
- AC6: Desktop and mobile post-game/history UI show authoritative outcome, nicknames, heroes, K/D/A, damage breakdown, progression and rating changes. Results/history survive live-scene teardown and are accessible after reconnect. Dashboard modal blocks gameplay input; mobile rows/actions fit phone preview with touch scrolling.
- AC7: Meaningful statistics, identity, queue, transaction/reopen/idempotency, protocol and native UI tests pass. Version/changelog/features/contributor/runbook notes and evidence document actual capabilities and remaining limits. Preserve unrelated user work; do not deploy production services or change secrets.

## Design constraints

Start with one game server arena and a bounded queue; expose a clean storage/service boundary for later multi-arena scale. SQLite is the proposed local beta database pending dependency approval; no external DB deployment. Device-key authentication is proposed for beta without requiring paid avatars or a wallet, with explicit recovery/cross-device limits. This is not a claim of bot detection, smurf prevention or mature competitive rating calibration. Avoid forced10-second dismissal of results. Native phone preview does not replace physical-device verification.

## Verification and disk budget

The former57GB native cache was removed externally. Only about2.8GB free was observed at task start. Reuse existing artifacts where possible, disable debug/incremental artifacts for new checks, and measure disk usage before heavy builds. Never delete user data or large folders without explicit authorization. If full native verification needs more space, finish independent work and report the exact requirement rather than claiming an unrun check.
