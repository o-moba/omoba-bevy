# 2026-09-24 — Server match rules (roadmap step 7, first slice)

## Goal
The match mode (`release`, `dev`, `practice`) was compared at nineteen
sites across eight server modules, each deciding one thing (team of a
joining player, when the match starts, whether bots fill, whether debug
commands are accepted, whether the result counts). Step 7 wants one policy
object that decides everything once so the next slices (career trait,
transport and clock traits) can be built on it.

## Changes
- New `server/src/match_rules.rs`: `MatchMode`, `MatchConfig` and the env
  parsers moved here from `formation.rs` verbatim; `MatchMode::id()` replaces
  `MatchConfig::mode_id()`. `MatchRules::for_mode(mode, team_size)` derives
  every decision; `From<MatchConfig>` builds it in `ServerRuntime::new_with_map`.
- `ServerRuntime.rules: MatchRules` replaces `match_config`;
  `advance_formation_on_join`, `tick_match_formation` and
  `prematch::snapshot` take `MatchRules`.
- Replaced sites (one field each, the runtime condition kept at the site):
  `runtime/dispatch.rs` (team assignment, bot replacement on join, the two
  debug commands), `runtime/mod.rs` (Combat Sandbox startup check; the banner
  still matches on `rules.mode`), `formation.rs` (`StartPolicy`), `bots.rs`
  (practice join, bot fill, practice commands, `simulate_bots`),
  `session.rs` (bots dropped on restart), `sandbox.rs` (`sandbox_allowed`),
  `prematch.rs` (`RosterPolicy` twice), `career_runtime.rs` (career queue,
  guest participant cap, rating eligibility, durable/public-casual, unrated
  reason, checkpoint, reset un-join, play-again wait).
- Tests: `match_rules::tests::rules_table_per_mode` pins the table below;
  `rules_derive_from_config_and_keep_the_mode_id` pins the conversion.
  Fixtures keep constructing `MatchConfig`; `tests/formation.rs` uses
  `MatchRules::release(n)` / `MatchRules::dev()`; `sandbox/tests.rs` sets
  `rt.rules = MatchRules::for_mode(..)` where it used to poke `match_config.mode`.

## The table

| field | Release | Dev | Practice |
| --- | --- | --- | --- |
| `team_assignment` | `Balanced` | `ClientChoice` | `PracticeSeat` |
| `start` | `FullRoster` | `FirstJoin` | `FirstJoin` |
| `prematch_roster` | `Full` | `Present` | `Full` |
| `fills_with_bots` | false | false | true |
| `debug_commands` | false | true | true |
| `combat_sandbox_allowed` | false | true | false |
| `career_credit` | true | false | false |
| `local_results` | false | false | true |
| `career_flow` | true | true | false |

## Checks
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --no-deps -- -D warnings` clean.
- `cargo test -p shared -p server`: server 280 passed (278 + 2 new), 3 ignored; shared 78.
- `cargo test -p harness --no-run` builds; `cargo test -p client --lib` unchanged.
- The black-box harness runs before merge (maintainer).

## Notes for the next slices
- No respawn policy exists per mode (respawns are a `StatModifiers` flag);
  the plan's "respawn policy" item is dropped.
- `match_service.worker()` is the second policy axis: eleven career and bot
  sites combine a rule with it. The career trait slice should give that
  axis a name in the backend contract rather than adding fields here.
- `targeting_qa::enabled(mode)` stays a mode-taking env switch; it is a
  capture-QA gate, not a match rule.
