# 2026-09-24 — One wire protocol definition

## Goal
The gameplay UDP/JSON protocol was hand-copied in three places: the server
(`server/src/main.rs`), the client (`client/src/net.rs`) and the harness
(`harness/src/protocol.rs`). The copies had drifted. Server, client and harness
now share one definition in `shared/src/protocol/wire.rs` (`shared::wire`).

## Drift the move fixed
- Client `PlayerState`, `StructureState` and `ServerPacket::Snapshot` listed
  fields in a different order than the server, so a client-built packet did not
  serialize to the same bytes as the server's.
- The client `Snapshot` carried a `career: CareerView` field the server never
  sends (career data has its own `ServerPacket::Career` datagram); it always
  decoded to the default and its "apply" branch was dead. It is gone.
- The harness `HeroClass` mirror lacked `warden`, its `ServerPacket` knew only
  `Snapshot`, and `Join` lacked `passport_ticket`; undecodable packets were
  dropped with `.ok()?` and never reported. The harness now re-exports the
  shared types, logs the first decode failure per bot and skips (rather than
  fails on) social/career envelopes.
- The client `Team` and `Lane` copies and the server `Team`/`Lane` enums are
  replaced by `shared::map::{Team, Lane}`; the hand-written converters in
  `world.rs`, `career_runtime.rs`, `prematch.rs`, `match_allocation.rs` and
  `practice_tests.rs` became identity and were deleted.

## What stays local
- `client::team::Team` and `client::net::StructureKind` remain Bevy
  `Component`s (used by ~50 client files as ECS tags). They convert at the
  network boundary with `From` in both directions and `PartialEq` across the
  pair; they are not serialized.
- Server balance for team buffs (`duration`, `damage_multiplier`,
  `hp_regen_per_second`) lives in a server-only `TeamBuffBalance` trait on the
  shared `TeamBuffKind`. `TeamBuffKind::ALL`/`index`, `NeutralCampType::is_boss`
  /`team_buff_kind`, `From<JungleCampKind>` and `TargetId::player` moved to
  shared because they are format facts, not gameplay tuning.
- Client-only decode leniency (`team`, `hp`, `mana`, `level`, `next_level_xp`,
  minion `state`, projectile `owner_team`, snapshot vectors) is kept as
  `serde(default)` on the shared type, so old fixtures and servers still decode;
  serialization is unchanged because defaults never skip fields.

## Golden verification
Before touching the server, a throwaway test printed the exact JSON the
0.23.0-rc.6 server produced for a fully populated `Snapshot`, `set_god_mode`,
`transform`, `basic_attack`, `join`, `social` and `career`. Those strings are
pinned in `shared/src/protocol/wire.rs` tests (`golden_strings_match_the_server_before_the_move`)
together with a decode-and-re-encode check, a round-trip test for every
`ClientPacket` (18 variants) and `ServerPacket` (3 variants) variant, the legacy
defaults, and `HeroClass::ALL` round-tripping including `warden`.

## Rule
Wire types live only in `shared/src/protocol/wire.rs`; never copy them into a
crate. Add a field there, with a `serde(default)` when it is additive, and
extend the golden test.
