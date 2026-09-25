//! Socket-free practice, using the normal client snapshot/render/input pipeline.
//! This deliberately has no career backend, matchmaking, rewards, or persistence.
use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};
use std::collections::{HashMap, VecDeque};

use shared::combat::{CombatEntityKind, CombatEvent, ProjectileStyle};
use shared::debug::{
    DUMMY_DISTANCE, DUMMY_MAX_HP, DebugAccess, DebugCommand, OFFLINE_PRACTICE_MODE,
};
use shared::map::Team;
use shared::protocol::{JoinRejection, SnapshotMeta};
use shared::wire::{
    ClientPacket, GameState, PlayerState, ProjectileState, ServerPacket, TargetId, TargetKind,
};
use shared::{HeroClass, PlayerActionKind};

use crate::maps::MapLayout;

use super::session::ClientSession;
use super::transport::NetThreadSignal;
use shared::{SkillSlot, TargetingMode, hero_balance as balance, shop::ItemBonuses, utility::*};

pub(super) const ADDRESS: &str = "offline-practice";
const LOCAL_ID: u64 = 1;
const LEVEL: u32 = 6; // Every class slot is available for character testing.
use balance::{BOT_ENGAGE_RANGE as VISION, PROJECTILE_SPEED};
use shared::math::hero_yaw_towards as yaw_towards;
const BOT_RESPAWN_SECS: f32 = 3.0;
const LOCAL_RESPAWN_SECS: f32 = balance::RESPAWN_DELAY_SECS as f32;

pub(super) fn shipped_avatar(slug: Option<&str>) -> bool {
    slug.is_none_or(|s| {
        omoba_passport::avatars::avatar_roster()
            .iter()
            .any(|a| a.slug == s && a.passport.is_none())
    })
}

#[derive(Resource)]
pub(super) struct LocalPractice {
    commands: Receiver<ClientPacket>,
    snapshots: Sender<ServerPacket>,
    _signals: Sender<NetThreadSignal>,
    simulation: Simulation,
}
impl LocalPractice {
    pub(super) fn new(
        commands: Receiver<ClientPacket>,
        snapshots: Sender<ServerPacket>,
        signals: Sender<NetThreadSignal>,
    ) -> Self {
        Self {
            commands,
            snapshots,
            _signals: signals,
            simulation: Simulation::default(),
        }
    }
}

pub(super) fn step(practice: Option<ResMut<LocalPractice>>, time: Res<Time>) {
    let Some(mut practice) = practice else {
        return;
    };
    while let Ok(packet) = practice.commands.try_recv() {
        practice.simulation.command(packet);
    }
    practice.simulation.advance(time.delta_secs().min(0.1));
    let snapshot = practice.simulation.snapshot();
    let _ = practice.snapshots.try_send(snapshot);
}

