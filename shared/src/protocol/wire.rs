//! The one definition of the gameplay UDP/JSON wire protocol.
//!
//! Server, client and harness all speak this format; none of them keeps its
//! own copy. Field order is the JSON key order the server emits, and every
//! `serde(default)` is the leniency the client already relied on for legacy
//! snapshots and fixtures. Additive fields go here, once, with a default.
//!
//! Wire conventions:
//! - Packets use `#[serde(tag = "type", rename_all = "snake_case")]`
//!   (internally tagged: `{"type":"join", ...}`).
//! - Plain enums use `#[serde(rename_all = "snake_case")]`
//!   (e.g. `Team::Green` -> `"green"`).
//!
//! The golden tests at the bottom pin the exact strings the server produced
//! before this module existed; keep them passing when a field is added.

use serde::{Deserialize, Serialize};

use crate::combat::{CombatEntityKind, CombatEvent, MinionKind, ProjectileStyle};
use crate::jungle::JungleCampKind;
use crate::map::{Lane, Team};
use crate::protocol::{JoinRejection, SnapshotMeta};
use crate::shop::{ItemBonuses, ItemId, PurchaseReceipt};
use crate::{HeroClass, PlayerActionKind};

/// Legacy character carried by `Join` and replicated on every player. Owned
/// here (step 13) with the exact wire form of the Ekza SDK's `EkzaCharacter`
/// it replaced: snake_case ids, `ipfs` by default, unknown ids rejected. The
/// client converts it to the SDK type where it loads SDK models.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterChoice {
    #[default]
    Ipfs,
    Toka,
    Wang,
    Cube,
    /// CC0 VRM humanoid avatar (glTF 2.0 binary loaded through the glTF pipeline).
    Paco,
}

impl CharacterChoice {
    pub const ALL: [Self; 5] = [Self::Ipfs, Self::Toka, Self::Wang, Self::Cube, Self::Paco];

    /// Display label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ipfs => "IPFS",
            Self::Toka => "Toka",
            Self::Wang => "Wang",
            Self::Cube => "Cube",
            Self::Paco => "Paco",
        }
    }

    /// Stable id, identical to the wire form.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Ipfs => "ipfs",
            Self::Toka => "toka",
            Self::Wang => "wang",
            Self::Cube => "cube",
            Self::Paco => "paco",
        }
    }
}

/// Level a snapshot from a server that predates progression decodes to.
const LEGACY_PLAYER_LEVEL: u32 = crate::hero_balance::STARTING_LEVEL;
/// Next-level XP for the same legacy snapshots: the first real threshold.
const LEGACY_NEXT_LEVEL_XP: u32 = crate::hero_balance::LEVEL_XP_THRESHOLDS[0];
/// Mana pool used before per-class balance existed.
const LEGACY_MAX_MANA: f32 = crate::hero_balance::MAX_MANA;

