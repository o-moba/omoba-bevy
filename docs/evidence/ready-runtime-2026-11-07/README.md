# Committed ready consent in the actual lobby runtime

The production worker port now activates durable reserve/publish/answer/poll/cancel
jobs off the game tick. The lobby exposes confirmation only after SQL publication,
credits committed answers only and starts a worker only after committed unanimity.
Votes bind the signed account/session and current search. A decline stays final;
late responses, unrelated completions, channel acceptance and retry do not count
as consent. Per-seat answer coalescing and bounded batches avoid ten serial SQL
round trips per roster. The worker result ID uses the approved allocation/check ID.

SQL timeout snapshots update the lobby cooldown cache without creating a second
producer event. Maintenance cancels without blaming a player. After a draft exit,
shared claims remain held until the exact draft-dodge event commits, then release
so nine retained players can form another roster without waiting the old handoff
TTL. Existing FIFO and party selection predicates are unchanged.

## Actual checks

- [All551 server cases](server-all.log), including the ten PostgreSQL cases, PASS
  with zero failures/ignores. The complete actual `server/src/main.rs` test target
  is linked; no engine modules were copied or stubbed. New coordinator11, port2
  and actual MatchService3 tests are included.
- [All82 store cases](store-all.log), actual disposable PostgreSQL18.6, PASS with
  zero failures/ignores. The new bounded answer-batch case validates every seat
  before timeout/vote effects and checks duplicate/foreign/decline behavior.
- Strict Clippy of the actual complete server test and production targets and
  store test target PASS with the existing workspace lint allowances. Formatting,
  Node syntax/four policy self-checks and all5 harness tooling tests PASS.
- [Two separate live lobbies](cross-lobby-ready.json), independent epochs/outboxes
  and one owned DB:19 distinct Ranked profiles and a second authenticated endpoint
  for one account. Another lobby refuses that account during the live confirmation,
  then sees the shared missed-answer cooldown. No worker starts without unanimity;
  nine accepted peers keep FIFO age; B keeps its own nine waiting peers; no copied
  penalty file or duplicate producer event is used.
- [Five-stack draft exit and replacement](party-draft.json): actual signed invites,
  ten human seats, leave, nine requeued with unchanged profiles/empty histories,
  one dodge, fresh request consent and a replacement Ranked match Running.
- [Maintenance/restart/Resume](drain.json): a free arena remains unused while
  draining, current worker ticks continue, an old decline creates no strike,
  reopening needs fresh consent, and a lobby restart adopts live workers and keeps
  the exact drain operation while allowing Resume.

These are local loopback UDP journeys with real signatures and the actual current
server executable. Ranked eligibility setup is explicitly synthetic prior public
receipts, not ten played games. Maintenance is BotPractice with two arenas; it is
not a second-host Ranked fault/capacity certification. [Rejected attempts](rejected-local-attempts.json)
are preserved, with the production handoff fix and probe setup fix distinguished.

- [Final-source ready timeout/replacement](ready-check.json): nine accepted peers
  retain FIFO age after15s, zero workers before unanimity, late accept cannot
  revive the old check, and a fresh replacement roster creates exactly one worker.

## Build scope and remaining release gates

Root disk is below the20GB heavy-build floor. Current common/passport/shared/store
and complete server sources are compiled against a coherent, read-only cached
pinned dependency graph. No third-party/graphics compile or Cargo-cache writes,
new target directory or dependency occurs. Production common excludes test-support.
[Exact compiler command/dependencies](server-compile.json), [graph](cached-graph.json)
and the archived workspace-specific drivers document reproduction. Small native
artifacts remain in the ignored `.agent/codex-ranked-readiness/ready-runtime`.
Ordinary Cargo/full Bevy/platform validation remains delegated to fresh CI.

Parent39d2fd53 CI42319660756 passed PG/public journeys, headless, Python, Android
and bot jobs. Core and dev-client failed E0716 in the new translation test; static
keys fix that in separate commit ba4bba35. Those old results do not certify these
runtime changes. Fresh full CI, current rendered client/physical devices, genuine
multi-host/WAN fault/Resume, legacy penalty floors, ready/journal health/retention,
schema12 recovery, production trust provisioning,1000-player RU capacity and the
pending C2/C4 host-tool decisions remain open. Original68 ledger remains62 done,
5 partial,1 open. No RU service, production role/secret or deployment is changed.
