# Architecture

This page is the map of the repository for people changing it: which crate
owns what, how a frame flows through the client, how a tick flows through the
server, the rules that keep the two in step, and the refactoring roadmap the
maintainers are working through. Feature-level documentation lives in
[features.md](features.md); operations in [RUNBOOK.md](../RUNBOOK.md).

## Crate map

| Crate | Role | Depends on |
| --- | --- | --- |
| `shared` (MPL) | The gameplay model both sides agree on: hero classes and ability kits, hero growth, items, map geometry and navigation, the wire protocol, prematch/draft, social/career/account contracts, sandbox and practice commands. No Bevy, no I/O in the model itself. | serde |
| `server` (AGPL) | The authoritative simulation and UDP endpoint: match lifecycle, bots, combat, shop, career settlement, public transport signing, allocation workers. Binary only. | shared, passport, career-store |
| `career-store` (AGPL) | Trusted career persistence (Postgres, migrations) and the bounded queue policy, linked by the server and the account API without the engine. | shared, sqlx |
| `client` (MPL) | The Bevy game: networking, prediction, presentation (2D sprites and 3D models), UI, mobile input, offline practice, QA harnesses. | shared, passport, bevy, ekza-bevy-sdk |
| `harness` | Black-box UDP players and gameplay/matchmaking checks that launch the server binary. | shared |
| `passport` | Ekza passport contract: tickets, device and web accounts, store admission. | shared, ekza-bevy-sdk |
| `account-api` | Axum/Postgres HTTP service over the career store (portal, devices, supporter billing). | shared, career-store |
| `arena-sync` | CLI that pulls Ekza Arena avatars and merges the avatar manifest. | reqwest |

Rules that follow from the map:

- Anything both client and server must agree on lives in `shared`. That
  includes every wire type; never copy a wire struct into another crate.
- `shared` must stay free of Bevy so the server and tools compile without an
  engine. Client-side ECS components wrap shared types instead.
- Tuning numbers have one home. Hero growth, speed, mana, respawn, XP
  thresholds and projectile speed are in `shared::hero_balance`; facing
  conventions in `shared::math`; server-only simulation numbers (minions,
  towers, neutrals) in `server/src/balance.rs`; tower stats in the map JSON.

## Client frame

Plugins are registered in `client/src/lib.rs`. The `Update` schedule is
ordered by a small number of system sets; read these together to understand
frame order:

1. `ClientNetPipeline` (`client/src/net/mod.rs`): ingest UDP snapshot
   fragments, apply the authoritative snapshot to entities, age utility
   timers, interpolate remote entities and players. The `net` module is
   split by stage: `transport` (UDP thread, framing, decode, channels),
   `session` (`ClientSession`, join retries, teardown, reconnect),
   `commands` (`NetworkCommand` to packets), `ingest` (drain the channel,
   stage one snapshot per frame), `apply` (snapshot to ECS), `interpolate`
   (remote poses, grounding), `components` (replicated components and
   `GameStateSnapshot`), `status_ui` (connection panel), `offline`
   (practice simulation) and `public_transport` (signed public datagrams).
   `mod.rs` re-exports the public surface, so other modules keep importing
   `crate::net::X`.
2. `InputContextSet::{Social, Modal, Resolve, Actions}`
   (`client/src/input_context.rs`): modal UI (pause menu, shop, career,
   social, help) runs first and decides whether gameplay input is allowed;
   `Actions` holds movement (`WorldMovementInputSet`), targeting and casting
   (the combat chain in `client/src/combat.rs`) and mobile controls.
3. `ClientNetPipeline::SendLocalState` and `SendCommands` after `Actions`:
   the local transform and the queued `NetworkCommand`s become packets.
4. `PostUpdate`: grounding, target presentation, UI layout adjustments.

Presentation is chosen once per run by `PlayerVisualMode` (3D models by
default, 2D sprites with `OMOBA_PLAYER_VISUAL_MODE=sprite2d`). Simulation
positions are always XZ on the ground plane; 2D rendering maps them to XY.

