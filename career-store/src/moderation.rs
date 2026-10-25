//! Moderation (LR-15, career schema 5): sanctions on profiles, their audit
//! trail and the database-wide chat switch.
//!
//! Game servers read with [`CareerStore::access`] (at every 10-second access
//! refresh) and [`CareerStore::active_sanctions`] (at sign-in), and poll
//! [`CareerStore::chat_enabled`]. Operators write through the
//! `omoba-account-api sanction|chat|player` commands, which call the methods
//! below with the portal role. Every write commits one
//! `career_moderation_events` row in the same transaction.
use super::{CareerStore, StoreError, StoreResult, check_id, retry};
use shared::{
    career::ProfileSummary,
    moderation::{Sanction, SanctionKind},
};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow, types::Json};

/// A player's own rename while a `name_reset` is active.
pub const NAME_LOCKED_ERROR: &str = shared::moderation::NAME_LOCKED_TEXT;
/// The longest sanction with an end: ten years. Longer ones are permanent.
pub const MAX_SANCTION_SECS: i64 = 10 * 366 * 86_400;
const MAX_REASON_CHARS: usize = 500;
const MAX_ACTOR_CHARS: usize = 64;
/// The forced name of a `name_reset`: `Player#NNNN` with a free tag.
pub const RESET_NAME: &str = "Player";
/// Active: not lifted and not expired, in SQL over `career_sanctions`.
const ACTIVE: &str = "lifted_at IS NULL AND (expires_at IS NULL OR expires_at > clock_timestamp())";

/// One `career_sanctions` row as an operator sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanctionRecord {
    pub sanction_id: i64,
    pub profile_id: String,
    pub kind: SanctionKind,
    pub reason: String,
    pub created_at: i64,
    pub created_by: String,
    /// Unix seconds; `None` until lifted.
    pub expires_at: Option<i64>,
    pub lifted_at: Option<i64>,
    pub lifted_by: Option<String>,
    pub lift_reason: Option<String>,
    /// Neither lifted nor expired now.
    pub active: bool,
}

impl SanctionRecord {
    /// What a game client sees of it.
    pub fn sanction(&self) -> Sanction {
        Sanction {
            kind: self.kind,
            until: self.expires_at,
        }
    }
}

/// A sanction to apply.
#[derive(Debug, Clone, Copy)]
pub struct NewSanction<'a> {
    pub profile_id: &'a str,
    pub kind: SanctionKind,
    /// Why, for the audit trail; never shown to the player.
    pub reason: &'a str,
    /// How long it lasts; `None` until lifted.
    pub duration_secs: Option<i64>,
    /// Who applies it (the operator's login).
    pub actor: &'a str,
}

/// An applied sanction; a `name_reset` also reports the old and the new name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub sanction: SanctionRecord,
    pub renamed: Option<(String, String)>,
}

/// What the game's access refresh reads in one query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    /// The key is this profile's and not revoked.
    pub key_active: bool,
    /// The profile's current name (a moderator may have reset it).
    pub nickname: Option<String>,
    pub sanctions: Vec<Sanction>,
}

/// The database-wide chat switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSwitch {
    pub enabled: bool,
    pub changed_at: i64,
    pub changed_by: String,
}

/// One device key of a profile, as the operator's player lookup lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySummary {
    /// The first 16 hex digits of the public key.
    pub key_prefix: String,
    pub label: String,
    pub created_at: i64,
    pub revoked: bool,
}

/// One row of the moderation audit trail.
#[derive(Debug, Clone, PartialEq)]
pub struct ModerationEvent {
    pub seq: i64,
    pub created_at: i64,
    pub actor: String,
    pub action: String,
    pub profile_id: Option<String>,
    pub sanction_id: Option<i64>,
    pub detail: serde_json::Value,
}

fn text(value: &str, max: usize, what: &str) -> StoreResult<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(format!("The {what} needs 1–{max} characters on one line.").into());
    }
    Ok(value.to_owned())
}

fn actor(value: &str) -> StoreResult<String> {
    let value = text(value, MAX_ACTOR_CHARS, "operator name")?;
    if !value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"._@-".contains(&b))
    {
        return Err("The operator name may use letters, digits and . _ @ - only.".into());
    }
    Ok(value)
}

