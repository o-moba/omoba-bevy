//! Test mirror of the server's UDP/JSON wire protocol.
//!
//! These types are a **deliberate, minimal mirror** of the structures the
//! authoritative server (`server/src/main.rs`) serializes over UDP. They are
//! NOT shared with the server crate on purpose: the harness must exercise the
//! server purely through its public wire format, exactly like a real client.
//!
//! If the server protocol changes, this file must be updated to match. The
//! source of truth lives in:
//!   - `server/src/main.rs` — `enum ClientPacket`, `enum ServerPacket`,
//!     `struct PlayerState`, `enum Team`, `struct TargetId`, `enum TargetKind`.
//!
//! Wire conventions copied from the server:
//!   - Packets use `#[serde(tag = "type", rename_all = "snake_case")]`
//!     (internally tagged: `{"type":"join", ...}`).
//!   - Plain enums use `#[serde(rename_all = "snake_case")]`
//!     (e.g. `Team::Green` -> `"green"`).
//!
//! Only the fields the harness asserts on are modeled here. `#[serde(default)]`
//! is used liberally so the harness tolerates the server adding or omitting
//! fields without breaking deserialization (internally tagged enums already
//! ignore unknown fields).

use serde::{Deserialize, Serialize};

/// Team selection. Mirrors `server::Team`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Team {
    Green,
    Blue,
}

/// Kind of entity a cast can target. Mirrors `server::TargetKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Player,
    Minion,
    Structure,
    Neutral,
}

/// Playable character selection sent in a `Join`.
///
/// Mirrors `ekza_bevy_sdk::EkzaCharacter` (`rename_all = "snake_case"`,
/// variants `EkzaCharacter::ALL`). Modeling it as an enum — instead of a raw
/// string — turns an invalid character into a compile-time error rather than a
/// runtime packet the server silently rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Character {
    Ipfs,
    Toka,
    Wang,
    Cube,
}

/// Hero class selection sent in a `Join`. Mirrors `shared::HeroClass` wire
/// format (snake_case string; the server decodes unknown values as Warrior).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HeroClass {
    Warrior,
    Mage,
    Ranger,
    Cleric,
}

/// Cosmetic action kind mirrored from `shared::PlayerActionKind`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerActionKind {
    Attack,
    Cast,
    #[default]
    #[serde(other)]
    None,
}

/// A cast target reference. Mirrors `server::TargetId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetId {
    pub kind: TargetKind,
    pub id: u64,
}

impl TargetId {
    /// Convenience constructor for targeting an enemy player by id.
    pub fn player(id: u64) -> Self {
        Self {
            kind: TargetKind::Player,
            id,
        }
    }
}

/// Outbound client -> server packets.
///
/// Mirror of `server::ClientPacket`. The harness only ever *sends* these, so
/// the type derives `Serialize` only. This is a curated mirror: it models the
/// variants and field names the harness needs, matching the server's
/// `#[serde(tag = "type", rename_all = "snake_case")]` wire shape (it is not an
/// exhaustive copy of every server variant).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientPacket {
    Transform {
        x: f32,
        y: f32,
        z: f32,
        yaw: f32,
    },
    Cast {
        target: TargetId,
        slot: u8,
    },
    Join {
        team: Team,
        character: Character,
        hero_class: HeroClass,
        avatar: Option<String>,
        #[serde(default)]
        sprite_character: Option<String>,
        session_id: Option<String>,
    },
    Ping,
    SetGodMode {
        enabled: bool,
    },
    SetSpeedBoost {
        enabled: bool,
    },
    UpgradeSkill {
        slot: u8,
    },
}

/// A single player's networked state. Minimal mirror of `server::PlayerState`.
///
/// Fields the harness does not assert on (e.g. `y`, `yaw`, `next_level_xp`)
/// are omitted; `#[serde(default)]` keeps deserialization resilient.
#[derive(Debug, Clone, Deserialize)]
pub struct PlayerState {
    pub id: u64,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub z: f32,
    #[serde(default)]
    pub team: Option<Team>,
    #[serde(default)]
    pub hp: f32,
    #[serde(default)]
    pub max_hp: f32,
    #[serde(default)]
    pub mana: f32,
    #[serde(default)]
    pub max_mana: f32,
    #[serde(default)]
    pub gold: u32,
    #[serde(default)]
    pub xp: u32,
    #[serde(default)]
    pub level: u32,
    #[serde(default)]
    pub skill_points: u32,
    /// Per-slot ability ranks (Q/W/E/R). Base rank is 1.
    #[serde(default = "default_ranks")]
    pub ranks: [u8; 4],
    /// Authoritative hero class id (snake_case, e.g. `"mage"`). Read as a raw
    /// string so the mirror never lags behind new server-side classes.
    #[serde(default)]
    pub hero_class: Option<String>,
    /// Replicated cosmetic avatar slug (roster avatar) or `None` for the
    /// legacy character model.
    #[serde(default)]
    pub avatar: Option<String>,
    /// Replicated 2D sprite character id.
    #[serde(default)]
    pub sprite_character: Option<String>,
    /// Advances exactly once for each accepted authoritative cast.
    #[serde(default)]
    pub action_sequence: u64,
    #[serde(default)]
    pub action_kind: PlayerActionKind,
    #[serde(default)]
    pub action_slot: u8,
}

fn default_ranks() -> [u8; 4] {
    [1; 4]
}