The offline practice playground (`client/src/net/offline.rs`) is a
socket-free simulation that speaks the same packets through the same channels
as the UDP transport, so the rest of the client does not know it is offline.

## Server tick

`ServerRuntime` (`server/src/runtime/mod.rs`) owns the socket, the
sub-runtimes (bots, prematch, career, social, sandbox) and one `GameWorld`
(`server/src/game_world.rs`): the entity maps (players, disconnected
sessions, projectiles, structures, minions, neutrals), team buffs, forest
pickups, the game state, the map layout and config, the id allocators and
the wave clock. Simulation functions and request handlers take
`&mut GameWorld` plus a `TickCtx { now, dt }` (or just `now`) instead of a
parameter per map; only the leaf damage helpers (`apply_*_damage`) still take
individual maps because they run inside loops that hold other fields.
`main.rs` is the module list plus `fn main`. `runtime::run` is a plain
fixed-step loop (`SIMULATION_STEP_SLEEP`, 10 ms): `prepare_tick`, then
`tick`, then sleep the remainder of the step. There is no Bevy `App` and no
ECS mirror on the server; the `GameWorld` maps are the only copy of the
state. A tick is:

1. `ServerRuntime::prepare_tick`: `runtime::dispatch::receive_packets` decodes
   datagrams; public roles verify signed commands first (`public_transport`).
   `handle_packet` admits identity, then `handle_packet_authorized` applies
   gameplay commands. The wall-clock `dt` is clamped to 100 ms and, in the
   Combat Sandbox, replaced by the sandbox's virtual clock.
2. `ServerRuntime::tick(now, dt)` (`runtime/tick.rs`): mana regeneration
   (`sim::regenerate_mana`), the minion-targeted projectile pass, then
   formation (`formation`), bots, and the `sim` modules (`minions`, `towers`,
   the remaining `projectiles`, `neutrals`, plus regeneration and respawns in
   `sim/mod.rs` and `session`); it records combat receipts into the round
   ledger, and `snapshot::broadcast_snapshots` builds one snapshot per
   recipient (vision filtered) and sends it. Projectile flight is one
   function, `sim::projectiles::simulate_projectiles_filtered`, run twice with
   a target-kind filter only to keep the minion pass at the point in the
   frame where the old ECS systems ran; folding it into one pass is the
   next slice.

Hero state and views: the server never stores the wire `PlayerState`.
`ConnectedPlayer` (`entities.rs`) holds `hero: Hero` (`server/src/hero.rs`:
`HeroIdentity` for id, bot flag, team, class, character, avatar, sprite and
supporter aura, set at join and never by the simulation; position, yaw, HP
and mana; `HeroProgress` for XP, level, skill points and ranks;
`HeroUtility` for the utility request marks; `HeroAction`, the last accepted
cosmetic action), `economy: HeroEconomy` (gold, earned gold, inventory, item
bonuses, last purchase receipt, the basic-attack and purchase request marks
and the passive-income remainder) and `timers: HeroTimers`
(`server/src/hero_timers.rs`: last movement, per-slot casts, last basic
strike, dash and haste readiness, haste expiry, respawn), next to the
transport and admission fields. `hero_timers` also holds the pure reads over
the instants (`basic_attack_remaining`, `skill_cooldown_remaining`,
`skill_recovery_remaining`, `dash_remaining`, `haste_remaining`,
`haste_active`), used as gates by the request handlers and by the sandbox
telemetry, and `normalize_hero_timers`, the one place that clears instants
from derived conditions (death, sandbox `no_cooldowns`), run once per tick
after respawns. `PlayerState` is built only by the views:
`ConnectedPlayer::owner_view(now, map, phase)` maps `hero` and `economy`
onto the wire struct and fills the cooldown, utility-clock and shop fields at
the tick's `now`; `public_view` is the hook for redaction and equals the
owner view today. `snapshot::build_players_snapshot(world, now)` builds every
replicated player through it. `server/src/tests/player_view.rs` pins the
mapping and the view's bytes through a hero lifetime.

