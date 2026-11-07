//! Durable consent, separate from match results. All operations lock sorted
//! profiles before check rows. No gameplay/15-second wait holds a transaction.
use super::*;
use shared::{career::PlayCooldown, match_service::MatchPreference};

pub const OWNED_ELSEWHERE: &str = shared::career::error_code::PROFILE_HAS_ANOTHER_SESSION;
pub const CLOSED: &str = shared::career::error_code::ALLOCATION_CANCELLED_OR_EXPIRED;
const WINDOW_MS: i64 = 15_000;
const HEARTBEAT_MS: i64 = 3_000;
const HANDOFF_MS: i64 = 90_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    pub profile_id: String,
    pub session_id: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Forming,
    Open,
    Approved,
    Cancelled,
    Expired,
    Consumed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub seat: Seat,
    pub accepted: bool,
    pub dodged: bool,
    pub event_id: String,
    pub observed_at_ms: Option<i64>,
    pub cooldown: Option<PlayCooldown>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub check_id: String,
    pub preference: MatchPreference,
    pub phase: Phase,
    pub deadline_ms: i64,
    pub opened_at_ms: Option<i64>,
    pub members: Vec<Member>,
}
#[derive(Clone, Debug)]
pub enum Action {
    Publish,
    Poll,
    Answer {
        profile_id: String,
        session_id: String,
        accepted: bool,
    },
    /// Privileged server maintenance/failure, never a player's decline.
    Cancel,
}
struct Check {
    owner: String,
    queue: String,
    state: String,
    deadline: i64,
    opened: Option<i64>,
    live: i64,
    count: i32,
}
fn valid_check(id: &str) -> StoreResult<()> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("Invalid ready identity.".into());
    }
    Ok(())
}
fn queue_name(queue: MatchPreference) -> &'static str {
    match queue {
        MatchPreference::Ranked => "humans_only",
        MatchPreference::Casual => "quick",
        MatchPreference::BotPractice => "bot_practice",
    }
}
async fn now(tx: &mut Transaction<'_, Postgres>) -> StoreResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp())*1000)::bigint")
            .fetch_one(&mut **tx)
            .await?,
    )
}
async fn check(tx: &mut Transaction<'_, Postgres>, id: &str) -> StoreResult<Check> {
    let row = sqlx::query("SELECT * FROM career_ready_checks WHERE check_id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(CLOSED)?;
    Ok(Check {
        owner: row.try_get("owner_id")?,
        queue: row.try_get("queue")?,
        state: row.try_get("state")?,
        deadline: row.try_get("deadline_ms")?,
        opened: row.try_get("opened_at_ms")?,
        live: row.try_get("live_until_ms")?,
        count: row.try_get("member_count")?,
    })
}
async fn members(tx: &mut Transaction<'_, Postgres>, id: &str) -> StoreResult<Vec<Member>> {
    let rows=sqlx::query("SELECT m.*,CASE WHEN p.cooldown_until>clock_timestamp() THEN ceil(extract(epoch FROM p.cooldown_until))::bigint END AS until,p.strikes FROM career_ready_members m LEFT JOIN career_queue_penalties p USING(profile_id) WHERE check_id=$1 ORDER BY profile_id")
        .bind(id).fetch_all(&mut **tx).await?;
    rows.into_iter()
        .map(|r| {
            let until: Option<i64> = r.try_get("until")?;
            Ok(Member {
                seat: Seat {
                    profile_id: r.try_get("profile_id")?,
                    session_id: r.try_get("session_id")?,
                },
                accepted: r.try_get("accepted")?,
                dodged: r.try_get("dodged")?,
                event_id: r.try_get("dodge_event_id")?,
                observed_at_ms: r.try_get("dodge_observed_ms")?,
                cooldown: until
                    .map(|until| {
                        Ok::<_, StoreError>(PlayCooldown {
                            until,
                            strikes: u32::try_from(r.try_get::<i32, _>("strikes")?)
                                .map_err(|_| "Invalid queue strikes.")?,
                        })
                    })
                    .transpose()?,
            })
        })
        .collect()
}
async fn snapshot(tx: &mut Transaction<'_, Postgres>, id: &str) -> StoreResult<Snapshot> {
    let c = check(tx, id).await?;
    Ok(Snapshot {
        check_id: id.into(),
        preference: match c.queue.as_str() {
            "humans_only" => MatchPreference::Ranked,
            "quick" => MatchPreference::Casual,
            "bot_practice" => MatchPreference::BotPractice,
            _ => return Err("Invalid ready queue.".into()),
        },
        phase: match c.state.as_str() {
            "forming" => Phase::Forming,
            "open" => Phase::Open,
            "approved" => Phase::Approved,
            "cancelled" => Phase::Cancelled,
            "expired" => Phase::Expired,
            "consumed" => Phase::Consumed,
            _ => return Err("Invalid ready state.".into()),
        },
        deadline_ms: c.deadline,
        opened_at_ms: c.opened,
        members: members(tx, id).await?,
    })
}
async fn ids(tx: &mut Transaction<'_, Postgres>, id: &str) -> StoreResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT profile_id FROM career_ready_members WHERE check_id=$1 ORDER BY profile_id",
    )
    .bind(id)
    .fetch_all(&mut **tx)
    .await?)
}
/// Expand existing claims before locking, then verify no previously unseen
/// check appeared while acquiring locks. A concurrent change refuses/retries;
/// it never locks new profiles out of order or assumes absent ownership.
async fn lock_profiles(
    tx: &mut Transaction<'_, Postgres>,
    requested: &[String],
) -> StoreResult<Vec<String>> {
    let checks:Vec<String>=sqlx::query_scalar("SELECT DISTINCT check_id FROM career_ready_claims WHERE profile_id=ANY($1) ORDER BY check_id")
        .bind(requested).fetch_all(&mut **tx).await?;
    let mut all = requested.to_vec();
    let more: Vec<String> =
        sqlx::query_scalar("SELECT profile_id FROM career_ready_members WHERE check_id=ANY($1)")
            .bind(&checks)
            .fetch_all(&mut **tx)
            .await?;
    all.extend(more);
    all.sort();
    all.dedup();
    let locked:Vec<String>=sqlx::query_scalar("SELECT profile_id FROM career_profiles WHERE profile_id=ANY($1) ORDER BY profile_id FOR UPDATE")
        .bind(&all).fetch_all(&mut **tx).await?;
    if locked != all {
        return Err("Ready participant profile is missing.".into());
    }
    let current:Vec<String>=sqlx::query_scalar("SELECT DISTINCT check_id FROM career_ready_claims WHERE profile_id=ANY($1) ORDER BY check_id")
        .bind(requested).fetch_all(&mut **tx).await?;
    if current.iter().any(|id| !checks.contains(id)) {
        return Err("Ready ownership changed. Retry shortly.".into());
    }
    // No profile locks may be acquired after this point.
    for id in &checks {
        check(tx, id).await?;
    }
    Ok(checks)
}
async fn close(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    state: &str,
    excluded: &[String],
    observed: i64,
) -> StoreResult<()> {
    for m in members(tx, id).await? {
        if excluded.contains(&m.seat.profile_id) && !m.dodged {
            super::queue_discipline::record_in_tx(tx, &m.event_id, &m.seat.profile_id, observed)
                .await?;
            sqlx::query("UPDATE career_ready_members SET dodged=true,dodge_observed_ms=$3 WHERE check_id=$1 AND profile_id=$2")
                .bind(id).bind(&m.seat.profile_id).bind(observed).execute(&mut **tx).await?;
        }
    }
    sqlx::query(
        "UPDATE career_ready_checks SET state=$2,updated_at=clock_timestamp() WHERE check_id=$1",
    )
    .bind(id)
    .bind(state)
    .execute(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM career_ready_claims WHERE check_id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
async fn reconcile(tx: &mut Transaction<'_, Postgres>, id: &str, at: i64) -> StoreResult<()> {
    let c = check(tx, id).await?;
    if !matches!(c.state.as_str(), "forming" | "open" | "approved") {
        return Ok(());
    }
    let roster = members(tx, id).await?;
    if roster.len() != c.count as usize {
        return close(tx, id, "cancelled", &[], at).await;
    }
    if at < c.deadline {
        return Ok(());
    }
    if c.state == "open" && c.live >= c.deadline {
        let excluded: Vec<_> = roster
            .iter()
            .filter(|m| !m.accepted)
            .map(|m| m.seat.profile_id.clone())
            .collect();
        close(tx, id, "expired", &excluded, c.deadline).await
    } else {
        // An unpublished candidate, failed owner or failed worker handoff is a
        // server fault. A lease that died before the deadline proves no dodge.
        close(tx, id, "cancelled", &[], at).await
    }
}

impl CareerStore {
    /// Reserve immutable consent ownership before exposing a found match. A
    /// separate Publish starts the actual15-second window, never a retry.
    pub async fn ready_reserve(
        &self,
        id: &str,
        preference: MatchPreference,
        seats: Vec<Seat>,
    ) -> Result<Snapshot, String> {
        retry(|| self.ready_reserve_inner(id, preference, seats.clone())).await
    }
    async fn ready_reserve_inner(
        &self,
        id: &str,
        preference: MatchPreference,
        seats: Vec<Seat>,
    ) -> StoreResult<Snapshot> {
        valid_check(id)?;
        if seats.is_empty() || seats.len() > 10 || (preference.rated() && seats.len() != 10) {
            return Err("Invalid ready roster size.".into());
        }
        let mut expected = seats.clone();
        expected.sort_by(|a, b| a.profile_id.cmp(&b.profile_id));
        let mut sessions = HashSet::new();
        for seat in &expected {
            check_id(&seat.profile_id)?;
            if seat.session_id.is_empty()
                || seat.session_id.len() > 64
                || !seat
                    .session_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                || !sessions.insert(&seat.session_id)
            {
                return Err("Invalid ready session.".into());
            }
        }
        if expected
            .windows(2)
            .any(|p| p[0].profile_id == p[1].profile_id)
        {
            return Err("Duplicate ready profile.".into());
        }
        let requested: Vec<_> = expected.iter().map(|s| s.profile_id.clone()).collect();
        let mut tx = self.pool.begin().await?;
        let previous = lock_profiles(&mut tx, &requested).await?;
        let at = now(&mut tx).await?;
        for old in previous {
            reconcile(&mut tx, &old, at).await?;
        }
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM career_ready_checks WHERE check_id=$1)",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if exists {
            let c = check(&mut tx, id).await?;
            let actual: Vec<_> = members(&mut tx, id)
                .await?
                .into_iter()
                .map(|m| m.seat)
                .collect();
            if c.owner != self.owner || c.queue != queue_name(preference) || actual != expected {
                return Err("Ready identity conflicts with its immutable owner/roster.".into());
            }
            let view = snapshot(&mut tx, id).await?;
            tx.commit().await?;
            return Ok(view);
        }
        let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM career_ready_claims WHERE profile_id=ANY($1)) OR EXISTS(SELECT 1 FROM career_active_profiles WHERE profile_id=ANY($1))")
                .bind(&requested).fetch_one(&mut *tx).await?;
        if owned {
            tx.commit().await?;
            return Err(OWNED_ELSEWHERE.into());
        }
        if let Err(error) = Self::check_queue_discipline(&mut tx, &requested).await {
            tx.commit().await?;
            return Err(error);
        }
        let play_blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM career_play_penalties WHERE profile_id=ANY($1) AND cooldown_until>clock_timestamp())")
                .bind(&requested).fetch_one(&mut *tx).await?;
        if play_blocked {
            tx.commit().await?;
            return Err(shared::career::error_code::PLAY_COOLDOWN.into());
        }
        sqlx::query("INSERT INTO career_ready_checks(check_id,owner_id,queue,member_count,state,deadline_ms,live_until_ms) VALUES($1,$2,$3,$4,'forming',$5,$6)")
                .bind(id).bind(&self.owner).bind(queue_name(preference)).bind(seats.len() as i32).bind(at+WINDOW_MS).bind(at).execute(&mut *tx).await?;
        for seat in &expected {
            sqlx::query(
                "INSERT INTO career_ready_members(check_id,profile_id,session_id) VALUES($1,$2,$3)",
            )
            .bind(id)
            .bind(&seat.profile_id)
            .bind(&seat.session_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query("INSERT INTO career_ready_claims(profile_id,check_id) VALUES($1,$2)")
                .bind(&seat.profile_id)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        let view = snapshot(&mut tx, id).await?;
        tx.commit().await?;
        Ok(view)
    }
    /// The trusted owning lobby alone publishes, heartbeats, records a bound
    /// signed answer, or cancels for maintenance. Missing/late answers never
    /// turn into acceptance; retries cannot reset the deadline or event IDs.
    pub async fn ready_update(&self, id: &str, action: Action) -> Result<Snapshot, String> {
        retry(|| self.ready_update_inner(id, action.clone())).await
    }
    async fn ready_update_inner(&self, id: &str, action: Action) -> StoreResult<Snapshot> {
        valid_check(id)?;
        let mut tx = self.pool.begin().await?;
        let requested = ids(&mut tx, id).await?;
        lock_profiles(&mut tx, &requested).await?;
        let c = check(&mut tx, id).await?;
        if c.owner != self.owner {
            return Err(OWNED_ELSEWHERE.into());
        }
        let at = now(&mut tx).await?;
        if matches!(action, Action::Cancel) {
            if matches!(c.state.as_str(), "forming" | "open" | "approved") {
                close(&mut tx, id, "cancelled", &[], at).await?;
            }
        } else {
            // Wrong identities cannot cause even deadline side effects.
            if let Action::Answer {
                profile_id,
                session_id,
                ..
            } = &action
                && !members(&mut tx, id)
                    .await?
                    .iter()
                    .any(|m| m.seat.profile_id == *profile_id && m.seat.session_id == *session_id)
            {
                return Err("Ready response does not belong to this seat.".into());
            }
            reconcile(&mut tx, id, at).await?;
            let c = check(&mut tx, id).await?;
            match &action {
                Action::Publish if c.state == "forming" => {
                    sqlx::query("UPDATE career_ready_checks SET state='open',deadline_ms=$2,live_until_ms=$3,opened_at_ms=$4,updated_at=clock_timestamp() WHERE check_id=$1")
                            .bind(id).bind(at+WINDOW_MS).bind(at+HEARTBEAT_MS).bind(at).execute(&mut *tx).await?;
                }
                Action::Poll if c.state == "open" => {
                    sqlx::query("UPDATE career_ready_checks SET live_until_ms=$2,updated_at=clock_timestamp() WHERE check_id=$1")
                            .bind(id).bind(at+HEARTBEAT_MS).execute(&mut *tx).await?;
                }
                Action::Answer {
                    profile_id,
                    accepted,
                    ..
                } if c.state == "open" || (c.state == "approved" && !accepted) => {
                    if *accepted {
                        sqlx::query("UPDATE career_ready_members SET accepted=true WHERE check_id=$1 AND profile_id=$2")
                                .bind(id).bind(profile_id).execute(&mut *tx).await?;
                        if members(&mut tx, id).await?.iter().all(|m| m.accepted) {
                            sqlx::query("UPDATE career_ready_checks SET state='approved',deadline_ms=$2,updated_at=clock_timestamp() WHERE check_id=$1")
                                    .bind(id).bind(at+HANDOFF_MS).execute(&mut *tx).await?;
                        }
                    } else {
                        close(
                            &mut tx,
                            id,
                            "cancelled",
                            std::slice::from_ref(profile_id),
                            at,
                        )
                        .await?;
                    }
                }
                _ => {}
            }
        }
        let view = snapshot(&mut tx, id).await?;
        tx.commit().await?;
        Ok(view)
    }
    /// Any host may reconcile a dead/expired claim, but only the owning live
    /// host may continue it. The returned penalty and released claims commit
    /// together, closing the asynchronous outbox/host-switch gap.
    pub async fn ready_queue_access(&self, profile: &str) -> Result<(), String> {
        retry(|| self.ready_queue_access_inner(profile)).await
    }
    async fn ready_queue_access_inner(&self, profile: &str) -> StoreResult<()> {
        check_id(profile)?;
        let mut tx = self.pool.begin().await?;
        let checks = lock_profiles(&mut tx, &[profile.into()]).await?;
        let at = now(&mut tx).await?;
        for id in &checks {
            reconcile(&mut tx, id, at).await?;
        }
        let owner:Option<String>=sqlx::query_scalar("SELECT r.owner_id FROM career_ready_claims c JOIN career_ready_checks r USING(check_id) WHERE c.profile_id=$1")
                .bind(profile).fetch_optional(&mut *tx).await?;
        let blocked = owner.is_some_and(|o| o != self.owner);
        // Reconciliation must commit even when the subsequent search is
        // refused. Returning an error inside this transaction would pardon
        // a missed answer by rolling back its durable event.
        tx.commit().await?;
        if blocked {
            Err(OWNED_ELSEWHERE.into())
        } else {
            Ok(())
        }
    }
}

