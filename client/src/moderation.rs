//! Moderation refusals in the player's language (LR-15): a suspended account,
//! a matchmaking or ranked ban, a locked rename, a chat mute and chat switched
//! off on the server. The server sends a stable code and, with the career view,
//! each active sanction's end; the text and the date are the client's.
// i18n-strict
use crate::i18n::{tr, trf};
use shared::moderation::{Sanction, SanctionKind, code};

/// How long a suspended client waits before it signs in again: lifting the
/// ban restores access without restarting the game.
pub(crate) const SUSPENDED_RETRY: std::time::Duration = std::time::Duration::from_secs(60);

/// The wall clock in Unix seconds, as sanctions measure their end.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().min(i64::MAX as u64) as i64)
}

/// The text of a sanction of `kind` with its end, or without a date when it
/// lasts until lifted (or the end is unknown).
pub(crate) fn sanction_text(kind: SanctionKind, until: Option<i64>) -> Option<String> {
    let (dated, plain) = match kind {
        SanctionKind::AccountBan => (
            "moderation.account_banned.until",
            "moderation.account_banned.plain",
        ),
        SanctionKind::MatchmakingBan => (
            "moderation.matchmaking_ban.until",
            "moderation.matchmaking_ban.plain",
        ),
        SanctionKind::RankedBan => ("moderation.ranked_ban.until", "moderation.ranked_ban.plain"),
        SanctionKind::NameReset => (
            "moderation.name_locked.until",
            "moderation.name_locked.plain",
        ),
        SanctionKind::ChatMute => ("moderation.chat_muted.until", "moderation.chat_muted.plain"),
        SanctionKind::Unknown => return None,
    };
    Some(match until {
        Some(until) => trf(dated, &[("until", &crate::supporter::utc_timestamp(until))]),
        None => tr(plain).to_owned(),
    })
}

/// The refusal text of a moderation `code`, with the end of the matching
/// sanction from `sanctions`; `None` for any other code.
pub(crate) fn refusal_text(code: &str, sanctions: &[Sanction]) -> Option<String> {
    if code == code::CHAT_DISABLED {
        return Some(tr("moderation.chat_disabled").to_owned());
    }
    let kind = shared::moderation::sanction_for_code(code)?;
    let until = shared::moderation::active(sanctions, kind, unix_now()).and_then(|s| s.until);
    sanction_text(kind, until)
}

/// What the chat panel says instead of offering to type: chat is off on
/// this server, or this player is muted.
pub(crate) fn chat_notice(chat_disabled: bool, mute: Option<Sanction>) -> Option<String> {
    if chat_disabled {
        return Some(tr("moderation.chat_disabled").to_owned());
    }
    mute.filter(|mute| mute.active_at(unix_now()))
        .and_then(|mute| sanction_text(SanctionKind::ChatMute, mute.until))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::App;

    /// Every refusal reads in the player's language with its end; the
    /// account ban, in Russian, says until when.
    #[test]
    fn refusals_follow_the_language_and_name_their_end() {
        if crate::i18n::testing::isolated(
            "moderation::tests::refusals_follow_the_language_and_name_their_end",
        ) {
            return;
        }
        use crate::i18n::{I18nPlugin, Locale, LocaleId};
        let until = 1_793_005_200; // 2026-10-26 09:00 UTC
        let ban = |kind| Sanction {
            kind,
            until: Some(until),
        };
        let sanctions = [
            ban(SanctionKind::AccountBan),
            ban(SanctionKind::MatchmakingBan),
            ban(SanctionKind::RankedBan),
            ban(SanctionKind::NameReset),
            ban(SanctionKind::ChatMute),
        ];
        let mut app = App::new();
        app.add_plugins(I18nPlugin::default());
        for (locale, banned, disabled) in [
            (
                "en",
                "Your account is suspended until",
                "Chat is turned off",
            ),
            (
                "ru",
                "Ваш аккаунт заблокирован до",
                "Чат на этом сервере выключен",
            ),
            (
                "zh-Hans",
                "你的账号已被暂停使用，直到",
                "此服务器已关闭聊天",
            ),
        ] {
            app.world_mut()
                .resource_mut::<Locale>()
                .set(LocaleId::parse(locale).unwrap());
            let text = refusal_text(code::ACCOUNT_BANNED, &sanctions).unwrap();
            assert!(text.starts_with(banned), "{locale}: {text}");
            assert!(text.contains("2026-10-26 09:00"), "{locale}: {text}");
            for code in [
                code::MATCHMAKING_BAN,
                code::RANKED_BAN,
                code::NAME_LOCKED,
                code::CHAT_MUTED,
            ] {
                let text = refusal_text(code, &sanctions).unwrap();
                assert!(text.contains("2026-10-26 09:00"), "{locale} {code}: {text}");
            }
            assert!(
                refusal_text(code::CHAT_DISABLED, &[])
                    .unwrap()
                    .starts_with(disabled),
                "{locale}"
            );
            // Without a known end the text has no date.
            let plain = refusal_text(code::ACCOUNT_BANNED, &[]).unwrap();
            assert!(!plain.contains("2026"), "{locale}: {plain}");
        }
    }

    #[test]
    fn only_moderation_codes_have_moderation_text() {
        assert_eq!(refusal_text("queue_full", &[]), None);
        assert_eq!(sanction_text(SanctionKind::Unknown, None), None);
        assert!(chat_notice(false, None).is_none());
        let ended = Sanction {
            kind: SanctionKind::ChatMute,
            until: Some(1),
        };
        assert!(chat_notice(false, Some(ended)).is_none(), "an ended mute");
        assert!(chat_notice(true, None).is_some());
        assert!(
            chat_notice(
                false,
                Some(Sanction {
                    kind: SanctionKind::ChatMute,
                    until: None
                })
            )
            .is_some()
        );
    }
}
