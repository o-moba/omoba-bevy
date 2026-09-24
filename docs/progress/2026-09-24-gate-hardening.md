# 2026-09-24 — Gate hardening and hygiene

Follow-ups from [ARCHITECTURE_REPORT.md](../ARCHITECTURE_REPORT.md): O1
(PostgreSQL tests in CI), O20 (harness diagnosability), the `match_pool`
flake, and the quick wins Q1–Q9 (§8.1). Every claim was re-checked on
939571d before acting; line numbers in the report had drifted.

## O1: PostgreSQL-backed tests

- Found: 35 `#[ignore]`d tests need a database: career-store 15 (14 in
  `career_store_tests.rs` on per-test schemas, 1 supporter test that needs
  the Account API `portal` schema), account-api 17 (three need the
  restricted runtime role from `account-api/ops/grants.sql`,
  `OMOBA_PORTAL_ROLE_TEST_URL`), server 3. A fourth server test,
  `postgres_live_udp_signed_profiles_queue_real_cast_and_durable_history`, is
  not ignored and returned early (counted as passed) without
  `OMOBA_TEST_DATABASE_URL`. The three passport ignores need an external
  registry and are out of scope.
- New `scripts/postgres_tests.py`: checks the database is reachable (`psql
  SELECT 1`, fails loudly otherwise), runs `omoba-account-api migrate`
  (career and portal migrations), creates `omoba_portal_test` (login) and
  `omoba_game_test` if missing, applies `grants.sql`, derives
  `OMOBA_PORTAL_ROLE_TEST_URL` from the owner URL, sets
  `OMOBA_REQUIRE_TEST_DATABASE=1`, then runs
  `cargo test --locked -p omoba-career-store -p omoba-account-api -p server
  -- --include-ignored --test-threads=1`. `scripts/test_postgres_tests.py`
  covers the URL derivation.
- `postgres_live_udp` now panics when `OMOBA_REQUIRE_TEST_DATABASE` is set
  but the URL is missing; locally, without either variable, it still skips.
  With a URL that cannot be reached it already panicked (`expect`).
- CI: new `postgres` job with a `postgres:16` service (health check
  `pg_isready`), running the script on push to `main`, on pull requests and
  nightly (`schedule`, 03:23 UTC; the other jobs run nightly too).
- `make test-postgres` (not part of `make check`). The documented command
  `cargo test -p server career_store -- --ignored`, which selects 0 tests
  since the tests moved to career-store in #21, is fixed in
  `career_store_tests.rs`, `career-store/migrations/postgres/README.md` and
  `docs/public-mvp.md`.
- Local run (PostgreSQL 16.13 from Ubuntu packages, fresh database and again
  on the same database): all green. account-api 6 unit + 7 devices + 6
  postgres + 4 supporter, career-store 28, server 286 (every ignored test
  included, `postgres_live_udp` against the database, no SKIP line).

## Flake: `only_one_coordinator_can_own_a_root_and_drop_releases_it`

- Root cause: the coordinator lock is a `flock` on `.coordinator.lock`. A
  `flock` belongs to the open file description, and it is released only when
  every descriptor that refers to it is closed. The other `match_pool` tests
  spawn `/usr/bin/true` from parallel test threads; between fork and `exec`
  the child holds a duplicate of every open descriptor (`O_CLOEXEC` closes
  them only at `exec`), including this test's lock. When the test dropped
  its `Pool` inside that window the lock survived the drop and the re-lock
  failed (`match_pool.rs:391`). Reproduced before the fix: 2 failures in
  2 000 runs of the `match_pool` tests (8 parallel loops), panicking at the
  post-drop assertion.
- Fix: `impl Drop for Pool` calls `File::unlock()` (`LOCK_UN`), which
  releases the lock on the description whatever other descriptors still
  refer to it. The production lobby gets the same guarantee when it drops
  its pool while spawning workers. The test now also holds a `try_clone()` of
  the lock across the drop, the deterministic stand-in for a forked child:
  without the fix it fails every time, with it it passes.
- Loop: the test alone 200 times and the `match_pool` tests 2 000 times in 8
  parallel loops, 0 failures.

## O20: harness diagnosability

- `harness/src/server.rs` pipes the server's stdout and stderr (stderr went
  to `/dev/null`) into a ring buffer of the last 200 lines, one reader thread
  per stream (stdout still signals readiness). When a test panics while it
  holds a `ServerProcess`, its drop prints the buffer, so it appears with the
  failing test's output. Early exits and ready timeouts include the buffer in
  their error.
- The harness warns once per test binary, on the real stderr, when the
  prebuilt `target/debug/server` is older than the newest file under
  `server/src` or `shared/src`.

## Hygiene Q1–Q9

- Q1: `server` no longer depends on `bevy` (no `bevy::` use since #25).
  `cargo tree -p server -i bevy` showed the server was the only path to Bevy
  in its own graph (the SDK with default features off does not use it), so
  the server build loses Bevy entirely. The client gets its Bevy features from
  its own manifest, so unification for client builds is unchanged. Lock
  change: the `bevy` line in the `server` entry.
- Q2: server `sqlx` moved to `[dev-dependencies]` (all uses are in the
  PostgreSQL fixtures; same features as career-store's, so no feature change).
- Q3: unused `serde` removed from career-store.
- Q4: the second `## [Unreleased]` heading in `CHANGELOG.md` is merged into
  the first (its four sections moved under it, below the newer ones).
- Q5: see O1.
- Q6: harness `step_toward` and the `bots` queue wander use
  `shared::math::hero_yaw_towards` (only the yaw lines changed).
- Q7: `max_hp_for_level` is used by the offline duelist, which computed the
  same formula. The server's `hero_stats::max_hp` keeps its own closed form
  because the base can be overridden (`modifiers.base_max_hp`).
- Q8: five stale `allow(dead_code)` removed (`RequestRematch`,
  `NetworkProjectile.owner_team`, `SKILL_UPGRADE_KEY`,
  `HeroAnimationState::Walk`; the server's `SKILL_SLOT_COUNT` was used only by
  its test and is deleted, the test compares with `SkillSlot::ALL`). Kept:
  the five `presentation2d.rs` manifest fields, `Activated.source` and the
  iOS `cfg_attr`. Five local clippy allows that repeat workspace allows are
  removed (`large_enum_variant` in `wire.rs`, `too_many_arguments` in
  `gesture.rs`, `widgets.rs`, `practice_sandbox.rs`, `pause_menu.rs`).
  `LICENSING.md` drops `skills/` and lists `career-store/` under AGPL. The SDK
  revision is one `[workspace.dependencies]` entry (default features off);
  the desktop client asks for `default`, passport for `http`.
- Q9: `REFACTORING.md` "eleven" `worker()` sites → twenty (counted), "this
  PR" → #37/#38/#41; `ARCHITECTURE.md` crate map (shared's SDK dependency and
  its one I/O, the avatar roster; server without Bevy, sqlx dev-only) and the
  loop described as paced with a variable `dt` instead of "fixed-step";
  `CHANGELOG.md` and the hero-stats progress note no longer claim
  `PlayerEquipment` is local-only (remote heroes carry it, only the local
  player is queried); `mobile/ios/TESTFLIGHT.md` points at `client/src/net/`.

## Checks

GATE_PLACEHOLDER