/// Called only after new allocation's participant profile locks, inside its
/// transaction. Existing admissions/recovery return before this guard.
pub(super) async fn admit(
    tx: &mut Transaction<'_, Postgres>,
    result_id: &str,
    profiles: &[String],
) -> StoreResult<()> {
    let claims:Vec<String>=sqlx::query_scalar("SELECT DISTINCT check_id FROM career_ready_claims WHERE profile_id=ANY($1) ORDER BY check_id")
        .bind(profiles).fetch_all(&mut **tx).await?;
    if claims.iter().any(|id| id != result_id) {
        return Err(OWNED_ELSEWHERE.into());
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM career_ready_checks WHERE check_id=$1)")
            .bind(result_id)
            .fetch_one(&mut **tx)
            .await?;
    if !exists {
        return Ok(());
    } // Standalone/legacy allocation, no shared claim.
    let c = check(tx, result_id).await?;
    let seats = ids(tx, result_id).await?;
    if c.state != "approved"
        || now(tx).await? >= c.deadline
        || profiles.is_empty()
        || profiles.iter().any(|p| !seats.contains(p))
        || seats.len() != c.count as usize
    {
        return Err(CLOSED.into());
    }
    sqlx::query("UPDATE career_ready_checks SET state='consumed',updated_at=clock_timestamp() WHERE check_id=$1")
        .bind(result_id).execute(&mut **tx).await?;
    sqlx::query("DELETE FROM career_ready_claims WHERE check_id=$1")
        .bind(result_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
