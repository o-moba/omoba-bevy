//! Ephemeral match communication and an authoritative reaction catalog.
//!
//! Catalog access is separate from local image presentation. Entitlements are
//! trusted server input, never client claims. The existing avatar Passport may
//! authorize an explicitly configured avatar companion pack after successful
//! ticket consumption; it does not verify arbitrary sticker NFTs.
use std::{collections::HashSet, sync::OnceLock};

use ekza_bevy_sdk::passport::{ConsumedTicket, ProtectedAvatar, validate_avatar_id};
use serde::{Deserialize, Serialize};

pub const MAX_CHAT_CHARS: usize = 160;
pub const MAX_CHAT_BYTES: usize = 640;
pub const MAX_REACTION_ID_BYTES: usize = 64;
pub const MAX_SOCIAL_EVENTS: usize = 64;
pub const SOCIAL_EVENT_TTL_MS: u32 = 30_000;
const MAX_CATALOG_BYTES: usize = 64 * 1024;
const MAX_PACKS: usize = 64;
const MAX_REACTIONS: usize = 256;
const BASE_REACTIONS: [(&str, &str); 4] = [
    ("thumbs_up", "Thumbs up"),
    ("thumbs_down", "Thumbs down"),
    ("heart", "Heart"),
    ("laugh", "Laugh"),
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocialChannel {
    #[default]
    Team,
    Match,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocialTeam {
    Green,
    Blue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SocialCommand {
    /// Opt in to social snapshots. The server must not emit an event or charge
    /// a chat token for this command; match/session authorization still applies.
    Subscribe,
    Chat {
        channel: SocialChannel,
        text: String,
    },
    Reaction {
        reaction_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocialRequest {
    pub request_id: u64,
    pub server_epoch: u64,
    pub match_id: u64,
    pub session_id: String,
    pub command: SocialCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SocialEventKind {
    Chat {
        channel: SocialChannel,
        text: String,
    },
    Reaction {
        reaction_id: String,
    },
}

/// Every field is supplied by the server after admission and audience checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocialEvent {
    pub id: u64,
    pub player_id: u64,
    pub nickname: String,
    pub team: SocialTeam,
    /// Age at serialization, so a delayed/repeated event cannot restart a bubble.
    pub age_ms: u32,
    pub kind: SocialEventKind,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SocialView {
    pub events: Vec<SocialEvent>,
    pub request_id: Option<u64>,
    pub error: Option<String>,
    pub allowed_reactions: Vec<String>,
}

fn unsafe_text_character(character: char) -> bool {
    character.is_control()
        || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Single-line Unicode; preserve ordinary scripts and emoji (including ZWJ).
pub fn normalize_chat_text(raw: &str) -> Result<String, &'static str> {
    if raw.len() > MAX_CHAT_BYTES || raw.chars().any(unsafe_text_character) {
        return Err("Chat must be one line without control characters, up to 160 characters.");
    }
    let text = raw.trim();
    if text.is_empty() || text.chars().count() > MAX_CHAT_CHARS {
        return Err("Chat must contain between 1 and 160 characters.");
    }
    Ok(text.to_owned())
}

/// IDs never double as URLs, local file paths, mint addresses or proof claims.
pub fn valid_reaction_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_REACTION_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte))
}

/// Structural validation only: the server must additionally compare the actual
/// epoch/match/session, deduplicate, rate-limit and authorize the requested pack.
pub fn validate_request(request: &SocialRequest) -> Result<(), &'static str> {
    if request.request_id == 0 || request.server_epoch == 0 || request.match_id == 0 {
        return Err("This social request has an invalid match or request ID.");
    }
    if request.session_id.is_empty()
        || request.session_id.len() > 64
        || !request
            .session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err("This social request has an invalid session.");
    }
    match &request.command {
        SocialCommand::Subscribe => Ok(()),
        SocialCommand::Chat { text, .. } => normalize_chat_text(text).map(|_| ()),
        SocialCommand::Reaction { reaction_id } => {
            if !valid_reaction_id(reaction_id) || reaction(reaction_id).is_none() {
                Err("This reaction is not in the server catalog.")
            } else {
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PackAccess {
    Free,
    /// An explicit operator grant, not a claim of token ownership.
    OperatorGrant,
    /// Companion content deliberately associated with this approved avatar.
    VerifiedAvatar {
        avatar_id: String,
    },
    /// Reserved provider namespace. No generic NFT verifier ships in this version.
    Nft {
        provider: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReactionPack {
    pub id: String,
    pub label: String,
    pub access: PackAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReactionDefinition {
    pub id: String,
    pub label: String,
    pub pack_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReactionCatalog {
    pub schema_version: u32,
    pub packs: Vec<ReactionPack>,
    pub reactions: Vec<ReactionDefinition>,
}

impl ReactionCatalog {
    pub fn from_json(json: &str) -> Result<Self, &'static str> {
        if json.len() > MAX_CATALOG_BYTES {
            return Err("Reaction catalog is too large.");
        }
        let catalog: Self = serde_json::from_str(json).map_err(|_| "Invalid reaction catalog.")?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1
            || self.packs.is_empty()
            || self.packs.len() > MAX_PACKS
            || self.reactions.is_empty()
            || self.reactions.len() > MAX_REACTIONS
        {
            return Err("Unsupported or unbounded reaction catalog.");
        }
        let mut packs = HashSet::new();
        for pack in &self.packs {
            if !valid_reaction_id(&pack.id) || !valid_label(&pack.label) || !packs.insert(&pack.id)
            {
                return Err("Invalid or duplicate reaction pack.");
            }
            match &pack.access {
                PackAccess::VerifiedAvatar { avatar_id }
                    if validate_avatar_id(avatar_id).is_err() =>
                {
                    return Err("Invalid companion avatar identity.");
                }
                PackAccess::Nft { provider } if !valid_reaction_id(provider) => {
                    return Err("Invalid NFT provider identity.");
                }
                _ => {}
            }
        }
        let mut reactions = HashSet::new();
        for reaction in &self.reactions {
            if !valid_reaction_id(&reaction.id)
                || !valid_label(&reaction.label)
                || !packs.contains(&reaction.pack_id)
                || !reactions.insert(&reaction.id)
            {
                return Err("Invalid, duplicate or orphan reaction.");
            }
        }
        Ok(())
    }

    pub fn reaction(&self, id: &str) -> Option<&ReactionDefinition> {
        self.reactions.iter().find(|reaction| reaction.id == id)
    }

    pub fn reaction_allowed(&self, id: &str, entitlements: &Entitlements) -> bool {
        let Some(reaction) = self.reaction(id) else {
            return false;
        };
        let Some(pack) = self.packs.iter().find(|pack| pack.id == reaction.pack_id) else {
            return false;
        };
        match &pack.access {
            PackAccess::Free => true,
            PackAccess::OperatorGrant => entitlements.operator_pack_ids.contains(&pack.id),
            PackAccess::VerifiedAvatar { avatar_id } => {
                entitlements.verified_avatar_ids.contains(avatar_id)
            }
            // A catalog entry is not evidence of NFT ownership. A future trusted
            // verifier needs an explicit new grant path and session expiry rules.
            PackAccess::Nft { .. } => false,
        }
    }

    pub fn allowed_reactions(&self, entitlements: &Entitlements) -> Vec<String> {
        self.reactions
            .iter()
            .filter(|reaction| self.reaction_allowed(&reaction.id, entitlements))
            .map(|reaction| reaction.id.clone())
            .collect()
    }
}

fn valid_label(label: &str) -> bool {
    !label.trim().is_empty()
        && label.len() <= 256
        && label.chars().count() <= 64
        && !label.chars().any(unsafe_text_character)
}

/// Server-only trust boundary: deliberately has no Serialize/Deserialize.
/// Scope this to the admitted session and clear it on reconnect/round changes.
#[derive(Debug, Default, Clone)]
pub struct Entitlements {
    operator_pack_ids: HashSet<String>,
    verified_avatar_ids: HashSet<String>,
}

impl Entitlements {
    /// Call from trusted operator configuration, never a packet or client file.
    pub fn grant_operator_pack(&mut self, pack_id: &str) -> Result<(), &'static str> {
        if !valid_reaction_id(pack_id)
            || (self.operator_pack_ids.len() >= MAX_PACKS
                && !self.operator_pack_ids.contains(pack_id))
        {
            return Err("Invalid or excessive operator reaction grant.");
        }
        self.operator_pack_ids.insert(pack_id.to_owned());
        Ok(())
    }

    /// Only call after successful one-use ticket consumption at the server's
    /// trusted Passport API for the current admitted session. This checks the
    /// returned exact avatar/rendition, not the network origin, expiry or session;
    /// the trusted service/caller has already verified those. A client-created
    /// ConsumedTicket must never reach this method as evidence.
    pub fn grant_verified_avatar(
        &mut self,
        expected: &ProtectedAvatar,
        consumed: &ConsumedTicket,
    ) -> Result<(), &'static str> {
        expected.validate_consumed_ticket(consumed)?;
        if self.verified_avatar_ids.len() >= MAX_PACKS
            && !self.verified_avatar_ids.contains(&expected.avatar_id)
        {
            return Err("Excessive companion reaction grants.");
        }
        self.verified_avatar_ids.insert(expected.avatar_id.clone());
        Ok(())
    }
}

fn fallback_catalog() -> ReactionCatalog {
    ReactionCatalog {
        schema_version: 1,
        packs: vec![ReactionPack {
            id: "base".into(),
            label: "Base reactions".into(),
            access: PackAccess::Free,
        }],
        reactions: BASE_REACTIONS
            .into_iter()
            .map(|(id, label)| ReactionDefinition {
                id: id.into(),
                label: label.into(),
                pack_id: "base".into(),
            })
            .collect(),
    }
}

fn catalog_from_packaged(json: &str) -> ReactionCatalog {
    match ReactionCatalog::from_json(json) {
        Ok(catalog)
            if BASE_REACTIONS
                .iter()
                .all(|(id, _)| catalog.reaction_allowed(id, &Entitlements::default())) =>
        {
            catalog
        }
        _ => fallback_catalog(),
    }
}

/// Embedded on every platform; malformed customization cannot remove the free
/// baseline or turn missing metadata into an entitlement. No remote loading.
pub fn catalog() -> &'static ReactionCatalog {
    static CATALOG: OnceLock<ReactionCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| catalog_from_packaged(include_str!("../assets/reactions.json")))
}

pub fn reaction(id: &str) -> Option<&'static ReactionDefinition> {
    catalog().reaction(id)
}
pub fn allowed_reactions(entitlements: &Entitlements) -> Vec<String> {
    catalog().allowed_reactions(entitlements)
}
pub fn reaction_allowed(id: &str, entitlements: &Entitlements) -> bool {
    catalog().reaction_allowed(id, entitlements)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(command: SocialCommand) -> SocialRequest {
        SocialRequest {
            request_id: 1,
            server_epoch: 2,
            match_id: 3,
            session_id: "session-1".into(),
            command,
        }
    }

    #[test]
    fn chat_preserves_unicode_but_rejects_controls_bidi_and_excess() {
        assert_eq!(
            normalize_chat_text("  Привет 世界 👩‍🚀  ").unwrap(),
            "Привет 世界 👩‍🚀"
        );
        assert!(normalize_chat_text(&"界".repeat(MAX_CHAT_CHARS)).is_ok());
        for text in [
            "",
            "   ",
            "hi\nteam",
            "x\ty",
            "x\0y",
            "x\u{202e}y",
            "x\u{2066}y",
            "x\u{2028}y",
        ] {
            assert!(normalize_chat_text(text).is_err(), "{text:?}");
        }
        assert!(normalize_chat_text(&"a".repeat(MAX_CHAT_CHARS + 1)).is_err());
        assert!(normalize_chat_text(&" ".repeat(MAX_CHAT_BYTES + 1)).is_err());
    }

    #[test]
    fn requests_reject_bad_envelopes_unknown_reactions_and_client_sender_claims() {
        let subscribe = request(SocialCommand::Subscribe);
        assert!(validate_request(&subscribe).is_ok());
        assert_eq!(
            serde_json::to_value(&subscribe.command).unwrap(),
            serde_json::json!({"kind": "subscribe"})
        );
        let mut unscoped = subscribe;
        unscoped.server_epoch = 0;
        assert!(validate_request(&unscoped).is_err());
        let valid = request(SocialCommand::Reaction {
            reaction_id: "heart".into(),
        });
        assert!(validate_request(&valid).is_ok());
        for field in ["request_id", "server_epoch", "match_id"] {
            let mut value = serde_json::to_value(&valid).unwrap();
            value[field] = 0.into();
            assert!(validate_request(&serde_json::from_value(value).unwrap()).is_err());
        }
        for id in [
            "unknown",
            "https://example.test/heart.png",
            "../heart",
            "HEART",
        ] {
            assert!(
                validate_request(&request(SocialCommand::Reaction {
                    reaction_id: id.into()
                }))
                .is_err()
            );
        }
        let mut value = serde_json::to_value(&valid).unwrap();
        value["player_id"] = 99.into();
        assert!(serde_json::from_value::<SocialRequest>(value).is_err());
        let mut bad_session = valid;
        bad_session.session_id = "other/session".into();
        assert!(validate_request(&bad_session).is_err());
    }

    #[test]
    fn packaged_catalog_has_free_baseline_and_independent_safe_fallback() {
        let expected: Vec<_> = BASE_REACTIONS
            .iter()
            .map(|(id, _)| id.to_string())
            .collect();
        assert_eq!(allowed_reactions(&Entitlements::default()), expected);
        assert_eq!(catalog_from_packaged("broken"), fallback_catalog());
        let mut locked = fallback_catalog();
        locked.packs[0].access = PackAccess::Nft {
            provider: "future".into(),
        };
        assert_eq!(
            catalog_from_packaged(&serde_json::to_string(&locked).unwrap()),
            fallback_catalog()
        );
    }

    #[test]
    fn catalog_rejects_duplicates_unknown_fields_paths_and_missing_packs() {
        let baseline = fallback_catalog();
        let mut duplicate = baseline.clone();
        duplicate.reactions.push(duplicate.reactions[0].clone());
        assert!(duplicate.validate().is_err());
        let mut missing = baseline.clone();
        missing.reactions[0].pack_id = "missing".into();
        assert!(missing.validate().is_err());
        let mut path = baseline.clone();
        path.reactions[0].id = "https://example.test/a.png".into();
        assert!(path.validate().is_err());
        let mut value = serde_json::to_value(baseline).unwrap();
        value["reactions"][0]["url"] = "https://example.test/a.png".into();
        assert!(ReactionCatalog::from_json(&value.to_string()).is_err());
        assert!(ReactionCatalog::from_json(&" ".repeat(MAX_CATALOG_BYTES + 1)).is_err());
    }

    #[test]
    fn grants_are_explicit_and_never_unlock_generic_nft_packs() {
        let mut catalog = fallback_catalog();
        for (id, access) in [
            ("operator", PackAccess::OperatorGrant),
            (
                "future_nft",
                PackAccess::Nft {
                    provider: "ekza_passport".into(),
                },
            ),
        ] {
            catalog.packs.push(ReactionPack {
                id: id.into(),
                label: id.into(),
                access,
            });
            catalog.reactions.push(ReactionDefinition {
                id: id.into(),
                label: id.into(),
                pack_id: id.into(),
            });
        }
        catalog.validate().unwrap();
        let mut entitlements = Entitlements::default();
        assert!(!catalog.reaction_allowed("operator", &entitlements));
        entitlements.grant_operator_pack("operator").unwrap();
        entitlements.grant_operator_pack("future_nft").unwrap();
        assert!(catalog.reaction_allowed("operator", &entitlements));
        assert!(!catalog.reaction_allowed("future_nft", &entitlements));
        assert!(!catalog.reaction_allowed("unknown", &entitlements));
        assert!(entitlements.grant_operator_pack("../secret").is_err());
    }

    #[test]
    fn companion_grants_require_exact_approved_avatar_response_and_stay_scoped() {
        use ekza_bevy_sdk::passport::{ProjectSupport, Rendition};
        // Trusted-service response fixture, not a real NFT ownership assertion.
        let expected = ProtectedAvatar {
            avatar_id: format!("solana:devnet:avatar-data:{}", "1".repeat(32)),
            support: ProjectSupport {
                project_id: "omoba".into(),
                platform: "desktop".into(),
                profile: "humanoid-glb-v1".into(),
                status: "approved".into(),
                rendition: Rendition {
                    id: "r1".into(),
                    url: "https://example.test/avatar.glb".into(),
                    sha256: "a".repeat(64),
                    size_bytes: 20,
                    format: "glb".into(),
                },
            },
        };
        let mut consumed = ConsumedTicket {
            wallet: "2".repeat(32),
            mint: "3".repeat(32),
            avatar_id: expected.avatar_id.clone(),
            expires_at: "2099-01-01T00:00:00Z".into(),
            support: expected.support.clone(),
        };
        let mut catalog = fallback_catalog();
        for (id, avatar_id) in [
            ("companion", expected.avatar_id.clone()),
            (
                "other_companion",
                format!("solana:devnet:avatar-data:{}", "4".repeat(32)),
            ),
        ] {
            catalog.packs.push(ReactionPack {
                id: id.into(),
                label: id.into(),
                access: PackAccess::VerifiedAvatar { avatar_id },
            });
            catalog.reactions.push(ReactionDefinition {
                id: id.into(),
                label: id.into(),
                pack_id: id.into(),
            });
        }
        catalog.validate().unwrap();
        let mut entitlements = Entitlements::default();
        consumed.support.rendition.sha256 = "b".repeat(64);
        assert!(
            entitlements
                .grant_verified_avatar(&expected, &consumed)
                .is_err()
        );
        assert!(!catalog.reaction_allowed("companion", &entitlements));
        consumed.support = expected.support.clone();
        entitlements
            .grant_verified_avatar(&expected, &consumed)
            .unwrap();
        assert!(catalog.reaction_allowed("companion", &entitlements));
        assert!(!catalog.reaction_allowed("other_companion", &entitlements));
        assert!(!catalog.reaction_allowed("companion", &Entitlements::default()));
    }

    #[test]
    fn trusted_grant_storage_is_bounded_and_duplicates_are_idempotent() {
        let mut entitlements = Entitlements::default();
        for n in 0..MAX_PACKS {
            entitlements
                .grant_operator_pack(&format!("pack_{n}"))
                .unwrap();
        }
        entitlements.grant_operator_pack("pack_0").unwrap();
        assert_eq!(entitlements.operator_pack_ids.len(), MAX_PACKS);
        assert!(entitlements.grant_operator_pack("overflow").is_err());
    }

    #[test]
    fn views_round_trip_without_implying_entitlement_from_old_payloads() {
        let view: SocialView = serde_json::from_str("{}").unwrap();
        assert!(view.allowed_reactions.is_empty());
        let event = SocialEvent {
            id: 2,
            player_id: 7,
            nickname: "英雄".into(),
            team: SocialTeam::Blue,
            age_ms: 400,
            kind: SocialEventKind::Chat {
                channel: SocialChannel::Team,
                text: "走吧".into(),
            },
        };
        let encoded = serde_json::to_vec(&event).unwrap();
        assert_eq!(
            serde_json::from_slice::<SocialEvent>(&encoded).unwrap(),
            event
        );
    }
}
