//! Replicated ECS components and the snapshot resources other client modules read.

use bevy::prelude::*;
use std::collections::HashMap;

use shared::combat::{CombatEntityKind, CombatEvent, MinionKind, ProjectileStyle};
use shared::protocol::SnapshotMeta;
use shared::wire::{PlayerState, StructureState};
use shared::{HeroClass, PlayerActionKind};

pub use crate::domain::RemotePlayer;
use crate::team::{CharacterChoice, Team};

use super::{GameState, Lane, MinionBrainState, NeutralAiState, NeutralCampType, TeamBuffState};

#[derive(Resource, Default, Clone)]
pub struct GameStateSnapshot {
    pub sandbox: Option<shared::sandbox::SandboxSnapshot>,
    pub forest_pickups: Vec<shared::forest_pickups::ForestPickupState>,
    pub vision: Option<shared::vision::TeamVision>,
    pub your_id: u64,
    pub prematch: Option<shared::prematch::PrematchSnapshot>,
    pub match_mode: String,
    pub geometry_id: String,
    pub map_profile: String,
    pub meta: SnapshotMeta,
    pub state: GameState,
    pub rematch_in_secs: Option<u64>,
    /// Active boss team buffs replicated from the server.
    pub team_buffs: Vec<TeamBuffState>,
    /// Recent confirmed hits; presentation deduplicates by epoch, match and event id.
    pub combat_events: Vec<CombatEvent>,
    /// Current-round authoritative ledger. None means unavailable/legacy server.
    pub scoreboard: Option<shared::live_score::LiveScoreboard>,
}

#[derive(Resource, Default)]
pub(in crate::net) struct NetworkState {
    pub(in crate::net) local_id: Option<u64>,
    pub(in crate::net) local_team: Option<Team>,
    pub(in crate::net) remote_players: HashMap<u64, Entity>,
    pub(in crate::net) projectiles: HashMap<u64, Entity>,
    pub(in crate::net) structures: HashMap<u64, Entity>,
    pub(in crate::net) minions: HashMap<u64, Entity>,
    pub(in crate::net) neutrals: HashMap<u64, Entity>,
    /// Mirror of `DebugSpeedBoost`, so snapshot reconcile can widen the snap
    /// threshold while boosting without exceeding the 16-param system limit.
    pub(in crate::net) speed_boost_active: bool,
    pub(in crate::net) local_dash_ack: Option<(u64, u64, u64, u64)>,
}

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkPlayerId(pub u64);

#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct NetworkBot(pub bool);

/// Server-authoritative utility timers and the accepted dash acknowledgment.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerUtility {
    pub state: shared::utility::UtilityState,
}

impl From<&PlayerState> for PlayerUtility {
    fn from(player: &PlayerState) -> Self {
        Self {
            state: player.utility,
        }
    }
}

/// Latest server strike deadline and replay acknowledgment. Reconcile local
/// feedback from snapshots; skill cooldown mirrors remain independent.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerBasicAttackCooldown {
    pub duration_secs: f32,
    pub remaining_secs: f32,
    pub last_request_id: u64,
}

impl From<&PlayerState> for PlayerBasicAttackCooldown {
    fn from(player: &PlayerState) -> Self {
        let duration_secs = if player.basic_attack_cooldown_secs.is_finite() {
            player.basic_attack_cooldown_secs.max(0.0)
        } else {
            0.0
        };
        let remaining_secs = if player.basic_attack_remaining_secs.is_finite() {
            player.basic_attack_remaining_secs.clamp(0.0, duration_secs)
        } else {
            0.0
        };
        Self {
            duration_secs,
            remaining_secs,
            last_request_id: player.basic_attack_request_id,
        }
    }
}

/// Server skill deadlines, including the shared interval between different slots.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PlayerSkillCooldowns {
    pub remaining_secs: [f32; 4],
    pub recovery_secs: f32,
}

impl From<&PlayerState> for PlayerSkillCooldowns {
    fn from(player: &PlayerState) -> Self {
        let finite = |value: f32| {
            if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            }
        };
        Self {
            remaining_secs: player.skill_cooldown_remaining_secs.map(finite),
            recovery_secs: finite(player.skill_recovery_remaining_secs),
        }
    }
}

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkCharacterChoice(pub CharacterChoice);

/// Roster avatar slug replicated from the server (`None` = legacy character model).
#[derive(Component, Clone, Debug)]
pub struct NetworkAvatar(pub Option<String>);

/// Sprite character id replicated from the server (`None` = manifest default).
#[derive(Component, Clone, Debug)]
pub struct NetworkSpriteCharacter(pub Option<String>);

/// Latest authoritative cosmetic action for a player. The sequence lets
/// renderers distinguish a new cast from repeated delivery in snapshots.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerCosmeticAction {
    pub sequence: u64,
    pub kind: PlayerActionKind,
    pub slot: u8,
}

impl From<&PlayerState> for PlayerCosmeticAction {
    fn from(player: &PlayerState) -> Self {
        Self {
            sequence: player.action_sequence,
            kind: player.action_kind,
            slot: player.action_slot,
        }
    }
}

