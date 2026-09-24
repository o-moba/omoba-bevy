//! Server-side entity records: connected players, projectiles, structures,
//! minions, neutrals, team buffs and the ECS mirror components.

use shared::wire::StructureState;

use crate::balance::BOTTOM_BOSS_BUFF_DAMAGE_MULT;

use shared::map::Lane;

use shared::wire::TargetId;

use crate::hero_timers::HeroTimers;

use crate::balance::TOP_BOSS_BUFF_DAMAGE_MULT;

use crate::balance::PLAYER_GROUND_Y;

use crate::hero_timers;

use shared::wire::PlayerState;

use shared::wire::TeamBuffState;

use crate::balance::BOTTOM_BOSS_BUFF_DURATION;

use crate::balance::TOP_BOSS_BUFF_HP_REGEN_PER_SECOND;

use crate::hero::HeroEconomy;

use crate::prematch;

use shared::wire::MinionTargetKind;

use shared::wire::GameState;

use std::time::Duration;

use shared::wire::MinionState;

use crate::hero_stats;

use crate::hero_stats::StatModifiers;

use shared::wire::NeutralState;

use crate::shop::shop_is_available;

use shared::wire::ProjectileState;

use std::time::Instant;

use shared::shop::ItemBonuses;

use crate::balance::TOP_BOSS_BUFF_DURATION;

use shared::wire::TeamBuffKind;

use crate::hero::Hero;

use shared::SkillSlot;

use shared::map::Team;

/// Server-side balance of each replicated boss buff (`shared::wire::TeamBuffKind`).
pub(crate) trait TeamBuffBalance {
    fn duration(self) -> Duration;
    fn damage_multiplier(self) -> f32;
    fn hp_regen_per_second(self) -> f32;
}

impl TeamBuffBalance for TeamBuffKind {
    fn duration(self) -> Duration {
        match self {
            TeamBuffKind::WendigoFavor => BOTTOM_BOSS_BUFF_DURATION,
            TeamBuffKind::MutatioMight => TOP_BOSS_BUFF_DURATION,
        }
    }

    fn damage_multiplier(self) -> f32 {
        match self {
            TeamBuffKind::WendigoFavor => BOTTOM_BOSS_BUFF_DAMAGE_MULT,
            TeamBuffKind::MutatioMight => TOP_BOSS_BUFF_DAMAGE_MULT,
        }
    }

    fn hp_regen_per_second(self) -> f32 {
        match self {
            TeamBuffKind::WendigoFavor => 0.0,
            TeamBuffKind::MutatioMight => TOP_BOSS_BUFF_HP_REGEN_PER_SECOND,
        }
    }
}

pub(crate) fn team_index(team: Team) -> usize {
    match team {
        Team::Green => 0,
        Team::Blue => 1,
    }
}

/// Authoritative active team buffs keyed by (team, kind) with absolute expiry
/// instants. A re-kill refreshes the expiry (no stacking of the same kind);
/// different kinds combine multiplicatively for damage.
#[derive(Default)]
pub(crate) struct TeamBuffs {
    /// `expires[team_index][kind_index]`
    pub(crate) expires: [[Option<Instant>; TeamBuffKind::ALL.len()]; 2],
}

impl TeamBuffs {
    pub(crate) fn grant(&mut self, team: Team, kind: TeamBuffKind, now: Instant) {
        self.expires[team_index(team)][kind.index()] = Some(now + kind.duration());
    }

    pub(crate) fn is_active(&self, team: Team, kind: TeamBuffKind, now: Instant) -> bool {
        self.expires[team_index(team)][kind.index()].is_some_and(|expiry| now < expiry)
    }

    /// Combined outgoing ability-damage multiplier for a team (1.0 = no buff).
    pub(crate) fn damage_multiplier(&self, team: Team, now: Instant) -> f32 {
        TeamBuffKind::ALL
            .iter()
            .filter(|kind| self.is_active(team, **kind, now))
            .map(|kind| kind.damage_multiplier())
            .product()
    }

    /// Combined flat HP regen per second for a team (0.0 = no buff).
    pub(crate) fn hp_regen_per_second(&self, team: Team, now: Instant) -> f32 {
        TeamBuffKind::ALL
            .iter()
            .filter(|kind| self.is_active(team, **kind, now))
            .map(|kind| kind.hp_regen_per_second())
            .sum()
    }

    pub(crate) fn clear(&mut self) {
        self.expires = Default::default();
    }