/// Jungle neutral / raid-boss camp type. Mirrors `server::NeutralCampType`
/// (snake_case wire names, e.g. `"wendigo_boss"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeutralCampType {
    Skirmisher,
    Bruiser,
    Spitter,
    WendigoBoss,
    KingMutatioBoss,
}

impl NeutralCampType {
    /// True for the TASK-19 raid bosses.
    pub fn is_boss(self) -> bool {
        matches!(
            self,
            NeutralCampType::WendigoBoss | NeutralCampType::KingMutatioBoss
        )
    }
}

/// One replicated jungle neutral. Minimal mirror of `server::NeutralState`.
#[derive(Debug, Clone, Deserialize)]
pub struct NeutralState {
    pub id: u64,
    pub camp_type: NeutralCampType,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub z: f32,
    #[serde(default)]
    pub hp: f32,
    #[serde(default)]
    pub max_hp: f32,
}

/// Boss team-buff kind. Mirrors `server::TeamBuffKind` (snake_case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamBuffKind {
    WendigoFavor,
    MutatioMight,
}

/// One active team buff. Mirrors `server::TeamBuffState`.
#[derive(Debug, Clone, Deserialize)]
pub struct TeamBuffState {
    pub team: Team,
    pub kind: TeamBuffKind,
    #[serde(default)]
    pub remaining_secs: f32,
}

/// One replicated lane minion. Mirrors `server::MinionState` (only the
/// fields the harness targets on; unknown fields are ignored).
#[derive(Debug, Clone, Deserialize)]
pub struct MinionState {
    pub id: u64,
    #[serde(default)]
    pub team: Option<Team>,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub z: f32,
    #[serde(default)]
    pub hp: f32,
}

/// One replicated structure (tower). Mirrors `server::StructureState`
/// (targeting fields only).
#[derive(Debug, Clone, Deserialize)]
pub struct StructureState {
    pub id: u64,
    #[serde(default)]
    pub team: Option<Team>,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub z: f32,
    #[serde(default)]
    pub hp: f32,
}

/// Match phase. Mirrors `server::GameState` (internally tagged, snake_case).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameState {
    #[default]
    Lobby,
    /// Release-mode matchmaking: players joined so far vs. roster size.
    Forming {
        ready: u32,
        needed: u32,
    },
    /// Full roster found; match starts when the countdown elapses.
    Starting {
        countdown_ms: u32,
    },
    Running,
    Victory {
        winner: Team,
    },
}

/// Inbound server -> client packets. Mirror of `server::ServerPacket`.
///
/// Only the `Snapshot` variant exists today. Extra snapshot fields the harness
/// does not use (projectiles, structures, minions, ...) are intentionally not
/// modeled and are ignored during deserialization.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerPacket {
    Snapshot {
        your_id: u64,
        #[serde(default)]
        players: Vec<PlayerState>,
        /// Alive (non-respawn-gated) jungle neutrals, including raid bosses.
        #[serde(default)]
        neutrals: Vec<NeutralState>,
        /// Active boss team buffs (additive `serde(default)` field).
        #[serde(default)]
        team_buffs: Vec<TeamBuffState>,
        /// Match phase (Lobby/Forming/Starting/Running/Victory).
        #[serde(default)]
        game_state: GameState,
        /// Lane minions (bot AI targeting).
        #[serde(default)]
        minions: Vec<MinionState>,
        /// Structures/towers (bot AI targeting).
        #[serde(default)]
        structures: Vec<StructureState>,
    },
}

impl ServerPacket {
    /// Returns the receiving client's own player id from a snapshot.
    pub fn your_id(&self) -> u64 {
        match self {
            ServerPacket::Snapshot { your_id, .. } => *your_id,
        }
    }

    /// Borrows the players carried by a snapshot.
    pub fn players(&self) -> &[PlayerState] {
        match self {
            ServerPacket::Snapshot { players, .. } => players,
        }
    }

    /// Finds a player by id within a snapshot.
    pub fn player(&self, id: u64) -> Option<&PlayerState> {
        self.players().iter().find(|player| player.id == id)
    }

    /// Borrows the neutrals carried by a snapshot.
    pub fn neutrals(&self) -> &[NeutralState] {
        match self {
            ServerPacket::Snapshot { neutrals, .. } => neutrals,
        }
    }

    /// Finds the first neutral of a camp type within a snapshot.
    pub fn neutral_of_type(&self, camp_type: NeutralCampType) -> Option<&NeutralState> {
        self.neutrals()
            .iter()
            .find(|neutral| neutral.camp_type == camp_type)
    }

    /// Borrows the active team buffs carried by a snapshot.
    pub fn team_buffs(&self) -> &[TeamBuffState] {
        match self {
            ServerPacket::Snapshot { team_buffs, .. } => team_buffs,
        }
    }

    /// Borrows the match phase carried by a snapshot.
    pub fn game_state(&self) -> &GameState {
        match self {
            ServerPacket::Snapshot { game_state, .. } => game_state,
        }
    }

    /// Borrows the lane minions carried by a snapshot.
    pub fn minions(&self) -> &[MinionState] {
        match self {
            ServerPacket::Snapshot { minions, .. } => minions,
        }
    }

    /// Borrows the structures carried by a snapshot.
    pub fn structures(&self) -> &[StructureState] {
        match self {
            ServerPacket::Snapshot { structures, .. } => structures,
        }
    }
}
