//! Moderation (LR-15): the account-ban check of every website session and
//! sign-in, and the operator commands of the `omoba-account-api` binary.
//!
//! ```text
//! omoba-account-api player <nickname#1234 | profile-id>
//! omoba-account-api sanction add <player> <kind> (--for <duration> | --permanent) --reason <text> --by <operator>
//! omoba-account-api sanction list <player> [--all]
//! omoba-account-api sanction lift <sanction-id> --reason <text> --by <operator>
//! omoba-account-api rename <player> (--for <duration> | --permanent) --reason <text> --by <operator>
//! omoba-account-api chat [status | on | off --reason <text> --by <operator>]
//! ```
//!
//! Kinds: `chat_mute`, `matchmaking_ban`, `ranked_ban`, `account_ban`,
//! `name_reset` (`rename` is `sanction add … name_reset`: the name becomes
//! `Player#NNNN` and the player cannot rename until it ends). Durations:
//! `<n>m`, `<n>h`, `<n>d` or `<n>w`. The commands use `OMOBA_DATABASE_URL`
//! (the portal role is enough) and change nothing but the moderation tables
//! and, for a name reset, the name. Game servers apply a change within about
//! ten seconds, without a restart. `RUNBOOK.md` › Moderation is the procedure.
use crate::{Error, Result};
use axum::http::StatusCode;
use omoba_career_store::career_store::{
    CareerStore,
    moderation::{NewSanction, SanctionRecord},
};
use shared::moderation::SanctionKind;
use std::fmt::Write as _;

/// `account_banned` (403) while `profile_id` has an active account ban: no
/// website session works and none is created.
pub async fn refuse_banned<'e, E>(executor: E, profile_id: &str) -> Result<()>
where
    E: sqlx::PgExecutor<'e>,
{
    let banned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM career_sanctions WHERE profile_id=$1 AND kind='account_ban' \
         AND lifted_at IS NULL AND (expires_at IS NULL OR expires_at > clock_timestamp()))",
    )
    .bind(profile_id)
    .fetch_one(executor)
    .await?;
    if banned {
        return Err(Error(StatusCode::FORBIDDEN, "account_banned"));
    }
    Ok(())
}

/// One parsed operator command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Player {
        player: String,
    },
    Add {
        player: String,
        kind: SanctionKind,
        duration_secs: Option<i64>,
        reason: String,
        by: String,
    },
    List {
        player: String,
        all: bool,
    },
    Lift {
        sanction_id: i64,
        reason: String,
        by: String,
    },
    ChatStatus,
    Chat {
        enabled: bool,
        reason: String,
        by: String,
    },
}

/// The first word of every operator command.
pub const COMMANDS: [&str; 4] = ["player", "sanction", "rename", "chat"];

pub const USAGE: &str = "\
usage:
  omoba-account-api player <nickname#1234 | profile-id>
  omoba-account-api sanction add <player> <kind> (--for <duration> | --permanent) --reason <text> --by <operator>
  omoba-account-api sanction list <player> [--all]
  omoba-account-api sanction lift <sanction-id> --reason <text> --by <operator>
  omoba-account-api rename <player> (--for <duration> | --permanent) --reason <text> --by <operator>
  omoba-account-api chat [status | on | off --reason <text> --by <operator>]
kinds: chat_mute, matchmaking_ban, ranked_ban, account_ban, name_reset
durations: <n>m, <n>h, <n>d or <n>w (for example 30m, 12h, 7d, 2w)";

/// `30m`, `12h`, `7d` or `2w` in seconds.
pub fn duration(value: &str) -> std::result::Result<i64, String> {
    let (number, unit) = value.split_at(value.len().saturating_sub(1));
    let scale = match unit {
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 7 * 86_400,
        _ => {
            return Err(format!(
                "Invalid duration {value:?}: use 30m, 12h, 7d or 2w."
            ));
        }
    };
    number
        .parse::<i64>()
        .ok()
        .filter(|n| *n > 0)
        .and_then(|n| n.checked_mul(scale))
        .ok_or_else(|| format!("Invalid duration {value:?}: use 30m, 12h, 7d or 2w."))
}

