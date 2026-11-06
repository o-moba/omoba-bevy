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

191 shared tests pass, including tier boundaries, the twentieth placement,
all valid reset inputs, signed integer extremes and unknown future metadata.
The matchmaking crate reuses the same maximum rating; 14 focused checks pass.

Still required before LR-53/P6 completion:

- Durable season metadata and per-season counts, preserving lifetime career,
  public eligibility, XP and immutable historical receipts.
- An explicit close-season operator command with expected-current identity,
  exactly-once archive/badge/reset and safe coordination with late settlement.
- Document initial-season creation, expiry admission, badge eligibility and
  recovery. Live ratings must never reset through an incidental read/startup.
- Save input in participant results and provide a bounded win-rate comparison
  with sample counts; preserve unknown legacy data and the shared queue.
- Native/portal tier, placement, season and badge presentation; localized
  promotion/demotion feedback and directly affected visual checks.
- Real PostgreSQL retry/concurrency/rollback and migration tests, signed public
  journeys, final CI, then separately reviewed production application.

Current helpers do not start a season, modify live ratings, persist input or
change the active UI. Follow the full lobby TODO for the remaining work.
