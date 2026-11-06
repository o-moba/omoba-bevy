# Ranked seasons and input statistics

Status: in progress. Owner decisions O-5/O-18 accepted 2026-11-06; production
rollover/reset remains a separately authorized action.

The shared rules are implemented in `shared/src/ranked.rs`:

- 20 current-season Ranked games before a calibrated tier.
- Bronze 0–999, Silver 1000–1499, Gold 1500–1999, Platinum 2000–2499,
  Diamond 2500–2999, Master 3000+.
- 90-day seasons. Integer soft reset is `1000 + (old_rating - 1000) / 2`, with
  the signed division truncated toward 1000. Thus 999/1000/1001 reset to 1000,
  0 resets to 500, and 5000 to 3000. Defensive bounds preserve the existing 0–5000
  rating domain.
- One Ranked queue for touch and mouse. Declared input starts unknown, keeps
  known evidence across missing reports, and becomes mixed after a switch.
  This describes controls rather than verified hardware or operating system.

192 shared tests pass, including tier boundaries, the twentieth placement,
all valid reset inputs, signed integer extremes and unknown future metadata.
The matchmaking crate reuses the same maximum rating; all55 store/matching checks pass on real PostgreSQL.

Still required before LR-53/P6 completion:

- Durable season metadata and per-season counts, preserving lifetime career,
  public eligibility, XP and immutable historical receipts.
- An explicit close-season operator command with expected-current identity,
  exactly-once archive/badge/reset and safe coordination with late settlement.
- Document initial-season creation, expiry admission, badge eligibility and
  recovery. Live ratings must never reset through an incidental read/startup.
- Native/portal tier, placement, season and badge presentation; localized
  promotion/demotion feedback and directly affected visual checks.
- Real PostgreSQL retry/concurrency/rollback and migration tests, signed public
  journeys, final CI, then separately reviewed production application.

Current season helpers do not start a season or modify live ratings. Input
persistence/query is implemented below. Follow the full lobby TODO for the
remaining work.


Declared-input persistence/query is now implemented; see
[controls comparison](../ranked-input-statistics.md). Native selected controls
are signed; per-seat evidence and durable checkpoints preserve switches across
reclaim/recovery. Old receipts remain unknown. The read-only bounded comparison
reports counts and coverage; no queue split or matching weight was introduced.
192 shared,17 fair-play,55 real PG/store/matching and33 harness cases PASS;
native test targets and strict native clippy PASS. Current full CI remains required. Season/archive/badges
are still pending; the pure season helpers do not reset live ratings.


P6 season schema/contract checkpoint (not runtime integration): SeasonProfile
separates current placements from lifetime totals; SeasonBadge supports a
provisional participation award.193 shared tests and strict five-crate all-target
clippy PASS. New migration010 is drafted with a90-day initial window, FK-checked
profile season snapshots/counters, immutable allocation mapping/archive/closure
receipts and deferred exactly-one-open-window protection. Registration takes a
shared season lock before stamping metadata. An owned local transaction applies
all migrations, verifies unchanged2300 rating/40 lifetime Ranked/55 played/50
public/12000XP, zero initial season counters, new registration and immutable
season deletion, then ROLLBACKs. Existing test DB remains schema9.
SCHEMA_VERSIONS remains9; profile reads still return no season summary. Migration
activation, allocation/settlement locks/counters, explicit close command, durable
season identity through manifests/results/recovery, queue expiry/cached profiles,
role-grant draft, full PG concurrency/rollback tests and UI remain required.
No production season or rating reset has been performed.
