//! Idempotent, database-wide ready/draft dodge receipts. The caller is a trusted
//! server, not a client. No match receipt, rating, XP or abandon count changes.
use super::*;
use shared::career::PlayCooldown;

const WINDOW_MS: i64 = 86_400_000;
const CLOCK_SKEW_MS: i64 = 5_000;
const ACTIVE_SQL: &str = "SELECT ceil(extract(epoch FROM cooldown_until))::bigint AS until, strikes FROM career_queue_penalties WHERE profile_id=$1 AND cooldown_until>clock_timestamp()";

fn cooldown(row: Option<PgRow>) -> StoreResult<Option<PlayCooldown>> {
    row.map(|row| {
        Ok(PlayCooldown {
            until: row.try_get("until")?,
            strikes: u32::try_from(row.try_get::<i32, _>("strikes")?)
                .map_err(|_| "Invalid matchmaking strike counter.")?,
        })
    })
    .transpose()
}

impl CareerStore {
    /// Fresh new-search authorization. Ordinary access/Resume must remain
    /// usable while a queue penalty is active. The final allocation transaction
    /// rechecks discipline under profile locks; this read alone is no start lease.
    pub async fn queue_access(
        &self,
        public_key: &str,
        profile: &str,
    ) -> Result<moderation::Access, String> {
        let access = self.access(public_key, profile).await?;
        if access.key_active {
            self.ready_queue_access(profile).await?;
        }
        if access.key_active && self.queue_dodge_cooldown(profile).await?.is_some() {
            return Err(shared::career::error_code::DODGE_COOLDOWN.into());
        }
        Ok(access)
    }

    /// Caller holds every participant profile lock. Record and admission share
    /// this serialization point across hosts; never call before locking profiles.
    pub(super) async fn check_queue_discipline(
        tx: &mut Transaction<'_, Postgres>,
        profiles: &[String],
    ) -> StoreResult<()> {
        let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM career_queue_penalties WHERE profile_id=ANY($1) AND cooldown_until>clock_timestamp())")
            .bind(profiles).fetch_one(&mut **tx).await?;
        if blocked {
            return Err(shared::career::error_code::DODGE_COOLDOWN.into());
        }
        Ok(())
    }

    /// Read committed shared discipline without consulting a lobby's local
    /// state. Storage/permission/schema failures remain errors, never no-ban.
    pub async fn queue_dodge_cooldown(
        &self,
        profile: &str,
    ) -> Result<Option<PlayCooldown>, String> {
        retry(|| async {
            check_id(profile)?;
            cooldown(
                sqlx::query(ACTIVE_SQL)
                    .bind(profile)
                    .fetch_optional(&self.pool)
                    .await?,
            )
        })
        .await
    }

    /// Apply one stable server event exactly once across owners/retries. The
    /// same event ID with different identity/time is a conflict, not a second
    /// strike. Preserve existing 60/300/900/3600s escalation, resetting after
    /// 24 hours without a new dodge. A delayed outbox retry cannot restart an
    /// expired penalty: time is anchored to observation, not delivery.
    pub async fn record_queue_dodge(
        &self,
        event_id: &str,
        profile: &str,
        observed_at_ms: u64,
    ) -> Result<Option<PlayCooldown>, String> {
        retry(|| async {
            check_id(profile)?;
            if event_id.len() != 32
                || !event_id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("Invalid matchmaking event identity.".into());
            }
            let observed = i64::try_from(observed_at_ms)
                .map_err(|_| "Invalid matchmaking observation time.")?;
            let mut tx = self.pool.begin().await?;
            // Start/settlement use the same profile row locks. Separate lobbies
            // cannot overwrite each other's strike increment.
            let found: Option<String> = sqlx::query_scalar(
                "SELECT profile_id FROM career_profiles WHERE profile_id=$1 FOR UPDATE",
            )
            .bind(profile)
            .fetch_optional(&mut *tx)
            .await?;
            if found.is_none() {
                return Err("Matchmaking event has no stored profile.".into());
            }
            let result = record_in_tx(&mut tx, event_id, profile, observed).await?;
            tx.commit().await?;
            Ok(result)
        })
        .await
    }
}

/// The caller has already locked this profile. Ready resolution writes the
/// event, penalty and claim release in this same transaction.
pub(super) async fn record_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    event_id: &str,
    profile: &str,
    observed: i64,
) -> StoreResult<Option<PlayCooldown>> {
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp())*1000)::bigint")
            .fetch_one(&mut **tx)
            .await?;
    if observed > now.saturating_add(CLOCK_SKEW_MS) {
        return Err("Matchmaking observation is ahead of the database clock.".into());
    }
    let inserted = sqlx::query("INSERT INTO career_queue_dodge_events(event_id,profile_id,observed_at_ms) VALUES($1,$2,$3) ON CONFLICT(event_id) DO NOTHING")
                .bind(event_id).bind(profile).bind(observed).execute(&mut **tx).await?;
    if inserted.rows_affected() == 0 {
        let row = sqlx::query(
            "SELECT profile_id,observed_at_ms FROM career_queue_dodge_events WHERE event_id=$1",
        )
        .bind(event_id)
        .fetch_one(&mut **tx)
        .await?;
        if row.try_get::<String, _>("profile_id")? != profile
            || row.try_get::<i64, _>("observed_at_ms")? != observed
        {
            return Err("Matchmaking event identity conflicts with its stored receipt.".into());
        }
    } else if now.saturating_sub(observed) < WINDOW_MS {
        let previous = sqlx::query(
            "SELECT last_observed_ms,strikes FROM career_queue_penalties WHERE profile_id=$1",
        )
        .bind(profile)
        .fetch_optional(&mut **tx)
        .await?;
        let mut last = observed.min(now);
        let mut strikes = 1_i32;
        if let Some(previous) = previous {
            let previous_last: i64 = previous.try_get("last_observed_ms")?;
            if now.saturating_sub(previous_last) < WINDOW_MS {
                strikes = previous
                    .try_get::<i32, _>("strikes")?
                    .checked_add(1)
                    .ok_or("Matchmaking strike counter is full.")?;
                last = last.max(previous_last);
            }
        }
        let seconds = match strikes {
            1 => 60,
            2 => 300,
            3 => 900,
            _ => 3600,
        };
        let until = last
            .checked_add(seconds * 1000)
            .ok_or("Invalid matchmaking cooldown time.")?;
        sqlx::query("INSERT INTO career_queue_penalties(profile_id,last_observed_ms,cooldown_until,strikes) VALUES($1,$2,to_timestamp($3::double precision/1000),$4) ON CONFLICT(profile_id) DO UPDATE SET last_observed_ms=GREATEST(career_queue_penalties.last_observed_ms,EXCLUDED.last_observed_ms),cooldown_until=GREATEST(career_queue_penalties.cooldown_until,EXCLUDED.cooldown_until),strikes=EXCLUDED.strikes")
                    .bind(profile).bind(last).bind(until).bind(strikes).execute(&mut **tx).await?;
    }
    cooldown(
        sqlx::query(ACTIVE_SQL)
            .bind(profile)
            .fetch_optional(&mut **tx)
            .await?,
    )
}
