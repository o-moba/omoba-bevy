//! Additive opt-in character draft and asset readiness protocol.
use crate::wire::CharacterChoice;
use crate::{HeroClass, map::Team};
use serde::{Deserialize, Serialize};

pub const DRAFT_SELECTION_MS: u32 = 30_000;
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
        #[serde(
            default,
            skip_serializing_if = "crate::handheld::HandheldSelection::is_default"
        )]
        handheld: crate::handheld::HandheldSelection,
        character: CharacterChoice,
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
    #[serde(
        default,
        skip_serializing_if = "crate::handheld::HandheldSelection::is_default"
    )]
    pub handheld: crate::handheld::HandheldSelection,
    pub player_id: u64,
    pub nickname: String,
    pub team: Team,
    pub character: CharacterChoice,
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
    /// Server-owned phase deadline. Draft is zero while gathering the roster
    /// or when the selection window expired but avatar admission is pending.
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
    fn equipment_preserves_legacy_draft_shapes_and_nondefault_choices() {
        use crate::handheld::HandheldSelection;
        const SELECT: &str = r#"{"kind":"select","character":"ipfs","hero_class":"warrior","avatar":null,"sprite_character":null,"role":"jungle","passport_ticket":null}"#;
        const PLAYER: &str = r#"{"player_id":7,"nickname":"Pilot","team":"green","character":"ipfs","hero_class":"warrior","avatar":null,"sprite_character":null,"role":"jungle","is_bot":false,"locked":false,"loaded":false}"#;
        let mut action: PrematchAction = serde_json::from_str(SELECT).unwrap();
        let mut player: DraftPlayer = serde_json::from_str(PLAYER).unwrap();
        assert!(matches!(
            &action,
            PrematchAction::Select {
                handheld: HandheldSelection::ClassDefault,
                ..
            }
        ));
        assert_eq!(player.handheld, HandheldSelection::ClassDefault);
        assert_eq!(serde_json::to_string(&action).unwrap(), SELECT);
        assert_eq!(serde_json::to_string(&player).unwrap(), PLAYER);
        for selection in [
            HandheldSelection::Unequipped,
            HandheldSelection::Item("forge-sword".into()),
        ] {
            let PrematchAction::Select { handheld, .. } = &mut action else {
                unreachable!()
            };
            *handheld = selection.clone();
            player.handheld = selection.clone();
            let action_value = serde_json::to_value(&action).unwrap();
            let player_value = serde_json::to_value(&player).unwrap();
            assert_eq!(
                action_value["handheld"],
                serde_json::to_value(&selection).unwrap()
            );
            assert_eq!(player_value["handheld"], action_value["handheld"]);
            let decoded_action: PrematchAction = serde_json::from_value(action_value).unwrap();
            let decoded_player: DraftPlayer = serde_json::from_value(player_value).unwrap();
            let PrematchAction::Select { handheld, .. } = decoded_action else {
                panic!("not a selection")
            };
            assert_eq!(handheld, selection);
            assert_eq!(decoded_player.handheld, selection);
        }
    }

    #[test]
    fn request_roundtrip_retains_namespace_and_role() {
        let request = PrematchRequest {
            server_epoch: 7,
            match_id: 2,
            generation: 4,
            request_id: 9,
            action: PrematchAction::Select {
                handheld: Default::default(),
                character: CharacterChoice::Ipfs,
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