Supporting modules: `entities` (the server-side records), `sim/cast.rs`
(ability casts), `vision` (server-owned sight, takes `&GameWorld`). Unit
tests for these live under `server/src/tests/`.

Bots are ordinary `ConnectedPlayer`s on unspecified IPv6 addresses; their
addresses never accept network commands. Practice, development and release
modes differ in team assignment, bot filling, debug commands and career
credit.

## Protocol rules

- `PROTOCOL_VERSION` (`shared::protocol`) is bumped only for incompatible
  changes; peers with another version are rejected at `Hello`.
- Compatible changes are additive: new fields carry `#[serde(default)]`,
  new enum values are only added where the decoder tolerates unknown values.
- Gameplay commands that must not replay carry `server_epoch`, `match_id`
  and a monotonic `request_id`; transforms carry `dash_sequence`.
- Snapshots are trimmed to the UDP payload limit by dropping the oldest
  cosmetic combat events, never gameplay state.

## Adding content

- **Hero class:** `HeroClass` and its ability kit in `shared/src/lib.rs`,
  growth in `shared/src/hero_balance.rs`, recommended items in
  `shared/src/shop.rs`, bot composition in `server/src/bots.rs`, class visuals
  in `client/assets/config/combat_visuals.json`, the skill atlas, audio cues,
  and the Python choice lists under `scripts/`. Client and server ship
  together; an old client decodes an unknown class as Warrior.
- **Item:** `ItemId` and `ITEMS` in `shared/src/shop.rs` (the array length is
  tied to the inventory capacity today) plus each class's recommended order.
- **Map:** structure placement and stats in a map JSON
  (`docs/map-customization.md`); arena geometry and collision are fixed.

## Roadmap

Ordered by value over cost. Each step is a separate change with the full
`make check` gate green.

1. CI gate, pinned toolchain, `make check` (done).
2. One wire protocol in `shared`, golden JSON tests, harness on shared types
   (done).
3. Balance constants and the hero facing convention in one place (done).
4. Crate hygiene: retire the orphan `skills` crate, move the career store out
   of the server package (done); keep I/O out of the shared model (the avatar
   and sprite rosters still embed client manifests and read env vars; open).
5. Server `GameWorld` + tick context instead of many-map parameters; split
   `main.rs` into runtime (dispatch, tick), snapshot, formation, entities,
   ECS and simulation modules (done; splitting `handle_packet_authorized`
   into per-command handlers and removing the crate-root glob re-exports are
   the follow-up).
6. One server tick (done: the Bevy `App`, the ECS mirror of players and
   minions, the duplicate mana regeneration and the ECS-only minion
   projectile path are gone; `ServerRuntime::tick` is the whole step);
   replicated `PlayerState` as a view over authoritative state and
   `HeroTimers` (done: `hero_timers.rs`, `owner_view`/`public_view`, no
   per-tick copies); authoritative hero state in server-owned structs (done:
   `hero.rs` with `Hero`, `HeroIdentity`, `HeroProgress`, `HeroUtility`,
   `HeroAction` and `HeroEconomy`; `ConnectedPlayer.state` is gone and the
   views are the only place that builds a `PlayerState`); `StatModifiers`
   (`hero_stats.rs`) and snapshot redaction through `public_view` remain.
7. Match rules as one policy object; career, transport and clock behind
   traits.
8. Client `net.rs` split into transport, session, commands, ingest, apply
   and interpolation (done, verbatim moves under `client/src/net/`);
   session events instead of cross-module writes (`SessionEvent`,
   `SnapshotApplied`, splitting `apply_server_snapshot`) are the open
   follow-up.
9. One UI kit (theme, widgets, gestures, scroll, actions) and a modal
   registry (pilot done: `client/src/ui/` with theme, tap recognizer, typed
   actions and widgets; the pause menu and the practice sandbox use it;
   remaining steps in `docs/ui-kit.md`).
10. Client domain module, combat/player split, render backends behind
    `run_if`, plugin groups, QA behind a cargo feature.
11. One debug tooling family shared by Combat Test, practice and offline.
12. Data-driven hero and item catalogs.