/// Authoritative hero class replicated from the server.
#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkHeroClass(pub HeroClass);

/// Equipment is authoritative, including receipt and current base eligibility.
#[derive(Component, Clone, Debug, Default)]
pub struct PlayerEquipment {
    pub gold: u32,
    pub inventory: Vec<shared::shop::ItemId>,
    pub item_bonuses: shared::shop::ItemBonuses,
    pub shop_available: bool,
    pub last_purchase: Option<shared::shop::PurchaseReceipt>,
}

#[derive(Component, Clone, Copy, Debug)]
pub struct PlayerProgression {
    pub sandbox_unlocked: Option<[bool; 4]>,
    pub level: u32,
    pub xp: u32,
    pub next_level_xp: u32,
    pub skill_points: u32,
    /// Per-slot ability ranks (1-based) mirrored from the server snapshot (TASK03).
    pub ranks: [u8; 4],
}

impl PlayerProgression {
    pub fn unlocked(&self) -> [bool; 4] {
        self.sandbox_unlocked
            .unwrap_or_else(|| shared::unlocked_slots_for_level(self.level.max(1)))
    }
}

impl Default for PlayerProgression {
    fn default() -> Self {
        Self {
            sandbox_unlocked: None,
            level: 1,
            xp: 0,
            next_level_xp: 0,
            skill_points: 0,
            ranks: [1; 4],
        }
    }
}

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkProjectile {
    pub id: u64,
    pub owner_id: u64,
    pub source_kind: CombatEntityKind,
    pub style: ProjectileStyle,
    pub action_slot: Option<u8>,
    pub direction: Vec3,
    #[allow(dead_code)] // Consumed by the optional 2D presentation plugin.
    pub owner_team: Team,
}

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkStructureId(pub u64);

/// Server map-object identity; presentation uses a packaged profile with safe defaults.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkMapStructure {
    pub lane: Option<Lane>,
    pub tier: u8,
    pub key: String,
    pub visual_profile: String,
}

impl From<&StructureState> for NetworkMapStructure {
    fn from(value: &StructureState) -> Self {
        Self {
            lane: value.lane,
            tier: value.tier,
            key: value.map_key.clone(),
            visual_profile: value.visual_profile.clone(),
        }
    }
}

#[derive(Component)]
pub struct NetworkStructure;

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct NetworkStructureProtected(pub bool);

/// ECS tag for a replicated structure, like [`crate::team::Team`]. The wire
/// enum is `shared::wire::StructureKind`; convert only at the network boundary.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StructureKind {
    Tower,
    BaseTower,
}

impl From<shared::wire::StructureKind> for StructureKind {
    fn from(kind: shared::wire::StructureKind) -> Self {
        match kind {
            shared::wire::StructureKind::Tower => Self::Tower,
            shared::wire::StructureKind::BaseTower => Self::BaseTower,
        }
    }
}

impl From<StructureKind> for shared::wire::StructureKind {
    fn from(kind: StructureKind) -> Self {
        match kind {
            StructureKind::Tower => Self::Tower,
            StructureKind::BaseTower => Self::BaseTower,
        }
    }
}

#[derive(Component)]
pub struct NetworkMinion;

/// Replicated minion AI state, mirrored onto the entity so the minion
/// presentation systems can animate walking and attacks.
#[derive(Component, Clone, Copy)]
pub struct NetworkMinionBrainState(pub MinionBrainState);

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct NetworkMinionKind(pub MinionKind);

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct NetworkMinionAction(pub u64);

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkMinionId(pub u64);

#[derive(Component)]
pub struct NetworkNeutral;

#[derive(Component, Clone, Copy, Debug)]
pub struct NetworkNeutralId(pub u64);

/// Authoritative camp identity retained for creature visuals and minimap status.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkNeutralCampType(pub NeutralCampType);

/// Replicated neutral AI state (drives the boss idle/walk animation switch).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeutralAiStateTag(pub NeutralAiState);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replicated_basic_timing_defaults_safely_and_is_separate_from_q_action() {
        let legacy = json!({"id":1,"x":0.0,"y":0.5,"z":0.0,"yaw":0.0,"team":"green"});
        let mut player: PlayerState = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            PlayerBasicAttackCooldown::from(&player),
            PlayerBasicAttackCooldown::default()
        );
        player.basic_attack_cooldown_secs = 0.9;
        player.basic_attack_remaining_secs = 0.4;
        player.basic_attack_request_id = 8;
        player.action_kind = PlayerActionKind::Attack;
        player.action_slot = shared::BASIC_ATTACK_ACTION_SLOT;
        let decoded: PlayerState =
            serde_json::from_slice(&serde_json::to_vec(&player).unwrap()).unwrap();
        let cooldown = PlayerBasicAttackCooldown::from(&decoded);
        assert_eq!(
            cooldown,
            PlayerBasicAttackCooldown {
                duration_secs: 0.9,
                remaining_secs: 0.4,
                last_request_id: 8
            }
        );
        assert_eq!(PlayerCosmeticAction::from(&decoded).slot, u8::MAX);
        assert_eq!(decoded.ranks, [1; 4]);
    }
}
