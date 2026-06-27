# 2026-06-27 — Upgradable skills (TASK03) + God Mode (TASK04)

## Goal
Two authoritative network features requested during playtest, tracked in
`tasks/task03.md` and `tasks/task04.md`.

## TASK04 — God Mode (debug invulnerability)
- Server: `god_mode: bool` on `ConnectedPlayer` (`server/src/main.rs`), initialized
  in `server/src/session.rs`. New `ClientPacket::SetGodMode { enabled }` sets it.
  All three player-damage sites (projectile/cast, neutral attack, minion attack)
  skip damage when the flag is set.
- Client: `client/src/god_mode.rs` left-side toggle button (red when ON) sends
  `NetworkCommand::SetGodMode`. Registered as `GodModePlugin`.
- Limitation: server resets `god_mode` on a fresh connection; the client toggle is
  not auto-resent on reconnect.

## TASK03 — Upgradable skills
- Server: `ranks: [u8;4]` added to `PlayerState` (snapshot, default `[1;4]`), reset
  on match reset. `ClientPacket::UpgradeSkill { slot }` spends one skill point and
  raises the rank up to `MAX_SKILL_RANK`. Cast damage uses
  `PRIMARY_ABILITY_DAMAGE_BY_RANK` indexed by the Q rank; the table is now
  increasing (`20, 28, 36, 44, 52`).
- Client: `PlayerProgression` carries `ranks`; the bottom skill bar shows each
  slot's `Lv N` and an upgrade ↑ button (green when a point is spendable). Arrow
  click and the `U` key send `UpgradeSkill` (U → Q / slot 0).
- Scope: only Q is authoritative in combat today; W/E/R show/raise rank but do not
  yet affect the simulation (no distinct server abilities).

## Checks
- `cargo build --workspace` — pass.
- `cargo test -p server` — 24 passed. `cargo test -p client` — 25 passed.

## Remaining risks / follow-ups
- No live visual pass (headless): confirm rank number updates, Q damage grows, the
  upgrade arrow lights only with a spendable point, and God Mode HP stays full.
- Server upgrade handler is inline in the recv loop; add a unit test by extracting it.
- Reconnect re-sync of God Mode toggle (see task04).
