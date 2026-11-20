//! Participation validation and exactly-once discipline inside settlement's TX.
use super::*;
use shared::career::RatingChange;

pub(super) fn proved_abandon(participant: &ParticipantResult) -> bool {
    participant.participation.is_some_and(|p| p.abandoned)
}

pub(super) fn reward_ineligible(participant: &ParticipantResult) -> bool {
    // Old pending receipts have no time accounting: retain the minimum no-win
    // reward protection, but never fabricate a durable strike from that flag.
    participant
        .participation
        .map_or(participant.disconnected, |p| p.abandoned)
}

pub(super) fn rating_award(
    base: &RatingChange,
    participant: &ParticipantResult,
    winner: Team,
    teammate_abandoned: bool,
) -> RatingChange {
    let requested = if proved_abandon(participant) {
        base.delta.min(-32)
    } else if reward_ineligible(participant) {
        base.delta.min(0)
    } else if participant.team != winner && teammate_abandoned {
        (f64::from(base.delta) / 2.0).round() as i32
    } else {
        base.delta
    };
    let after = base
        .before
        .saturating_add(requested)
        .clamp(0, crate::matchmaking::MAX_RATING);
    RatingChange {
        before: base.before,
        after,
        delta: after - base.before,
    }
}

pub(super) fn validate_participation(result: &MatchResult) -> StoreResult<()> {
    for participant in &result.participants {
        let Some(p) = participant.participation else {
            continue;
        };
        if participant.is_bot {
            return Err("A bot cannot carry human participation time.".into());
        }
        if p.connected_ms
            .checked_add(p.disconnected_ms)
            .is_none_or(|total| total > result.duration_ms)
            || p.afk_ms > result.duration_ms
        {
            return Err("Participation time exceeds the observed match duration.".into());
        }
        if !result.gameplay_started && p != shared::career::Participation::default() {
            return Err("An unplayed allocation cannot carry gameplay participation.".into());
        }
    }
    Ok(())
}

pub(super) fn validate_participation_progress(
    old: &MatchResult,
    new: &MatchResult,
) -> StoreResult<()> {
    for previous in &old.participants {
        let Some(before) = previous.participation else {
            continue;
        };
        let Some(current) = new
            .participants
            .iter()
            .find(|p| p.player_id == previous.player_id)
        else {
            continue;
        };
        let Some(after) = current.participation else {
            return Err("A checkpoint cannot erase participation accounting.".into());
        };
        if after.connected_ms < before.connected_ms
            || after.disconnected_ms < before.disconnected_ms
            || after.afk_ms < before.afk_ms
            || (before.abandoned && !after.abandoned)
        {
            return Err("A checkpoint cannot reduce participation or erase an abandon.".into());
        }
    }
    Ok(())
}

/// Caller has locked the allocation and profiles and established an eligible
/// public played abandon. `None` means this exact profile/result was recorded
/// already; it must not increment a total or extend a cooldown again.
pub(super) async fn record_abandon(
    tx: &mut Transaction<'_, Postgres>,
    profile: &str,
    result_id: &str,
) -> StoreResult<Option<u32>> {
    let inserted = sqlx::query("INSERT INTO career_abandon_events(profile_id,result_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(profile).bind(result_id).execute(&mut **tx).await?;
    if inserted.rows_affected() == 0 {
        return Ok(None);
    }
    let strikes: i64 = sqlx::query_scalar("SELECT count(*) FROM career_abandon_events WHERE profile_id=$1 AND created_at>=clock_timestamp()-interval '24 hours'")
        .bind(profile).fetch_one(&mut **tx).await?;
    let seconds: i64 = match strikes {
        1 => 300,
        2 => 1800,
        _ => 86_400,
    };
    let strikes = i32::try_from(strikes)
        .map_err(|_| StoreError::Invalid("Abandon window counter is full.".into()))?;
    sqlx::query("INSERT INTO career_play_penalties(profile_id,cooldown_until,strikes) VALUES($1,clock_timestamp()+make_interval(secs=>($2::bigint)::double precision),$3) ON CONFLICT(profile_id) DO UPDATE SET cooldown_until=GREATEST(career_play_penalties.cooldown_until,EXCLUDED.cooldown_until),strikes=EXCLUDED.strikes")
        .bind(profile).bind(seconds).bind(strikes).execute(&mut **tx).await?;
    let count: i64 = sqlx::query_scalar(
        "UPDATE career_profiles SET abandons=abandons+1 WHERE profile_id=$1 RETURNING abandons",
    )
    .bind(profile)
    .fetch_one(&mut **tx)
    .await?;
    Ok(Some(u32::try_from(count).map_err(|_| {
        StoreError::Invalid("Abandon total is full.".into())
    })?))
}
