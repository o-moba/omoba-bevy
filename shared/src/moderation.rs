//! Moderation as the game sees it (LR-15): the sanctions an operator puts on a
//! profile, the codes a refusal carries, and the server-wide chat switch.
//!
//! Sanctions live in the career database (career schema 5) and are applied with
//! the `omoba-account-api sanction` commands (`RUNBOOK.md` › Moderation). Game
//! servers read a profile's active sanctions at sign-in and with every access
//! refresh, so applying or lifting one takes effect without a restart. Clients
//! receive only the kind and the end of each active sanction, never the
//! operator's reason.
use serde::{Deserialize, Serialize};

use crate::match_service::MatchPreference;

/// The server environment variable that switches text chat: `on` (the default)
/// or `off`. Reactions are never switched off. The RU lobby unit sets `off`
/// (owner default O-9); `omoba-account-api chat off` turns chat off at runtime
/// on every server of the database without a restart.
pub const CHAT_ENV: &str = "OMOBA_CHAT";

/// English fallback of [`code::ACCOUNT_BANNED`]; clients show their own copy.
pub const ACCOUNT_BANNED_TEXT: &str = "This account is suspended.";
/// English fallback of [`code::NAME_LOCKED`].
pub const NAME_LOCKED_TEXT: &str = "Your name was reset by a moderator and cannot be changed yet.";
/// English fallback of [`code::CHAT_MUTED`].
pub const CHAT_MUTED_TEXT: &str = "You are muted in chat.";
/// English fallback of [`code::CHAT_DISABLED`].
pub const CHAT_DISABLED_TEXT: &str = "Chat is turned off on this server. Reactions still work.";

/// Stable codes of moderation refusals: `CareerView::error_code`, a lobby
/// `MatchServiceView::Failed` code, or `SocialView::error_code`.
pub mod code {
    /// Sign-in refused: the profile has an active `account_ban`.
    pub const ACCOUNT_BANNED: &str = "account_banned";
    /// `FindMatch` refused: an active `matchmaking_ban`.
    pub const MATCHMAKING_BAN: &str = "matchmaking_ban";
    /// A Ranked `FindMatch` refused: an active `ranked_ban`.
    pub const RANKED_BAN: &str = "ranked_ban";
    /// A rename refused: an active `name_reset`.
    pub const NAME_LOCKED: &str = "name_locked";
    /// A chat message refused: an active `chat_mute`.
    pub const CHAT_MUTED: &str = "chat_muted";
    /// A chat message refused: chat is switched off on this server.
    pub const CHAT_DISABLED: &str = "chat_disabled";
}

/// What a sanction restricts. Stored as its [`SanctionKind::id`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SanctionKind {
    /// No chat messages; reactions stay.
    ChatMute,
    /// No match search in any queue.
    MatchmakingBan,
    /// No Ranked search; Casual PvP and vs Bots stay.
    RankedBan,
    /// No game sign-in and no website session.
    AccountBan,
    /// The name was reset to `Player#NNNN`; the player cannot rename until it ends.
    NameReset,
    /// A kind this build does not know (a newer server): restricts nothing here.
    #[serde(other)]
    Unknown,
}

impl SanctionKind {
    /// Every kind an operator may apply.
    pub const ALL: [Self; 5] = [
        Self::ChatMute,
        Self::MatchmakingBan,
        Self::RankedBan,
        Self::AccountBan,
        Self::NameReset,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::ChatMute => "chat_mute",
            Self::MatchmakingBan => "matchmaking_ban",
            Self::RankedBan => "ranked_ban",
            Self::AccountBan => "account_ban",
            Self::NameReset => "name_reset",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    /// Whether party presence hides a profile with this sanction: it can
    /// neither sign in nor search for a match, so nobody should invite it.
    pub const fn hides_presence(self) -> bool {
        matches!(self, Self::AccountBan | Self::MatchmakingBan)
    }
}

/// One active sanction as a game client and server see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sanction {
    pub kind: SanctionKind,
    /// When it ends, in Unix seconds; `None` until an operator lifts it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
}

impl Sanction {
    /// Whether it still applies at `now` (Unix seconds).
    pub fn active_at(&self, now: i64) -> bool {
        self.kind != SanctionKind::Unknown && self.until.is_none_or(|until| until > now)
    }
}

/// The active sanction of `kind` that ends last (a permanent one first).
pub fn active(sanctions: &[Sanction], kind: SanctionKind, now: i64) -> Option<Sanction> {
    sanctions
        .iter()
        .filter(|s| s.kind == kind && s.active_at(now))
        .max_by_key(|s| s.until.unwrap_or(i64::MAX))
        .copied()
}

/// The sanction that refuses a `FindMatch` for `preference`, with its lobby
/// failure code: a `matchmaking_ban` refuses every queue, a `ranked_ban`
/// only Ranked.
pub fn queue_refusal(
    sanctions: &[Sanction],
    preference: MatchPreference,
    now: i64,
) -> Option<(&'static str, Sanction)> {
    if let Some(ban) = active(sanctions, SanctionKind::MatchmakingBan, now) {
        return Some((code::MATCHMAKING_BAN, ban));
    }
    if preference == MatchPreference::Ranked {
        return active(sanctions, SanctionKind::RankedBan, now).map(|ban| (code::RANKED_BAN, ban));
    }
    None
}