struct Shot {
    state: ProjectileState,
    target: TargetId,
    damage: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum BotKind {
    /// The default playground: one hero per class circling near the base.
    Ring { slot: usize },
    /// Sandbox target: stands on its spot, never moves or attacks.
    Dummy,
    /// Sandbox 1v1 opponent: walks mid toward the local hero and fights back.
    Duelist,
}
struct Bot {
    kind: BotKind,
    /// Mid-lane waypoints from the enemy base toward the local base.
    route: Vec<[f32; 2]>,
    waypoint: usize,
}
#[derive(Default)]
struct Simulation {
    players: Vec<PlayerState>,
    bots: HashMap<u64, Bot>,
    shots: Vec<Shot>,
    events: VecDeque<CombatEvent>,
    /// Kills and deaths per player id, for the live scoreboard.
    stats: HashMap<u64, (u32, u32)>,
    tick: u64,
    sequence: u64,
    next_id: u64,
    elapsed: f32,
    respawn: HashMap<u64, f32>,
    god_mode: bool,
    error: Option<JoinRejection>,
}

fn hero(
    id: u64,
    class: HeroClass,
    avatar: Option<String>,
    x: f32,
    z: f32,
    team: Team,
) -> PlayerState {
    // Keep protocol defaults in one place; explicitly set practice-only facts.
    serde_json::from_value(serde_json::json!({
        "id":id, "is_bot":id != LOCAL_ID, "x":x, "y":MapLayout::default().terrain_height_3d(x,z)+0.5, "z":z, "yaw":0.0,
        "team":team, "hero_class":class, "avatar":avatar, "level":LEVEL,
        "hp":balance::max_hp_for_level(class, LEVEL, 0.0), "max_hp":balance::max_hp_for_level(class, LEVEL, 0.0),
        "mana":balance::max_mana_for_level(LEVEL, 0.0), "max_mana":balance::max_mana_for_level(LEVEL, 0.0), "ranks":[1,1,1,1],
        "basic_attack_cooldown_secs":balance::basic_cooldown(class, LEVEL, ItemBonuses::NONE).as_secs_f32()
    })).expect("static practice player contract")
}
impl Simulation {
    fn command(&mut self, packet: ClientPacket) {
        if let Some(command) = DebugCommand::from_packet(&packet) {
            self.debug(command);
            return;
        }
        match packet {
            ClientPacket::Join {
                character,
                hero_class,
                avatar,
                sprite_character,
                ..
            } => {
                if !self.players.is_empty() {
                    return;
                }
                if !shipped_avatar(avatar.as_deref()) {
                    self.error = Some(JoinRejection::AvatarNotAuthorized);
                    return;
                }
                self.error = None;
                let [x, z] = shared::map::geometry().home;
                let mut local = hero(LOCAL_ID, hero_class, avatar, x + 6.0, z + 6.0, Team::Green);
                local.character = character;
                local.sprite_character = sprite_character;
                self.players.push(local);
                self.next_id = LOCAL_ID + 1;
                self.spawn_ring();
            }
            ClientPacket::Leave => {
                self.players.clear();
                self.bots.clear();
                self.shots.clear();
                self.events.clear();
                self.respawn.clear();
                self.stats.clear();
                self.god_mode = false;
            }
            ClientPacket::Transform {
                x,
                y,
                z,
                yaw,
                dash_sequence,
            } => {
                if let Some(p) = self.players.first_mut() {
                    if [x, y, z, yaw].iter().all(|v| v.is_finite())
                        && dash_sequence == p.utility.dash_sequence
                    {
                        p.x = x;
                        p.y = y;
                        p.z = z;
                        p.yaw = yaw;
                    }
                }
            }
            ClientPacket::BasicAttack {
                target, request_id, ..
            } => {
                let Some(p) = self.players.first_mut() else {
                    return;
                };
                if request_id <= p.basic_attack_request_id {
                    return;
                }
                p.basic_attack_request_id = request_id;
                if p.basic_attack_remaining_secs > 0.0 {
                    return;
                }
                let def = shared::basic_attack_for_class(p.hero_class);
                let damage = balance::basic_damage(p.hero_class, LEVEL, ItemBonuses::NONE);
                if self.valid_target(target, def.range) {
                    self.players[0].basic_attack_remaining_secs =
                        self.players[0].basic_attack_cooldown_secs;
                    self.attack(target, damage, None);
                }
            }
            ClientPacket::Cast { target, slot } => {
                let Some(index) = SkillSlot::from_index(slot) else {
                    return;
                };
                let Some(p) = self.players.first() else {
                    return;
                };
                let def = shared::ability_for_class_slot(p.hero_class, index);
                let rank = p.ranks[slot as usize].clamp(1, def.max_rank);
                let mana_cost = shared::scaled_mana_cost(def, rank);
                if p.hp <= 0.0
                    || p.skill_cooldown_remaining_secs[slot as usize] > 0.0
                    || p.skill_recovery_remaining_secs > 0.0
                    || p.mana < mana_cost
                {
                    return;
                }
                if def.targeting == TargetingMode::UnitTarget
                    && !self.valid_target(target, def.cast_range)
                {
                    return;
                }
                // The server's effect scale: rank and level power apply to
                // heals, mana restores and damage alike (`sim/cast.rs`).
                let effect_scale = shared::rank_effect_scale(rank)
                    * balance::ability_power_multiplier(p.hero_class, LEVEL);
                let p = &mut self.players[0];
                p.mana = (p.mana - mana_cost + def.self_mana_restore.unwrap_or(0.0) * effect_scale)
                    .min(p.max_mana);
                p.hp = (p.hp + def.self_heal.unwrap_or(0.0) * effect_scale).min(p.max_hp);
                p.skill_cooldown_remaining_secs[slot as usize] =
                    balance::ability_cooldown(p.hero_class, LEVEL, rank, index, ItemBonuses::NONE)
                        .as_secs_f32();
                p.skill_recovery_remaining_secs = balance::skill_recovery_secs(LEVEL);
                p.action_sequence += 1;
                p.action_kind = PlayerActionKind::for_cast(index);
                p.action_slot = slot;
                if let Some(damage) = def.projectile_damage {
                    self.attack(target, damage * effect_scale, Some(slot));
                }
            }
            ClientPacket::Utility {
                action,
                direction,
                request_id,
                ..
            } => {
                let Some(p) = self.players.first_mut() else {
                    return;
                };
                if request_id <= p.utility.last_request_id {
                    return;
                }
                p.utility.last_request_id = request_id;
                match action {
                    UtilityAction::Dash if p.utility.dash_remaining_secs <= 0.0 => {
                        let v = Vec2::from_array(direction);
                        if !v.is_finite() || v.length_squared() < 0.01 {
                            return;
                        }
                        // Use the same authored navigation as regular movement.
                        let target = Vec2::new(p.x, p.z) + v.normalize() * DASH_DISTANCE;
                        let target = shared::navigation::world_navigation()
                            .clip_movement([p.x, p.z], target.to_array());
                        p.x = target[0];
                        p.z = target[1];
                        p.utility.dash_sequence += 1;
                        p.utility.dash_remaining_secs = DASH_COOLDOWN_SECS;
                    }
                    UtilityAction::Haste if p.utility.haste_remaining_secs <= 0.0 => {
                        p.utility.haste_active_secs = HASTE_DURATION_SECS;
                        p.utility.haste_remaining_secs = HASTE_COOLDOWN_SECS;
                    }
                    _ => {}
                }
            }
            // No purchase, auth, signed career, network, or ranked result path exists here.
            _ => {}
        }
    }
    /// Every debug command offline practice accepts (all of them: offline
    /// is always practice).
    fn debug(&mut self, command: DebugCommand) {
        match command {
            DebugCommand::GodMode(enabled) => {
                if self.players.is_empty() {
                    return;
                }
                self.god_mode = enabled;
                if enabled {
                    let p = &mut self.players[0];
                    p.hp = p.max_hp;
                    p.mana = p.max_mana;
                    self.respawn.remove(&LOCAL_ID);
                }
            }
            // Deliberately a no-op: the offline simulation accepts any finite
            // client transform (see `Transform` above), so the boosted local
            // movement already works and there is no speed clamp to raise.
            DebugCommand::SpeedBoost(_) => {}
            DebugCommand::Practice(command) => self.practice(command),
        }
    }
    fn valid_target(&self, target: TargetId, range: f32) -> bool {
        target.kind == TargetKind::Player
            && self.players.iter().any(|p| {
                p.id == target.id
                    && p.id != LOCAL_ID
                    && p.hp > 0.0
                    && Vec2::new(p.x - self.players[0].x, p.z - self.players[0].z).length()
                        <= range + shared::PLAYER_TARGET_RADIUS
            })
    }
    fn attack(&mut self, target: TargetId, damage: f32, slot: Option<u8>) {
        self.shoot(0, target, damage, slot);
    }
    /// A homing shot from `players[owner]`; the cosmetic action sequence
    /// advances for basic strikes (casts advance it where mana is paid).
    fn shoot(&mut self, owner: usize, target: TargetId, damage: f32, slot: Option<u8>) {
        let p = &mut self.players[owner];
        if slot.is_none() {
            p.action_sequence += 1;
            p.action_kind = PlayerActionKind::Attack;
            p.action_slot = shared::BASIC_ATTACK_ACTION_SLOT;
        }
        self.sequence += 1;
        self.shots.push(Shot {
            state: ProjectileState {
                id: self.sequence,
                owner_id: p.id,
                owner_team: p.team,
                source_kind: CombatEntityKind::Player,
                style: ProjectileStyle::for_class(p.hero_class),
                action_slot: slot,
                direction: [0.0; 3],
                x: p.x,
                y: p.y + 0.8,
                z: p.z,
            },
            target,
            damage,
        });
    }
    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id.max(LOCAL_ID + 1);
        self.next_id = id + 1;
        id
    }
    fn spawn_ring(&mut self) {
        let [x, z] = shared::map::geometry().home;
        for (i, class) in HeroClass::ALL.into_iter().enumerate() {
            let avatar = omoba_passport::avatars::avatar_roster()
                .iter()
                .filter(|a| a.passport.is_none())
                .nth(i)
                .map(|a| a.slug.to_owned());
            let id = self.alloc_id();
            self.players.push(hero(
                id,
                class,
                avatar,
                x + 11.0 + i as f32 * 3.0,
                z + 11.0,
                Team::Blue,
            ));
            self.bots.insert(
                id,
                Bot {
                    kind: BotKind::Ring { slot: i },
                    route: Vec::new(),
                    waypoint: 0,
                },
            );
        }
    }
    fn remove_bots(&mut self) {
        self.players.retain(|p| p.id == LOCAL_ID);
        self.bots.clear();
        self.respawn.retain(|id, _| *id == LOCAL_ID);
        self.shots.retain(|s| s.state.owner_id == LOCAL_ID);
    }
    fn remove_bot(&mut self, id: u64) {
        self.players.retain(|p| p.id != id);
        self.bots.remove(&id);
        self.respawn.remove(&id);
    }
    /// Pause-menu sandbox: the same commands the practice server accepts.
    fn practice(&mut self, command: shared::practice::PracticeCommand) {
        use shared::practice::{MAX_DUMMIES, PracticeCommand};
        if self.players.is_empty() {
            return;
        }
        match command {
            PracticeCommand::Roster => {
                self.remove_bots();
                self.spawn_ring();
            }
            PracticeCommand::ClearBots => self.remove_bots(),
            PracticeCommand::SpawnDummy => {
                let mut dummies: Vec<_> = self
                    .bots
                    .iter()
                    .filter(|(_, b)| b.kind == BotKind::Dummy)
                    .map(|(id, _)| *id)
                    .collect();
                dummies.sort_unstable();
                if dummies.len() >= MAX_DUMMIES {
                    self.remove_bot(dummies[0]);
                }
                let local = &self.players[0];
                let origin = [local.x, local.z];
                let geometry = shared::map::geometry();
                let dir = Vec2::new(
                    geometry.away[0] - geometry.home[0],
                    geometry.away[1] - geometry.home[1],
                )
                .normalize_or(Vec2::ONE);
                let nav = shared::navigation::world_navigation();
                let mut anchor = origin;
                for distance in [DUMMY_DISTANCE, DUMMY_DISTANCE * 0.6, 2.0] {
                    let candidate = nav.clip_movement(
                        origin,
                        [origin[0] + dir.x * distance, origin[1] + dir.y * distance],
                    );
                    if (candidate[0] - origin[0]).hypot(candidate[1] - origin[1]) > 1.0 {
                        anchor = candidate;
                        break;
                    }
                }
                let id = self.alloc_id();
                let mut dummy = hero(
                    id,
                    HeroClass::Warrior,
                    None,
                    anchor[0],
                    anchor[1],
                    Team::Blue,
                );
                dummy.max_hp = DUMMY_MAX_HP;
                dummy.hp = DUMMY_MAX_HP;
                dummy.yaw = yaw_towards(origin[0] - anchor[0], origin[1] - anchor[1]);
                self.players.push(dummy);
                self.bots.insert(
                    id,
                    Bot {
                        kind: BotKind::Dummy,
                        route: Vec::new(),
                        waypoint: 0,
                    },
                );
            }
            PracticeCommand::StartDuel { level, gold } => {
                self.remove_bots();
                let level = level.clamp(1, balance::MAX_LEVEL);
                let class = self.players[0].hero_class;
                let [x, z] = shared::map::geometry().away;
                let id = self.alloc_id();
                let mut duelist = hero(id, class, None, x - 6.0, z - 6.0, Team::Blue);
                // The budget tops up the ordinary starting wallet, like a bot
                // that earned `gold` before shopping at its base. Ranks and
                // items follow the same shared plans as the server's duelist.
                let budget = shared::shop::STARTING_GOLD + gold;
                let inventory = shared::shop::plan_purchases(class, budget, &[]);
                let spent: u32 = inventory
                    .iter()
                    .map(|item| shared::shop::item(*item).cost)
                    .sum();
                let bonuses = shared::shop::item_bonuses(&inventory);
                let mut ranks = [1_u8; 4];
                let order =
                    shared::progression::skill_upgrade_order(class, level, ranks, level - 1);
                for slot in order {
                    ranks[slot as usize] += 1;
                }
                duelist.level = level;
                duelist.ranks = ranks;
                duelist.inventory = inventory;
                duelist.item_bonuses = bonuses;
                duelist.gold = budget - spent;
                duelist.max_hp = balance::max_hp_for_level(class, level, bonuses.max_hp);
                duelist.hp = duelist.max_hp;
                duelist.max_mana = balance::max_mana_for_level(level, bonuses.max_mana);
                duelist.mana = duelist.max_mana;
                duelist.basic_attack_cooldown_secs =
                    balance::basic_cooldown(class, level, bonuses).as_secs_f32();
                duelist.yaw = yaw_towards(-x, -z);
                self.players.push(duelist);
                // Blue walks the authored mid road in reverse, toward Green's base.
                let mut route: Vec<[f32; 2]> = shared::map::lane_points(shared::map::Lane::Mid);
                route.reverse();
                self.bots.insert(
                    id,
                    Bot {
                        kind: BotKind::Duelist,
                        route,
                        waypoint: 0,
                    },
                );
            }
            // A newer client's command: nothing to do.
            PracticeCommand::Unsupported => {}
        }
    }
    /// One duelist decision per tick: engage the local hero in sight, else
    /// follow the mid road; strikes and casts use the shared growth tables.
    fn drive_duelist(&mut self, index: usize, dt: f32) {
        let local_alive = self.players[0].hp > 0.0;
        let (lx, lz) = (self.players[0].x, self.players[0].z);
        let bot = &self.players[index];
        let id = bot.id;
        let class = bot.hero_class;
        let level = bot.level;
        let bonuses = bot.item_bonuses;
        let ranks = bot.ranks;
        let (bx, bz) = (bot.x, bot.z);
        let to_local = Vec2::new(lx - bx, lz - bz);
        let distance = to_local.length();
        let engaged = local_alive && distance <= VISION;
        let reach = shared::basic_attack_for_class(class).range + shared::PLAYER_TARGET_RADIUS;
        let speed = crate::player::PLAYER_SPEED
            * balance::movement_multiplier(class, level)
            * bonuses.move_speed_multiplier
            * dt;
        let target = TargetId {
            kind: TargetKind::Player,
            id: LOCAL_ID,
        };
        if engaged && distance <= reach - 0.2 {
            let bot = &mut self.players[index];
            bot.yaw = yaw_towards(to_local.x, to_local.y);
            if bot.basic_attack_remaining_secs <= 0.0 {
                bot.basic_attack_remaining_secs = bot.basic_attack_cooldown_secs;
                let damage = balance::basic_damage(class, level, bonuses);
                self.shoot(index, target, damage, None);
            }
        } else {
            let goal = if engaged {
                Some([lx, lz])
            } else {
                let bot = self.bots.get_mut(&id).unwrap();
                while bot.waypoint + 1 < bot.route.len()
                    && (bot.route[bot.waypoint][0] - bx).hypot(bot.route[bot.waypoint][1] - bz)
                        < 2.0
                {
                    bot.waypoint += 1;
                }
                bot.route
                    .get(bot.waypoint)
                    .copied()
                    .filter(|point| (point[0] - bx).hypot(point[1] - bz) > 1.0)
            };
            if let Some(goal) = goal {
                let dir = Vec2::new(goal[0] - bx, goal[1] - bz).normalize_or_zero();
                let next = shared::navigation::world_navigation()
                    .clip_movement([bx, bz], [bx + dir.x * speed, bz + dir.y * speed]);
                let bot = &mut self.players[index];
                let (dx, dz) = (next[0] - bot.x, next[1] - bot.z);
                if dx.hypot(dz) > 0.0001 {
                    bot.yaw = yaw_towards(dx, dz);
                }
                bot.x = next[0];
                bot.z = next[1];
                bot.y = MapLayout::default().terrain_height_3d(next[0], next[1]) + 0.5;
            }
        }
        if !engaged {
            return;
        }
        // Unlocked hostile-target skills, on the shared cooldown and mana rules.
        let unlocked = shared::unlocked_slots_for_level(level);
        for slot in 0..4_u8 {
            let index_slot = SkillSlot::from_index(slot).unwrap();
            let def = shared::ability_for_class_slot(class, index_slot);
            let rank = ranks[slot as usize].clamp(1, def.max_rank);
            let Some(base_damage) = def.projectile_damage else {
                continue;
            };
            let bot = &self.players[index];
            if !unlocked[slot as usize]
                || def.targeting != TargetingMode::UnitTarget
                || bot.skill_cooldown_remaining_secs[slot as usize] > 0.0
                || bot.skill_recovery_remaining_secs > 0.0
                || bot.mana < shared::scaled_mana_cost(def, rank)
                || distance > shared::scaled_cast_range(def, rank) + shared::PLAYER_TARGET_RADIUS
            {
                continue;
            }
            let bot = &mut self.players[index];
            bot.mana -= shared::scaled_mana_cost(def, rank);
            bot.skill_cooldown_remaining_secs[slot as usize] =
                balance::ability_cooldown(class, level, rank, index_slot, bonuses).as_secs_f32();
            bot.skill_recovery_remaining_secs = balance::skill_recovery_secs(level);
            bot.action_sequence += 1;
            bot.action_kind = PlayerActionKind::for_cast(index_slot);
            bot.action_slot = slot;
            bot.yaw = yaw_towards(to_local.x, to_local.y);
            let damage = base_damage
                * balance::ability_power_multiplier(class, level)
                * shared::rank_effect_scale(rank)
                * bonuses.damage_multiplier;
            self.shoot(index, target, damage, Some(slot));
            break;
        }
    }
    fn advance(&mut self, dt: f32) {
        self.elapsed += dt;
        for p in &mut self.players {
            for timer in [
                &mut p.basic_attack_remaining_secs,
                &mut p.skill_recovery_remaining_secs,
                &mut p.utility.dash_remaining_secs,
                &mut p.utility.haste_remaining_secs,
                &mut p.utility.haste_active_secs,
            ]
            .into_iter()
            .chain(p.skill_cooldown_remaining_secs.iter_mut())
            {
                *timer = (*timer - dt).max(0.0);
            }
            // The dead do not regenerate, as on the server.
            if p.hp > 0.0 {
                p.mana = (p.mana + balance::MANA_REGEN_PER_SECOND * dt).min(p.max_mana);
            }
            if p.id == LOCAL_ID {
                if self.god_mode {
                    p.hp = p.max_hp;
                    self.respawn.remove(&LOCAL_ID);
                } else if p.hp <= 0.0 {
                    let timer = self.respawn.entry(LOCAL_ID).or_insert(LOCAL_RESPAWN_SECS);
                    *timer -= dt;
                    if *timer <= 0.0 {
                        // Home spawn, like the authoritative respawn; the client
                        // snaps to a correction this large.
                        let [x, z] = shared::map::geometry().home;
                        p.x = x + 6.0;
                        p.z = z + 6.0;
                        p.y = MapLayout::default().terrain_height_3d(p.x, p.z) + 0.5;
                        p.hp = p.max_hp;
                        p.mana = p.max_mana;
                        self.respawn.remove(&LOCAL_ID);
                    }
                }
                continue;
            }
            let kind = self.bots.get(&p.id).map(|b| b.kind);
            if p.hp <= 0.0 {
                let timer = self.respawn.entry(p.id).or_insert(BOT_RESPAWN_SECS);
                *timer -= dt;
                if *timer <= 0.0 {
                    p.hp = p.max_hp;
                    p.mana = p.max_mana;
                    if kind == Some(BotKind::Duelist) {
                        let [x, z] = shared::map::geometry().away;
                        p.x = x - 6.0;
                        p.z = z - 6.0;
                        if let Some(bot) = self.bots.get_mut(&p.id) {
                            bot.waypoint = 0;
                        }
                    }
                    self.respawn.remove(&p.id);
                }
            } else if let Some(BotKind::Ring { slot }) = kind {
                let [x, z] = shared::map::geometry().home;
                // The nearest target stands still for melee; others demonstrate
                // running, and face the way they run (models look along -Z).
                if slot > 0 {
                    let phase = self.elapsed * 0.9 + p.id as f32;
                    p.x = x + 11.0 + slot as f32 * 3.0 + phase.sin() * 2.0;
                    p.z = z + 11.0 + phase.cos() * 2.0;
                    p.yaw = yaw_towards(phase.cos(), -phase.sin());
                }
            }
        }
        let duelists: Vec<usize> = self
            .players
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.hp > 0.0 && self.bots.get(&p.id).map(|b| b.kind) == Some(BotKind::Duelist)
            })
            .map(|(i, _)| i)
            .collect();
        for index in duelists {
            self.drive_duelist(index, dt);
        }
        let shots = std::mem::take(&mut self.shots);
        for mut shot in shots {
            let Some(p) = self
                .players
                .iter_mut()
                .find(|p| p.id == shot.target.id && p.hp > 0.0)
            else {
                continue;
            };
            let position = Vec3::new(shot.state.x, shot.state.y, shot.state.z);
            let target = Vec3::new(p.x, p.y + 0.8, p.z);
            let delta = target - position;
            if delta.length() <= PROJECTILE_SPEED * dt + 0.3 {
                if p.id == LOCAL_ID && self.god_mode {
                    continue;
                }
                let amount = shot.damage.min(p.hp);
                p.hp -= amount;
                let killed = p.hp <= 0.0;
                let (victim, owner) = (p.id, shot.state.owner_id);
                if killed {
                    self.stats.entry(victim).or_default().1 += 1;
                    if owner != victim {
                        self.stats.entry(owner).or_default().0 += 1;
                    }
                }
                let p = &self.players[self.players.iter().position(|q| q.id == victim).unwrap()];
                self.sequence += 1;
                self.events.push_back(CombatEvent {
                    id: self.sequence,
                    source: shared::combat::CombatEntity {
                        kind: CombatEntityKind::Player,
                        id: owner,
                    },
                    target: shared::combat::CombatEntity {
                        kind: CombatEntityKind::Player,
                        id: victim,
                    },
                    amount,
                    x: p.x,
                    y: p.y + 0.8,
                    z: p.z,
                    style: shot.state.style,
                    action_slot: shot.state.action_slot,
                    killed,
                });
                if self.events.len() > 32 {
                    self.events.pop_front();
                }
            } else {
                let direction = delta.normalize();
                let next = position + direction * PROJECTILE_SPEED * dt;
                shot.state.x = next.x;
                shot.state.y = next.y;
                shot.state.z = next.z;
                shot.state.direction = direction.to_array();
                self.shots.push(shot);
            }
        }
    }
    fn snapshot(&mut self) -> ServerPacket {
        self.tick += 1;
        ServerPacket::Snapshot {
            meta: SnapshotMeta::new(u64::MAX, 1, self.tick),
            geometry_id: shared::map::GEOMETRY_ID.into(),
            map_profile: "verdant".into(),
            match_mode: OFFLINE_PRACTICE_MODE.into(),
            join_error: self.error,
            your_id: LOCAL_ID,
            players: self.players.clone(),
            projectiles: self.shots.iter().map(|s| s.state.clone()).collect(),
            combat_events: self.events.iter().cloned().collect(),
            game_state: if self.players.is_empty() {
                GameState::Lobby
            } else {
                GameState::Running
            },
            sandbox: None,
            // The offline simulation is its own host: it accepts both the
            // toggles and the practice commands (`Simulation::debug`).
            debug_access: Some(DebugAccess::for_match_mode(OFFLINE_PRACTICE_MODE)),
            vision: None,
            forest_pickups: vec![],
            scoreboard: (!self.players.is_empty()).then(|| shared::live_score::LiveScoreboard {
                players: self
                    .players
                    .iter()
                    .map(|p| {
                        let (kills, deaths) = self.stats.get(&p.id).copied().unwrap_or_default();
                        shared::live_score::LiveScorePlayer {
                            player_id: p.id,
                            nickname: if p.id == LOCAL_ID {
                                "You".into()
                            } else {
                                format!("Bot {}", p.id)
                            },
                            team: p.team,
                            hero_class: p.hero_class,
                            kills,
                            deaths,
                            assists: 0,
                            earned_gold: 0,
                            level: p.level,
                            connected: true,
                        }
                    })
                    .collect(),
            }),
            prematch: None,
            structures: vec![],
            minions: vec![],
            neutrals: vec![],
            team_buffs: vec![],
            rematch_in_secs: None,
        }
    }
}