/// Client -> server request datagram.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientPacket {
    Sandbox {
        request: crate::sandbox::SandboxRequest,
    },
    /// The player chose to leave the match or the queue. The seat is released
    /// immediately instead of being held for a reconnect; the endpoint itself
    /// stays connected for career, friends and the menus.
    Leave,
    Social {
        request: crate::social::SocialRequest,
    },
    /// Party presence, invites and launch. Ignored by match workers.
    Party {
        command: crate::party::PartyCommand,
    },
    Career {
        request: crate::career::CareerRequest,
    },
    Hello {
        protocol_version: u16,
    },
    Transform {
        #[serde(default)]
        dash_sequence: u64,
        x: f32,
        y: f32,
        z: f32,
        yaw: f32,
    },
    Cast {
        target: TargetId,
        /// Hotbar slot index (0=Q .. 3=R); the server resolves the ability
        /// from the caster's class kit. Defaults to Q for legacy packets.
        #[serde(default)]
        slot: u8,
    },
    Utility {
        action: crate::utility::UtilityAction,
        direction: [f32; 2],
        server_epoch: u64,
        match_id: u64,
        request_id: u64,
    },
    BasicAttack {
        target: TargetId,
        server_epoch: u64,
        match_id: u64,
        request_id: u64,
    },
    Join {
        #[serde(default)]
        prematch: bool,
        team: Team,
        #[serde(default = "default_character_choice")]
        character: CharacterChoice,
        /// Selected class; unknown wire values decode as the default class.
        #[serde(default)]
        hero_class: HeroClass,
        /// Cosmetic roster avatar slug; validated against the shipped roster.
        #[serde(default)]
        avatar: Option<String>,
        /// Optional sprite cosmetic; validated against the frozen shared roster.
        #[serde(default)]
        sprite_character: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        passport_ticket: Option<String>,
    },
    Prematch {
        request: crate::prematch::PrematchRequest,
    },
    Ping,
    RequestRematch,
    SetGodMode {
        enabled: bool,
    },
    SetSpeedBoost {
        enabled: bool,
    },
    /// Local practice sandbox (roster, dummies, 1v1). Ignored elsewhere.
    Practice {
        command: crate::practice::PracticeCommand,
    },
    UpgradeSkill {
        slot: u8,
    },
    BuyItem {
        item_id: String,
        request_id: u64,
        match_id: u64,
        server_epoch: u64,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Player,
    Minion,
    Structure,
    Neutral,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TargetId {
    pub kind: TargetKind,
    pub id: u64,
}

impl TargetId {
    /// Targets a player by id.
    pub const fn player(id: u64) -> Self {
        Self {
            kind: TargetKind::Player,
            id,
        }
    }
}

/// The legacy SDK character a packet without one decodes to.
pub fn default_character_choice() -> CharacterChoice {
    CharacterChoice::Ipfs
}

/// All ability ranks start at 1 (1-based; rank 1 = base power).
pub fn default_skill_ranks() -> [u8; 4] {
    [1; 4]
}

fn default_team() -> Team {
    Team::Green
}

fn default_hp() -> f32 {
    crate::hero_balance::base_hp(HeroClass::Warrior)
}

fn default_mana() -> f32 {
    LEGACY_MAX_MANA
}

fn default_player_level() -> u32 {
    LEGACY_PLAYER_LEVEL
}

fn default_next_level_xp() -> u32 {
    LEGACY_NEXT_LEVEL_XP
}

fn default_minion_brain_state() -> MinionBrainState {
    MinionBrainState::Marching
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    /// Cosmetic only, authorized from persisted profile grants by the game server.
    #[serde(default)]
    pub supporter_aura: Option<crate::supporter::AuraStyle>,
    #[serde(default)]
    pub is_bot: bool,
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    #[serde(default = "default_team")]
    pub team: Team,
    #[serde(default = "default_hp")]
    pub hp: f32,
    #[serde(default = "default_hp")]
    pub max_hp: f32,
    #[serde(default = "default_mana")]
    pub mana: f32,
    #[serde(default = "default_mana")]
    pub max_mana: f32,
    #[serde(default)]
    pub gold: u32,
    #[serde(default)]
    pub earned_gold: u32,
    #[serde(default)]
    pub utility: crate::utility::UtilityState,
    #[serde(default)]
    pub inventory: Vec<ItemId>,
    #[serde(default)]
    pub item_bonuses: ItemBonuses,
    #[serde(default)]
    pub shop_available: bool,
    #[serde(default)]
    pub last_purchase: Option<PurchaseReceipt>,
    #[serde(default)]
    pub basic_attack_cooldown_secs: f32,
    #[serde(default)]
    pub basic_attack_remaining_secs: f32,
    /// Authoritative skill clocks, including reconnect and sandbox changes.
    #[serde(default)]
    pub skill_cooldown_remaining_secs: [f32; 4],
    #[serde(default)]
    pub skill_recovery_remaining_secs: f32,
    /// Replay high-water mark, retained across reconnect and respawn in this round.
    #[serde(default)]
    pub basic_attack_request_id: u64,
    #[serde(default)]
    pub xp: u32,
    #[serde(default = "default_player_level")]
    pub level: u32,
    #[serde(default = "default_next_level_xp")]
    pub next_level_xp: u32,
    #[serde(default)]
    pub skill_points: u32,
    #[serde(default = "default_skill_ranks")]
    pub ranks: [u8; 4],
    #[serde(default = "default_character_choice")]
    pub character: CharacterChoice,
    /// Authoritative class assigned at join time (kit resolution key).
    #[serde(default)]
    pub hero_class: HeroClass,
    /// Cosmetic roster avatar slug replicated to every client; `None` means
    /// the legacy `character` model is used.
    #[serde(default)]
    pub avatar: Option<String>,
    /// Cosmetic sprite id replicated to clients; old packets default safely.
    #[serde(default)]
    pub sprite_character: Option<String>,
    /// Monotonic cosmetic event id. Advances after an accepted skill or basic attack.
    #[serde(default)]
    pub action_sequence: u64,
    /// Last accepted cosmetic action; unknown/legacy values are safely inert.
    #[serde(default)]
    pub action_kind: PlayerActionKind,
    /// Q/W/E/R index, or BASIC_ATTACK_ACTION_SLOT for a basic strike.
    #[serde(default)]
    pub action_slot: u8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StructureKind {
    Tower,
    BaseTower,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructureState {
    #[serde(default)]
    pub protected: bool,
    #[serde(default)]
    pub map_key: String,
    #[serde(default)]
    pub visual_profile: String,
    #[serde(default)]
    pub lane: Option<Lane>,
    #[serde(default)]
    pub tier: u8,
    pub id: u64,
    pub kind: StructureKind,
    pub team: Team,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub hp: f32,
    pub max_hp: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinionState {
    #[serde(default)]
    pub kind: MinionKind,
    #[serde(default)]
    pub attack_sequence: u64,
    pub id: u64,
    pub team: Team,
    pub lane: Lane,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub hp: f32,
    pub max_hp: f32,
    #[serde(default = "default_minion_brain_state")]
    pub state: MinionBrainState,
    #[serde(default)]
    pub target_kind: Option<MinionTargetKind>,
    #[serde(default)]
    pub target_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MinionBrainState {
    Marching,
    Chasing,
    Attacking,
    Dead,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MinionTargetKind {
    Player,
    Minion,
    Structure,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NeutralCampType {
    Skirmisher,
    Bruiser,
    Spitter,
    /// Bottom raid boss ("Wendigo", dragon-slot objective).
    WendigoBoss,
    /// Top raid boss ("King Mutatio", Baron-slot objective).
    KingMutatioBoss,
}

impl NeutralCampType {
    pub const fn is_boss(self) -> bool {
        matches!(
            self,
            NeutralCampType::WendigoBoss | NeutralCampType::KingMutatioBoss
        )
    }

    /// Team buff granted to the killer's team when this neutral dies.
    pub const fn team_buff_kind(self) -> Option<TeamBuffKind> {
        match self {
            NeutralCampType::WendigoBoss => Some(TeamBuffKind::WendigoFavor),
            NeutralCampType::KingMutatioBoss => Some(TeamBuffKind::MutatioMight),
            _ => None,
        }
    }
}

impl From<JungleCampKind> for NeutralCampType {
    fn from(kind: JungleCampKind) -> Self {
        match kind {
            JungleCampKind::Skirmisher => Self::Skirmisher,
            JungleCampKind::Bruiser => Self::Bruiser,
            JungleCampKind::Spitter => Self::Spitter,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NeutralAiState {
    Idle,
    Aggro,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeutralState {
    pub id: u64,
    pub camp_type: NeutralCampType,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub hp: f32,
    pub max_hp: f32,
    pub ai_state: NeutralAiState,
}

/// Team-wide buff kinds granted by raid-boss kills (TASK-19).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TeamBuffKind {
    /// Bottom boss (Wendigo): +ability damage.
    WendigoFavor,
    /// Top boss (King Mutatio): +ability damage and HP regen.
    MutatioMight,
}

impl TeamBuffKind {
    pub const ALL: [TeamBuffKind; 2] = [TeamBuffKind::WendigoFavor, TeamBuffKind::MutatioMight];

    /// Stable slot in `ALL`, for per-kind tables.
    pub const fn index(self) -> usize {
        match self {
            TeamBuffKind::WendigoFavor => 0,
            TeamBuffKind::MutatioMight => 1,
        }
    }
}

/// Replicated team-buff entry (additive snapshot field, `serde(default)`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamBuffState {
    pub team: Team,
    pub kind: TeamBuffKind,
    #[serde(default)]
    pub remaining_secs: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectileState {
    #[serde(default)]
    pub source_kind: CombatEntityKind,
    #[serde(default)]
    pub style: ProjectileStyle,
    #[serde(default)]
    pub action_slot: Option<u8>,
    #[serde(default)]
    pub direction: [f32; 3],
    pub id: u64,
    pub owner_id: u64,
    #[serde(default = "default_team")]
    pub owner_team: Team,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Server -> client datagram.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
// Outbound packets are serialized immediately, never queued as enum values.
// Keep both envelopes inline to avoid an extra allocation per gameplay snapshot.
pub enum ServerPacket {
    Party {
        server_epoch: u64,
        sequence: u64,
        party: crate::party::PartyView,
    },
    Social {
        server_epoch: u64,
        match_id: u64,
        sequence: u64,
        social: crate::social::SocialView,
    },
    Career {
        server_epoch: u64,
        sequence: u64,
        career: crate::career::CareerView,
    },
    Snapshot {
        #[serde(default)]
        vision: Option<crate::vision::TeamVision>,
        #[serde(default)]
        sandbox: Option<crate::sandbox::SandboxSnapshot>,
        /// What debug commands the server accepts from this recipient
        /// (additive, step 11f). Sent to joined players only; absent (never
        /// `null`) otherwise, so older snapshots and older peers are
        /// unaffected. A client that gets no value falls back to
        /// `DebugAccess::for_match_mode(match_mode)`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        debug_access: Option<crate::debug::DebugAccess>,
        #[serde(default)]
        match_mode: String,
        #[serde(default)]
        geometry_id: String,
        #[serde(default)]
        map_profile: String,
        #[serde(flatten, default)]
        meta: SnapshotMeta,
        #[serde(default)]
        join_error: Option<JoinRejection>,
        your_id: u64,
        players: Vec<PlayerState>,
        #[serde(default)]
        scoreboard: Option<crate::live_score::LiveScoreboard>,
        #[serde(default)]
        prematch: Option<crate::prematch::PrematchSnapshot>,
        #[serde(default)]
        projectiles: Vec<ProjectileState>,
        #[serde(default)]
        combat_events: Vec<CombatEvent>,
        #[serde(default)]
        structures: Vec<StructureState>,
        #[serde(default)]
        minions: Vec<MinionState>,
        #[serde(default)]
        neutrals: Vec<NeutralState>,
        /// Active boss team buffs (additive field; absent = no buffs).
        #[serde(default)]
        team_buffs: Vec<TeamBuffState>,
        #[serde(default)]
        forest_pickups: Vec<crate::forest_pickups::ForestPickupState>,
        #[serde(default)]
        game_state: GameState,
        #[serde(default)]
        rematch_in_secs: Option<u64>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameState {
    #[default]
    Lobby,
    /// Release-mode match formation: players joined so far vs. roster size.
    Forming {
        ready: u32,
        needed: u32,
    },
    /// Full roster assembled; match starts when the countdown elapses.
    Starting {
        countdown_ms: u32,
    },
    Running,
    Victory {
        winner: Team,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// Every `ClientPacket` variant, as the server accepts it.
    const CLIENT_PACKET_TAGS: [&str; 18] = [
        "sandbox",
        "leave",
        "social",
        "career",
        "hello",
        "transform",
        "cast",
        "utility",
        "basic_attack",
        "join",
        "prematch",
        "ping",
        "request_rematch",
        "set_god_mode",
        "set_speed_boost",
        "practice",
        "upgrade_skill",
        "buy_item",
    ];

    fn client_packet_fixtures() -> Vec<Value> {
        vec![
            json!({"type":"sandbox","request":{"server_epoch":7,"match_id":3,"request_id":1,"command":{"action":"spawn_wave"}}}),
            json!({"type":"leave"}),
            json!({"type":"social","request":{"request_id":2,"server_epoch":7,"match_id":3,"session_id":"s","command":{"kind":"subscribe"}}}),
            json!({"type":"career","request":{"action":"supporter_status","request_id":3}}),
            json!({"type":"hello","protocol_version":2}),
            json!({"type":"transform","dash_sequence":3,"x":1.5,"y":0.0,"z":-2.25,"yaw":0.5}),
            json!({"type":"cast","target":{"kind":"player","id":4},"slot":2}),
            json!({"type":"utility","action":"dash","direction":[1.0,0.0],"server_epoch":7,"match_id":3,"request_id":5}),
            json!({"type":"basic_attack","target":{"kind":"minion","id":9},"server_epoch":7,"match_id":3,"request_id":11}),
            json!({"type":"join","prematch":true,"team":"green","character":"ipfs","hero_class":"warden","avatar":"agnes","sprite_character":null,"session_id":"session-1","passport_ticket":null}),
            json!({"type":"prematch","request":{"server_epoch":7,"match_id":3,"generation":1,"request_id":6,"action":{"kind":"loaded"}}}),
            json!({"type":"ping"}),
            json!({"type":"request_rematch"}),
            json!({"type":"set_god_mode","enabled":true}),
            json!({"type":"set_speed_boost","enabled":false}),
            json!({"type":"practice","command":{"kind":"roster"}}),
            json!({"type":"upgrade_skill","slot":1}),
            json!({"type":"buy_item","item_id":"ember_blade","request_id":8,"match_id":3,"server_epoch":7}),
        ]
    }

    fn populated_snapshot() -> ServerPacket {
        ServerPacket::Snapshot {
            vision: None,
            sandbox: None,
            debug_access: None,
            match_mode: "dev".into(),
            geometry_id: "verdant".into(),
            map_profile: "verdant_default".into(),
            meta: SnapshotMeta::new(7, 3, 42),
            join_error: Some(JoinRejection::MatchFull),
            your_id: 1,
            players: vec![PlayerState {
                supporter_aura: Some(crate::supporter::AuraStyle::Solar),
                is_bot: true,
                id: 1,
                x: 1.5,
                y: 0.5,
                z: -2.25,
                yaw: 0.75,
                team: Team::Blue,
                hp: 90.5,
                max_hp: 210.0,
                mana: 40.25,
                max_mana: 100.0,
                gold: 350,
                earned_gold: 125,
                utility: crate::utility::UtilityState {
                    dash_remaining_secs: 1.5,
                    haste_remaining_secs: 0.0,
                    haste_active_secs: 2.5,
                    last_request_id: 4,
                    dash_sequence: 2,
                },
                inventory: vec![ItemId::EmberBlade],
                item_bonuses: ItemBonuses {
                    damage_multiplier: 1.25,
                    attack_speed_multiplier: 1.0,
                    move_speed_multiplier: 1.0,
                    spell_haste_multiplier: 1.0,
                    max_hp: 20.0,
                    max_mana: 0.0,
                },
                shop_available: true,
                last_purchase: Some(PurchaseReceipt {
                    request_id: 5,
                    match_id: 3,
                    item_id: Some(ItemId::EmberBlade),
                    error: None,
                }),
                basic_attack_cooldown_secs: 1.25,
                basic_attack_remaining_secs: 0.5,
                skill_cooldown_remaining_secs: [0.0, 1.5, 0.0, 12.0],
                skill_recovery_remaining_secs: 0.25,
                basic_attack_request_id: 9,
                xp: 45,
                level: 2,
                next_level_xp: 180,
                skill_points: 1,
                ranks: [2, 1, 1, 1],
                character: CharacterChoice::Ipfs,
                hero_class: HeroClass::Warden,
                avatar: Some("agnes".into()),
                sprite_character: Some("cathedral-moth-bellringer".into()),
                action_sequence: 6,
                action_kind: PlayerActionKind::Cast,
                action_slot: 3,
            }],
            scoreboard: None,
            prematch: None,
            projectiles: vec![ProjectileState {
                source_kind: CombatEntityKind::Player,
                style: ProjectileStyle::Claw,
                action_slot: Some(0),
                direction: [0.0, 0.0, 1.0],
                id: 20,
                owner_id: 1,
                owner_team: Team::Blue,
                x: 2.0,
                y: 1.0,
                z: -3.0,
            }],
            combat_events: Vec::new(),
            structures: vec![StructureState {
                protected: true,
                map_key: "green_top_t1".into(),
                visual_profile: "verdant_tower".into(),
                lane: Some(Lane::Top),
                tier: 1,
                id: 30,
                kind: StructureKind::Tower,
                team: Team::Green,
                x: 10.0,
                y: 3.0,
                z: -20.0,
                hp: 240.0,
                max_hp: 240.0,
            }],
            minions: vec![MinionState {
                kind: MinionKind::Caster,
                attack_sequence: 3,
                id: 40,
                team: Team::Green,
                lane: Lane::Mid,
                x: 4.0,
                y: 0.5,
                z: -6.0,
                yaw: 2.75,
                hp: 65.0,
                max_hp: 65.0,
                state: MinionBrainState::Chasing,
                target_kind: Some(MinionTargetKind::Structure),
                target_id: Some(30),
            }],
            neutrals: vec![NeutralState {
                id: 50,
                camp_type: NeutralCampType::KingMutatioBoss,
                x: 60.0,
                y: 0.0,
                z: 60.0,
                yaw: 0.0,
                hp: 1200.0,
                max_hp: 1500.0,
                ai_state: NeutralAiState::Aggro,
            }],
            team_buffs: vec![TeamBuffState {
                team: Team::Blue,
                kind: TeamBuffKind::WendigoFavor,
                remaining_secs: 30.5,
            }],
            forest_pickups: Vec::new(),
            game_state: GameState::Victory { winner: Team::Blue },
            rematch_in_secs: Some(5),
        }
    }

    // Captured from `server/src/main.rs` (0.23.0-rc.6) before the wire types
    // moved here: the exact bytes the authoritative server emitted. Changing
    // any of these strings changes the protocol.
    const GOLDEN_SNAPSHOT: &str = r#"{"type":"snapshot","vision":null,"sandbox":null,"match_mode":"dev","geometry_id":"verdant","map_profile":"verdant_default","protocol_version":2,"server_epoch":7,"match_id":3,"snapshot_tick":42,"join_error":"match_full","your_id":1,"players":[{"supporter_aura":"solar","is_bot":true,"id":1,"x":1.5,"y":0.5,"z":-2.25,"yaw":0.75,"team":"blue","hp":90.5,"max_hp":210.0,"mana":40.25,"max_mana":100.0,"gold":350,"earned_gold":125,"utility":{"dash_remaining_secs":1.5,"haste_remaining_secs":0.0,"haste_active_secs":2.5,"last_request_id":4,"dash_sequence":2},"inventory":["ember_blade"],"item_bonuses":{"damage_multiplier":1.25,"attack_speed_multiplier":1.0,"move_speed_multiplier":1.0,"spell_haste_multiplier":1.0,"max_hp":20.0,"max_mana":0.0},"shop_available":true,"last_purchase":{"request_id":5,"match_id":3,"item_id":"ember_blade","error":null},"basic_attack_cooldown_secs":1.25,"basic_attack_remaining_secs":0.5,"skill_cooldown_remaining_secs":[0.0,1.5,0.0,12.0],"skill_recovery_remaining_secs":0.25,"basic_attack_request_id":9,"xp":45,"level":2,"next_level_xp":180,"skill_points":1,"ranks":[2,1,1,1],"character":"ipfs","hero_class":"warden","avatar":"agnes","sprite_character":"cathedral-moth-bellringer","action_sequence":6,"action_kind":"cast","action_slot":3}],"scoreboard":null,"prematch":null,"projectiles":[{"source_kind":"player","style":"claw","action_slot":0,"direction":[0.0,0.0,1.0],"id":20,"owner_id":1,"owner_team":"blue","x":2.0,"y":1.0,"z":-3.0}],"combat_events":[],"structures":[{"protected":true,"map_key":"green_top_t1","visual_profile":"verdant_tower","lane":"top","tier":1,"id":30,"kind":"tower","team":"green","x":10.0,"y":3.0,"z":-20.0,"hp":240.0,"max_hp":240.0}],"minions":[{"kind":"caster","attack_sequence":3,"id":40,"team":"green","lane":"mid","x":4.0,"y":0.5,"z":-6.0,"yaw":2.75,"hp":65.0,"max_hp":65.0,"state":"chasing","target_kind":"structure","target_id":30}],"neutrals":[{"id":50,"camp_type":"king_mutatio_boss","x":60.0,"y":0.0,"z":60.0,"yaw":0.0,"hp":1200.0,"max_hp":1500.0,"ai_state":"aggro"}],"team_buffs":[{"team":"blue","kind":"wendigo_favor","remaining_secs":30.5}],"forest_pickups":[],"game_state":{"type":"victory","winner":"blue"},"rematch_in_secs":5}"#;
    const GOLDEN_SET_GOD_MODE: &str = r#"{"type":"set_god_mode","enabled":true}"#;
    const GOLDEN_TRANSFORM: &str =
        r#"{"type":"transform","dash_sequence":3,"x":1.5,"y":0.0,"z":-2.25,"yaw":0.5}"#;
    const GOLDEN_BASIC_ATTACK: &str = r#"{"type":"basic_attack","target":{"kind":"minion","id":9},"server_epoch":7,"match_id":3,"request_id":11}"#;
    const GOLDEN_JOIN: &str = r#"{"type":"join","prematch":true,"team":"green","character":"ipfs","hero_class":"warden","avatar":"agnes","sprite_character":null,"session_id":"session-1","passport_ticket":null}"#;
    const GOLDEN_SOCIAL: &str = r#"{"type":"social","server_epoch":7,"match_id":3,"sequence":2,"social":{"events":[],"request_id":null,"error":null,"allowed_reactions":[]}}"#;

    #[test]
    fn every_client_packet_variant_round_trips() {
        let fixtures = client_packet_fixtures();
        assert_eq!(fixtures.len(), CLIENT_PACKET_TAGS.len());
        for (fixture, tag) in fixtures.iter().zip(CLIENT_PACKET_TAGS) {
            assert_eq!(fixture["type"], tag);
            let packet: ClientPacket = serde_json::from_value(fixture.clone())
                .unwrap_or_else(|error| panic!("{tag}: {error}"));
            let encoded = serde_json::to_value(&packet).unwrap();
            assert_eq!(&encoded, fixture, "{tag} changed on the way through");
            let decoded: ClientPacket = serde_json::from_value(encoded.clone()).unwrap();
            assert_eq!(serde_json::to_value(&decoded).unwrap(), encoded, "{tag}");
        }
    }

    #[test]
    fn every_server_packet_variant_round_trips() {
        let packets = [
            ServerPacket::Social {
                server_epoch: 7,
                match_id: 3,
                sequence: 2,
                social: Default::default(),
            },
            ServerPacket::Career {
                server_epoch: 7,
                sequence: 2,
                career: Default::default(),
            },
            populated_snapshot(),
        ];
        let tags = ["social", "career", "snapshot"];
        for (packet, tag) in packets.iter().zip(tags) {
            let encoded = serde_json::to_string(packet).unwrap();
            let value: Value = serde_json::from_str(&encoded).unwrap();
            assert_eq!(value["type"], tag);
            let decoded: ServerPacket = serde_json::from_str(&encoded).unwrap();
            assert_eq!(serde_json::to_string(&decoded).unwrap(), encoded, "{tag}");
        }
    }

    #[test]
    fn golden_strings_match_the_server_before_the_move() {
        assert_eq!(
            serde_json::to_string(&populated_snapshot()).unwrap(),
            GOLDEN_SNAPSHOT
        );
        assert_eq!(
            serde_json::to_string(&ClientPacket::SetGodMode { enabled: true }).unwrap(),
            GOLDEN_SET_GOD_MODE
        );
        assert_eq!(
            serde_json::to_string(&ClientPacket::Transform {
                dash_sequence: 3,
                x: 1.5,
                y: 0.0,
                z: -2.25,
                yaw: 0.5,
            })
            .unwrap(),
            GOLDEN_TRANSFORM
        );
        assert_eq!(
            serde_json::to_string(&ClientPacket::BasicAttack {
                target: TargetId {
                    kind: TargetKind::Minion,
                    id: 9,
                },
                server_epoch: 7,
                match_id: 3,
                request_id: 11,
            })
            .unwrap(),
            GOLDEN_BASIC_ATTACK
        );
        assert_eq!(
            serde_json::to_string(&ClientPacket::Join {
                prematch: true,
                team: Team::Green,
                character: CharacterChoice::Ipfs,
                hero_class: HeroClass::Warden,
                avatar: Some("agnes".into()),
                sprite_character: None,
                session_id: Some("session-1".into()),
                passport_ticket: None,
            })
            .unwrap(),
            GOLDEN_JOIN
        );
        assert_eq!(
            serde_json::to_string(&ServerPacket::Social {
                server_epoch: 7,
                match_id: 3,
                sequence: 2,
                social: Default::default(),
            })
            .unwrap(),
            GOLDEN_SOCIAL
        );
    }

    #[test]
    fn character_choice_wire_ids_are_the_snake_case_slugs() {
        for choice in CharacterChoice::ALL {
            let wire = serde_json::to_string(&choice).unwrap();
            assert_eq!(wire, format!("\"{}\"", choice.slug()));
            assert_eq!(
                serde_json::from_str::<CharacterChoice>(&wire).unwrap(),
                choice
            );
        }
        assert_eq!(CharacterChoice::default(), CharacterChoice::Ipfs);
        assert!(serde_json::from_str::<CharacterChoice>("\"IPFS\"").is_err());
        assert!(serde_json::from_str::<CharacterChoice>("\"other\"").is_err());
    }

    #[test]
    fn golden_snapshot_decodes_back_to_the_same_bytes() {
        let decoded: ServerPacket = serde_json::from_str(GOLDEN_SNAPSHOT).unwrap();
        assert_eq!(serde_json::to_string(&decoded).unwrap(), GOLDEN_SNAPSHOT);
    }

    /// Step 11f: `debug_access` is additive. With a value it round-trips and
    /// sits after `sandbox`; `None` leaves the golden bytes untouched (checked
    /// above), and a snapshot from a server that predates it decodes to
    /// `None`.
    #[test]
    fn snapshot_debug_access_is_additive() {
        let mut value: Value = serde_json::from_str(GOLDEN_SNAPSHOT).unwrap();
        assert!(
            value.get("debug_access").is_none(),
            "None is not serialized"
        );
        let ServerPacket::Snapshot { debug_access, .. } =
            serde_json::from_value(value.clone()).unwrap()
        else {
            panic!("snapshot");
        };
        assert_eq!(debug_access, None, "an old snapshot has no access");

        value.as_object_mut().unwrap().insert(
            "debug_access".into(),
            json!({"toggles": true, "practice": false}),
        );
        let mut decoded: ServerPacket = serde_json::from_value(value).unwrap();
        let ServerPacket::Snapshot { debug_access, .. } = &decoded else {
            panic!("snapshot");
        };
        let access = crate::debug::DebugAccess {
            toggles: true,
            practice: false,
        };
        assert_eq!(*debug_access, Some(access));
        let encoded = serde_json::to_string(&decoded).unwrap();
        assert!(
            encoded.contains(
                r#""sandbox":null,"debug_access":{"toggles":true,"practice":false},"match_mode""#
            ),
            "{encoded}"
        );
        let again: ServerPacket = serde_json::from_str(&encoded).unwrap();
        assert_eq!(serde_json::to_string(&again).unwrap(), encoded);
        if let ServerPacket::Snapshot { debug_access, .. } = &mut decoded {
            *debug_access = None;
        }
        assert_eq!(serde_json::to_string(&decoded).unwrap(), GOLDEN_SNAPSHOT);
    }

    #[test]
    fn legacy_snapshots_decode_with_the_client_defaults() {
        let legacy = json!({
            "type": "snapshot",
            "your_id": 1,
            "players": [{"id": 1, "x": 0.0, "y": 0.0, "z": 0.0, "yaw": 0.0}],
            "projectiles": [{"id": 2, "owner_id": 1, "x": 0.0, "y": 0.0, "z": 0.0}],
            "minions": [{"id": 3, "team": "blue", "lane": "bot", "x": 0.0, "y": 0.0, "z": 0.0, "yaw": 0.0, "hp": 1.0, "max_hp": 1.0}],
            "team_buffs": [{"team": "green", "kind": "mutatio_might"}]
        });
        let ServerPacket::Snapshot {
            players,
            projectiles,
            minions,
            team_buffs,
            structures,
            game_state,
            meta,
            ..
        } = serde_json::from_value(legacy).unwrap()
        else {
            panic!("snapshot");
        };
        let player = &players[0];
        assert_eq!(player.team, Team::Green);
        assert_eq!(player.hp, crate::hero_balance::base_hp(HeroClass::Warrior));
        assert_eq!(player.max_hp, player.hp);
        assert_eq!(player.mana, LEGACY_MAX_MANA);
        assert_eq!(player.level, LEGACY_PLAYER_LEVEL);
        assert_eq!(player.next_level_xp, LEGACY_NEXT_LEVEL_XP);
        assert_eq!(player.ranks, [1; 4]);
        assert_eq!(player.character, CharacterChoice::Ipfs);
        assert_eq!(player.hero_class, HeroClass::Warrior);
        assert_eq!(projectiles[0].owner_team, Team::Green);
        assert_eq!(minions[0].state, MinionBrainState::Marching);
        assert_eq!(minions[0].target_kind, None);
        assert_eq!(team_buffs[0].remaining_secs, 0.0);
        assert!(structures.is_empty());
        assert_eq!(game_state, GameState::Lobby);
        assert_eq!(meta, SnapshotMeta::default());
    }

    #[test]
    fn hero_classes_round_trip_including_warden() {
        for class in HeroClass::ALL {
            let encoded = serde_json::to_string(&class).unwrap();
            let decoded: HeroClass = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, class);
        }
        assert_eq!(
            serde_json::to_string(&HeroClass::Warden).unwrap(),
            "\"warden\""
        );
        assert_eq!(
            serde_json::from_str::<HeroClass>("\"warden\"").unwrap(),
            HeroClass::Warden
        );
        assert!(HeroClass::ALL.contains(&HeroClass::Warden));
        let join: ClientPacket = serde_json::from_str(GOLDEN_JOIN).unwrap();
        assert!(matches!(
            join,
            ClientPacket::Join {
                hero_class: HeroClass::Warden,
                ..
            }
        ));
    }

    #[test]
    fn game_state_and_target_helpers() {
        assert_eq!(
            serde_json::to_string(&GameState::Forming {
                ready: 3,
                needed: 10
            })
            .unwrap(),
            r#"{"type":"forming","ready":3,"needed":10}"#
        );
        assert_eq!(
            serde_json::to_string(&GameState::Starting { countdown_ms: 3000 }).unwrap(),
            r#"{"type":"starting","countdown_ms":3000}"#
        );
        assert_eq!(
            serde_json::to_string(&GameState::Running).unwrap(),
            r#"{"type":"running"}"#
        );
        assert_eq!(
            serde_json::to_string(&GameState::Lobby).unwrap(),
            r#"{"type":"lobby"}"#
        );
        assert_eq!(
            serde_json::to_string(&TargetId::player(4)).unwrap(),
            r#"{"kind":"player","id":4}"#
        );
        assert!(NeutralCampType::WendigoBoss.is_boss());
        assert_eq!(
            NeutralCampType::KingMutatioBoss.team_buff_kind(),
            Some(TeamBuffKind::MutatioMight)
        );
        assert_eq!(
            NeutralCampType::from(JungleCampKind::Spitter),
            NeutralCampType::Spitter
        );
        for (index, kind) in TeamBuffKind::ALL.into_iter().enumerate() {
            assert_eq!(kind.index(), index);
        }
    }
}
