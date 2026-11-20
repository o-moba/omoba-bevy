//! Season writes share one lock order: season -> allocation/match -> profiles.
use super::*;
use shared::ranked::{SeasonBadge, SeasonClosure, SeasonProfile, SeasonWindow};

const LOCK: &str = "SELECT pg_advisory_xact_lock_shared(721946010)";

pub(super) async fn lock(tx: &mut Transaction<'_, Postgres>) -> StoreResult<()> {
    sqlx::query(LOCK).execute(&mut **tx).await?;
    Ok(())
}

fn uint(row: &PgRow, name: &str) -> StoreResult<u64> {
    u64::try_from(row.try_get::<i64, _>(name)?)
        .map_err(|_| "Invalid stored Ranked season value.".into())
}

pub(super) fn profile(row: &PgRow) -> StoreResult<SeasonProfile> {
    let counter = |name| -> StoreResult<u32> {
        u32::try_from(uint(row, name)?).map_err(|_| "Invalid seasonal counter.".into())
    };
    Ok(SeasonProfile {
        season_id: uint(row, "ranked_season_id")?,
        started_at_ms: uint(row, "ranked_started_at_ms")?,
        ends_at_ms: uint(row, "ranked_ends_at_ms")?,
        rated_matches: counter("season_rated_matches")?,
        wins: counter("season_wins")?,
        losses: counter("season_losses")?,
        completed_matches: counter("season_completed_matches")?,
        latest_badge: row
            .try_get::<Option<Json<SeasonBadge>>, _>("latest_ranked_badge")?
            .map(|b| b.0),
    })
}

