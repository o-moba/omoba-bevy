//! In-process practice host. Its only inputs are admitted commands and elapsed time.
//! There is no transport, account backend, persistence, or wall-clock polling.
use crate::{
    bots::BotControllers,
    combat_feedback::CombatLog,
    game_world::GameWorld,
    host::CombatHost,
    match_rules::{MatchMode, MatchRules},
};
use shared::{
    debug::{DebugAccess, DebugCommand, OFFLINE_PRACTICE_MODE},
    protocol::{JoinRejection, SnapshotMeta},
    wire::{ClientPacket, GameState, ServerPacket},
};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};
pub const START_LEVEL: u32 = 6;
pub const EPOCH: u64 = u64::MAX;
pub const LOCAL_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0);
pub struct PracticeSession {
    pub world: GameWorld,
    pub bots: BotControllers,
    pub combat_log: CombatLog,
    pub now: Instant,
    pub match_id: u64,
    tick: u64,
    error: Option<JoinRejection>,
}
impl PracticeSession {
    pub fn new(now: Instant) -> Self {
        Self {
            world: GameWorld::new(Default::default(), now),
            bots: Default::default(),
            combat_log: Default::default(),
            now,
            match_id: 1,
            tick: 0,
            error: None,
        }
    }
    pub fn host(&mut self) -> CombatHost<'_> {
        CombatHost {
            world: &mut self.world,
            bots: &mut self.bots,
            combat_log: &mut self.combat_log,
            rules: MatchRules::for_mode(MatchMode::Practice, 5),
            match_id: self.match_id,
        }
    }
    pub fn reject_join(&mut self, reason: JoinRejection) {
        self.error = Some(reason);
    }
    /// Avatar entitlement/asset admission is supplied by the UI host before this call.
    pub fn command(&mut self, packet: ClientPacket) {
        if let Some(debug) = DebugCommand::from_packet(&packet) {
            match debug {
                DebugCommand::Practice(command) => {
                    let now = self.now;
                    self.host()
                        .handle_practice_command(LOCAL_ADDR, command, now);
                    self.register_scoreboard();
                }
                DebugCommand::GodMode(enabled) => {
                    if let Some(p) = self.world.players.get_mut(&LOCAL_ADDR) {
                        p.modifiers.god_mode = enabled;
                        p.modifiers.infinite_resource = enabled;
                        if enabled {
                            p.hero.hp = p.hero.max_hp;
                            p.hero.mana = p.hero.max_mana;
                            p.timers.respawn_at = None;
                        }
                    }
                }
                DebugCommand::SpeedBoost(enabled) => {
                    if let Some(p) = self.world.players.get_mut(&LOCAL_ADDR) {
                        p.modifiers.move_speed_mult = if enabled {
                            crate::balance::DEBUG_SPEED_MULTIPLIER
                        } else {
                            1.0
                        };
                    }
                }
            }
            return;
        }
        match packet {
            ClientPacket::Join {
                handheld,
                team,
                character,
                hero_class,
                avatar,
                sprite_character,
                ..
            } => {
                if self
                    .world
                    .players
                    .get(&LOCAL_ADDR)
                    .is_some_and(|p| p.joined)
                {
                    return;
                }
                self.error = None;
                self.world.ensure_connected(LOCAL_ADDR, self.now);
                let p = self.world.players.get_mut(&LOCAL_ADDR).unwrap();
                crate::session::handle_join_request_with_sprite(
                    p,
                    team,
                    character,
                    hero_class,
                    avatar.as_deref(),
                    sprite_character.as_deref(),
                    &self.world.map_layout,
                    self.now,
                );
                p.hero.identity.handheld = handheld;
                // Explicit practice configuration: all four slots available; authoritative stats/ranks.
                while p.hero.progress.level < START_LEVEL {
                    let xp = p.hero.progress.next_level_xp;
                    crate::progression::grant_player_xp(&mut p.hero, xp);
                }
                crate::bots::auto_rank_skills(p);
                p.hero.hp = p.hero.max_hp;
                p.hero.mana = p.hero.max_mana;
                self.world.game_state = GameState::Running;
                self.world.last_wave_spawn_at = self.now
                    - (crate::balance::MINION_WAVE_INTERVAL
                        - crate::balance::FIRST_MINION_WAVE_DELAY);
                crate::neutrals::schedule_boss_spawns(&mut self.world.neutrals, self.now);
                self.combat_log
                    .ledger
                    .begin(Vec::new())
                    .expect("fresh practice ledger");
                let now = self.now;
                self.host().fill_practice_bots(now);
                self.register_scoreboard();
            }
            ClientPacket::Leave => {
                let round = self.match_id + 1;
                *self = Self::new(self.now);
                self.match_id = round;
            }
            ClientPacket::RequestRematch => {
                let Some(player) = self.world.players.get(&LOCAL_ADDR) else {
                    return;
                };
                let identity = &player.hero.identity;
                let join = ClientPacket::Join {
                    handheld: identity.handheld.clone(),
                    prematch: false,
                    team: identity.team,
                    character: identity.character,
                    hero_class: identity.hero_class,
                    avatar: identity.avatar.clone(),
                    sprite_character: identity.sprite_character.clone(),
                    session_id: None,
                    passport_ticket: None,
                };
                let round = self.match_id + 1;
                *self = Self::new(self.now);
                self.match_id = round;
                self.command(join);
            }
            packet => {
                crate::command::apply(
                    &mut self.world,
                    LOCAL_ADDR,
                    &packet,
                    EPOCH,
                    self.match_id,
                    self.now,
                );
            }
        }
    }
    pub fn advance(&mut self, dt: f32) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.1)
        } else {
            0.0
        };
        self.now += Duration::from_secs_f32(dt);
        self.tick += 1;
        crate::tick::prepare(&mut self.world, &mut self.combat_log, self.now, dt);
        crate::tick::skills(&mut self.world, &mut self.combat_log, self.now, dt);
        let now = self.now;
        self.host().simulate_bots(now, dt);
        crate::tick::finish(
            &mut self.world,
            &mut self.combat_log,
            self.now,
            dt,
            dt,
            Default::default(),
        );
    }
    fn register_scoreboard(&mut self) {
        for p in self.world.players.values().filter(|p| p.joined) {
            let _ = self
                .combat_log
                .ledger
                .register(shared::career::ParticipantResult {
                    player_id: p.hero.identity.id,
                    is_bot: p.hero.identity.is_bot,
                    profile_id: None,
                    nickname: format!(
                        "{} {}",
                        if p.hero.identity.is_bot {
                            "Bot"
                        } else {
                            "Practice"
                        },
                        p.hero.identity.id
                    ),
                    team: p.hero.identity.team,
                    hero_class: p.hero.identity.hero_class,
                    character: "ipfs".into(),
                    avatar: p.hero.identity.avatar.clone(),
                    sprite_character: p.hero.identity.sprite_character.clone(),
                    stats: shared::career::MatchStats {
                        final_level: p.hero.progress.level,
                        ..Default::default()
                    },
                    disconnected: false,
                    rating: None,
                    progression_xp_gained: 0,
                });
        }
    }
    pub fn snapshot(&mut self) -> ServerPacket {
        let world = &self.world;
        let now = self.now;
        let local = world.players.get(&LOCAL_ADDR);
        let your_id = local.map_or(0, |p| p.hero.identity.id);
        for p in world.players.values().filter(|p| p.joined) {
            self.combat_log
                .ledger
                .update_player(p.hero.identity.id, p.hero.progress.level, false);
            self.combat_log
                .ledger
                .update_earned_gold(p.hero.identity.id, p.economy.earned_gold);
        }
        let mut structures: Vec<_> = world
            .structures
            .values()
            .filter(|s| s.state.hp > 0.0)
            .map(|s| {
                let mut s = s.state.clone();
                s.protected = crate::sim::towers::structure_is_protected(&world.structures, s.id);
                s
            })
            .collect();
        structures.sort_by_key(|s| s.id);
        let mut minions: Vec<_> = world
            .minions
            .values()
            .filter(|m| m.state.hp > 0.0)
            .map(|m| m.state.clone())
            .collect();
        minions.sort_by_key(|m| m.id);
        let mut neutrals: Vec<_> = world
            .neutrals
            .values()
            .filter(|n| n.dead_until.is_none() && n.state.hp > 0.0)
            .map(|n| n.state.clone())
            .collect();
        neutrals.sort_by_key(|n| n.id);
        let mut projectiles: Vec<_> = world
            .projectiles
            .values()
            .map(|p| p.state.clone())
            .collect();
        projectiles.sort_by_key(|p| p.id);
        let mut packet = ServerPacket::Snapshot {
            vision: None,
            sandbox: None,
            debug_access: local.filter(|p| p.joined).map(|_| DebugAccess {
                toggles: true,
                practice: true,
            }),
            match_mode: OFFLINE_PRACTICE_MODE.into(),
            geometry_id: world.map_config.geometry_id.clone(),
            map_profile: world.map_config.map_profile.clone(),
            meta: SnapshotMeta::new(EPOCH, self.match_id, self.tick),
            join_error: self.error,
            your_id,
            players: crate::snapshot::build_players_snapshot(world, Some(your_id), now),
            scoreboard: self.combat_log.ledger.live_scoreboard(),
            prematch: None,
            skill_effects: crate::skills::effects(world, now),
            projectiles,
            combat_events: self.combat_log.snapshot(now),
            structures,
            minions,
            neutrals,
            team_buffs: world.team_buffs.snapshot(now),
            forest_pickups: world.forest_pickups.snapshot(&world.game_state),
            game_state: world.game_state.clone(),
            rematch_in_secs: None,
        };
        if let Some(local) = local {
            crate::vision::filter_snapshot(&mut packet, local, world, now);
        }
        packet
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{
        HeroClass,
        map::Team,
        practice::PracticeCommand,
        wire::{CharacterChoice, TargetId, TargetKind},
    };
    fn join(class: HeroClass) -> ClientPacket {
        ClientPacket::Join {
            handheld: Default::default(),
            prematch: false,
            team: Team::Green,
            character: CharacterChoice::Ipfs,
            hero_class: class,
            avatar: None,
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        }
    }
    fn joined(class: HeroClass) -> PracticeSession {
        let mut p = PracticeSession::new(Instant::now());
        p.command(join(class));
        p.command(ClientPacket::Practice {
            command: PracticeCommand::ClearBots,
        });
        p
    }
    fn run(p: &mut PracticeSession, seconds: f32) {
        for _ in 0..(seconds / 0.05).ceil() as u32 {
            p.advance(0.05);
        }
    }
    #[test]
    fn every_class_joins_without_transport_and_publishes_real_kit() {
        for class in HeroClass::ALL {
            let mut p = joined(class);
            let hero = &p.world.players[&LOCAL_ADDR].hero;
            assert_eq!(hero.identity.hero_class, class);
            assert_eq!(hero.progress.level, START_LEVEL);
            assert!(hero.progress.ranks.iter().all(|rank| *rank > 0));
            assert_eq!(
                hero.skills.loadout,
                shared::loadout::preset_for_class(class)
            );
            let ServerPacket::Snapshot {
                players,
                debug_access,
                match_mode,
                ..
            } = p.snapshot()
            else {
                panic!()
            };
            assert_eq!(players.len(), 1);
            assert_eq!(match_mode, OFFLINE_PRACTICE_MODE);
            assert!(debug_access.unwrap().practice);
        }
    }
    #[test]
    fn practice_controls_leave_reentry_and_manual_clock_are_clean() {
        let mut p = joined(HeroClass::Dawnweaver);
        let before = p.now;
        p.advance(f32::NAN);
        assert_eq!(p.now, before);
        p.command(ClientPacket::Practice {
            command: PracticeCommand::SpawnDummy,
        });
        let dummy = p
            .world
            .players
            .values()
            .find(|p| p.hero.identity.is_bot)
            .unwrap();
        assert_eq!(dummy.hero.max_hp, shared::debug::DUMMY_MAX_HP);
        let aim = [dummy.hero.x, dummy.hero.z];
        p.command(ClientPacket::CastSkill {
            slot: 0,
            aim,
            server_epoch: EPOCH,
            match_id: 1,
            request_id: 1,
        });
        run(&mut p, 0.75);
        assert!(
            p.world
                .players
                .values()
                .find(|p| p.hero.identity.is_bot)
                .unwrap()
                .hero
                .hp
                < shared::debug::DUMMY_MAX_HP
        );
        p.command(ClientPacket::SetGodMode { enabled: true });
        assert!(p.world.players[&LOCAL_ADDR].modifiers.god_mode);
        p.command(ClientPacket::SetSpeedBoost { enabled: true });
        assert!(p.world.players[&LOCAL_ADDR].modifiers.move_speed_mult > 1.0);
        p.command(ClientPacket::Practice {
            command: PracticeCommand::StartDuel {
                level: 7,
                gold: 500,
            },
        });
        let bot = p
            .world
            .players
            .values()
            .find(|p| p.hero.identity.is_bot)
            .unwrap();
        assert_eq!(bot.hero.progress.level, 7);
        assert!(!bot.economy.inventory.is_empty());
        let pos = [bot.hero.x, bot.hero.z];
        run(&mut p, 1.0);
        let bot = p
            .world
            .players
            .values()
            .find(|p| p.hero.identity.is_bot)
            .unwrap();
        assert_ne!(pos, [bot.hero.x, bot.hero.z]);
        p.command(ClientPacket::Practice {
            command: PracticeCommand::Roster,
        });
        assert_eq!(p.world.players.len(), 10);
        p.command(ClientPacket::Leave);
        assert!(p.world.players.is_empty());
        assert!(p.combat_log.snapshot(p.now).is_empty());
        assert!(crate::skills::effects(&p.world, p.now).is_empty());
        p.command(join(HeroClass::Wildspark));
        let hero = &p.world.players[&LOCAL_ADDR];
        assert_eq!(hero.hero.skills.request_id, 0);
        assert!(!hero.modifiers.god_mode);
        assert!(hero.timers.last_cast_at.iter().all(Option::is_none));
        assert_eq!(p.match_id, 2);
    }
    #[test]
    fn rematch_reapplies_practice_config_and_clears_owned_effects_and_replay() {
        let mut p = joined(HeroClass::Dawnweaver);
        p.command(ClientPacket::Practice {
            command: PracticeCommand::SpawnDummy,
        });
        let dummy = p
            .world
            .players
            .values()
            .find(|p| p.hero.identity.is_bot)
            .unwrap();
        p.command(ClientPacket::CastSkill {
            slot: 2,
            aim: [dummy.hero.x, dummy.hero.z],
            server_epoch: EPOCH,
            match_id: 1,
            request_id: 9,
        });
        assert!(!crate::skills::effects(&p.world, p.now).is_empty());
        p.command(ClientPacket::RequestRematch);
        let local = &p.world.players[&LOCAL_ADDR];
        assert_eq!(local.hero.progress.level, START_LEVEL);
        assert!(local.hero.progress.ranks.iter().all(|r| *r > 0));
        assert_eq!(local.hero.skills.request_id, 0);
        assert!(local.timers.last_cast_at.iter().all(Option::is_none));
        assert!(crate::skills::effects(&p.world, p.now).is_empty());
        assert!(p.combat_log.snapshot(p.now).is_empty());
        assert_eq!(p.world.players.len(), 10);
        assert_eq!(p.match_id, 2);
        assert!(!p.bots.sandbox);
        assert_eq!(
            p.now.duration_since(p.world.last_wave_spawn_at),
            crate::balance::MINION_WAVE_INTERVAL - crate::balance::FIRST_MINION_WAVE_DELAY
        );
        p.command(ClientPacket::CastSkill {
            slot: 0,
            aim: [0.0, 0.0],
            server_epoch: EPOCH,
            match_id: 1,
            request_id: 10,
        });
        assert_eq!(
            p.world.players[&LOCAL_ADDR].hero.skills.request_id, 0,
            "previous-round request rejected"
        );
    }
    #[test]
    fn authoritative_movement_budget_and_dash_sequence_are_preserved_offline() {
        let mut p = joined(HeroClass::Ranger);
        p.world.structures.clear();
        let hero = &mut p.world.players.get_mut(&LOCAL_ADDR).unwrap().hero;
        hero.x = 10.0;
        hero.z = 0.0;
        run(&mut p, 0.1);
        p.command(ClientPacket::Transform {
            x: 10.5,
            y: 0.5,
            z: 0.0,
            yaw: 0.0,
            dash_sequence: 0,
        });
        assert!((p.world.players[&LOCAL_ADDR].hero.x - 10.5).abs() < 0.01);
        p.command(ClientPacket::Utility {
            action: shared::utility::UtilityAction::Dash,
            direction: [1.0, 0.0],
            server_epoch: EPOCH,
            match_id: 1,
            request_id: 1,
        });
        let after = p.world.players[&LOCAL_ADDR].hero.x;
        p.command(ClientPacket::Transform {
            x: 10.0,
            y: 0.5,
            z: 0.0,
            yaw: 0.0,
            dash_sequence: 0,
        });
        assert_eq!(p.world.players[&LOCAL_ADDR].hero.x, after);
    }
    #[test]
    fn offline_authoritative_damage_and_death_reach_every_world_category() {
        for kind in [
            TargetKind::Player,
            TargetKind::Minion,
            TargetKind::Structure,
            TargetKind::Neutral,
        ] {
            let mut p = joined(HeroClass::Warrior);
            let target = match kind {
                TargetKind::Player => {
                    p.command(ClientPacket::Practice {
                        command: PracticeCommand::SpawnDummy,
                    });
                    let bot = p
                        .world
                        .players
                        .values_mut()
                        .find(|p| p.hero.identity.is_bot)
                        .unwrap();
                    bot.hero.x = 13.0;
                    bot.hero.z = 0.0;
                    bot.hero.hp = 1.0;
                    TargetId {
                        kind,
                        id: bot.hero.identity.id,
                    }
                }
                TargetKind::Minion => {
                    crate::world::spawn_minion_wave_for_team_lane(
                        &p.world.map_layout,
                        &mut p.world.minions,
                        &mut p.world.next_minion_id,
                        Team::Blue,
                        shared::map::Lane::Mid,
                    );
                    let id = *p.world.minions.keys().min().unwrap();
                    p.world.minions.retain(|key, _| *key == id);
                    let m = p.world.minions.get_mut(&id).unwrap();
                    m.state.x = 13.0;
                    m.state.z = 0.0;
                    m.state.hp = 1.0;
                    TargetId { kind, id }
                }
                TargetKind::Structure => {
                    let id = p
                        .world
                        .structures
                        .values()
                        .filter(|s| {
                            s.state.team == Team::Blue
                                && s.state.kind == shared::wire::StructureKind::Tower
                        })
                        .map(|s| s.state.id)
                        .min()
                        .unwrap();
                    p.world.structures.retain(|key, _| *key == id);
                    let s = p.world.structures.get_mut(&id).unwrap();
                    s.state.x = 13.0;
                    s.state.z = 0.0;
                    s.state.hp = 1.0;
                    TargetId { kind, id }
                }
                TargetKind::Neutral => {
                    let id = *p.world.neutrals.keys().min().unwrap();
                    p.world.neutrals.retain(|key, _| *key == id);
                    let n = p.world.neutrals.get_mut(&id).unwrap();
                    n.state.x = 13.0;
                    n.state.z = 0.0;
                    n.anchor.x = 13.0;
                    n.anchor.z = 0.0;
                    n.state.hp = 1.0;
                    TargetId { kind, id }
                }
            };
            p.bots = Default::default();
            let local = p.world.players.get_mut(&LOCAL_ADDR).unwrap();
            local.hero.x = 11.0;
            local.hero.z = 0.0;
            p.command(ClientPacket::BasicAttack {
                target,
                server_epoch: EPOCH,
                match_id: 1,
                request_id: 1,
            });
            run(&mut p, 0.5);
            let dead = match kind {
                TargetKind::Player => {
                    p.world
                        .players
                        .values()
                        .find(|p| p.hero.identity.id == target.id)
                        .unwrap()
                        .hero
                        .hp
                        <= 0.0
                }
                TargetKind::Minion => !p.world.minions.contains_key(&target.id),
                TargetKind::Structure => p.world.structures[&target.id].state.hp <= 0.0,
                TargetKind::Neutral => p.world.neutrals[&target.id].dead_until.is_some(),
            };
            assert!(dead, "{kind:?}");
            assert!(
                p.combat_log
                    .snapshot(p.now)
                    .iter()
                    .any(|e| e.target.id == target.id && e.killed),
                "{kind:?} receipt"
            );
            if kind == TargetKind::Player {
                run(&mut p, crate::balance::RESPAWN_DELAY.as_secs_f32() + 0.1);
                assert!(
                    p.world
                        .players
                        .values()
                        .find(|p| p.hero.identity.id == target.id)
                        .unwrap()
                        .hero
                        .hp
                        > 0.0
                );
            }
        }
    }
}
