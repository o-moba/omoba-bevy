//! Explicitly opt-in local combat laboratory protocol. Configurations are atomic.
use crate::{HeroClass, shop::ItemId};
use serde::{Deserialize, Serialize};

pub const PRESET_VERSION: u32 = 1;
pub const TIME_SCALES: [f32; 5] = [0.1, 0.25, 0.5, 1.0, 2.0];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxActor {
    Player,
    Enemy,
    Dummy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotBehavior {
    Stationary,
    Flee,
    Attack,
    Fight,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorConfig {
    #[serde(deserialize_with = "strict_hero")]
    pub hero: HeroClass,
    pub avatar: Option<String>,
    pub level: u32,
    pub xp: u32,
    pub ranks: [u8; 4],
    pub unlock_all: bool,
    /// Base HP, before level growth and equipment.
    pub max_hp: f32,
    pub armor: f32,
    pub resistance: f32,
    pub move_speed: f32,
    pub attack_speed: f32,
    pub damage_multiplier: f32,
    pub god_mode: bool,
    pub infinite_resource: bool,
    pub no_cooldowns: bool,
    pub inventory: Vec<ItemId>,
    pub position: [f32; 2],
}
impl Default for ActorConfig {
    fn default() -> Self {
        Self {
            hero: HeroClass::Warrior,
            avatar: None,
            level: 1,
            xp: 0,
            ranks: [1; 4],
            unlock_all: false,
            max_hp: 100.0,
            armor: 0.0,
            resistance: 0.0,
            move_speed: 1.0,
            attack_speed: 1.0,
            damage_multiplier: 1.0,
            god_mode: false,
            infinite_resource: false,
            no_cooldowns: false,
            inventory: Vec::new(),
            position: [-3.0, 0.0],
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnemyConfig {
    pub enabled: bool,
    pub actor: ActorConfig,
    pub behavior: BotBehavior,
    pub aggression_range: f32,
    pub attack_distance: f32,
    pub auto_respawn: bool,
}
impl Default for EnemyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            actor: ActorConfig {
                hero: HeroClass::Mage,
                position: [3.0, 0.0],
                ..Default::default()
            },
            behavior: BotBehavior::Stationary,
            aggression_range: 20.0,
            attack_distance: 2.0,
            auto_respawn: true,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DummyConfig {
    pub enabled: bool,
    pub max_hp: f32,
    pub armor: f32,
    pub resistance: f32,
    pub infinite_hp: bool,
    pub moving: bool,
    pub position: [f32; 2],
}
impl Default for DummyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_hp: 10000.0,
            armor: 0.0,
            resistance: 0.0,
            infinite_hp: true,
            moving: false,
            position: [0.0, 3.0],
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentConfig {
    pub minions: bool,
    pub minions_paused: bool,
    pub time_scale: f32,
    pub paused: bool,
}
impl Default for EnvironmentConfig {
    fn default() -> Self {
        Self {
            minions: false,
            minions_paused: false,
            time_scale: 1.0,
            paused: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxConfig {
    pub version: u32,
    pub player: ActorConfig,
    pub enemy: EnemyConfig,
    pub dummy: DummyConfig,
    pub environment: EnvironmentConfig,
}
impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            version: PRESET_VERSION,
            player: Default::default(),
            enemy: Default::default(),
            dummy: Default::default(),
            environment: Default::default(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxRequest {
    pub server_epoch: u64,
    pub match_id: u64,
    pub request_id: u64,
    pub command: SandboxCommand,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SandboxCommand {
    ApplyConfig {
        config: SandboxConfig,
    },
    Refill {
        actor: SandboxActor,
    },
    ResetCooldowns {
        actor: SandboxActor,
    },
    Teleport {
        actor: SandboxActor,
        position: [f32; 2],
    },
    ResetActor {
        actor: SandboxActor,
    },
    AddXp {
        actor: SandboxActor,
        amount: i32,
    },
    GrantItem {
        actor: SandboxActor,
        item: ItemId,
    },
    ResetDuel,
    ResetAnalytics,
    SpawnWave,
    FrameStep,
    ForceCast {
        actor: SandboxActor,
        slot: u8,
        #[serde(default)]
        target_id: Option<u64>,
    },
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxAck {
    pub request_id: u64,
    pub accepted: bool,
    pub message: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DamageBreakdown {
    pub source_kind: crate::combat::CombatEntityKind,
    pub last_event_id: u64,
    pub source_id: u64,
    pub target_id: u64,
    pub slot: Option<u8>,
    pub hits: u64,
    pub damage: f64,
    pub last_hit: f32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DamageAnalytics {
    pub elapsed_secs: f64,
    pub damage: f64,
    pub hits: u64,
    pub last_hit: f32,
    pub dps: f64,
    pub breakdown: Vec<DamageBreakdown>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorTelemetry {
    pub actor: SandboxActor,
    pub id: u64,
    pub position: [f32; 2],
    pub hp: f32,
    pub mana: f32,
    pub armor: f32,
    pub resistance: f32,
    pub move_speed: f32,
    pub attack_speed: f32,
    pub attack_damage: f32,
    pub cooldowns: [f32; 4],
    pub unlocked: [bool; 4],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxSnapshot {
    pub config: SandboxConfig,
    pub ack: Option<SandboxAck>,
    #[serde(default)]
    pub last_request_id: u64,
    pub actors: Vec<ActorTelemetry>,
    pub analytics: DamageAnalytics,
    pub simulation_secs: f64,
    pub frame: u64,
}

fn strict_hero<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<HeroClass, D::Error> {
    let id = String::deserialize(deserializer)?;
    HeroClass::from_id(&id).ok_or_else(|| serde::de::Error::custom("Unknown sandbox hero"))
}
