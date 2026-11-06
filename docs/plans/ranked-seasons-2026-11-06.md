# Ranked seasons and input statistics

Status: in progress. Owner O-5/O-18 decisions accepted 2026-11-06. The season
storage/close engine, API and presentation are implemented. Schema10 core full
journeys and native test execution pass; expiry display, remaining native visual
evidence, current-source CI and deployment review remain required.
Production rollover/reset is a separately authorized action.

## Rules

- 20 current-season Ranked games before a calibrated tier. Initial migration
  starts the first90-day window at zero seasonal games without resetting rating.
- Bronze0–999, Silver1000–1499, Gold1500–1999, Platinum2000–2499,
  Diamond2500–2999, Master3000+.
- Integer reset: `1000 + (old_rating - 1000) / 2`, truncated toward1000.
  999/1000/1001 become1000;0 becomes500;5000 becomes3000.
- Lifetime matches, rated games, wins/losses, XP and completed-public eligibility
  never reset. Ranked remains accessible after ten completed public games, no VK.
- Touch and mouse share one queue; declared controls are retained per human seat.
  [Bounded input comparison](../ranked-input-statistics.md) reports wins, sample
  sizes and coverage, including unknown/mixed groups.

## Storage and transaction boundaries

Source runtime requires career schema10. Migration010 creates the initial window,
FK-checked profile season snapshots/counters, immutable allocation-season mapping,
archives and close receipts. Older receipts omit the new optional season ID.
Missing legacy season identity is accepted only for the initial window; explicit
old identities cannot become new post-reset allocations. Worker manifests freeze
identity before handoff; an old reserved worker cannot enter a newly reset season.

Allocation and settlement take a shared season lock before allocation/match and
profile locks. Close takes the exclusive lock first, locks profiles in stable
order and refuses every unsettled original Ranked intent, even when the final
intent is unrated. Existing games can settle after expiry. New Ranked admission
and expired ready checks stop without dodge penalties; Casual remains available.

Registration from both game and portal is stamped by a narrow owner trigger.
Its search path is pinned to the owning schema before `pg_temp`; PUBLIC execution
is revoked. The owner row lock prevents stale Repeatable Read registrations,
while runtime roles receive no season UPDATE privilege. Tests cover a caller's
fake temporary window and actual lock-waiting registrations.

Closing an expired expected-current season archives final standings, awards a
badge, applies the reset and opens a new90-day window in one transaction. Retry
returns the original close receipt. A conflicting archive rolls everything back.
At commit exactly one open window must remain. Closed metadata, archives, close
receipts and allocation identities are immutable.

Any computed Ranked award/forfeit increments seasonal games and personal outcome,
consistent with existing discipline. A badge requires at least one completed
Ranked game without the player's proved abandonment. Below20 games it is a
participation badge; after calibration it includes the final tier. A season with
no new badge preserves the previously earned badge.

## Operator boundary

The `ranked-season` example validates existing schema and never migrates:

- `show` uses read-capable `DATABASE_URL`.
- `close --expected-season ID --apply` requires a separately authorized
  `OMOBA_SEASON_OPERATOR_DATABASE_URL`. The game and portal roles cannot close,
  archive or consume operator season sequence IDs.

Before production migration, drain all old workers. Before production close,
drain all hosts, including unjoined worker reservations, and refresh/restart
cached lobby profiles before admission resumes. Database checks alone do not
inspect every host process. No read/startup/timer closes a season. Deployment
must account for schema9-to10 rollback compatibility; returning to an old
schema9 binary after migration is not a valid rollback.

The prepared deployment-role patch grants metadata reads, portal archive reads
and game INSERT/SELECT on immutable allocation mappings. The owner approved this repository patch and it is applied in
`account-api/ops/grants.sql`; fresh full-role/public verification remains required.
No production migration, reset, role change or RU deployment was performed.

## Evidence and remaining acceptance

- 193 shared tests PASS, including tier boundaries, reset rounding, seasonal
  placement fallback and preserved lifetime/public access.
- All63 store/matching tests (real PostgreSQL included) PASS. Eight season cases
  cover concurrent/idempotent close, initial state, rollback, expiry and original
  intent fencing, registration races/Repeatable Read, deferred open-window
  protection and restricted roles/temp-table shadowing. The expanded portal
  archive-read delta is rechecked in the focused role test.
