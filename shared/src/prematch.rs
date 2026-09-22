//! Additive opt-in character draft and asset readiness protocol.
use crate::{HeroClass, map::Team};
use ekza_bevy_sdk::EkzaCharacter;
use serde::{Deserialize, Serialize};

pub const COUNTDOWN_MS: u32 = 3_000;
pub const LOADING_TIMEOUT_MS: u32 = 30_000;

/// Intended lane/team duty, independent of the selected class's actual kit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Solo,
    Jungle,
    #[default]
    Mid,
    Carry,
    Support,
}
impl Role {
    pub const ALL: [Self; 5] = [
        Self::Solo,
        Self::Jungle,
        Self::Mid,
        Self::Carry,
        Self::Support,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Solo => "Solo",
            Self::Jungle => "Jungle",
            Self::Mid => "Mid",
            Self::Carry => "Carry",
            Self::Support => "Support",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrematchRequest {
    pub server_epoch: u64,
    pub match_id: u64,
    pub generation: u64,
    pub request_id: u64,
    pub action: PrematchAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PrematchAction {
    Select {
        character: EkzaCharacter,
        hero_class: HeroClass,
        avatar: Option<String>,
        sprite_character: Option<String>,
        role: Role,
        #[serde(default)]
        passport_ticket: Option<String>,
    },
    Lock {
        locked: bool,
    },
    Loaded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrematchPhase {
    Draft,
    Countdown,
    Loading,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftPlayer {
    pub player_id: u64,
    pub nickname: String,
    pub team: Team,
    pub character: EkzaCharacter,
    pub hero_class: HeroClass,
    pub avatar: Option<String>,
    pub sprite_character: Option<String>,
    pub role: Role,
    pub is_bot: bool,
    pub locked: bool,
    pub loaded: bool,
}

/// Sent only to clients that explicitly opted in through Join.prematch.
/// Epoch and match identity come from the enclosing snapshot metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrematchSnapshot {
    pub generation: u64,
    pub phase: PrematchPhase,
    pub remaining_ms: u32,
    pub needed: u32,
    pub players: Vec<DraftPlayer>,
    pub last_request_id: u64,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_roundtrip_retains_namespace_and_role() {
        let request = PrematchRequest {
            server_epoch: 7,
            match_id: 2,
            generation: 4,
            request_id: 9,
            action: PrematchAction::Select {
                character: EkzaCharacter::Ipfs,
                hero_class: HeroClass::default(),
                avatar: None,
                sprite_character: None,
                role: Role::Jungle,
                passport_ticket: None,
            },
        };
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(encoded.contains("\"kind\":\"select\""));
        let decoded: PrematchRequest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.generation, 4);
        assert!(matches!(
            decoded.action,
            PrematchAction::Select {
                role: Role::Jungle,
                ..
            }
        ));
    }
}
