# 2026-09-24 — Warden jungler and five-role teams

## Goal
Five-player teams need one hero per duty. Four classes covered Solo, Mid, Carry
and Support; nothing was built for the Jungle role, and practice bots never
farmed the forest.

## Changes
- `shared`: `HeroClass::Warden` (kit, basic attack, HP/growth, `ProjectileStyle::Claw`,
  item order), `HeroClass::primary_role`, Forest Tracker multipliers in `jungle.rs`.
- `server`: Warden multipliers applied in `apply_neutral_damage` and
  `award_neutral_kill_to_player`; bots fill a Mid/Solo/Carry/Jungle/Support
  composition and the Warden bot clears own-half camps (`jungle_camp`, `jungle_target`).
- `client`: claw combat visuals and audio, minimap letter `J`, five-wide draft
  class row, class pick proposes its role, skill atlas extended to 4×5 with a
  Warden row generated in Higgsfield (`gpt_image_2_5`, original atlas as style reference).

## Checks
- `cargo test -p shared -p server -p client` all pass (68 / 269 / 528).
- New tests: five distinct roles, Warden passive damage/rewards, five-class bot
  teams, both bot Wardens clear an own-half camp within 45 simulated seconds,
  draft role proposal.
- Balance probe with five classes stays within bounds (see `docs/balance-tuning.md`).
- Front-end QA capture shows the Warden on the hero-select screen; live combat
  capture (`scripts/capture_combat.py --class warden --touch-controls`) shows the
  Warden icons on touch buttons. That script's observer check fails identically
  for Warrior on the base commit (projectile hidden from the observer since team
  vision), so it is a pre-existing harness issue, not a Warden regression.
- `cargo clippy` reports no warnings in changed lines.

## Remaining risks
- Older clients decode `warden` as Warrior: client and server must ship together.
- No human playtest yet of jungle pacing, ganks or lane pressure without a jungler.
- The bot jungler only clears and fights heroes it sees; it does not gank or take bosses.
