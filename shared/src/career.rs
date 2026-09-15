//! Authoritative career and match-history wire contracts, independent of rendering.
use crate::{HeroClass, map::Team};
use serde::{Deserialize, Serialize};

pub const HISTORY_PAGE_SIZE: usize = 10;
pub const MAX_NICKNAME_CHARS: usize = 20;
pub const MAX_PLAYER_HANDLE_CHARS: usize = MAX_NICKNAME_CHARS + 5;
pub const MAX_PARTICIPANTS: usize = 32;
pub const INITIAL_RATING: i32 = 1000;
pub const NEWCOMER_MATCHES: u32 = 20;

pub fn normalize_nickname(raw: &str) -> Result<String, &'static str> {
    let raw = raw.trim();
    let (name, tag) = match raw.split_once('#') {
        Some((name, tag)) if tag.len() == 4 && tag.bytes().all(|b| b.is_ascii_digit()) => {
            (name, Some(tag))
        }
        Some(_) => return Err("Use nickname#1234 with exactly four digits."),
        None => (raw, None),
    };
    if name.is_empty() || name.chars().count() > MAX_NICKNAME_CHARS || name.len() > 80 {
        return Err("Use a name with 1–20 characters.");
    }
    if name
        .chars()
        .any(|c| !(c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.')))
    {
        return Err("Use letters, numbers, spaces, dots, hyphens or underscores.");
    }
    Ok(match tag {
        Some(tag) => format!("{name}#{tag}"),
        None => name.to_owned(),
    })
}

pub fn normalize_player_handle(raw: &str) -> Result<String, &'static str> {
    let value = normalize_nickname(raw)?;
    if !value.contains('#') {
        return Err("Use nickname#1234 with exactly four digits.");
    }
    Ok(value)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlayerReference {
    pub profile_id: String,
    pub nickname: String,
}

pub fn valid_profile_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MatchStats {
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub damage_to_heroes: f64,
    pub damage_to_structures: f64,
    pub damage_to_creeps: f64,
    pub damage_taken: f64,
    pub minion_last_hits: u32,
    pub jungle_last_hits: u32,
    pub structures_destroyed: u32,
    pub final_level: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RatingChange {
    pub before: i32,
    pub after: i32,
    pub delta: i32,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParticipantResult {
    pub player_id: u64,
    // Preserve rc6 JSON receipts exactly when this additive human default is absent.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_bot: bool,
    pub profile_id: Option<String>,
    pub nickname: String,
    pub team: Team,
    pub hero_class: HeroClass,
    pub character: String,
    pub avatar: Option<String>,
    pub sprite_character: Option<String>,
    pub stats: MatchStats,
    pub disconnected: bool,
    pub rating: Option<RatingChange>,
    pub progression_xp_gained: u32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchOutcome {
    #[default]
    Completed,
    Abandoned,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchResult {
    pub result_id: String,
    pub server_epoch: u64,
    pub match_id: u64,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub duration_ms: u64,
    pub map_profile: String,
    pub ruleset: String,
    pub outcome: MatchOutcome,
    pub winner: Option<Team>,
    pub rated: bool,
    pub unrated_reason: Option<String>,
    pub participants: Vec<ParticipantResult>,
    /// True only after durable transaction acknowledgement.
    #[serde(default)]
    pub saved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchSummary {
    pub result_id: String,
    pub ended_at_ms: u64,
    pub duration_ms: u64,
    pub outcome: MatchOutcome,
    pub won: Option<bool>,
    pub hero_class: HeroClass,
    pub avatar: Option<String>,
    pub sprite_character: Option<String>,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub damage_to_heroes: f64,
    pub rating: Option<RatingChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileSummary {
    pub profile_id: String,
    pub nickname: String,
    pub rating: i32,
    pub rated_matches: u32,
    pub matches_played: u32,
    pub wins: u32,
    pub losses: u32,
    pub progression_xp: u64,
}
impl ProfileSummary {
    pub fn new(profile_id: String, nickname: String) -> Self {
        Self {
            profile_id,
            nickname,
            rating: INITIAL_RATING,
            rated_matches: 0,
            matches_played: 0,
            wins: 0,
            losses: 0,
            progression_xp: 0,
        }
    }
    pub fn newcomer(&self) -> bool {
        self.rated_matches < NEWCOMER_MATCHES
    }
    pub fn level(&self) -> u64 {
        1 + self.progression_xp / 1000
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum QueueView {
    #[default]
    Idle,
    Waiting {
        compatible: u32,
        needed: u32,
        elapsed_secs: u64,
        newcomer: bool,
    },
    Selected,
    Playing,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthChallenge {
    pub public_key: String,
    pub nonce: String,
    pub server_epoch: u64,
    pub session_id: String,
    pub nickname: String,
}
impl AuthChallenge {
    pub fn signing_bytes(&self) -> Vec<u8> {
        // Length-delimited JSON tuple under a domain/version prefix. Never sign
        // arbitrary remote bytes or another application's challenge.
        serde_json::to_vec(&(
            "omoba.career.auth.v1",
            &self.public_key,
            &self.nonce,
            self.server_epoch,
            &self.session_id,
            &self.nickname,
        ))
        .expect("string tuple serializes")
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FriendAction {
    Request,
    Accept,
    Reject,
    Cancel,
    Remove,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FriendPresence {
    #[default]
    Offline,
    Online,
    Playing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendProfile {
    pub profile: ProfileSummary,
    pub presence: FriendPresence,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendsView {
    pub friends: Vec<FriendProfile>,
    pub incoming: Vec<FriendProfile>,
    pub outgoing: Vec<FriendProfile>,
}

/// Account operations are signed separately from the transport address.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CareerAction {
    Social {
        request: crate::social::SocialRequest,
    },
    History {
        request_id: u64,
        before: Option<u64>,
    },
    Detail {
        request_id: u64,
        result_id: String,
    },
    Friends {
        request_id: u64,
    },
    Friend {
        request_id: u64,
        profile_id: String,
        #[serde(rename = "friend_action")]
        action: FriendAction,
    },
    Profile {
        request_id: u64,
        profile_id: String,
    },
    LookupPlayer {
        request_id: u64,
        handle: String,
    },
    Rename {
        request_id: u64,
        nickname: String,
    },
    CancelQueue,
}

pub fn authorized_signing_bytes(
    server_epoch: u64,
    session_nonce: &str,
    sequence: u64,
    action: &CareerAction,
) -> Vec<u8> {
    serde_json::to_vec(&(
        "omoba.career.request.v1",
        server_epoch,
        session_nonce,
        sequence,
        action,
    ))
    .expect("career action serializes")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CareerRequest {
    Social {
        request: crate::social::SocialRequest,
    },
    Challenge {
        public_key: String,
        nickname: String,
        session_id: String,
    },
    Authenticate {
        challenge: AuthChallenge,
        signature: String,
    },
    History {
        #[serde(default)]
        request_id: u64,
        before: Option<u64>,
    },
    Detail {
        #[serde(default)]
        request_id: u64,
        result_id: String,
    },
    Friends {
        request_id: u64,
    },
    Friend {
        request_id: u64,
        profile_id: String,
        #[serde(rename = "friend_action")]
        action: FriendAction,
    },
    Profile {
        request_id: u64,
        profile_id: String,
    },
    LookupPlayer {
        request_id: u64,
        handle: String,
    },
    Rename {
        request_id: u64,
        nickname: String,
    },
    Authorized {
        session_nonce: String,
        sequence: u64,
        #[serde(rename = "request")]
        action: CareerAction,
        signature: String,
    },
    CancelQueue,
}

impl CareerRequest {
    pub fn account_action(&self) -> Option<CareerAction> {
        Some(match self {
            Self::Social { request } => CareerAction::Social {
                request: request.clone(),
            },
            Self::History { request_id, before } => CareerAction::History {
                request_id: *request_id,
                before: *before,
            },
            Self::Detail {
                request_id,
                result_id,
            } => CareerAction::Detail {
                request_id: *request_id,
                result_id: result_id.clone(),
            },
            Self::Friends { request_id } => CareerAction::Friends {
                request_id: *request_id,
            },
            Self::Friend {
                request_id,
                profile_id,
                action,
            } => CareerAction::Friend {
                request_id: *request_id,
                profile_id: profile_id.clone(),
                action: *action,
            },
            Self::Profile {
                request_id,
                profile_id,
            } => CareerAction::Profile {
                request_id: *request_id,
                profile_id: profile_id.clone(),
            },
            Self::LookupPlayer { request_id, handle } => CareerAction::LookupPlayer {
                request_id: *request_id,
                handle: handle.clone(),
            },
            Self::Rename {
                request_id,
                nickname,
            } => CareerAction::Rename {
                request_id: *request_id,
                nickname: nickname.clone(),
            },
            Self::CancelQueue => CareerAction::CancelQueue,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CareerView {
    /// Configuration capability, independent of temporary database availability.
    pub storage_enabled: bool,
    /// Echo the History/Detail request ID for its payload or error.
    pub response_id: Option<u64>,
    pub challenge: Option<AuthChallenge>,
    pub auth_nonce: Option<String>,
    pub friends: Option<FriendsView>,
    pub visited_profile: Option<ProfileSummary>,
    pub found_player: Option<PlayerReference>,
    pub profile: Option<ProfileSummary>,
    pub history: Vec<MatchSummary>,
    pub history_loaded: bool,
    pub history_next: Option<u64>,
    pub detail: Option<MatchResult>,
    pub last_result: Option<MatchResult>,
    pub queue: QueueView,
    pub error: Option<String>,
    pub loading: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handles_require_exactly_four_digits_and_allow_unicode_names() {
        for value in ["MossFox#0000", "小明#0123", "Лиса#9999"] {
            assert_eq!(normalize_player_handle(value).unwrap(), value);
        }
        for value in [
            "MossFox",
            "A#123",
            "A#12345",
            "A#１２３４",
            "A#12a4",
            "A##1234",
            "#1234",
        ] {
            assert!(normalize_player_handle(value).is_err(), "{value}");
        }
        assert!(normalize_player_handle(&format!("{}#1234", "界".repeat(20))).is_ok());
        assert!(normalize_player_handle(&format!("{}#1234", "界".repeat(21))).is_err());
        let action = CareerRequest::LookupPlayer {
            request_id: 42,
            handle: "MossFox#0042".into(),
        }
        .account_action()
        .unwrap();
        assert!(matches!(
            action,
            CareerAction::LookupPlayer { request_id: 42, .. }
        ));
    }
    #[test]
    fn social_account_action_signs_the_complete_match_scoped_request() {
        use crate::social::{SocialCommand, SocialRequest};
        let social = SocialRequest {
            request_id: 10,
            server_epoch: 20,
            match_id: 30,
            session_id: "session".into(),
            command: SocialCommand::Reaction {
                reaction_id: "heart".into(),
            },
        };
        let request = CareerRequest::Social {
            request: social.clone(),
        };
        let decoded: CareerRequest =
            serde_json::from_slice(&serde_json::to_vec(&request).unwrap()).unwrap();
        let action = decoded.account_action().unwrap();
        assert_eq!(
            action,
            CareerAction::Social {
                request: social.clone()
            }
        );
        let original = authorized_signing_bytes(20, "nonce", 1, &action);
        let mut changed = social;
        changed.match_id += 1;
        assert_ne!(
            original,
            authorized_signing_bytes(20, "nonce", 1, &CareerAction::Social { request: changed })
        );
    }

    #[test]
    fn older_participant_receipts_default_to_human() {
        let original = serde_json::json!({
            "player_id": 1, "profile_id": null, "nickname": "Guest", "team": "green",
            "hero_class": "mage", "character": "ipfs", "avatar": null,
            "sprite_character": null, "stats": serde_json::to_value(MatchStats::default()).unwrap(), "disconnected": false,
            "rating": null, "progression_xp_gained": 0
        });
        let participant: ParticipantResult = serde_json::from_value(original.clone()).unwrap();
        assert!(!participant.is_bot);
        assert_eq!(serde_json::to_value(&participant).unwrap(), original);

        let old_result = serde_json::json!({
            "result_id": "rc6-receipt", "server_epoch": 1, "match_id": 2,
            "started_at_ms": 1000, "ended_at_ms": 2000, "duration_ms": 1000,
            "map_profile": "verdant_default", "ruleset": "verdant-default-v1",
            "outcome": "completed", "winner": "green", "rated": false,
            "unrated_reason": "dev", "participants": [original], "saved": false
        });
        let result: MatchResult = serde_json::from_value(old_result.clone()).unwrap();
        assert_eq!(serde_json::to_value(result).unwrap(), old_result);

        let mut bot = participant;
        bot.is_bot = true;
        assert_eq!(serde_json::to_value(&bot).unwrap()["is_bot"], true);
        assert!(
            serde_json::from_slice::<ParticipantResult>(&serde_json::to_vec(&bot).unwrap())
                .unwrap()
                .is_bot
        );
    }
    #[test]
    fn query_ids_round_trip_and_legacy_fields_default() {
        let legacy: CareerRequest =
            serde_json::from_str(r#"{"action":"history","before":null}"#).unwrap();
        assert!(matches!(
            legacy,
            CareerRequest::History {
                request_id: 0,
                before: None
            }
        ));
        let request = CareerRequest::Detail {
            request_id: 42,
            result_id: "match-1".into(),
        };
        let encoded = serde_json::to_string(&request).unwrap();
        let decoded: CareerRequest = serde_json::from_str(&encoded).unwrap();
        assert!(
            matches!(decoded, CareerRequest::Detail { request_id: 42, result_id } if result_id == "match-1")
        );
        let view: CareerView = serde_json::from_str("{}").unwrap();
        assert_eq!(view.response_id, None);
        let view = CareerView {
            response_id: Some(42),
            ..CareerView::default()
        };
        let decoded: CareerView =
            serde_json::from_str(&serde_json::to_string(&view).unwrap()).unwrap();
        assert_eq!(decoded.response_id, Some(42));
    }
    #[test]
    fn nickname_validation_preserves_unicode_and_rejects_controls() {
        assert_eq!(normalize_nickname("  Дмитрий-7  ").unwrap(), "Дмитрий-7");
        assert_eq!(normalize_nickname("小明").unwrap(), "小明");
        for name in [
            "",
            "  ",
            "a\nb",
            "a\u{202e}b",
            "<admin>",
            "abcdefghijklmnopqrstuvwxyz",
        ] {
            assert!(normalize_nickname(name).is_err(), "{name:?}");
        }
    }
    #[test]
    fn auth_message_is_domain_bound_and_injective_for_separators() {
        let a = AuthChallenge {
            public_key: "a".repeat(64),
            nonce: "b".repeat(64),
            server_epoch: 1,
            session_id: "s".into(),
            nickname: "n".into(),
        };
        let mut b = a.clone();
        b.nickname = "n\ns".into();
        b.session_id = "".into();
        assert_ne!(a.signing_bytes(), b.signing_bytes());
        b = a.clone();
        b.server_epoch = 2;
        assert_ne!(a.signing_bytes(), b.signing_bytes());
    }
}
