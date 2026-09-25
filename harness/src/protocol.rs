//! The server's UDP/JSON wire protocol, as the harness speaks it.
//!
//! These are the **shared** definitions from `shared::wire`, re-exported so
//! scenarios keep short names. The harness exercises the server purely through
//! its public wire format, exactly like a real client, and that format now has
//! one definition for server, client and harness: never copy a wire type here.
//!
//! Wire conventions:
//!   - Packets use `#[serde(tag = "type", rename_all = "snake_case")]`
//!     (internally tagged: `{"type":"join", ...}`).
//!   - Plain enums use `#[serde(rename_all = "snake_case")]`
//!     (e.g. `Team::Green` -> `"green"`).

pub use shared::map::Team;
pub use shared::wire::{
    ClientPacket, GameState, MinionState, NeutralCampType, NeutralState, PlayerState,
    ProjectileState, ServerPacket, StructureKind, StructureState, TargetId, TargetKind,
    TeamBuffKind, TeamBuffState,
};
pub use shared::{HeroClass, PlayerActionKind};

/// Playable character selection sent in a `Join`. The wire type is
/// `shared::wire::CharacterChoice` (`rename_all = "snake_case"`), so an invalid
/// character is a compile-time error rather than a silently rejected packet.
pub use shared::wire::CharacterChoice as Character;

static LOBBY: GameState = GameState::Lobby;

/// Snapshot accessors for scenarios. The harness only consumes `Snapshot`
/// packets; on a `Social` or `Career` envelope every accessor is empty.
pub trait SnapshotView {
    fn prematch(&self) -> Option<&shared::prematch::PrematchSnapshot>;
    fn scoreboard(&self) -> Option<&shared::live_score::LiveScoreboard>;
    fn geometry_id(&self) -> &str;
    fn map_profile(&self) -> &str;
    fn combat_events(&self) -> &[shared::combat::CombatEvent];
    fn projectiles(&self) -> &[ProjectileState];
    fn meta(&self) -> shared::protocol::SnapshotMeta;
    fn join_error(&self) -> Option<shared::protocol::JoinRejection>;
    /// The receiving client's own player id from a snapshot.
    fn your_id(&self) -> u64;
    /// The players carried by a snapshot.
    fn players(&self) -> &[PlayerState];
    /// Finds a player by id within a snapshot.
    fn player(&self, id: u64) -> Option<&PlayerState> {
        self.players().iter().find(|player| player.id == id)
    }
    /// The neutrals carried by a snapshot.
    fn neutrals(&self) -> &[NeutralState];
    /// Finds the first neutral of a camp type within a snapshot.
    fn neutral_of_type(&self, camp_type: NeutralCampType) -> Option<&NeutralState> {
        self.neutrals()
            .iter()
            .find(|neutral| neutral.camp_type == camp_type)
    }
    /// The active team buffs carried by a snapshot.
    fn team_buffs(&self) -> &[TeamBuffState];
    /// The match phase carried by a snapshot.
    fn game_state(&self) -> &GameState;
    /// The lane minions carried by a snapshot.
    fn minions(&self) -> &[MinionState];
    /// The structures carried by a snapshot.
    fn structures(&self) -> &[StructureState];
}

impl SnapshotView for ServerPacket {
    fn prematch(&self) -> Option<&shared::prematch::PrematchSnapshot> {
        match self {
            Self::Snapshot { prematch, .. } => prematch.as_ref(),
            _ => None,
        }
    }
    fn scoreboard(&self) -> Option<&shared::live_score::LiveScoreboard> {
        match self {
            Self::Snapshot { scoreboard, .. } => scoreboard.as_ref(),
            _ => None,
        }
    }
    fn geometry_id(&self) -> &str {
        match self {
            Self::Snapshot { geometry_id, .. } => geometry_id,
            _ => "",
        }
    }
    fn map_profile(&self) -> &str {
        match self {
            Self::Snapshot { map_profile, .. } => map_profile,
            _ => "",
        }
    }
    fn combat_events(&self) -> &[shared::combat::CombatEvent] {
        match self {
            Self::Snapshot { combat_events, .. } => combat_events,
            _ => &[],
        }
    }
    fn projectiles(&self) -> &[ProjectileState] {
        match self {
            Self::Snapshot { projectiles, .. } => projectiles,
            _ => &[],
        }
    }
    fn meta(&self) -> shared::protocol::SnapshotMeta {
        match self {
            Self::Snapshot { meta, .. } => *meta,
            _ => shared::protocol::SnapshotMeta::default(),
        }
    }
    fn join_error(&self) -> Option<shared::protocol::JoinRejection> {
        match self {
            Self::Snapshot { join_error, .. } => *join_error,
            _ => None,
        }
    }
    fn your_id(&self) -> u64 {
        match self {
            Self::Snapshot { your_id, .. } => *your_id,
            _ => 0,
        }
    }
    fn players(&self) -> &[PlayerState] {
        match self {
            Self::Snapshot { players, .. } => players,
            _ => &[],
        }
    }
    fn neutrals(&self) -> &[NeutralState] {
        match self {
            Self::Snapshot { neutrals, .. } => neutrals,
            _ => &[],
        }
    }
    fn team_buffs(&self) -> &[TeamBuffState] {
        match self {
            Self::Snapshot { team_buffs, .. } => team_buffs,
            _ => &[],
        }
    }
    fn game_state(&self) -> &GameState {
        match self {
            Self::Snapshot { game_state, .. } => game_state,
            _ => &LOBBY,
        }
    }
    fn minions(&self) -> &[MinionState] {
        match self {
            Self::Snapshot { minions, .. } => minions,
            _ => &[],
        }
    }
    fn structures(&self) -> &[StructureState] {
        match self {
            Self::Snapshot { structures, .. } => structures,
            _ => &[],
        }
    }
}
