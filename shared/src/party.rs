//! Party: a small group of players on one server who queue together and are
//! seated on the same team. The server owns every decision; clients send
//! [`PartyCommand`]s and render the [`PartyView`] they receive.
use crate::match_service::MatchPreference;
use serde::{Deserialize, Serialize};

/// A party never outgrows one team.
pub const MAX_PARTY_SIZE: usize = 5;
/// An unanswered invite lapses after this long.
pub const PARTY_INVITE_TTL_SECS: u64 = 60;
/// Longest nickname a party view carries.
pub const MAX_PARTY_NICKNAME_CHARS: usize = 24;
/// Online players listed in one view (invite candidates).
pub const MAX_PARTY_ONLINE: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum PartyCommand {
    /// "I am here": nickname and the avatar other members should see. Sent
    /// periodically; it also subscribes the endpoint to party views.
    Presence {
        nickname: String,
        #[serde(default)]
        avatar: Option<String>,
    },
    /// Invite another player on this server. Creates the party if needed.
    Invite {
        player_id: u64,
    },
    Accept {
        party_id: u64,
    },
    Decline {
        party_id: u64,
    },
    Leave,
    /// Leader only.
    Kick {
        player_id: u64,
    },
    /// Leader only: every member moves to hero select for this queue.
    Launch {
        preference: MatchPreference,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyMember {
    pub player_id: u64,
    pub nickname: String,
    #[serde(default)]
    pub avatar: Option<String>,
    pub leader: bool,
    /// Seated in a match on this server.
    #[serde(default)]
    pub in_match: bool,
    /// Temporarily away (another server, reconnecting).
    #[serde(default)]
    pub away: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyLaunch {
    /// Increases with every launch on this server; a client acts once per value.
    pub sequence: u64,
    pub preference: MatchPreference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyInfo {
    pub party_id: u64,
    pub leader: u64,
    /// Leader first, then join order.
    pub members: Vec<PartyMember>,
    #[serde(default)]
    pub launch: Option<PartyLaunch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyInvite {
    pub party_id: u64,
    pub from_player_id: u64,
    pub from_nickname: String,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnlinePlayer {
    pub player_id: u64,
    pub nickname: String,
    #[serde(default)]
    pub avatar: Option<String>,
    /// An accepted friend of the viewer (career storage only).
    #[serde(default)]
    pub friend: bool,
    /// Already a member of some party.
    #[serde(default)]
    pub in_party: bool,
    #[serde(default)]
    pub in_match: bool,
    /// The viewer's party has a pending invite for this player.
    #[serde(default)]
    pub invited: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyView {
    /// The viewer's own player id on this server.
    pub you: u64,
    #[serde(default)]
    pub party: Option<PartyInfo>,
    #[serde(default)]
    pub invites: Vec<PartyInvite>,
    /// Other players on this server, friends first.
    #[serde(default)]
    pub online: Vec<OnlinePlayer>,
}

impl PartyView {
    pub fn is_leader(&self) -> bool {
        self.party.as_ref().is_some_and(|p| p.leader == self.you)
    }
    /// Solo players may always launch for themselves; a member must wait.
    pub fn can_launch(&self) -> bool {
        self.party.is_none() || self.is_leader()
    }
    pub fn member_count(&self) -> usize {
        self.party.as_ref().map_or(1, |p| p.members.len())
    }
}

/// Printable, trimmed, bounded nickname; empty input becomes "Player".
pub fn normalize_party_nickname(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_PARTY_NICKNAME_CHARS)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Player".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{ClientPacket, ServerPacket};

    #[test]
    fn commands_round_trip_through_the_client_packet() {
        for command in [
            PartyCommand::Presence {
                nickname: "Ann".into(),
                avatar: Some("agnes".into()),
            },
            PartyCommand::Invite { player_id: 7 },
            PartyCommand::Accept { party_id: 3 },
            PartyCommand::Decline { party_id: 3 },
            PartyCommand::Leave,
            PartyCommand::Kick { player_id: 9 },
            PartyCommand::Launch {
                preference: MatchPreference::BotPractice,
            },
        ] {
            let packet = ClientPacket::Party {
                command: command.clone(),
            };
            let bytes = serde_json::to_vec(&packet).unwrap();
            let ClientPacket::Party { command: decoded } = serde_json::from_slice(&bytes).unwrap()
            else {
                panic!("party packet decoded as another variant");
            };
            assert_eq!(decoded, command);
        }
    }

    #[test]
    fn view_round_trips_through_the_server_packet() {
        let view = PartyView {
            you: 1,
            party: Some(PartyInfo {
                party_id: 4,
                leader: 1,
                members: vec![PartyMember {
                    player_id: 1,
                    nickname: "Ann".into(),
                    avatar: None,
                    leader: true,
                    in_match: false,
                    away: false,
                }],
                launch: Some(PartyLaunch {
                    sequence: 2,
                    preference: MatchPreference::Quick,
                }),
            }),
            invites: vec![PartyInvite {
                party_id: 8,
                from_player_id: 2,
                from_nickname: "Bob".into(),
                expires_in_secs: 30,
            }],
            online: vec![OnlinePlayer {
                player_id: 2,
                nickname: "Bob".into(),
                avatar: Some("agnes".into()),
                friend: true,
                in_party: false,
                in_match: false,
                invited: false,
            }],
        };
        let bytes = serde_json::to_vec(&ServerPacket::Party {
            server_epoch: 5,
            sequence: 6,
            party: view.clone(),
        })
        .unwrap();
        let ServerPacket::Party {
            server_epoch,
            sequence,
            party,
        } = serde_json::from_slice(&bytes).unwrap()
        else {
            panic!("party view decoded as another variant");
        };
        assert_eq!((server_epoch, sequence), (5, 6));
        assert_eq!(party, view);
        // A minimal view (older server, empty lists) still decodes.
        let minimal: PartyView = serde_json::from_str(r#"{"you":3}"#).unwrap();
        assert_eq!(minimal.you, 3);
        assert!(minimal.can_launch());
    }

    #[test]
    fn only_a_leader_or_a_solo_player_can_launch() {
        let mut view = PartyView {
            you: 2,
            party: Some(PartyInfo {
                party_id: 1,
                leader: 1,
                members: Vec::new(),
                launch: None,
            }),
            ..Default::default()
        };
        assert!(!view.can_launch());
        view.you = 1;
        assert!(view.can_launch());
        view.party = None;
        assert!(view.can_launch());
    }

    #[test]
    fn nicknames_are_trimmed_bounded_and_printable() {
        assert_eq!(normalize_party_nickname("  Ann\u{7}  "), "Ann");
        assert_eq!(normalize_party_nickname(""), "Player");
        assert_eq!(
            normalize_party_nickname(&"x".repeat(80)).chars().count(),
            MAX_PARTY_NICKNAME_CHARS
        );
    }
}