fn record(row: &PgRow) -> StoreResult<SanctionRecord> {
    let kind: String = row.try_get("kind")?;
    Ok(SanctionRecord {
        sanction_id: row.try_get("sanction_id")?,
        profile_id: row.try_get("profile_id")?,
        kind: SanctionKind::parse(&kind).ok_or("Unknown stored sanction kind.")?,
        reason: row.try_get("reason")?,
        created_at: row.try_get("created")?,
        created_by: row.try_get("created_by")?,
        expires_at: row.try_get("expires")?,
        lifted_at: row.try_get("lifted")?,
        lifted_by: row.try_get("lifted_by")?,
        lift_reason: row.try_get("lift_reason")?,
        active: row.try_get("active")?,
    })
}

/// The columns [`record`] reads.
const RECORD: &str = "sanction_id,profile_id,kind,reason,created_by,lifted_by,lift_reason,\
    floor(extract(epoch FROM created_at))::bigint AS created,\
    ceil(extract(epoch FROM expires_at))::bigint AS expires,\
    floor(extract(epoch FROM lifted_at))::bigint AS lifted,\
    (lifted_at IS NULL AND (expires_at IS NULL OR expires_at > clock_timestamp())) AS active";

/// The active sanctions of a profile as JSON, for one query to read them all.
const ACTIVE_JSON: &str = "COALESCE((SELECT jsonb_agg(jsonb_build_object('kind',kind,\
    'until',ceil(extract(epoch FROM expires_at))::bigint) ORDER BY sanction_id) \
    FROM career_sanctions WHERE profile_id=$1 AND lifted_at IS NULL \
    AND (expires_at IS NULL OR expires_at > clock_timestamp())),'[]'::jsonb)";

/// Whether a `name_reset` is active for this profile, inside a transaction.
pub(super) async fn name_locked(
    tx: &mut Transaction<'_, Postgres>,
    profile_id: &str,
) -> StoreResult<bool> {
    Ok(sqlx::query_scalar(&format!(
        "SELECT EXISTS(SELECT 1 FROM career_sanctions WHERE profile_id=$1 AND kind='name_reset' AND {ACTIVE})"
    ))
    .bind(profile_id)
    .fetch_one(&mut **tx)
    .await?)
}