- 64 lobby tests and33 harness tests PASS; strict affected-crate all-target clippy
  and native test-target clippy PASS. Home uses current-season placement progress;
  the new Home regression compiles but awaits execution in current full CI.
- 12 Python rehearsal tests PASS, rejecting wrong season IDs/counters and a
  lifetime-K award after the seasonal placement counter resets.
- Actual operator CLI proof PASS in an owned synthetic expired-season schema:
  show/no-apply/wrong-expected refusal,2300->1650 once, exact retry and unchanged
  lifetime/XP/public totals. The schema was removed; this is not gameplay proof.
- Earlier schema9 checkpoint ddc59e1d has independently rechecked full7-job/
  18-stage CI42153618597 PASS. New schema10 source needs its own full run.

Privacy-preserving profile/current-season ladder and bounded archive reads are
implemented. All11 Account API PostgreSQL cases, including actual restricted
portal grants, and strict store/API all-target clippy PASS. The ladder keeps
lifetime fields but uses seasonal calibration/tie-breaks and hides old badges.
The first fresh full schema10 run stopped on a device-test startup503; unchanged
focused migration and all8 device cases PASS afterwards. Cause is unconfirmed;
The repeat local core236d1c13 PostgreSQL run passes all669 tests across18 suites,
including actual role installation. Public v2 failed during ready-check fixture
preparation; setup-before-Search repair preserves the strict15s game deadline.
Current local public v3 passes all18 stages (2346.68s).

Core236d1c13 full CI42168782869 independently rechecked PASS:all7 jobs/18 stages,
verified artifact13481319080 SHA256
8c0338d3886cfeb2dc1d2ce101845d832aa307e9a8cee477842fcfdec04f657c. Real Ranked
f7a85df6eadb8fd9c071bf1a6bdbea1f,454307ms, exact all-ten awards/profile/history/
seasonal counters, zeroAFK/disconnection. Real tenth-public/noVK and early crash
PASS. This backend source remains unchanged through56b86dec; new native UI has
its own gate.

Portal fa4dc7f season/tier/badge/archive presentation:247 tests,3 expected skips,
typecheck/build and six RU852x393 Chrome/Next fixture scenarios PASS. Browser
fixtures remain separate from actual PostgreSQL API proof.
Native Home/own/visited/friend badges, windows and scoped calibration/promotion/
demotion notices are implemented. The four new helper/system cases execute in
CI56b86dec; its one remaining client failure is a stale Home test expectation,
repaired to Silver here. Strict native and RU test-target clippy PASS.
Six actual RU852x393 native pixels are inspected PASS. Coherent personal-penalty
demotion is being recaptured; invalid fixture attempts are retained. Optional QA
paths use synthetic data and do not prove real account/legal/hardware behavior.
CJK/asset and formatting checks PASS.

Still required: coherent demotion pixels and final current native CI;
expiry/cache refresh UX; reviewed deployment/drain/rollback integration.
Follow the full lobby TODO for all other findings and release gates.


Connected-session rollover cache repair: the existing bounded access read now
returns the canonical own profile atomically with active-key status. Periodic
refresh and pre-Find authorization adopt a strictly newer season without
reconnect. Older action/settlement replies cannot undo it; same-season periodic
reads cannot undo a newer award. No additional SQL query or timer was added.
All63 store/matching and497 server unit cases, the affected actual PostgreSQL
concurrent-access case and strict store/server all-target clippy PASS. Operator
drain and profile refresh/restart checks remain release prerequisites; this is
not permission to run a production close/reset. Expiry display and final
transient-notice pixels remain open.


Server-clock availability is implemented as additive optional CareerView metadata
and a localized disabled interseason Ranked action on Home. Legacy omission
follows existing behavior; authoritative allocation validation is unchanged.
193 shared/498 server cases and strict RU client test-target clippy PASS.
Committed execution of the extended Home interaction case and actual expired/
demotion pixels remain pending; the Result QA now captures the visible settled
notice, and missing notices fail instead of yielding a misleading screenshot.


Current c826 native CI executes all1578 cases, including the extended Home
interseason interaction. RU build is ready; expired Home/demotion screenshots
are still blocked by the locked Mac. Portal ccee8fa adds scoped rank-change
banners with250 tests and seven inspected-scope browser checks. The unrelated
full-CI party setup failure was a loopback client taking a future worker UDP
port; the socket regression and exact92.27s party-disconnect replay now PASS.
Full current-source public verification and remaining visual/release gates
remain required.