/// Parses the operator arguments (everything after the binary name).
pub fn parse(args: &[String]) -> std::result::Result<Command, String> {
    let mut words = Vec::new();
    let mut reason = None;
    let mut by = None;
    let mut length = None;
    let mut permanent = false;
    let mut all = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = |name: &str| {
            rest.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value.\n{USAGE}"))
        };
        match arg.as_str() {
            "--reason" => reason = Some(value("--reason")?),
            "--by" => by = Some(value("--by")?),
            "--for" => length = Some(duration(&value("--for")?)?),
            "--permanent" => permanent = true,
            "--all" => all = true,
            flag if flag.starts_with("--") => {
                return Err(format!("Unknown option {flag}.\n{USAGE}"));
            }
            word => words.push(word.to_owned()),
        }
    }
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let need = |value: Option<String>, name: &str| {
        value
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| format!("{name} is required.\n{USAGE}"))
    };
    let lasting = || match (length, permanent) {
        (Some(secs), false) => Ok(Some(secs)),
        (None, true) => Ok(None),
        _ => Err(format!(
            "Give exactly one of --for <duration> or --permanent.\n{USAGE}"
        )),
    };
    let used = [
        ("--reason", reason.is_some()),
        ("--by", by.is_some()),
        ("--for", length.is_some()),
        ("--permanent", permanent),
        ("--all", all),
    ];
    let flags_unused = move |allowed: &[&str]| match used
        .iter()
        .find(|(flag, set)| *set && !allowed.contains(flag))
    {
        Some((flag, _)) => Err(format!("{flag} does not apply here.\n{USAGE}")),
        None => Ok(()),
    };
    let command = match words.as_slice() {
        ["player", player] => {
            flags_unused(&[])?;
            Command::Player {
                player: (*player).to_owned(),
            }
        }
        ["sanction", "add", player, kind] => Command::Add {
            player: (*player).to_owned(),
            kind: SanctionKind::parse(kind)
                .ok_or_else(|| format!("Unknown sanction kind {kind:?}.\n{USAGE}"))?,
            duration_secs: lasting()?,
            reason: need(reason, "--reason")?,
            by: need(by, "--by")?,
        },
        ["rename", player] => Command::Add {
            player: (*player).to_owned(),
            kind: SanctionKind::NameReset,
            duration_secs: lasting()?,
            reason: need(reason, "--reason")?,
            by: need(by, "--by")?,
        },
        ["sanction", "list", player] => {
            flags_unused(&["--all"])?;
            Command::List {
                player: (*player).to_owned(),
                all,
            }
        }
        ["sanction", "lift", id] => {
            flags_unused(&["--reason", "--by"])?;
            Command::Lift {
                sanction_id: id
                    .parse()
                    .map_err(|_| format!("Invalid sanction number {id:?}.\n{USAGE}"))?,
                reason: need(reason, "--reason")?,
                by: need(by, "--by")?,
            }
        }
        ["chat"] | ["chat", "status"] => {
            flags_unused(&[])?;
            Command::ChatStatus
        }
        ["chat", switch @ ("on" | "off")] => {
            flags_unused(&["--reason", "--by"])?;
            Command::Chat {
                enabled: *switch == "on",
                reason: need(reason, "--reason")?,
                by: need(by, "--by")?,
            }
        }
        _ => return Err(USAGE.into()),
    };
    if let Command::Add { .. } = command {
        flags_unused(&["--reason", "--by", "--for", "--permanent"])?;
    }
    Ok(command)
}

/// `YYYY-MM-DD HH:MM UTC` for Unix seconds.
pub fn utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let second = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        second / 3600,
        second % 3600 / 60
    )
}

fn until(expires_at: Option<i64>) -> String {
    expires_at.map_or_else(|| "until lifted".into(), |at| format!("until {}", utc(at)))
}