    /// Snapshot representation of every active buff (deterministic order).
    pub(crate) fn snapshot(&self, now: Instant) -> Vec<TeamBuffState> {
        let mut out = Vec::new();
        for team in [Team::Green, Team::Blue] {
            for kind in TeamBuffKind::ALL {
                if let Some(expiry) = self.expires[team_index(team)][kind.index()] {
                    if now < expiry {
                        out.push(TeamBuffState {
                            team,
                            kind,
                            remaining_secs: expiry.duration_since(now).as_secs_f32(),
                        });
                    }
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Vec3f {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
}

impl Vec3f {
    pub(crate) fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub(crate) fn add_scaled(self, velocity: Self, dt: f32) -> Self {
        Self {
            x: self.x + velocity.x * dt,
            y: self.y + velocity.y * dt,
            z: self.z + velocity.z * dt,
        }
    }

    pub(crate) fn distance_squared(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        dx * dx + dy * dy + dz * dz
    }

    pub(crate) fn distance(self, other: Self) -> f32 {
        self.distance_squared(other).sqrt()
    }

    pub(crate) fn normalize_or_zero(self) -> Self {
        let len_sq = self.x * self.x + self.y * self.y + self.z * self.z;
        if len_sq <= 0.000_001 {
            Self::new(0.0, 0.0, 0.0)
        } else {
            let inv_len = len_sq.sqrt().recip();
            Self::new(self.x * inv_len, self.y * inv_len, self.z * inv_len)
        }
    }
}

pub(crate) struct ConnectedPlayer {
    pub(crate) career_profile: Option<shared::career::ProfileSummary>,
    pub(crate) career_capable: bool,
    pub(crate) draft: prematch::DraftState,
    /// False until the endpoint sends a `Join` packet. Pre-join endpoints are
    /// kept for addressing (snapshots are still sent to them) but are excluded
    /// from the replicated player list and from all gameplay simulation.
    pub(crate) joined: bool,
    pub(crate) session_id: Option<String>,
    pub(crate) framed_snapshots: bool,
    pub(crate) protocol_compatible: bool,
    pub(crate) join_error: Option<shared::protocol::JoinRejection>,
    /// Transport liveness; not a gameplay clock.
    pub(crate) last_seen: Instant,
    /// Authoritative gameplay clocks (`hero_timers`).
    pub(crate) timers: HeroTimers,
    /// Development and sandbox overrides on top of class, level and gear
    /// (`hero_stats`); `Default` is normal play. Not networked.
    pub(crate) modifiers: StatModifiers,
    /// Authoritative hero core (`hero`).
    pub(crate) hero: Hero,
    /// Authoritative wallet and inventory (`hero`).
    pub(crate) economy: HeroEconomy,
}

impl ConnectedPlayer {
    /// The replicated `PlayerState` as the owning client sees it: the
    /// authoritative `hero` and `economy` plus the fields derived at
    /// replication time (the cooldown and utility clocks from `hero_timers`,
    /// shop availability). This and `public_view` are the only places that
    /// build a `PlayerState`.
    pub(crate) fn owner_view(
        &self,
        now: Instant,
        map: &MapLayoutState,
        phase: &GameState,
    ) -> PlayerState {
        let Self { hero, economy, .. } = self;
        PlayerState {
            supporter_aura: hero.identity.supporter_aura,
            is_bot: hero.identity.is_bot,
            id: hero.identity.id,
            x: hero.x,
            y: hero.y,
            z: hero.z,
            yaw: hero.yaw,
            team: hero.identity.team,
            hp: hero.hp,
            max_hp: hero.max_hp,
            mana: hero.mana,
            max_mana: hero.max_mana,
            gold: economy.gold,
            earned_gold: economy.earned_gold,
            utility: shared::utility::UtilityState {
                dash_remaining_secs: hero_timers::dash_remaining(self, now),
                haste_remaining_secs: hero_timers::haste_remaining(self, now),
                haste_active_secs: hero_timers::haste_active(self, now),
                last_request_id: hero.utility.last_request_id,
                dash_sequence: hero.utility.dash_sequence,
            },
            inventory: economy.inventory.clone(),
            item_bonuses: hero_stats::combat_bonuses(self),
            shop_available: shop_is_available(hero, map, phase),
            last_purchase: economy.last_purchase.clone(),
            basic_attack_cooldown_secs: hero_timers::basic_attack_cooldown(self),
            basic_attack_remaining_secs: hero_timers::basic_attack_remaining(self, now),
            skill_cooldown_remaining_secs: std::array::from_fn(|i| {
                hero_timers::skill_cooldown_remaining(self, SkillSlot::ALL[i], now)
            }),
            skill_recovery_remaining_secs: hero_timers::skill_recovery_remaining(self, now),
            basic_attack_request_id: economy.basic_attack_request_id,
            xp: hero.progress.xp,
            level: hero.progress.level,
            next_level_xp: hero.progress.next_level_xp,
            skill_points: hero.progress.skill_points,
            ranks: hero.progress.ranks,
            character: hero.identity.character,
            hero_class: hero.identity.hero_class,
            avatar: hero.identity.avatar.clone(),
            sprite_character: hero.identity.sprite_character.clone(),
            action_sequence: hero.last_action.sequence,
            action_kind: hero.last_action.kind,
            action_slot: hero.last_action.slot,
        }
    }

    /// The replicated `PlayerState` as every other client sees it, teammates
    /// included: the owner view with the private economy blanked (wallet,
    /// income, inventory, gear bonuses, purchase receipt) and the owner's
    /// request marks (basic-attack and utility request ids) zeroed. Level,
    /// XP, ranks and the cooldown copies stay public. Every blanked field is
    /// `#[serde(default)]` on the wire, so this is protocol-compatible.
    pub(crate) fn public_view(
        &self,
        now: Instant,
        map: &MapLayoutState,
        phase: &GameState,
    ) -> PlayerState {
        let view = self.owner_view(now, map, phase);
        PlayerState {
            gold: 0,
            earned_gold: 0,
            inventory: Vec::new(),
            item_bonuses: ItemBonuses::default(),
            last_purchase: None,
            basic_attack_request_id: 0,
            utility: shared::utility::UtilityState {
                last_request_id: 0,
                ..view.utility
            },
            ..view
        }
    }
}

pub(crate) struct DisconnectedSession {
    pub(crate) player: ConnectedPlayer,
    pub(crate) disconnected_at: Instant,
}

pub(crate) struct Projectile {
    pub(crate) state: ProjectileState,
    pub(crate) target: TargetId,
    pub(crate) velocity: Vec3f,
    pub(crate) homing: bool,
    pub(crate) guaranteed_hit: bool,
    pub(crate) damage: f32,
    pub(crate) radius: f32,
    pub(crate) expires_at: Instant,
}

pub(crate) struct Structure {
    pub(crate) state: StructureState,
    pub(crate) role: StructureRole,
    pub(crate) last_attack_at: Option<Instant>,
    pub(crate) attack_range: f32,
    pub(crate) attack_damage: f32,
    pub(crate) hero_damage_multiplier: f32,
    pub(crate) attack_cooldown: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StructureRole {
    LaneTower { lane: Lane },
    BaseTower,
}

pub(crate) struct Minion {
    pub(crate) state: MinionState,
    pub(crate) path: Vec<Vec3f>,
    pub(crate) next_waypoint: usize,
    pub(crate) last_attack_at: Option<Instant>,
    pub(crate) aggro_target: Option<MinionAggroTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MinionAggroTarget {
    Player(u64),
    Minion(u64),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NeutralTemplate {
    pub(crate) max_hp: f32,
    pub(crate) attack_damage: f32,
    pub(crate) attack_range: f32,
    pub(crate) kill_gold: u32,
    pub(crate) kill_xp: u32,
}

pub(crate) struct Neutral {
    pub(crate) state: NeutralState,
    pub(crate) anchor: Vec3f,
    pub(crate) target_player_id: Option<u64>,
    pub(crate) last_attack_at: Option<Instant>,
    pub(crate) dead_until: Option<Instant>,
}

impl MinionAggroTarget {
    pub(crate) fn id(self) -> u64 {
        match self {
            MinionAggroTarget::Player(id) | MinionAggroTarget::Minion(id) => id,
        }
    }

    pub(crate) fn kind(self) -> MinionTargetKind {
        match self {
            MinionAggroTarget::Player(_) => MinionTargetKind::Player,
            MinionAggroTarget::Minion(_) => MinionTargetKind::Minion,
        }
    }
}

pub(crate) struct MapLayoutState {
    pub(crate) home: Vec3f,
    pub(crate) away: Vec3f,
    pub(crate) min_x: f32,
    pub(crate) max_x: f32,
    pub(crate) min_z: f32,
    pub(crate) max_z: f32,
    #[cfg(test)]
    pub(crate) left_x: f32,
    #[cfg(test)]
    pub(crate) right_x: f32,
    #[cfg(test)]
    pub(crate) top_z: f32,
    #[cfg(test)]
    pub(crate) bottom_z: f32,
}

impl MapLayoutState {
    pub(crate) fn clamp_player_position(&self, position: Vec3f) -> Vec3f {
        Vec3f::new(
            position.x.clamp(self.min_x, self.max_x),
            PLAYER_GROUND_Y,
            position.z.clamp(self.min_z, self.max_z),
        )
    }
}
