//! The debug command family: the development toggles (god mode, speed boost)
//! and the practice sandbox commands, as one in-process type.
//!
//! [`DebugCommand`] has no wire format of its own. It maps onto the existing
//! `SetGodMode`, `SetSpeedBoost` and `Practice` packets, which stay the
//! encoding forever: `ClientPacket` is internally tagged and not tolerant of
//! unknown tags, so a new `{"type":"debug"}` would be dropped by an old
//! server. The Combat Test protocol (`crate::sandbox`) is a separate family:
//! it is acknowledged, sequenced and epoch-scoped, and dev-only.
//!
//! [`DebugAccess`] is the permission predicate. The server derives its own
//! from its rules and worker allocation; a client that only knows the
//! snapshot's `match_mode` string uses [`DebugAccess::for_match_mode`].

use crate::wire::ClientPacket;

pub use crate::practice::{self, PracticeCommand};

/// Hit points of a practice dummy: enough to test a full rotation before it
/// falls over and returns to its spot.
pub const DUMMY_MAX_HP: f32 = 600.0;
/// How far in front of the requester (toward the enemy base) a dummy stands.
pub const DUMMY_DISTANCE: f32 = 4.5;
/// `match_mode` of the socket-free offline practice simulation.
pub const OFFLINE_PRACTICE_MODE: &str = "offline_practice";

/// One debug command. In-process only: see the module docs for the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugCommand {
    /// Invulnerability (plus a full resource pool outside Combat Test).
    GodMode(bool),
    /// Development movement speed multiplier.
    SpeedBoost(bool),
    /// Practice sandbox: roster, dummies, 1v1 duel.
    Practice(PracticeCommand),
}

impl DebugCommand {
    /// The packet that carries this command.
    pub fn to_packet(self) -> ClientPacket {
        match self {
            Self::GodMode(enabled) => ClientPacket::SetGodMode { enabled },
            Self::SpeedBoost(enabled) => ClientPacket::SetSpeedBoost { enabled },
            Self::Practice(command) => ClientPacket::Practice { command },
        }
    }

    /// The command a packet carries; `None` for every non-debug packet
    /// (including the Combat Test `Sandbox` packet).
    pub fn from_packet(packet: &ClientPacket) -> Option<Self> {
        match *packet {
            ClientPacket::SetGodMode { enabled } => Some(Self::GodMode(enabled)),
            ClientPacket::SetSpeedBoost { enabled } => Some(Self::SpeedBoost(enabled)),
            ClientPacket::Practice { command } => Some(Self::Practice(command)),
            _ => None,
        }
    }
}

/// Which debug commands a match accepts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DebugAccess {
    /// God mode and the speed boost.
    pub toggles: bool,
    /// Practice sandbox commands.
    pub practice: bool,
}

impl DebugAccess {
    /// What a match with this `match_mode` accepts when it is not a
    /// worker-allocated round (a worker round reports `"practice"` but refuses
    /// both; only the server knows it is one). The ids are the server's
    /// `MatchMode::id` strings plus [`OFFLINE_PRACTICE_MODE`]; a server test
    /// pins this table against the server's own rules.
    pub fn for_match_mode(mode: &str) -> Self {
        match mode {
            "dev" => Self {
                toggles: true,
                practice: false,
            },
            "practice" | OFFLINE_PRACTICE_MODE => Self {
                toggles: true,
                practice: true,
            },
            _ => Self::default(),
        }
    }

    pub fn allows(self, command: DebugCommand) -> bool {
        match command {
            DebugCommand::GodMode(_) | DebugCommand::SpeedBoost(_) => self.toggles,
            DebugCommand::Practice(_) => self.practice,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The packet strings as the wire tests pin them (`GOLDEN_SET_GOD_MODE`
    /// and the client packet fixtures in `protocol::wire`, the duel string in
    /// `practice`): the command family must not change a byte.
    #[test]
    fn every_command_encodes_as_todays_packet() {
        let cases = [
            (
                DebugCommand::GodMode(true),
                r#"{"type":"set_god_mode","enabled":true}"#,
            ),
            (
                DebugCommand::GodMode(false),
                r#"{"type":"set_god_mode","enabled":false}"#,
            ),
            (
                DebugCommand::SpeedBoost(false),
                r#"{"type":"set_speed_boost","enabled":false}"#,
            ),
            (
                DebugCommand::SpeedBoost(true),
                r#"{"type":"set_speed_boost","enabled":true}"#,
            ),
            (
                DebugCommand::Practice(PracticeCommand::Roster),
                r#"{"type":"practice","command":{"kind":"roster"}}"#,
            ),
            (
                DebugCommand::Practice(PracticeCommand::ClearBots),
                r#"{"type":"practice","command":{"kind":"clear_bots"}}"#,
            ),
            (
                DebugCommand::Practice(PracticeCommand::SpawnDummy),
                r#"{"type":"practice","command":{"kind":"spawn_dummy"}}"#,
            ),
            (
                DebugCommand::Practice(PracticeCommand::duel(7, 300)),
                r#"{"type":"practice","command":{"kind":"start_duel","level":7,"gold":300}}"#,
            ),
        ];
        for (command, json) in cases {
            let packet = command.to_packet();
            assert_eq!(serde_json::to_string(&packet).unwrap(), json);
            let decoded: ClientPacket = serde_json::from_str(json).unwrap();
            assert_eq!(DebugCommand::from_packet(&decoded), Some(command), "{json}");
        }
        assert_eq!(DebugCommand::from_packet(&ClientPacket::Ping), None);
        assert_eq!(
            DebugCommand::from_packet(&ClientPacket::UpgradeSkill { slot: 1 }),
            None
        );
    }

    #[test]
    fn an_unknown_practice_kind_decodes_as_unsupported() {
        let packet: ClientPacket = serde_json::from_str(
            r#"{"type":"practice","command":{"kind":"spawn_turret","hp":900}}"#,
        )
        .unwrap();
        assert_eq!(
            DebugCommand::from_packet(&packet),
            Some(DebugCommand::Practice(PracticeCommand::Unsupported))
        );
        // A known kind with a malformed body still fails the packet.
        assert!(
            serde_json::from_str::<ClientPacket>(
                r#"{"type":"practice","command":{"kind":"start_duel"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn access_follows_the_match_mode() {
        let none = DebugAccess::default();
        let toggles_only = DebugAccess {
            toggles: true,
            practice: false,
        };
        let all = DebugAccess {
            toggles: true,
            practice: true,
        };
        assert_eq!(DebugAccess::for_match_mode("release"), none);
        assert_eq!(DebugAccess::for_match_mode("dev"), toggles_only);
        assert_eq!(DebugAccess::for_match_mode("practice"), all);
        assert_eq!(DebugAccess::for_match_mode(OFFLINE_PRACTICE_MODE), all);
        assert_eq!(DebugAccess::for_match_mode(""), none);
        assert_eq!(DebugAccess::for_match_mode("Practice"), none);

        let god = DebugCommand::GodMode(true);
        let speed = DebugCommand::SpeedBoost(false);
        let roster = DebugCommand::Practice(PracticeCommand::Roster);
        assert!(!none.allows(god) && !none.allows(speed) && !none.allows(roster));
        assert!(toggles_only.allows(god) && toggles_only.allows(speed));
        assert!(!toggles_only.allows(roster));
        assert!(all.allows(god) && all.allows(speed) && all.allows(roster));
    }
}