async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    actor: &str,
    action: &str,
    profile_id: Option<&str>,
    sanction_id: Option<i64>,
    detail: serde_json::Value,
) -> StoreResult<()> {
    sqlx::query("INSERT INTO career_moderation_events(actor,action,profile_id,sanction_id,detail) VALUES($1,$2,$3,$4,$5)")
        .bind(actor)
        .bind(action)
        .bind(profile_id)
        .bind(sanction_id)
        .bind(Json(detail))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

impl CareerStore {
    /// A profile's active sanctions (sign-in).
    pub async fn active_sanctions(&self, profile_id: &str) -> Result<Vec<Sanction>, String> {
        retry(|| async {
            check_id(profile_id)?;
            let Json(sanctions): Json<Vec<Sanction>> =
                sqlx::query_scalar(&format!("SELECT {ACTIVE_JSON}"))
                    .bind(profile_id)
                    .fetch_one(&self.pool)
                    .await?;
            Ok(sanctions)
        })
        .await
    }

    /// The game's access refresh in one query: the key is still active, the
    /// current name, and the active sanctions. Database errors fail closed.
    pub async fn access(&self, public_key: &str, profile_id: &str) -> Result<Access, String> {
        retry(|| async {
            check_id(public_key)?;
            check_id(profile_id)?;
            let row = sqlx::query(&format!(
                "SELECT EXISTS(SELECT 1 FROM career_keys WHERE public_key=$2 AND profile_id=$1 AND revoked_at IS NULL) AS key_active, \
                 (SELECT nickname FROM career_profiles WHERE profile_id=$1) AS nickname, {ACTIVE_JSON} AS sanctions"
            ))
            .bind(profile_id)
            .bind(public_key)
            .fetch_one(&self.pool)
            .await?;
            let Json(sanctions): Json<Vec<Sanction>> = row.try_get("sanctions")?;
            Ok(Access {
                key_active: row.try_get("key_active")?,
                nickname: row.try_get("nickname")?,
                sanctions,
            })
        })
        .await
    }

    /// Whether the database-wide chat switch is on.
    pub async fn chat_enabled(&self) -> Result<bool, String> {
        Ok(self.chat_switch().await?.enabled)
    }

    pub async fn chat_switch(&self) -> Result<ChatSwitch, String> {
        retry(|| async {
            let row = sqlx::query("SELECT enabled,changed_by,floor(extract(epoch FROM changed_at))::bigint AS changed FROM career_chat_switch")
                .fetch_one(&self.pool)
                .await?;
            Ok(ChatSwitch {
                enabled: row.try_get("enabled")?,
                changed_at: row.try_get("changed")?,
                changed_by: row.try_get("changed_by")?,
            })
        })
        .await
    }

    /// Turns chat on or off on every game server of this database within
    /// seconds, with an audit row.
    pub async fn set_chat(
        &self,
        enabled: bool,
        reason: &str,
        by: &str,
    ) -> Result<ChatSwitch, String> {
        retry(|| async {
            let reason = text(reason, MAX_REASON_CHARS, "reason")?;
            let by = actor(by)?;
            let mut tx = self.pool.begin().await?;
            sqlx::query("UPDATE career_chat_switch SET enabled=$1,changed_at=clock_timestamp(),changed_by=$2")
                .bind(enabled)
                .bind(&by)
                .execute(&mut *tx)
                .await?;
            audit(
                &mut tx,
                &by,
                "chat_switched",
                None,
                None,
                serde_json::json!({"enabled": enabled, "reason": reason}),
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        self.chat_switch().await
    }

    /// A player by `nickname#1234` or by the 64-hex profile ID.
    pub async fn find_profile(&self, query: &str) -> Result<ProfileSummary, String> {
        let query = query.trim();
        if shared::career::valid_profile_id(query) {
            return self.profile(query).await;
        }
        let reference = self.lookup_player(query).await?;
        self.profile(&reference.profile_id).await
    }

    /// Every device key of a profile, newest first (the key itself shortened).
    pub async fn profile_keys(&self, profile_id: &str) -> Result<Vec<KeySummary>, String> {
        retry(|| async {
            check_id(profile_id)?;
            let rows = sqlx::query("SELECT left(public_key,16) AS prefix,label,revoked_at IS NOT NULL AS revoked,floor(extract(epoch FROM created_at))::bigint AS created FROM career_keys WHERE profile_id=$1 ORDER BY created_at DESC LIMIT 64")
                .bind(profile_id)
                .fetch_all(&self.pool)
                .await?;
            rows.iter()
                .map(|row| {
                    Ok(KeySummary {
                        key_prefix: row.try_get("prefix")?,
                        label: row.try_get("label")?,
                        created_at: row.try_get("created")?,
                        revoked: row.try_get("revoked")?,
                    })
                })
                .collect()
        })
        .await
    }

    /// A profile's sanctions, newest first; `all` includes lifted and
    /// expired ones.
    pub async fn sanctions(
        &self,
        profile_id: &str,
        all: bool,
    ) -> Result<Vec<SanctionRecord>, String> {
        retry(|| async {
            check_id(profile_id)?;
            let filter = if all { "TRUE" } else { ACTIVE };
            let rows = sqlx::query(&format!(
                "SELECT {RECORD} FROM career_sanctions WHERE profile_id=$1 AND {filter} ORDER BY sanction_id DESC LIMIT 200"
            ))
            .bind(profile_id)
            .fetch_all(&self.pool)
            .await?;
            rows.iter().map(record).collect()
        })
        .await
    }

    /// Applies a sanction and writes its audit row. A `name_reset` renames
    /// the profile to `Player#NNNN` (a free tag) in the same transaction and
    /// locks renames until it ends. Takes effect on game servers at their
    /// next access refresh (10 seconds), without a restart.
    pub async fn add_sanction(&self, new: NewSanction<'_>) -> Result<Applied, String> {
        retry(|| async {
            check_id(new.profile_id)?;
            if new.kind == SanctionKind::Unknown {
                return Err("Unknown sanction kind.".into());
            }
            let reason = text(new.reason, MAX_REASON_CHARS, "reason")?;
            let by = actor(new.actor)?;
            if new
                .duration_secs
                .is_some_and(|secs| secs <= 0 || secs > MAX_SANCTION_SECS)
            {
                return Err(StoreError::from(
                    "A sanction lasts from one second to ten years; omit the duration for one that lasts until lifted.",
                ));
            }
            let mut tx = self.pool.begin().await?;
            let old_name: String = sqlx::query_scalar(
                "SELECT nickname FROM career_profiles WHERE profile_id=$1 FOR UPDATE",
            )
            .bind(new.profile_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or("Profile not found.")?;
            let renamed = if new.kind == SanctionKind::NameReset {
                let name: String = sqlx::query_scalar(
                    "SELECT $1 || '#' || lpad(t::text,4,'0') FROM generate_series(0,9999) t \
                     WHERE NOT EXISTS (SELECT 1 FROM career_profiles p \
                       WHERE lower(p.nickname COLLATE \"und-x-icu\") = lower(($1 || '#' || lpad(t::text,4,'0')) COLLATE \"und-x-icu\")) \
                     ORDER BY random() LIMIT 1",
                )
                .bind(RESET_NAME)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or("Every Player#NNNN tag is taken.")?;
                sqlx::query("UPDATE career_profiles SET nickname=$2 WHERE profile_id=$1")
                    .bind(new.profile_id)
                    .bind(&name)
                    .execute(&mut *tx)
                    .await?;
                Some((old_name, name))
            } else {
                None
            };
            let row = sqlx::query(&format!(
                "INSERT INTO career_sanctions(profile_id,kind,reason,created_at,expires_at,created_by) \
                 SELECT $1,$2,$3,t,t + make_interval(secs => $4::double precision),$5 \
                 FROM (SELECT clock_timestamp() AS t) n RETURNING {RECORD}"
            ))
            .bind(new.profile_id)
            .bind(new.kind.id())
            .bind(&reason)
            .bind(new.duration_secs.map(|secs| secs as f64))
            .bind(&by)
            .fetch_one(&mut *tx)
            .await?;
            let sanction = record(&row)?;
            let mut detail = serde_json::json!({
                "kind": new.kind.id(),
                "reason": reason,
                "expires_at": sanction.expires_at,
            });
            if let Some((old, new_name)) = &renamed {
                detail["old_name"] = old.clone().into();
                detail["new_name"] = new_name.clone().into();
            }
            audit(
                &mut tx,
                &by,
                "sanction_added",
                Some(new.profile_id),
                Some(sanction.sanction_id),
                detail,
            )
            .await?;
            tx.commit().await?;
            Ok(Applied { sanction, renamed })
        })
        .await
    }

    /// Lifts an active sanction early, with an audit row. Access returns at
    /// the game servers' next access refresh, without a restart.
    pub async fn lift_sanction(
        &self,
        sanction_id: i64,
        reason: &str,
        by: &str,
    ) -> Result<SanctionRecord, String> {
        retry(|| async {
            let reason = text(reason, MAX_REASON_CHARS, "reason")?;
            let by = actor(by)?;
            let mut tx = self.pool.begin().await?;
            let row = sqlx::query(&format!(
                "UPDATE career_sanctions SET lifted_at=clock_timestamp(),lifted_by=$2,lift_reason=$3 \
                 WHERE sanction_id=$1 AND {ACTIVE} RETURNING {RECORD}"
            ))
            .bind(sanction_id)
            .bind(&by)
            .bind(&reason)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or("No active sanction has this number (it may have ended or been lifted).")?;
            let sanction = record(&row)?;
            audit(
                &mut tx,
                &by,
                "sanction_lifted",
                Some(&sanction.profile_id),
                Some(sanction_id),
                serde_json::json!({"kind": sanction.kind.id(), "reason": reason}),
            )
            .await?;
            tx.commit().await?;
            Ok(sanction)
        })
        .await
    }

    /// The newest moderation events of one profile, or of chat switching
    /// with `None`.
    pub async fn moderation_events(
        &self,
        profile_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ModerationEvent>, String> {
        retry(|| async {
            if let Some(id) = profile_id {
                check_id(id)?;
            }
            let rows = sqlx::query("SELECT seq,actor,action,profile_id,sanction_id,detail,floor(extract(epoch FROM created_at))::bigint AS created FROM career_moderation_events WHERE profile_id IS NOT DISTINCT FROM $1 ORDER BY seq DESC LIMIT $2")
                .bind(profile_id)
                .bind(limit.clamp(1, 200))
                .fetch_all(&self.pool)
                .await?;
            rows.iter()
                .map(|row| {
                    let Json(detail): Json<serde_json::Value> = row.try_get("detail")?;
                    Ok(ModerationEvent {
                        seq: row.try_get("seq")?,
                        created_at: row.try_get("created")?,
                        actor: row.try_get("actor")?,
                        action: row.try_get("action")?,
                        profile_id: row.try_get("profile_id")?,
                        sanction_id: row.try_get("sanction_id")?,
                        detail,
                    })
                })
                .collect()
        })
        .await
    }
}
