# MATCH-PROGRESSION-2026-09-14 — PASS

Version **0.19.0-rc.6**, base `e3913fcbb644c577a155ecd5ca84278b919fbe92`.
Worktree: `omoba-bevy-match-progression`; branch: `feature/match-progression-2026-09-14`.
The frozen release/social addendum supersedes the earlier SQLite/dependency
checkpoint. The user approved PostgreSQL implementation and basic friends;
replays were explicitly deferred. No production infrastructure or secrets changed.

## Acceptance audit

| Criterion | Result | Evidence |
| --- | --- | --- |
| AC1 / RS2 — permanent signed identity | PASS | Private atomic key creation/reopen/corruption tests, challenge binding and signed-action replay rejection; real native client -> UDP server -> PostgreSQL profile in `raw/native-auth-proof.json`. |
| AC2 — authoritative immutable statistics | PASS | Lifetime ledger and terminal regressions count accepted typed damage, reconnect, assists and first outcome; real UDP test drives combat victory. |
| AC3 / RS1 — durable transactions and recovery | PASS | Real PG reopen, concurrent duplicate settlement, rollback, immutable conflicts, owner fencing, outbox restart, reply-buffer heartbeat/ACK retention and rejected-allocation tombstones. |
| AC4 — persisted rating/progression | PASS | K32 Elo, independent clamping, eligible roster/ruleset gates, atomic XP/MMR updates and fresh profiles before requeue. |
| AC5 / RS5 — actual matchmaking | PASS | Saved-rating/newcomer queue, balanced frozen release rosters, signed admission, durable Start ACK, reconnect, explicit play-again and stale cross-server allocation revalidation. |
| AC6 / RS3 — profiles/results/history on PC and phone | PASS | 301 executed client tests; twelve native captures with layout bounds and visual inspection. Physical gestures are outside preview proof. |
| RS4 — persistent friends | PASS | Real DB self/direction/reverse/duplicate/idempotence, contact profile access, history privacy and presence ownership/expiry; client correlation/retry and both layouts. |
| AC7 / RS6 — verification/docs | PASS | Workspace regression, final client/server/PG tests, strict Clippy, native build/login/screenshots, version/changelog/features/runbook/attribution/progress updates. |

## Fresh checks

- `raw/workspace-tests-final-2.log`: **558 passed**, 14 PostgreSQL tests ignored
  in this workspace run and executed separately. Includes actual UDP gameplay,
  bots, attacks, effects and snapshot framing integrations.
- `raw/career-tests-verified.log`: **301 client + 170 server passed, zero ignored**,
  with `OMOBA_TEST_DATABASE_URL` and `--include-ignored --test-threads=1` after
  final lint cleanup. These rerun affected components, not 471 additional distinct
  tests. The server total includes all 14 real PG cases.
- `raw/postgres-career-tests-final-2.log`: prior focused **14/14 PG cases passed**,
  including pending friend-profile authorization. Additional supporting evidence.
- `raw/clippy-verified.log`: `clippy --workspace --all-targets -- -D warnings`,
  exit 0. Explicit serde wire packet enums have documented local large-enum
  exceptions; queued worker records are boxed. No blanket warning suppression.
- `raw/native-build-verified.log`: final client/server build exit 0, no warnings.
- `raw/sanity-verified.log`: staged whitespace and `cargo fmt --all -- --check`,
  exit 0. Git's blank-at-EOF check is disabled for this invocation because
  unmodified Cargo output ends with a blank line; other whitespace checks remain.
- `raw/native-auth-proof.json`: final native binaries create a fresh isolated
  device key, sign in through the actual socket and create a PG profile with
  rating 1000; key mode 0600. The proof contains a public profile only. Logs and
  reproducible `raw/verify_native_auth.py` are included; private configs excluded.
- `raw/career-desktop-final/qa-summary.json` and
  `raw/career-phone-final/qa-summary.json`: **six captures each**, PASS, desktop
  1280×720 and phone preview 844×390. Header controls and modal roots fit without
  overlapping. Every PNG was inspected. Chinese/Cyrillic and long names render;
  loss history has negative rating deltas. Captures precede behavior-neutral lint
  cleanup only; final client tests/build cover that cleanup.

Screenshot data is synthetic and watermarked. The stages are own profile, friend
code/input, scrolled friendship lists, history, ten-player result and friend
profile. This proves presentation, not real match data or manual touch/IME.

## Reproduction

Rust 1.93.1 and checked-in Cargo.lock; PostgreSQL 18 on isolated loopback port
55439, database `omoba_career_test`. Each PG test creates its own schema and
cleans it after success. Never use a production database for these tests.

```sh
export CARGO_TARGET_DIR=.agent/tasks/MATCH-PROGRESSION-2026-09-14/target
export CARGO_INCREMENTAL=0
# Supply OMOBA_TEST_DATABASE_URL for a dedicated local PostgreSQL test database.
cargo --config profile.dev.debug=0 --config profile.test.debug=0 \
  --config profile.dev.opt-level=0 --config 'profile.dev.package."*".opt-level=0' \
  test --workspace --locked --offline
cargo --config profile.dev.debug=0 --config profile.test.debug=0 \
  --config profile.dev.opt-level=0 --config 'profile.dev.package."*".opt-level=0' \
  test -p client -p server --locked --offline -- --include-ignored --test-threads=1
cargo --config profile.dev.debug=0 --config profile.test.debug=0 \
  --config profile.dev.opt-level=0 --config 'profile.dev.package."*".opt-level=0' \
  clippy --workspace --all-targets --locked --offline -- -D warnings
```

For captures, set new absolute `OMOBA_CLIENT_CONFIG_DIR` and
`OMOBA_CAREER_QA_OUTPUT`, `GAME_SERVER_ADDR=127.0.0.1:59999`, then run the built
client. Desktop: `OMOBA_TOUCH_CONTROLS=0 OMOBA_QA_WIDTH=1280 OMOBA_QA_HEIGHT=720`;
phone: `OMOBA_TOUCH_CONTROLS=1 OMOBA_QA_WIDTH=844 OMOBA_QA_HEIGHT=390`.
QA exits after six readbacks or fails at 120s. The intentionally absent test
server explains connection-refused logs for these presentation-only fixtures.

## Limits and preservation

Completion applies to this tested beta implementation. One arena/queue per game
process, bounded worker and shared PostgreSQL profiles do not establish thousands
of global concurrent users, physical mobile acceptance or production failover.
Profiles currently identify installations; recovery/linking devices need separate
UI. Party matchmaking, invitations into a shared game, chat, website account API
and replays are not implemented. Friendship and presence are the delivered social
scope. Operator setup is in `docs/match-progression.md`.

No production deployment was performed. Four unrelated original-checkout
cinematic files remain untouched. Local DB files, keys/configs, outboxes, targets
and build caches are excluded from Git. Evidence is staged with an explicit
allowlist, not by adding the `.agent` directory. `evidence.json` records current
source and artifact hashes. Git history establishes actual commit/merge/push state.