fn line(out: &mut String, s: &SanctionRecord) {
    let state = if s.active {
        until(s.expires_at)
    } else if let (Some(at), Some(by)) = (s.lifted_at, &s.lifted_by) {
        format!(
            "lifted {} by {by}: {}",
            utc(at),
            s.lift_reason.as_deref().unwrap_or("")
        )
    } else {
        format!("ended {}", s.expires_at.map(utc).unwrap_or_default())
    };
    let _ = writeln!(
        out,
        "  #{} {} · {} · set {} by {} · {}",
        s.sanction_id,
        s.kind.id(),
        state,
        utc(s.created_at),
        s.created_by,
        s.reason
    );
}

/// Runs one operator command against `store` and returns what it prints.
pub async fn run(store: &CareerStore, command: Command) -> std::result::Result<String, String> {
    let mut out = String::new();
    match command {
        Command::Player { player } => {
            let profile = store.find_profile(&player).await?;
            let _ = writeln!(out, "{} ({})", profile.nickname, profile.profile_id);
            let _ = writeln!(
                out,
                "  matches {} · rated {} · rating {} · XP {}",
                profile.matches_played,
                profile.rated_matches,
                profile.rating,
                profile.progression_xp
            );
            let _ = writeln!(out, "device keys:");
            for key in store.profile_keys(&profile.profile_id).await? {
                let _ = writeln!(
                    out,
                    "  {}… {} · added {}{}",
                    key.key_prefix,
                    key.label,
                    utc(key.created_at),
                    if key.revoked { " · revoked" } else { "" }
                );
            }
            let sanctions = store.sanctions(&profile.profile_id, true).await?;
            let _ = writeln!(
                out,
                "sanctions:{}",
                if sanctions.is_empty() { " none" } else { "" }
            );
            for sanction in &sanctions {
                line(&mut out, sanction);
            }
            let events = store
                .moderation_events(Some(&profile.profile_id), 20)
                .await?;
            let _ = writeln!(
                out,
                "moderation log:{}",
                if events.is_empty() { " none" } else { "" }
            );
            for event in events {
                let _ = writeln!(
                    out,
                    "  {} {} {} {}",
                    utc(event.created_at),
                    event.actor,
                    event.action,
                    event.detail
                );
            }
        }
        Command::Add {
            player,
            kind,
            duration_secs,
            reason,
            by,
        } => {
            let profile = store.find_profile(&player).await?;
            let applied = store
                .add_sanction(NewSanction {
                    profile_id: &profile.profile_id,
                    kind,
                    reason: &reason,
                    duration_secs,
                    actor: &by,
                })
                .await?;
            let _ = writeln!(
                out,
                "Applied #{} {} to {} ({}), {}.",
                applied.sanction.sanction_id,
                kind.id(),
                profile.nickname,
                profile.profile_id,
                until(applied.sanction.expires_at)
            );
            if let Some((old, new)) = applied.renamed {
                let _ = writeln!(out, "Renamed {old} -> {new}.");
            }
            let _ = writeln!(out, "Game servers apply it within about 10 seconds.");
        }
        Command::List { player, all } => {
            let profile = store.find_profile(&player).await?;
            let sanctions = store.sanctions(&profile.profile_id, all).await?;
            let _ = writeln!(
                out,
                "{} ({}): {} {}sanction(s)",
                profile.nickname,
                profile.profile_id,
                sanctions.len(),
                if all { "" } else { "active " }
            );
            for sanction in &sanctions {
                line(&mut out, sanction);
            }
        }
        Command::Lift {
            sanction_id,
            reason,
            by,
        } => {
            let lifted = store.lift_sanction(sanction_id, &reason, &by).await?;
            let _ = writeln!(
                out,
                "Lifted #{} {} of {}. Game servers restore access within about 10 seconds.",
                lifted.sanction_id,
                lifted.kind.id(),
                lifted.profile_id
            );
        }
        Command::ChatStatus => {
            let switch = store.chat_switch().await?;
            let _ = writeln!(
                out,
                "Chat is {} in the database (since {}, by {}). A server started with OMOBA_CHAT=off keeps chat off whatever this says.",
                if switch.enabled { "on" } else { "off" },
                utc(switch.changed_at),
                switch.changed_by
            );
        }
        Command::Chat {
            enabled,
            reason,
            by,
        } => {
            store.set_chat(enabled, &reason, &by).await?;
            let _ = writeln!(
                out,
                "Chat turned {} for every game server of this database; they apply it within about 10 seconds. Reactions are not affected.",
                if enabled { "on" } else { "off" }
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut quoted = false;
        let mut word = String::new();
        for c in line.chars() {
            match c {
                '"' => quoted = !quoted,
                ' ' if !quoted => {
                    if !word.is_empty() {
                        out.push(std::mem::take(&mut word));
                    }
                }
                c => word.push(c),
            }
        }
        if !word.is_empty() {
            out.push(word);
        }
        out
    }

    #[test]
    fn commands_parse_with_explicit_length_reason_and_operator() {
        assert_eq!(
            parse(&args(
                "sanction add Fox#0001 chat_mute --for 12h --reason \"Spam in chat\" --by anna"
            )),
            Ok(Command::Add {
                player: "Fox#0001".into(),
                kind: SanctionKind::ChatMute,
                duration_secs: Some(12 * 3600),
                reason: "Spam in chat".into(),
                by: "anna".into()
            })
        );
        assert_eq!(
            parse(&args("rename Fox#0001 --permanent --reason Slur --by anna")),
            Ok(Command::Add {
                player: "Fox#0001".into(),
                kind: SanctionKind::NameReset,
                duration_secs: None,
                reason: "Slur".into(),
                by: "anna".into()
            })
        );
        assert_eq!(
            parse(&args("sanction list Fox#0001 --all")),
            Ok(Command::List {
                player: "Fox#0001".into(),
                all: true
            })
        );
        assert_eq!(
            parse(&args("sanction lift 7 --reason Appeal --by boris")),
            Ok(Command::Lift {
                sanction_id: 7,
                reason: "Appeal".into(),
                by: "boris".into()
            })
        );
        assert_eq!(parse(&args("chat")), Ok(Command::ChatStatus));
        assert_eq!(
            parse(&args("chat off --reason Raid --by anna")),
            Ok(Command::Chat {
                enabled: false,
                reason: "Raid".into(),
                by: "anna".into()
            })
        );
        assert_eq!(
            parse(&args("player Fox#0001")),
            Ok(Command::Player {
                player: "Fox#0001".into()
            })
        );
    }

    #[test]
    fn incomplete_or_ambiguous_commands_are_refused() {
        for line in [
            "sanction add Fox#0001 chat_mute --reason Spam --by anna",
            "sanction add Fox#0001 chat_mute --for 1h --permanent --reason Spam --by anna",
            "sanction add Fox#0001 chat_mute --for 1h --by anna",
            "sanction add Fox#0001 chat_mute --for 1h --reason Spam",
            "sanction add Fox#0001 voice_mute --for 1h --reason Spam --by anna",
            "sanction add Fox#0001 chat_mute --for 0d --reason Spam --by anna",
            "sanction add Fox#0001 chat_mute --for 3y --reason Spam --by anna",
            "sanction add Fox#0001 chat_mute --for 1h --reason Spam --by anna --all",
            "sanction lift seven --reason Appeal --by boris",
            "sanction lift 7 --by boris",
            "rename Fox#0001 extra --for 1d --reason Slur --by anna",
            "chat off --by anna",
            "chat status --by anna",
            "player",
            "player Fox#0001 --all",
            "sanction",
            "nonsense",
        ] {
            assert!(parse(&args(line)).is_err(), "{line}");
        }
    }

    #[test]
    fn durations_and_dates() {
        assert_eq!(duration("30m"), Ok(1800));
        assert_eq!(duration("2w"), Ok(14 * 86_400));
        assert!(duration("-1d").is_err() && duration("d").is_err() && duration("").is_err());
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(1_793_005_200), "2026-10-26 09:00 UTC");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00 UTC");
    }
}