/// The immutable intent, not a later unrated terminal flag, owns the season.
pub(super) async fn reserve(
    tx: &mut Transaction<'_, Postgres>,
    result: &MatchResult,
) -> StoreResult<()> {
    if !result.rated && result.ranked_season_id.is_none() {
        return Ok(());
    }
    let row = sqlx::query("SELECT season_id,started_at_ms,ends_at_ms, floor(extract(epoch FROM clock_timestamp())*1000)::bigint AS now_ms FROM career_ranked_seasons WHERE closed_at_ms IS NULL")
        .fetch_one(&mut **tx).await?;
    let id = uint(&row, "season_id")?;
    let now = uint(&row, "now_ms")?;
    // Initial deployment can replay old receipts without inventing metadata.
    // A missing old spool must never silently join a later season after reset.
    if now < uint(&row, "started_at_ms")?
        || now >= uint(&row, "ends_at_ms")?
        || result.ranked_season_id.is_some_and(|wanted| wanted != id)
        || (result.ranked_season_id.is_none() && id != 1)
    {
        return Err(shared::career::error_code::RANKED_SEASON_ENDED.into());
    }
    sqlx::query("INSERT INTO career_ranked_allocations(result_id,season_id) VALUES($1,$2)")
        .bind(&result.result_id)
        .bind(i64::try_from(id).map_err(|_| "Invalid season identity.")?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn allocation(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
) -> StoreResult<Option<u64>> {
    let value: Option<i64> =
        sqlx::query_scalar("SELECT season_id FROM career_ranked_allocations WHERE result_id=$1")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
    value
        .map(|v| u64::try_from(v).map_err(|_| "Invalid allocation season.".into()))
        .transpose()
}

pub(super) fn advance(
    profile: &mut ProfileSummary,
    participant: &ParticipantResult,
    season_id: Option<u64>,
    completed: bool,
    won: bool,
) -> StoreResult<()> {
    if participant.rating.is_none() {
        return Ok(());
    }
    let season = profile
        .ranked
        .as_mut()
        .ok_or("Season progress is missing.")?;
    if Some(season.season_id) != season_id {
        return Err("Allocated season differs from current profile season.".into());
    }
    let increment = |value: &mut u32| -> StoreResult<()> {
        *value = value.checked_add(1).ok_or("Season counter is full.")?;
        Ok(())
    };
    increment(&mut season.rated_matches)?;
    increment(if won {
        &mut season.wins
    } else {
        &mut season.losses
    })?;
    if completed && !fair_play::proved_abandon(participant) {
        increment(&mut season.completed_matches)?;
    }
    Ok(())
}

fn closure(row: &PgRow) -> StoreResult<SeasonClosure> {
    Ok(SeasonClosure {
        season_id: uint(row, "season_id")?,
        next_season_id: uint(row, "next_season_id")?,
        closed_at_ms: uint(row, "closed_at_ms")?,
        archived_profiles: uint(row, "archived_profiles")?,
        awarded_badges: uint(row, "awarded_badges")?,
    })
}

impl CareerStore {
    pub async fn ranked_season(&self) -> Result<SeasonWindow, String> {
        retry(|| async {
            let row = sqlx::query("SELECT season_id,started_at_ms,ends_at_ms FROM career_ranked_seasons WHERE closed_at_ms IS NULL")
                .fetch_one(&self.pool).await?;
            Ok(SeasonWindow { season_id: uint(&row,"season_id")?, started_at_ms: uint(&row,"started_at_ms")?, ends_at_ms: uint(&row,"ends_at_ms")? })
        }).await
    }

    /// Operator only: use a separately authorized owner connection after all
    /// hosts are drained. The game/portal runtime roles get no close privileges.
    /// No read, startup, cron or player command calls this method.
    pub async fn close_ranked_season(
        &self,
        expected_season_id: u64,
    ) -> Result<SeasonClosure, String> {
        let expected = i64::try_from(expected_season_id)
            .ok()
            .filter(|n| *n > 0)
            .ok_or("A valid expected Ranked season is required.")?;
        retry(|| self.close_ranked_season_inner(expected)).await
    }

    async fn close_ranked_season_inner(&self, expected: i64) -> StoreResult<SeasonClosure> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL statement_timeout='120s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT pg_advisory_xact_lock(721946010)")
            .execute(&mut *tx)
            .await?;
        if let Some(row) = sqlx::query("SELECT * FROM career_ranked_closures WHERE season_id=$1")
            .bind(expected)
            .fetch_optional(&mut *tx)
            .await?
        {
            let receipt = closure(&row)?;
            tx.commit().await?;
            return Ok(receipt);
        }
        let season = sqlx::query("SELECT *,floor(extract(epoch FROM clock_timestamp())*1000)::bigint AS now_ms FROM career_ranked_seasons WHERE closed_at_ms IS NULL FOR UPDATE")
            .fetch_one(&mut *tx).await?;
        if uint(&season, "season_id")? != expected as u64 {
            return Err("Ranked season changed; inspect the current season before closing.".into());
        }
        let now = uint(&season, "now_ms")?;
        if now < uint(&season, "ends_at_ms")? {
            return Err("The Ranked season has not ended.".into());
        }
        let unsettled: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM career_matches WHERE status!='settled' AND (allocation->>'rated'='true' OR allocation->>'ranked_season_id' IS NOT NULL))")
            .fetch_one(&mut *tx).await?;
        if unsettled {
            return Err(
                "Unsettled Ranked allocations must finish or recover before closing the season."
                    .into(),
            );
        }
        // Sorted row locks coordinate with rename/friendship as well as matches.
        let locked =
            sqlx::query("SELECT profile_id FROM career_profiles ORDER BY profile_id FOR UPDATE")
                .execute(&mut *tx)
                .await?
                .rows_affected();
        let counts = sqlx::query(r#"
            WITH archived AS (
                INSERT INTO career_ranked_archives(season_id,profile_id,rating,rated_matches,wins,losses,completed_matches,badge)
                SELECT ranked_season_id,profile_id,rating,season_rated_matches,season_wins,season_losses,season_completed_matches,
                    CASE WHEN season_completed_matches>0 THEN jsonb_strip_nulls(jsonb_build_object(
                        'season_id',ranked_season_id,'ended_at_ms',ranked_ends_at_ms,'rating',rating,
                        'rated_matches',season_rated_matches,'tier',CASE WHEN season_rated_matches<20 THEN NULL
                            WHEN rating<1000 THEN 'bronze' WHEN rating<1500 THEN 'silver'
                            WHEN rating<2000 THEN 'gold' WHEN rating<2500 THEN 'platinum'
                            WHEN rating<3000 THEN 'diamond' ELSE 'master' END)) END
                FROM career_profiles WHERE ranked_season_id=$1 AND season_rated_matches>0
                RETURNING badge
            ) SELECT count(*) AS archived_profiles,count(*) FILTER (WHERE badge IS NOT NULL) AS awarded_badges FROM archived
        "#).bind(expected).fetch_one(&mut *tx).await?;
        let now_i64 = i64::try_from(now).map_err(|_| "Invalid database time.")?;
        sqlx::query("UPDATE career_ranked_seasons SET closed_at_ms=$2 WHERE season_id=$1")
            .bind(expected)
            .bind(now_i64)
            .execute(&mut *tx)
            .await?;
        let next: i64 = sqlx::query_scalar("INSERT INTO career_ranked_seasons(started_at_ms,ends_at_ms) VALUES($1,$1+$2) RETURNING season_id")
            .bind(now_i64).bind(shared::ranked::SEASON_DURATION_MS as i64).fetch_one(&mut *tx).await?;
        let updated = sqlx::query(r#"
            UPDATE career_profiles p SET rating=1000+(rating-1000)/2,
                ranked_season_id=s.season_id,ranked_started_at_ms=s.started_at_ms,ranked_ends_at_ms=s.ends_at_ms,
                season_rated_matches=0,season_wins=0,season_losses=0,season_completed_matches=0,
                latest_ranked_badge=COALESCE((SELECT a.badge FROM career_ranked_archives a WHERE a.season_id=$1 AND a.profile_id=p.profile_id),p.latest_ranked_badge)
            FROM career_ranked_seasons s WHERE s.season_id=$2 AND p.ranked_season_id=$1
        "#).bind(expected).bind(next).execute(&mut *tx).await?.rows_affected();
        if updated != locked {
            return Err("Profile season coverage differs from the closing window.".into());
        }
        let archived = uint(&counts, "archived_profiles")?;
        let badges = uint(&counts, "awarded_badges")?;
        let row = sqlx::query("INSERT INTO career_ranked_closures(season_id,next_season_id,closed_at_ms,archived_profiles,awarded_badges) VALUES($1,$2,$3,$4,$5) RETURNING *")
            .bind(expected).bind(next).bind(now_i64)
            .bind(i64::try_from(archived).map_err(|_| "Archive count is full.")?)
            .bind(i64::try_from(badges).map_err(|_| "Badge count is full.")?)
            .fetch_one(&mut *tx).await?;
        let receipt = closure(&row)?;
        tx.commit().await?;
        Ok(receipt)
    }
}