#[derive(Component)]
pub(super) struct PracticeBanner;
pub(super) fn setup_banner(mut commands: Commands) {
    commands.spawn((
        Text::new("OFFLINE PRACTICE · Level 6 · No rewards"),
        TextFont {
            font_size: 13.0,
            ..default()
        },
        TextColor(crate::ui::theme::GOLD),
        BackgroundColor(crate::ui::theme::PANEL),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(68.0),
            left: Val::Percent(35.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
            display: Display::None,
            ..default()
        },
        ZIndex(22),
        PracticeBanner,
        Name::new("OfflinePracticeBanner"),
    ));
}
pub(super) fn sync_banner(
    session: Res<ClientSession>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    mut banners: Query<&mut Node, With<PracticeBanner>>,
) {
    let visible = session.is_offline()
        && screen.is_some_and(|s| *s.get() == crate::frontend::AppScreen::InMatch);
    for mut node in &mut banners {
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::transport::{NetworkChannels, spawn_network_transport};
    use crate::persistence::ResolvedServerAddressForPrefs;
    fn joined(class: HeroClass) -> Simulation {
        let mut sim = Simulation::default();
        sim.command(ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: default(),
            hero_class: class,
            avatar: Some("agnes".into()),
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        });
        sim
    }
    fn target() -> TargetId {
        TargetId {
            kind: TargetKind::Player,
            id: 2,
        }
    }
    #[test]
    fn every_class_can_move_cast_attack_and_recover_targets_without_io() {
        for class in HeroClass::ALL {
            let mut sim = joined(class);
            assert_eq!(sim.players.len(), 1 + HeroClass::ALL.len());
            assert_eq!(sim.players[0].avatar.as_deref(), Some("agnes"));
            let (x, y, z) = (sim.players[1].x, sim.players[1].y, sim.players[1].z - 2.0);
            sim.command(ClientPacket::Transform {
                x,
                y,
                z,
                yaw: 1.0,
                dash_sequence: 0,
            });
            assert_eq!(sim.players[0].z, z);
            sim.command(ClientPacket::BasicAttack {
                target: target(),
                server_epoch: u64::MAX,
                match_id: 1,
                request_id: 1,
            });
            assert_eq!(sim.shots.len(), 1);
            // Duplicate requests and cooldown spam must not create extra attacks.
            sim.command(ClientPacket::BasicAttack {
                target: target(),
                server_epoch: u64::MAX,
                match_id: 1,
                request_id: 1,
            });
            assert_eq!(sim.shots.len(), 1);
            for _ in 0..30 {
                sim.advance(0.05);
            }
            assert!(sim.players[1].hp < sim.players[1].max_hp);
            assert!(sim.events.iter().any(|e| e.amount > 0.0));
            for slot in 0..4 {
                sim.players[0].mana = 100.0;
                sim.players[0].hp = 80.0;
                let old = sim.players[0].action_sequence;
                sim.command(ClientPacket::Cast {
                    target: target(),
                    slot,
                });
                assert!(
                    sim.players[0].action_sequence > old,
                    "class {class:?} slot {slot}"
                );
                for _ in 0..12 {
                    sim.advance(0.05);
                }
            }
            sim.players[1].hp = 0.0;
            for _ in 0..80 {
                sim.advance(0.05);
            }
            assert_eq!(sim.players[1].hp, sim.players[1].max_hp);
            let ServerPacket::Snapshot {
                game_state,
                match_mode,
                scoreboard,
                ..
            } = sim.snapshot()
            else {
                panic!()
            };
            assert_eq!(game_state, GameState::Running);
            assert_eq!(match_mode, "offline_practice");
            let board = scoreboard.expect("offline rounds keep a live scoreboard");
            assert_eq!(board.players.len(), 1 + HeroClass::ALL.len());
            assert!(board.players.iter().any(|p| p.player_id == LOCAL_ID));
            sim.command(ClientPacket::Leave);
            assert!(sim.players.is_empty());
            assert!(sim.shots.is_empty());
        }
    }
    /// O16: offline practice follows the server's formulas: "Level 6"
    /// heroes get level-6 pools, heals scale with rank and level power, Q
    /// plays as an Attack, and the dead do not regenerate mana.
    #[test]
    fn offline_formulas_match_the_server() {
        let mut healed = false;
        for class in HeroClass::ALL {
            let mut sim = joined(class);
            for p in &sim.players {
                assert_eq!(
                    p.max_hp,
                    balance::max_hp_for_level(p.hero_class, LEVEL, 0.0)
                );
                assert_eq!(p.hp, p.max_hp);
                assert_eq!(p.max_mana, balance::max_mana_for_level(LEVEL, 0.0));
            }
            assert!(sim.players[0].max_hp > balance::base_hp(class));

            for slot in 0..4_u8 {
                let index = SkillSlot::from_index(slot).unwrap();
                let def = shared::ability_for_class_slot(class, index);
                let local = &mut sim.players[0];
                local.mana = local.max_mana;
                local.hp = 1.0;
                local.skill_cooldown_remaining_secs = [0.0; 4];
                local.skill_recovery_remaining_secs = 0.0;
                let before = local.action_sequence;
                sim.command(ClientPacket::Cast {
                    target: target(),
                    slot,
                });
                let local = &sim.players[0];
                if local.action_sequence == before {
                    continue; // A unit-target skill out of range.
                }
                let expected = if index == SkillSlot::Q {
                    PlayerActionKind::Attack
                } else {
                    PlayerActionKind::Cast
                };
                assert_eq!(local.action_kind, expected, "{class:?} slot {slot}");
                if let Some(heal) = def.self_heal {
                    let scaled = heal * balance::ability_power_multiplier(class, LEVEL);
                    assert!((local.hp - (1.0 + scaled)).abs() < 1e-4, "{class:?} heal");
                    assert!(scaled > heal);
                    healed = true;
                }
            }

            // A dead ring bot keeps its mana until it respawns.
            sim.players[1].hp = 0.0;
            sim.players[1].mana = 10.0;
            sim.advance(0.1);
            assert_eq!(sim.players[1].mana, 10.0);
        }
        assert!(healed, "some class has a self heal");
    }
    #[test]
    fn utilities_are_local_and_late_pre_dash_transforms_cannot_undo_dash() {
        let mut sim = joined(HeroClass::Ranger);
        let old_x = sim.players[0].x;
        let old_z = sim.players[0].z;
        sim.command(ClientPacket::Utility {
            action: UtilityAction::Dash,
            direction: [1.0, 0.0],
            server_epoch: u64::MAX,
            match_id: 1,
            request_id: 1,
        });
        assert!(sim.players[0].x > old_x);
        assert_eq!(sim.players[0].utility.dash_sequence, 1);
        sim.command(ClientPacket::Transform {
            x: old_x,
            y: 0.5,
            z: old_z,
            yaw: 0.0,
            dash_sequence: 0,
        });
        assert!(sim.players[0].x > old_x);
        sim.command(ClientPacket::Utility {
            action: UtilityAction::Haste,
            direction: [0.0; 2],
            server_epoch: u64::MAX,
            match_id: 1,
            request_id: 2,
        });
        assert!(sim.players[0].utility.movement_multiplier() > 1.0);
        sim.advance(HASTE_DURATION_SECS + 0.1);
        assert_eq!(sim.players[0].utility.movement_multiplier(), 1.0);
    }
    #[test]
    fn offline_session_never_reuses_online_channels_or_overwrites_saved_address() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(ClientSession {
            offline_return_addr: Some("127.0.0.1:49999".into()),
            ephemeral_endpoint: true,
            ..default()
        });
        app.insert_resource(ResolvedServerAddressForPrefs("127.0.0.1:49999".into()));
        app.add_systems(
            Startup,
            |mut commands: Commands, mut session: ResMut<ClientSession>| {
                spawn_network_transport(&mut commands, &mut session, ADDRESS.into())
            },
        );
        app.add_systems(Update, step);
        app.update();
        assert!(app.world().contains_resource::<LocalPractice>());
        assert_eq!(
            app.world().resource::<ResolvedServerAddressForPrefs>().0,
            "127.0.0.1:49999"
        );
        let channels = app.world().resource::<NetworkChannels>();
        let snapshot = channels
            .incoming
            .try_recv()
            .expect("local snapshot without a listener or worker thread");
        assert!(matches!(
            snapshot,
            ServerPacket::Snapshot {
                game_state: GameState::Lobby,
                ..
            }
        ));
    }
    fn run(sim: &mut Simulation, seconds: f32) {
        let steps = (seconds / 0.05).ceil() as usize;
        for _ in 0..steps {
            sim.advance(0.05);
        }
    }
    fn strike(sim: &mut Simulation, id: u64, request_id: u64) {
        sim.command(ClientPacket::BasicAttack {
            target: TargetId {
                kind: TargetKind::Player,
                id,
            },
            server_epoch: u64::MAX,
            match_id: 1,
            request_id,
        });
    }
    fn kda(sim: &mut Simulation, id: u64) -> (u32, u32) {
        let ServerPacket::Snapshot { scoreboard, .. } = sim.snapshot() else {
            panic!()
        };
        let row = scoreboard
            .unwrap()
            .players
            .into_iter()
            .find(|p| p.player_id == id)
            .unwrap();
        (row.kills, row.deaths)
    }
    #[test]
    fn ring_bots_face_the_way_they_run_and_kills_reach_the_scoreboard() {
        let mut sim = joined(HeroClass::Ranger);
        let runner = sim.players[2].id;
        // The first tick moves the runner from its seat onto the ring path.
        sim.advance(0.05);
        let before = (sim.players[2].x, sim.players[2].z);
        sim.advance(0.05);
        let after = (sim.players[2].x, sim.players[2].z);
        let (dx, dz) = (after.0 - before.0, after.1 - before.1);
        let yaw = sim.players[2].yaw;
        // Local -Z rotated by the yaw must point along the step it just took.
        let forward = Vec2::new(-yaw.sin(), -yaw.cos());
        let step = Vec2::new(dx, dz).normalize();
        assert!(
            forward.dot(step) > 0.99,
            "runner {runner} faces {forward:?}, moves {step:?}"
        );

        // Stand next to the stationary target and finish it: one kill, one death.
        let target = sim.players[1].id;
        let (x, z) = (sim.players[1].x, sim.players[1].z - 2.0);
        sim.command(ClientPacket::Transform {
            x,
            y: 0.5,
            z,
            yaw: 0.0,
            dash_sequence: 0,
        });
        sim.players[1].hp = 1.0;
        strike(&mut sim, target, 1);
        run(&mut sim, 1.0);
        assert_eq!(kda(&mut sim, LOCAL_ID), (1, 0));
        assert_eq!(kda(&mut sim, target), (0, 1));
    }
    #[test]
    fn sandbox_dummy_clear_roster_and_god_mode_work_offline() {
        use shared::practice::{MAX_DUMMIES, PracticeCommand};
        let mut sim = joined(HeroClass::Mage);
        sim.command(ClientPacket::Practice {
            command: PracticeCommand::ClearBots,
        });
        assert_eq!(sim.players.len(), 1);
        for _ in 0..MAX_DUMMIES + 2 {
            sim.command(ClientPacket::Practice {
                command: PracticeCommand::SpawnDummy,
            });
        }
        assert_eq!(sim.players.len(), 1 + MAX_DUMMIES);
        let local = (sim.players[0].x, sim.players[0].z);
        let dummy = sim.players.last().unwrap();
        let distance = (dummy.x - local.0).hypot(dummy.z - local.1);
        assert!(
            (1.0..=DUMMY_DISTANCE + 0.01).contains(&distance),
            "dummy at {distance}"
        );
        assert_eq!(dummy.max_hp, DUMMY_MAX_HP);
        let anchor = (dummy.x, dummy.z);
        run(&mut sim, 2.0);
        let dummy = sim.players.last().unwrap();
        assert_eq!((dummy.x, dummy.z), anchor, "dummies never move");
        assert!(sim.shots.is_empty(), "dummies never attack");

        sim.command(ClientPacket::SetGodMode { enabled: true });
        sim.players[0].hp = 0.0;
        sim.advance(0.05);
        assert_eq!(
            sim.players[0].hp, sim.players[0].max_hp,
            "god mode keeps the hero up"
        );

        sim.command(ClientPacket::Practice {
            command: PracticeCommand::Roster,
        });
        assert_eq!(sim.players.len(), 1 + HeroClass::ALL.len());
        assert!(
            sim.bots
                .values()
                .all(|b| matches!(b.kind, BotKind::Ring { .. }))
        );
    }
    #[test]
    fn duel_opponent_is_configured_walks_mid_and_fights_back() {
        use shared::practice::PracticeCommand;
        let mut sim = joined(HeroClass::Warrior);
        sim.command(ClientPacket::Practice {
            command: PracticeCommand::StartDuel {
                level: 7,
                gold: 500,
            },
        });
        assert_eq!(sim.players.len(), 2);
        let duelist = &sim.players[1];
        assert_eq!(
            duelist.hero_class,
            HeroClass::Warrior,
            "mirrors the local class"
        );
        assert_eq!(duelist.level, 7);
        assert_eq!(duelist.ranks, [3, 3, 1, 3]);
        assert_eq!(duelist.inventory.len(), shared::shop::INVENTORY_CAPACITY);
        assert!(
            duelist.max_hp
                > balance::base_hp(HeroClass::Warrior) + 6.0 * balance::LEVEL_UP_HP_BONUS
        );
        let start = (duelist.x, duelist.z);
        let [ax, az] = shared::map::geometry().away;
        assert!(
            (start.0 - ax).abs() < 8.0 && (start.1 - az).abs() < 8.0,
            "spawns at the enemy base"
        );
        // Far from the local hero it follows the mid road toward Green's base.
        run(&mut sim, 3.0);
        let moved = &sim.players[1];
        let travelled = (moved.x - start.0).hypot(moved.z - start.1);
        assert!(travelled > 5.0, "duelist advanced {travelled}");
        let [hx, hz] = shared::map::geometry().home;
        assert!(
            (moved.x - hx).hypot(moved.z - hz) < (start.0 - hx).hypot(start.1 - hz),
            "toward the local base"
        );
        // Standing in reach, it strikes and can kill the local hero, who respawns.
        let (x, z) = (sim.players[1].x, sim.players[1].z + 2.0);
        sim.command(ClientPacket::Transform {
            x,
            y: 0.5,
            z,
            yaw: 0.0,
            dash_sequence: 0,
        });
        sim.players[0].hp = 1.0;
        run(&mut sim, 2.0);
        assert!(sim.players[1].action_sequence > 0, "the duelist attacked");
        assert_eq!(kda(&mut sim, LOCAL_ID), (0, 1));
        let duelist_id = sim.players[1].id;
        assert_eq!(kda(&mut sim, duelist_id), (1, 0));
        run(&mut sim, LOCAL_RESPAWN_SECS + 0.2);
        let local = &sim.players[0];
        assert_eq!(local.hp, local.max_hp, "local hero respawned");
        assert!((local.x - hx - 6.0).abs() < 0.01, "at the home spawn");
    }
    #[test]
    fn unknown_or_store_only_avatars_cannot_trigger_download_or_admission() {
        assert!(!shipped_avatar(Some("not-bundled")));
        assert!(shipped_avatar(None));
        let mut sim = Simulation::default();
        sim.command(ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: default(),
            hero_class: HeroClass::Mage,
            avatar: Some("not-bundled".into()),
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        });
        assert!(sim.players.is_empty());
        assert_eq!(sim.error, Some(JoinRejection::AvatarNotAuthorized));
    }
}