/// Whether party presence hides a profile with these sanctions.
pub fn hides_presence(sanctions: &[Sanction], now: i64) -> bool {
    sanctions
        .iter()
        .any(|s| s.kind.hides_presence() && s.active_at(now))
}

/// The sanction a moderation code is about, for the client's "until …" text.
pub fn sanction_for_code(code: &str) -> Option<SanctionKind> {
    Some(match code {
        code::ACCOUNT_BANNED => SanctionKind::AccountBan,
        code::MATCHMAKING_BAN => SanctionKind::MatchmakingBan,
        code::RANKED_BAN => SanctionKind::RankedBan,
        code::NAME_LOCKED => SanctionKind::NameReset,
        code::CHAT_MUTED => SanctionKind::ChatMute,
        _ => return None,
    })
}

/// Reads [`CHAT_ENV`]: unset or empty is on; anything but `on`/`off` stops
/// the server, like the admission limits.
pub fn chat_from_env(value: Option<&str>) -> Result<bool, String> {
    match value.map(str::trim) {
        None | Some("") | Some("on") => Ok(true),
        Some("off") => Ok(false),
        Some(other) => Err(format!("{CHAT_ENV} must be on or off, not {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_and_unknown_kinds_restrict_nothing() {
        for kind in SanctionKind::ALL {
            assert_eq!(SanctionKind::parse(kind.id()), Some(kind));
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.id()));
        }
        assert_eq!(SanctionKind::parse("unknown"), None);
        // An older client reading a newer server's kind keeps the whole view.
        let future: Vec<Sanction> =
            serde_json::from_str(r#"[{"kind":"voice_mute","until":5},{"kind":"chat_mute"}]"#)
                .unwrap();
        assert_eq!(future[0].kind, SanctionKind::Unknown);
        assert!(!future[0].active_at(0));
        assert_eq!(
            future[1],
            Sanction {
                kind: SanctionKind::ChatMute,
                until: None
            }
        );
    }

    #[test]
    fn expiry_and_queue_refusals() {
        let mute = Sanction {
            kind: SanctionKind::ChatMute,
            until: Some(100),
        };
        assert!(mute.active_at(99));
        assert!(!mute.active_at(100));
        let ranked = Sanction {
            kind: SanctionKind::RankedBan,
            until: Some(200),
        };
        let all = [mute, ranked];
        assert_eq!(
            queue_refusal(&all, MatchPreference::Ranked, 150),
            Some((code::RANKED_BAN, ranked))
        );
        assert_eq!(queue_refusal(&all, MatchPreference::Casual, 150), None);
        assert_eq!(queue_refusal(&all, MatchPreference::Ranked, 200), None);
        let banned = [
            ranked,
            Sanction {
                kind: SanctionKind::MatchmakingBan,
                until: None,
            },
        ];
        for preference in [
            MatchPreference::Ranked,
            MatchPreference::Casual,
            MatchPreference::BotPractice,
        ] {
            assert_eq!(
                queue_refusal(&banned, preference, 150).map(|(code, _)| code),
                Some(code::MATCHMAKING_BAN)
            );
        }
        assert!(hides_presence(&banned, 150));
        assert!(!hides_presence(&all, 150));
    }

    #[test]
    fn the_longest_active_sanction_of_a_kind_wins() {
        let short = Sanction {
            kind: SanctionKind::ChatMute,
            until: Some(10),
        };
        let long = Sanction {
            kind: SanctionKind::ChatMute,
            until: Some(50),
        };
        assert_eq!(
            active(&[short, long], SanctionKind::ChatMute, 5),
            Some(long)
        );
        let forever = Sanction {
            kind: SanctionKind::ChatMute,
            until: None,
        };
        assert_eq!(
            active(&[long, forever], SanctionKind::ChatMute, 5),
            Some(forever)
        );
        assert_eq!(active(&[short], SanctionKind::ChatMute, 10), None);
    }

    #[test]
    fn the_chat_switch_is_on_unless_set_off() {
        assert_eq!(chat_from_env(None), Ok(true));
        assert_eq!(chat_from_env(Some("")), Ok(true));
        assert_eq!(chat_from_env(Some("on")), Ok(true));
        assert_eq!(chat_from_env(Some(" off ")), Ok(false));
        assert!(chat_from_env(Some("false")).is_err());
    }

    #[test]
    fn every_refusal_code_names_its_sanction() {
        for (code, kind) in [
            (code::ACCOUNT_BANNED, SanctionKind::AccountBan),
            (code::MATCHMAKING_BAN, SanctionKind::MatchmakingBan),
            (code::RANKED_BAN, SanctionKind::RankedBan),
            (code::NAME_LOCKED, SanctionKind::NameReset),
            (code::CHAT_MUTED, SanctionKind::ChatMute),
        ] {
            assert_eq!(sanction_for_code(code), Some(kind));
        }
        assert_eq!(sanction_for_code(code::CHAT_DISABLED), None);
    }
}
