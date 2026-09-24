//! PostgreSQL persistence for the trusted account worker. Network signature and
//! human/ruleset eligibility checks happen before this API is called.
use shared::{
    career::{
        FriendAction, FriendPresence, FriendProfile, FriendsView, HISTORY_PAGE_SIZE,
        MAX_PARTICIPANTS, MatchOutcome, MatchResult, MatchSummary, ParticipantResult,
        ProfileSummary, normalize_nickname, valid_profile_id,
    },
    map::Team,
};
use sqlx::{
    PgPool, Postgres, Row, Transaction,
    postgres::{PgPoolOptions, PgRow},
    types::Json,
};
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    future::Future,
    time::Duration,
};

const MIGRATION: &str = include_str!("../migrations/postgres/001_career.sql");
const RULESET: &str = "verdant-default-v1";
pub(crate) const PUBLIC_CASUAL_RULESET: &str = "public-casual-v1";
const MAX_JSON: usize = 1024 * 1024;
const MAX_SOCIAL: i64 = 64;

#[derive(Clone)]
pub struct CareerStore {
    pool: PgPool,
    owner: String,
}

#[derive(Debug)]
enum StoreError {
    Database(sqlx::Error),
    Invalid(String),
}
type StoreResult<T> = Result<T, StoreError>;
impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(e)
    }
}
impl From<&str> for StoreError {
    fn from(s: &str) -> Self {
        Self::Invalid(s.into())
    }
}
impl From<String> for StoreError {
    fn from(s: String) -> Self {
        Self::Invalid(s)
    }
}
impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
            Self::Database(e) => {
                // Never expose a DSN, credentials or arbitrary database detail.
                let code = e
                    .as_database_error()
                    .and_then(|e| e.code())
                    .map(|c| c.into_owned());
                if e.as_database_error().and_then(|e| e.constraint())
                    == Some("career_player_handle")
                {
                    return f.write_str("This nickname#tag is already taken. Choose another tag.");
                }
                match code.as_deref() {
                    Some("23505") => f.write_str(
                        "A conflicting career identity or active assignment already exists.",
                    ),
                    Some("23503" | "23514" | "22003") => {
                        f.write_str("Career data failed a database integrity check.")
                    }
                    _ => f.write_str(
                        "Career database unavailable. Your pending operation can be retried.",
                    ),
                }
            }
        }
    }
}

async fn retry<T, F, Fut>(mut operation: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = StoreResult<T>>,
{
    for attempt in 0..3 {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(StoreError::Database(ref e))
                if attempt < 2
                    && e.as_database_error()
                        .and_then(|e| e.code())
                        .is_some_and(|c| matches!(c.as_ref(), "40001" | "40P01")) =>
            {
                tokio::time::sleep(Duration::from_millis(25 * (attempt + 1))).await;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    unreachable!("bounded retry returns on the last attempt")
}

impl CareerStore {
    /// Read only provider-verified grants. Missing billing migrations fail closed.
    pub async fn supporter_status(
        &self,
        profile_id: &str,
    ) -> Result<shared::supporter::SupporterStatus, String> {
        use shared::supporter::{AuraStyle, SupporterGrantSummary, SupporterStatus};
        let rows = sqlx::query("SELECT provider, valid_from, valid_until, revoked_at, renewal_enabled, EXTRACT(EPOCH FROM now())::bigint AS now FROM portal.supporter_grants WHERE profile_id=$1 ORDER BY (revoked_at IS NULL AND valid_from<=EXTRACT(EPOCH FROM now())::bigint AND valid_until>EXTRACT(EPOCH FROM now())::bigint) DESC, valid_until DESC LIMIT 64")
            .bind(profile_id).fetch_all(&self.pool).await.map_err(|_| "Supporter storage is unavailable.".to_string())?;
        let mut status = SupporterStatus::default();
        for row in rows {
            let from: i64 = row.get("valid_from");
            let until: i64 = row.get("valid_until");
            let now: i64 = row.get("now");
            let revoked = row.get::<Option<i64>, _>("revoked_at").is_some();
            if !revoked && from <= now && until > now {
                status.active = true;
                status.active_until = Some(status.active_until.unwrap_or(0).max(until));
            }
            status.grants.push(SupporterGrantSummary {
                provider: row.get("provider"),
                valid_from: from,
                valid_until: until,
                revoked,
                renewal_enabled: row.get("renewal_enabled"),
            });
        }
        if status.active {
            let selected: Option<Option<String>> = sqlx::query_scalar(
                "SELECT aura FROM portal.supporter_preferences WHERE profile_id=$1",
            )
            .bind(profile_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| "Supporter storage is unavailable.".to_string())?;
            status.equipped_aura = selected
                .unwrap_or(Some("solar".into()))
                .as_deref()
                .and_then(AuraStyle::from_id);
        }
        Ok(status)
    }

    /// Caller has already verified a signed, active profile key. SQL checks the
    /// grant in the same statement that changes the cosmetic preference.
    pub async fn equip_supporter_aura(
        &self,
        profile_id: &str,
        aura: Option<shared::supporter::AuraStyle>,
    ) -> Result<shared::supporter::SupporterStatus, String> {
        let updated = sqlx::query("INSERT INTO portal.supporter_preferences(profile_id,aura) SELECT $1,$2 WHERE $2::text IS NULL OR EXISTS (SELECT 1 FROM portal.supporter_grants WHERE profile_id=$1 AND revoked_at IS NULL AND valid_from<=EXTRACT(EPOCH FROM now())::bigint AND valid_until>EXTRACT(EPOCH FROM now())::bigint) ON CONFLICT(profile_id) DO UPDATE SET aura=EXCLUDED.aura")
            .bind(profile_id).bind(aura.map(|a| a.id())).execute(&self.pool).await.map_err(|_| "Supporter storage is unavailable.".to_string())?;
        if updated.rows_affected() == 0 {
            return Err("An active Supporter membership is required to equip an aura.".into());
        }
        self.supporter_status(profile_id).await
    }

    pub async fn connect(database_url: &str) -> Result<Self, String> {
        let pool = Self::open_pool(database_url).await?;
        Self::from_pool(pool).await.map_err(|e| e.to_string())
    }

    /// Runtime connections validate the schema and never require DDL privileges.
    pub async fn connect_runtime(database_url: &str) -> Result<Self, String> {
        Self::runtime_from_pool(Self::open_pool(database_url).await?).await
    }

    pub async fn runtime_from_pool(pool: PgPool) -> Result<Self, String> {
        let versions: Vec<i32> =
            sqlx::query_scalar("SELECT version FROM career_schema_version ORDER BY version")
                .fetch_all(&pool)
                .await
                .map_err(|e| StoreError::from(e).to_string())?;
        if versions != [1, 2, 3] {
            return Err("Unsupported career database schema version.".into());
        }
        let owner: String = sqlx::query_scalar("SELECT gen_random_uuid()::text")
            .fetch_one(&pool)
            .await
            .map_err(|e| StoreError::from(e).to_string())?;
        Ok(Self { pool, owner })
    }

    async fn open_pool(database_url: &str) -> Result<PgPool, String> {
        // A worker owns one match and serializes its career operations. Keeping
        // one connection per worker leaves room for the lobby and account API
        // when the allocator runs the 100-room bot-practice capacity case.
        let role = std::env::var("OMOBA_SERVER_ROLE").ok();
        PgPoolOptions::new()
            .max_connections(pool_connections(role.as_deref()))
            .acquire_timeout(Duration::from_secs(5))
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET statement_timeout = '10s'")
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SET lock_timeout = '5s'")
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SET synchronous_commit = on")
                        .execute(&mut *connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(database_url)
            .await
            .map_err(|e| StoreError::from(e).to_string())
    }

    async fn from_pool(pool: PgPool) -> StoreResult<Self> {
        let mut tx = pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(721946001)")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS career_schema_version (version INTEGER PRIMARY KEY)",
        )
        .execute(&mut *tx)
        .await?;
        let versions: Vec<i32> =
            sqlx::query_scalar("SELECT version FROM career_schema_version ORDER BY version")
                .fetch_all(&mut *tx)
                .await?;
        if versions.is_empty() {
            sqlx::raw_sql(MIGRATION).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO career_schema_version VALUES (1)")
                .execute(&mut *tx)
                .await?;
        } else if versions != [1] && versions != [1, 2] && versions != [1, 2, 3] {
            return Err("Unsupported career database schema version.".into());
        }
        if !versions.contains(&2) {
            sqlx::query("SET LOCAL statement_timeout='120s'")
                .execute(&mut *tx)
                .await?;
            sqlx::raw_sql(include_str!(
                "../migrations/postgres/002_player_handles.sql"
            ))
            .execute(&mut *tx)
            .await?;
        }
        if !versions.contains(&3) {
            sqlx::raw_sql(include_str!("../migrations/postgres/003_device_keys.sql"))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        let owner: String = sqlx::query_scalar("SELECT gen_random_uuid()::text")
            .fetch_one(&pool)
            .await?;
        Ok(Self { pool, owner })
    }

    #[cfg(test)]
    pub async fn new_result_id(&self) -> Result<String, String> {
        retry(|| async {
            Ok(sqlx::query_scalar("SELECT gen_random_uuid()::text")
                .fetch_one(&self.pool)
                .await?)
        })
        .await
    }

    /// Call only after proving possession of public_key. Existing keys retain
    /// their saved nickname; renaming requires a separate authorized operation.
    pub async fn authenticate(
        &self,
        public_key: &str,
        nickname: &str,
    ) -> Result<ProfileSummary, String> {
        retry(|| self.authenticate_inner(public_key, nickname)).await
    }
    async fn authenticate_inner(
        &self,
        public_key: &str,
        nickname: &str,
    ) -> StoreResult<ProfileSummary> {
        check_id(public_key)?;
        let nickname = normalize_nickname(nickname)?;
        let mut tx = self.pool.begin().await?;
        // Locks the absent-key case too; hash collisions only serialize logins.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 721946002))")
            .bind(public_key)
            .execute(&mut *tx)
            .await?;
        if let Some(row) = sqlx::query("SELECT p.*, k.revoked_at IS NOT NULL AS revoked FROM career_profiles p JOIN career_keys k USING(profile_id) WHERE k.public_key=$1")
            .bind(public_key).fetch_optional(&mut *tx).await? {
            if row.try_get::<bool,_>("revoked")? {
                return Err("This device key was revoked. Link a new device or use a recovery code.".into());
            }
            let profile = profile_row(&row)?; tx.commit().await?; return Ok(profile);
        }
        let id: String = sqlx::query_scalar("SELECT replace(gen_random_uuid()::text,'-','') || replace(gen_random_uuid()::text,'-','')")
            .fetch_one(&mut *tx).await?;
        let row = sqlx::query(
            "INSERT INTO career_profiles(profile_id,nickname) VALUES($1,$2) RETURNING *",
        )
        .bind(&id)
        .bind(nickname)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO career_keys(public_key,profile_id) VALUES($1,$2)")
            .bind(public_key)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        let profile = profile_row(&row)?;
        tx.commit().await?;
        Ok(profile)
    }

    /// Session caches must revalidate this tombstone; DB errors fail closed.
    pub async fn key_is_active(&self, public_key: &str, profile_id: &str) -> Result<bool, String> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM career_keys WHERE public_key=$1 AND profile_id=$2 AND revoked_at IS NULL)")
            .bind(public_key).bind(profile_id).fetch_one(&self.pool).await
            .map_err(|e| StoreError::from(e).to_string())
    }

    /// Exact handle lookup reveals only the name and stable friend-operation ID.
    pub async fn lookup_player(
        &self,
        handle: &str,
    ) -> Result<shared::career::PlayerReference, String> {
        let handle = shared::career::normalize_player_handle(handle).map_err(str::to_owned)?;
        let row = sqlx::query("SELECT profile_id,nickname FROM career_profiles WHERE lower(nickname COLLATE \"und-x-icu\")=lower($1 COLLATE \"und-x-icu\")")
            .bind(handle).fetch_optional(&self.pool).await.map_err(|e| StoreError::from(e).to_string())?
            .ok_or("Player not found. Check nickname#1234.")?;
        Ok(shared::career::PlayerReference {
            profile_id: row.get("profile_id"),
            nickname: row.get("nickname"),
        })
    }

    pub async fn profile(&self, id: &str) -> Result<ProfileSummary, String> {
        retry(|| async {
            check_id(id)?;
            let row = sqlx::query("SELECT * FROM career_profiles WHERE profile_id=$1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .ok_or("Profile not found.")?;
            profile_row(&row)
        })
        .await
    }
    pub async fn rename(&self, id: &str, name: &str) -> Result<ProfileSummary, String> {
        retry(|| async {
            check_id(id)?;
            let name = normalize_nickname(name)?;
            let row = sqlx::query(
                "UPDATE career_profiles SET nickname=$2 WHERE profile_id=$1 RETURNING *",
            )
            .bind(id)
            .bind(name)
            .fetch_optional(&self.pool)
            .await?
            .ok_or("Profile not found.")?;
            profile_row(&row)
        })
        .await
    }
    pub async fn public_profile(
        &self,
        actor: &str,
        target: &str,
    ) -> Result<ProfileSummary, String> {
        retry(|| async {
            check_id(actor)?; check_id(target)?;
            let row = sqlx::query("SELECT p.* FROM career_profiles p WHERE p.profile_id=$2 AND ($1=$2 OR EXISTS (SELECT 1 FROM career_friendships f WHERE f.low_id=least($1,$2) AND f.high_id=greatest($1,$2)))")
                .bind(actor).bind(target).fetch_optional(&self.pool).await?.ok_or("This profile is available only to its owner, friends or pending friend contacts.")?;
            profile_row(&row)
        }).await
    }

    pub async fn history(
        &self,
        id: &str,
        before: Option<u64>,
    ) -> Result<(Vec<MatchSummary>, Option<u64>), String> {
        retry(|| async {
            check_id(id)?;
            let before = before.map(i64::try_from).transpose().map_err(|_| "Invalid history cursor.")?;
            let rows = sqlx::query("SELECT m.seq,m.result FROM career_matches m JOIN career_participants p USING(result_id) WHERE p.profile_id=$1 AND m.status='settled' AND ($2::bigint IS NULL OR m.seq<$2) ORDER BY m.seq DESC LIMIT $3")
                .bind(id).bind(before).bind((HISTORY_PAGE_SIZE + 1) as i64).fetch_all(&self.pool).await?;
            let more = rows.len() > HISTORY_PAGE_SIZE;
            let mut summaries = Vec::new(); let mut cursor = None;
            for row in rows.into_iter().take(HISTORY_PAGE_SIZE) {
                let result: Json<MatchResult> = row.try_get("result")?;
                let p = result.participants.iter().find(|p| p.profile_id.as_deref() == Some(id))
                    .ok_or("Stored result has an inconsistent participant relation.")?;
                cursor = Some(row.try_get::<i64,_>("seq")? as u64);
                summaries.push(MatchSummary { result_id: result.result_id.clone(), ended_at_ms: result.ended_at_ms,
                    duration_ms: result.duration_ms, outcome: result.outcome, won: result.winner.map(|w| w == p.team),
                    hero_class: p.hero_class, avatar: p.avatar.clone(), sprite_character: p.sprite_character.clone(),
                    kills:p.stats.kills, deaths:p.stats.deaths, assists:p.stats.assists,
                    damage_to_heroes:p.stats.damage_to_heroes, rating:p.rating.clone() });
            }
            Ok((summaries, more.then_some(cursor).flatten()))
        }).await
    }
    pub async fn detail(&self, id: &str, result_id: &str) -> Result<MatchResult, String> {
        retry(|| async {
            check_id(id)?;
            let result: Option<Json<MatchResult>> = sqlx::query_scalar("SELECT m.result FROM career_matches m JOIN career_participants p USING(result_id) WHERE p.profile_id=$1 AND m.result_id=$2 AND m.status='settled'")
                .bind(id).bind(result_id).fetch_optional(&self.pool).await?;
            Ok(result.ok_or("Saved match not found for this profile.")?.0)
        }).await
    }

    /// Trusted worker recovery only. User-facing queries must use `detail`,
    /// which verifies participant membership. This never changes a receipt.
    pub async fn settled_result(&self, result_id: &str) -> Result<Option<MatchResult>, String> {
        retry(|| async {
            let result: Option<Json<MatchResult>> = sqlx::query_scalar(
                "SELECT result FROM career_matches WHERE result_id=$1 AND status='settled'",
            )
            .bind(result_id)
            .fetch_optional(&self.pool)
            .await?;
            Ok(result.map(|result| result.0))
        })
        .await
    }

    pub async fn start(&self, result: MatchResult) -> Result<(), String> {
        retry(|| self.start_inner(result.clone())).await
    }
    async fn start_inner(&self, result: MatchResult) -> StoreResult<()> {
        self.allocation_inner(result, false).await
    }
    /// Reconcile durable spool metadata with an allocation, without treating its
    /// statistics as a new checkpoint. A live different owner is never adopted.
    pub async fn ensure_allocation(&self, result: MatchResult) -> Result<(), String> {
        retry(|| self.allocation_inner(result.clone(), true)).await
    }
    async fn allocation_inner(&self, result: MatchResult, recover: bool) -> StoreResult<()> {
        let result = normalize(result)?;
        let mut tx = self.pool.begin().await?;
        // Serialize retries even before the allocation row exists.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,721946003))")
            .bind(&result.result_id)
            .execute(&mut *tx)
            .await?;
        if let Some(row) = locked_match(&mut tx, &result.result_id).await? {
            let allocation: Json<MatchResult> = row.try_get("allocation")?;
            if recover {
                let checkpoint: Json<MatchResult> = row.try_get("checkpoint")?;
                // Original Start metadata can be a subset of a later unrated
                // checkpoint. Otherwise the supplied roster must preserve all
                // known checkpoint identities and can only append when unrated.
                if same_allocation(&allocation, &result, false).is_err() {
                    same_allocation(&checkpoint, &result, roster_can_grow(&checkpoint))?;
                }
                if row.try_get::<String, _>("status")? == "settled" {
                    tx.commit().await?;
                    return Ok(());
                }
                if !row.try_get::<bool, _>("live")? {
                    sqlx::query("UPDATE career_matches SET owner_id=$2,generation=generation+1,lease_until=clock_timestamp()+interval '90 seconds' WHERE result_id=$1")
                        .bind(&result.result_id).bind(&self.owner).execute(&mut *tx).await?;
                } else {
                    require_owner(&row, &self.owner)?;
                }
            } else {
                same_allocation(&allocation, &result, false)?;
                require_owner(&row, &self.owner)?;
            }
            tx.commit().await?;
            return Ok(());
        }
        if result.rated {
            validate_rated(&result)?;
        }
        sqlx::query("INSERT INTO career_matches(result_id,server_epoch,match_id,owner_id,lease_until,allocation,checkpoint) VALUES($1,$2,$3,$4,clock_timestamp()+interval '90 seconds',$5,$5)")
            .bind(&result.result_id).bind(result.server_epoch.to_string()).bind(result.match_id.to_string())
            .bind(&self.owner).bind(Json(&result)).execute(&mut *tx).await?;
        let profiles = append_participants(&mut tx, &result, &[]).await?;
        if result.rated {
            validate_current_cohort(&profiles)?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn checkpoint(&self, result: MatchResult) -> Result<(), String> {
        retry(|| self.checkpoint_inner(result.clone())).await
    }
    async fn checkpoint_inner(&self, result: MatchResult) -> StoreResult<()> {
        let result = normalize(result)?;
        let mut tx = self.pool.begin().await?;
        let row = locked_match(&mut tx, &result.result_id)
            .await?
            .ok_or("Match must be durably started first.")?;
        require_owner(&row, &self.owner)?;
        let previous: Json<MatchResult> = row.try_get("checkpoint")?;
        same_allocation(&previous, &result, roster_can_grow(&previous))?;
        if row.try_get::<String, _>("status")? != "running" {
            return Err("Terminal result is already frozen.".into());
        }
        if result.duration_ms < previous.duration_ms {
            // A delayed spool retry must not replace a newer recovery snapshot.
            tx.commit().await?;
            return Ok(());
        }
        append_participants(&mut tx, &result, &previous.participants).await?;
        sqlx::query("UPDATE career_matches SET checkpoint=$2 WHERE result_id=$1")
            .bind(&result.result_id)
            .bind(Json(&result))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Persist terminal intent separately; reward calculation has not happened
    /// when this returns. `settle` always performs this durable stage first.
    pub async fn stage(&self, result: MatchResult) -> Result<(), String> {
        retry(|| self.stage_inner(result.clone())).await
    }
    async fn stage_inner(&self, result: MatchResult) -> StoreResult<()> {
        let result = normalize_terminal(result)?;
        let mut tx = self.pool.begin().await?;
        let row = locked_match(&mut tx, &result.result_id)
            .await?
            .ok_or("Match must be durably started first.")?;
        if let Some(intent) = row.try_get::<Option<Json<MatchResult>>, _>("intent")? {
            if intent.0 != result {
                return Err("Conflicting immutable result for this match.".into());
            }
            // Replaying a settled receipt is safe even after process restart.
            if row.try_get::<String, _>("status")? != "settled" {
                require_owner(&row, &self.owner)?;
            }
            tx.commit().await?;
            return Ok(());
        }
        require_owner(&row, &self.owner)?;
        let previous: Json<MatchResult> = row.try_get("checkpoint")?;
        same_terminal_allocation(&previous, &result)?;
        append_participants(&mut tx, &result, &previous.participants).await?;
        sqlx::query("UPDATE career_matches SET intent=$2,status='pending' WHERE result_id=$1")
            .bind(&result.result_id)
            .bind(Json(&result))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn settle(&self, result: MatchResult) -> Result<MatchResult, String> {
        self.stage(result.clone()).await?;
        retry(|| self.finish_inner(&result.result_id, false)).await
    }
    async fn finish_inner(&self, result_id: &str, expired: bool) -> StoreResult<MatchResult> {
        let mut tx = self.pool.begin().await?;
        let row = locked_match(&mut tx, result_id)
            .await?
            .ok_or("Match allocation not found.")?;
        if row.try_get::<String, _>("status")? == "settled" {
            let result: Json<MatchResult> = row.try_get("result")?;
            tx.commit().await?;
            return Ok(result.0);
        }
        if expired {
            if row.try_get::<bool, _>("live")? {
                return Err("Another live game server still owns this match.".into());
            }
        } else {
            require_owner(&row, &self.owner)?;
        }
        let mut result: MatchResult = if let Some(intent) =
            row.try_get::<Option<Json<MatchResult>>, _>("intent")?
        {
            intent.0
        } else {
            if !expired {
                return Err("A terminal result must be staged before settlement.".into());
            }
            let checkpoint: Json<MatchResult> = row.try_get("checkpoint")?;
            let mut recovered = checkpoint.0;
            recovered.outcome = MatchOutcome::Interrupted;
            recovered.winner = None;
            recovered.rated = false;
            recovered.unrated_reason = Some("server_interrupted".into());
            let now_ms: i64 = sqlx::query_scalar(
                "SELECT floor(extract(epoch FROM clock_timestamp())*1000)::bigint",
            )
            .fetch_one(&mut *tx)
            .await?;
            recovered.ended_at_ms = u64::try_from(now_ms)
                .map_err(|_| "Database clock precedes Unix epoch.")?
                .max(recovered.started_at_ms)
                .max(recovered.ended_at_ms);
            recovered.duration_ms = recovered
                .duration_ms
                .max(recovered.ended_at_ms - recovered.started_at_ms);
            let recovered = normalize_terminal(recovered)?;
            sqlx::query("UPDATE career_matches SET intent=$2,status='pending',generation=generation+1 WHERE result_id=$1")
                .bind(result_id).bind(Json(&recovered)).execute(&mut *tx).await?;
            recovered
        };
        let mut profiles = locked_profiles(&mut tx, &result.participants).await?;
        let changes = if result.rated {
            validate_rated(&result)?;
            let players: Vec<_> = result
                .participants
                .iter()
                .map(|p| {
                    let profile =
                        &profiles[p.profile_id.as_ref().expect("validated rated identity")];
                    (p.player_id, p.team, profile.rating)
                })
                .collect();
            crate::matchmaking::rating_changes_for_players(
                &players,
                result.winner.expect("validated winner"),
            )
            .map_err(|e| StoreError::Invalid(e.to_string()))?
            .into_iter()
            .collect::<BTreeMap<_, _>>()
        } else {
            BTreeMap::new()
        };
        let public_casual = result.ruleset == PUBLIC_CASUAL_RULESET;
        for participant in &mut result.participants {
            let Some(id) = &participant.profile_id else {
                continue;
            };
            let profile = profiles
                .get_mut(id)
                .ok_or("Stored participant profile is missing.")?;
            profile.matches_played = profile
                .matches_played
                .checked_add(1)
                .ok_or("Career match counter is full.")?;
            if result.outcome == MatchOutcome::Completed {
                let won = result.winner == Some(participant.team);
                if won {
                    profile.wins = profile
                        .wins
                        .checked_add(1)
                        .ok_or("Career win counter is full.")?;
                } else {
                    profile.losses = profile
                        .losses
                        .checked_add(1)
                        .ok_or("Career loss counter is full.")?;
                }
                if result.rated {
                    let change = changes
                        .get(&participant.player_id)
                        .ok_or("Missing rating change.")?
                        .clone();
                    profile.rating = change.after;
                    participant.rating = Some(change);
                    profile.rated_matches = profile
                        .rated_matches
                        .checked_add(1)
                        .ok_or("Rated match counter is full.")?;
                }
                participant.progression_xp_gained = if result.rated {
                    if won { 150 } else { 100 }
                } else if public_casual {
                    if won { 50 } else { 25 }
                } else {
                    0
                };
                profile.progression_xp = profile
                    .progression_xp
                    .checked_add(u64::from(participant.progression_xp_gained))
                    .filter(|&xp| xp <= i64::MAX as u64)
                    .ok_or("Career progression counter is full.")?;
            }
            sqlx::query("UPDATE career_profiles SET rating=$2,rated_matches=$3,matches_played=$4,wins=$5,losses=$6,progression_xp=$7 WHERE profile_id=$1")
                .bind(id).bind(profile.rating).bind(i64::from(profile.rated_matches)).bind(i64::from(profile.matches_played))
                .bind(i64::from(profile.wins)).bind(i64::from(profile.losses)).bind(profile.progression_xp as i64)
                .execute(&mut *tx).await?;
        }
        result.saved = true;
        sqlx::query("UPDATE career_matches SET status='settled',result=$2,settled_at=clock_timestamp() WHERE result_id=$1")
            .bind(result_id).bind(Json(&result)).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM career_active_profiles WHERE result_id=$1")
            .bind(result_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE career_presence SET result_id=NULL WHERE result_id=$1")
            .bind(result_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn heartbeat(&self) -> Result<(), String> {
        retry(|| async {
            sqlx::query("UPDATE career_matches SET lease_until=clock_timestamp()+interval '90 seconds' WHERE owner_id=$1 AND status!='settled' AND lease_until>clock_timestamp()")
                .bind(&self.owner).execute(&self.pool).await?;
            Ok(())
        }).await
    }
    /// A new worker may replay its durable spool only after the old owner's
    /// lease expires. A live different owner is never taken over.
    pub async fn adopt_expired(&self, result_id: &str) -> Result<(), String> {
        retry(|| async {
            let mut tx = self.pool.begin().await?;
            let row = locked_match(&mut tx, result_id).await?.ok_or("Match allocation not found.")?;
            if row.try_get::<String,_>("status")? == "settled" { tx.commit().await?; return Ok(()); }
            if row.try_get::<bool,_>("live")? {
                require_owner(&row, &self.owner)?;
            } else {
                sqlx::query("UPDATE career_matches SET owner_id=$2,generation=generation+1,lease_until=clock_timestamp()+interval '90 seconds' WHERE result_id=$1")
                    .bind(result_id).bind(&self.owner).execute(&mut *tx).await?;
            }
            tx.commit().await?; Ok(())
        }).await
    }
    pub async fn recover_expired(&self) -> Result<Vec<MatchResult>, String> {
        self.recover_expired_scope(None).await
    }

    /// An allocated worker must never settle another worker's expired match
    /// before that worker has replayed its durable terminal outbox. Recovery
    /// children use the original worker epoch retained in the allocation status.
    pub async fn recover_expired_for_epoch(&self, epoch: u64) -> Result<Vec<MatchResult>, String> {
        if epoch == 0 {
            return Err("Recovery requires a valid original server epoch.".into());
        }
        self.recover_expired_scope(Some(epoch)).await
    }

    async fn recover_expired_scope(&self, epoch: Option<u64>) -> Result<Vec<MatchResult>, String> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT result_id FROM career_matches WHERE status!='settled' AND lease_until<=clock_timestamp() AND ($1::text IS NULL OR server_epoch=$1) ORDER BY seq LIMIT 32")
            .bind(epoch.map(|value| value.to_string()))
            .fetch_all(&self.pool).await.map_err(|e| StoreError::from(e).to_string())?;
        let mut recovered = Vec::new();
        for id in ids {
            // Recheck expiry under the match lock: another worker may have
            // adopted or settled it after the initial bounded scan.
            match retry(|| self.finish_inner(&id, true)).await {
                Ok(result) => recovered.push(result),
                Err(e) if e == "Another live game server still owns this match." => {}
                Err(e) => return Err(e),
            }
        }
        Ok(recovered)
    }

    #[cfg(test)]
    pub async fn touch_presence(
        &self,
        profile_id: &str,
        match_id: Option<&str>,
    ) -> Result<(), String> {
        self.touch_presences(&[(profile_id.to_owned(), match_id.map(str::to_owned))])
            .await
    }
    /// One atomic statement for the worker's bounded presence batch. All rows
    /// must validate; a foreign allocation cannot partially refresh other rows.
    pub async fn touch_presences(
        &self,
        entries: &[(String, Option<String>)],
    ) -> Result<(), String> {
        retry(|| async {
            if entries.len() > 512 { return Err("Presence batch exceeds the account-worker limit.".into()); }
            let mut unique = BTreeMap::<String, Option<String>>::new();
            for (id, game) in entries {
                check_id(id)?;
                if game.as_ref().is_some_and(|s| s.is_empty() || s.len() > 128) { return Err("Invalid presence allocation.".into()); }
                if let Some(previous) = unique.get(id) {
                    if previous.is_some() && game.is_some() && previous != game { return Err("Conflicting presence allocations for one profile.".into()); }
                    if previous.is_some() { continue; }
                }
                unique.insert(id.clone(), game.clone());
            }
            if unique.is_empty() { return Ok(()); }
            let (ids, games): (Vec<_>, Vec<_>) = unique.into_iter().unzip();
            let count = sqlx::query("WITH supplied AS MATERIALIZED (SELECT * FROM unnest($1::text[],$2::text[]) AS s(profile_id,result_id)), valid AS MATERIALIZED (SELECT s.* FROM supplied s JOIN career_profiles p USING(profile_id) WHERE s.result_id IS NULL OR EXISTS(SELECT 1 FROM career_active_profiles a JOIN career_matches m USING(result_id) WHERE a.profile_id=s.profile_id AND a.result_id=s.result_id AND m.owner_id=$3 AND m.status!='settled' AND m.lease_until>clock_timestamp())) INSERT INTO career_presence(profile_id,owner_id,result_id,expires_at) SELECT profile_id,$3,result_id,clock_timestamp()+interval '30 seconds' FROM valid WHERE (SELECT count(*) FROM valid)=(SELECT count(*) FROM supplied) ON CONFLICT(profile_id) DO UPDATE SET owner_id=EXCLUDED.owner_id,result_id=EXCLUDED.result_id,expires_at=EXCLUDED.expires_at")
                .bind(&ids).bind(&games).bind(&self.owner).execute(&self.pool).await?.rows_affected();
            if count != ids.len() as u64 { return Err("Presence requires existing profiles and this worker's live allocations.".into()); }
            Ok(())
        }).await
    }
    pub async fn friends(&self, actor: &str) -> Result<FriendsView, String> {
        self.friends_with_versions(actor)
            .await
            .map(|(view, _)| view)
    }
    pub async fn friends_with_versions(
        &self,
        actor: &str,
    ) -> Result<(FriendsView, BTreeMap<String, String>), String> {
        retry(|| async {
            check_id(actor)?;
            let rows = sqlx::query("SELECT p.*,f.requested_by,f.accepted,extract(epoch FROM f.created_at)::text AS created_epoch,CASE WHEN s.expires_at>clock_timestamp() THEN CASE WHEN EXISTS(SELECT 1 FROM career_active_profiles a JOIN career_matches m USING(result_id) WHERE a.profile_id=p.profile_id AND m.status!='settled' AND m.lease_until>clock_timestamp()) THEN 'playing' ELSE 'online' END ELSE 'offline' END AS presence FROM career_friendships f JOIN career_profiles p ON p.profile_id=CASE WHEN f.low_id=$1 THEN f.high_id ELSE f.low_id END LEFT JOIN career_presence s ON s.profile_id=p.profile_id WHERE f.low_id=$1 OR f.high_id=$1 ORDER BY p.profile_id LIMIT 257")
                .bind(actor).fetch_all(&self.pool).await?;
            let mut view = FriendsView::default();
            let mut versions = BTreeMap::new();
            for row in rows {
                let presence = match row.try_get::<String,_>("presence")?.as_str() {
                    "online" => FriendPresence::Online, "playing" => FriendPresence::Playing, _ => FriendPresence::Offline,
                };
                let friend = FriendProfile { profile: profile_row(&row)?, presence };
                versions.insert(friend.profile.profile_id.clone(), friendship_etag(
                    &row.try_get::<String,_>("created_epoch")?, &row.try_get::<String,_>("requested_by")?, row.try_get::<bool,_>("accepted")?));
                if row.try_get::<bool,_>("accepted")? { view.friends.push(friend); }
                else if row.try_get::<String,_>("requested_by")? == actor { view.outgoing.push(friend); }
                else { view.incoming.push(friend); }
            }
            Ok((view,versions))
        }).await
    }
    pub async fn friend_action(
        &self,
        actor: &str,
        target: &str,
        action: FriendAction,
    ) -> Result<FriendsView, String> {
        retry(|| self.friend_action_inner(actor, target, action)).await?;
        self.friends(actor).await
    }
    #[cfg(test)]
    pub async fn request_friend(&self, actor: &str, target: &str) -> Result<FriendsView, String> {
        self.friend_action(actor, target, FriendAction::Request)
            .await
    }
    #[cfg(test)]
    pub async fn accept_friend(&self, actor: &str, requester: &str) -> Result<FriendsView, String> {
        self.friend_action(actor, requester, FriendAction::Accept)
            .await
    }
    async fn friend_action_inner(
        &self,
        actor: &str,
        target: &str,
        action: FriendAction,
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await?;
        Self::friend_action_in_tx(&mut tx, actor, target, action, None).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The HTTP adapter holds its idempotency receipt in this same transaction.
    pub async fn apply_friend_action(
        tx: &mut Transaction<'_, Postgres>,
        actor: &str,
        target: &str,
        action: FriendAction,
        expected_etag: Option<&str>,
    ) -> Result<(), String> {
        Self::friend_action_in_tx(tx, actor, target, action, expected_etag)
            .await
            .map_err(|e| e.to_string())
    }

    async fn friend_action_in_tx(
        tx: &mut Transaction<'_, Postgres>,
        actor: &str,
        target: &str,
        action: FriendAction,
        expected_etag: Option<&str>,
    ) -> StoreResult<()> {
        check_id(actor)?;
        check_id(target)?;
        if actor == target {
            return Err("Choose a different profile.".into());
        }
        let (low, high) = if actor < target {
            (actor, target)
        } else {
            (target, actor)
        };
        // Includes absent friendship rows; two reverse requests cannot race.
        let ids: Vec<String> = sqlx::query_scalar("SELECT profile_id FROM career_profiles WHERE profile_id=$1 OR profile_id=$2 ORDER BY profile_id FOR UPDATE")
            .bind(low).bind(high).fetch_all(&mut **tx).await?;
        if ids.len() != 2 {
            return Err("Profile not found. Use the exact profile ID.".into());
        }
        let existing = sqlx::query("SELECT requested_by,accepted,extract(epoch FROM created_at)::text AS created_epoch FROM career_friendships WHERE low_id=$1 AND high_id=$2 FOR UPDATE")
            .bind(low).bind(high).fetch_optional(&mut **tx).await?;
        let state = existing
            .as_ref()
            .map(|r| {
                Ok::<_, sqlx::Error>((
                    r.try_get::<String, _>("requested_by")?,
                    r.try_get::<bool, _>("accepted")?,
                ))
            })
            .transpose()?;
        if let Some(expected) = expected_etag {
            let actual = existing
                .as_ref()
                .map(|row| -> StoreResult<String> {
                    Ok(friendship_etag(
                        &row.try_get::<String, _>("created_epoch")?,
                        &row.try_get::<String, _>("requested_by")?,
                        row.try_get::<bool, _>("accepted")?,
                    ))
                })
                .transpose()?;
            if actual.as_deref() != Some(expected) {
                return Err("Friend relationship changed; refresh before this action.".into());
            }
        }
        match action {
            FriendAction::Request => {
                match state {
                    Some((_, true)) => {}
                    Some((by, false)) if by == actor => {}
                    Some(_) => {
                        return Err(
                            "This player already invited you. Accept the incoming request.".into(),
                        );
                    }
                    None => {
                        for id in [low, high] {
                            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM career_friendships WHERE low_id=$1 OR high_id=$1")
                            .bind(id).fetch_one(&mut **tx).await?;
                            if count >= MAX_SOCIAL {
                                return Err("Friend and pending-request limit reached.".into());
                            }
                        }
                        sqlx::query("INSERT INTO career_friendships(low_id,high_id,requested_by) VALUES($1,$2,$3)")
                        .bind(low).bind(high).bind(actor).execute(&mut **tx).await?;
                    }
                }
            }
            FriendAction::Accept => match state {
                Some((_, true)) => {}
                Some((by, false)) if by == target => {
                    sqlx::query("UPDATE career_friendships SET accepted=TRUE WHERE low_id=$1 AND high_id=$2")
                    .bind(low).bind(high).execute(&mut **tx).await?;
                }
                _ => return Err("No incoming friend request to accept.".into()),
            },
            FriendAction::Reject | FriendAction::Cancel | FriendAction::Remove => {
                if let Some((by, accepted)) = state {
                    let allowed = match action {
                        FriendAction::Reject => !accepted && by == target,
                        FriendAction::Cancel => !accepted && by == actor,
                        FriendAction::Remove => accepted,
                        _ => false,
                    };
                    if !allowed {
                        return Err(
                            "Friend relationship changed; refresh before this action.".into()
                        );
                    }
                    sqlx::query("DELETE FROM career_friendships WHERE low_id=$1 AND high_id=$2")
                        .bind(low)
                        .bind(high)
                        .execute(&mut **tx)
                        .await?;
                }
            }
        }
        Ok(())
    }
}

pub fn friendship_etag(created_epoch: &str, requested_by: &str, accepted: bool) -> String {
    format!("v1:{created_epoch}:{requested_by}:{}", u8::from(accepted))
}

fn check_id(id: &str) -> StoreResult<()> {
    if !valid_profile_id(id) {
        Err("Invalid profile or public-key identifier.".into())
    } else {
        Ok(())
    }
}
fn profile_row(row: &PgRow) -> StoreResult<ProfileSummary> {
    let counter = |name| -> StoreResult<u32> {
        u32::try_from(row.try_get::<i64, _>(name)?)
            .map_err(|_| "Invalid stored profile counter.".into())
    };
    Ok(ProfileSummary {
        profile_id: row.try_get("profile_id")?,
        nickname: row.try_get("nickname")?,
        rating: row.try_get("rating")?,
        rated_matches: counter("rated_matches")?,
        matches_played: counter("matches_played")?,
        wins: counter("wins")?,
        losses: counter("losses")?,
        progression_xp: u64::try_from(row.try_get::<i64, _>("progression_xp")?)
            .map_err(|_| "Invalid stored progression.")?,
    })
}
async fn locked_match(tx: &mut Transaction<'_, Postgres>, id: &str) -> StoreResult<Option<PgRow>> {
    Ok(sqlx::query("SELECT *,lease_until>clock_timestamp() AS live FROM career_matches WHERE result_id=$1 FOR UPDATE")
        .bind(id).fetch_optional(&mut **tx).await?)
}
fn require_owner(row: &PgRow, owner: &str) -> StoreResult<()> {
    if row.try_get::<String, _>("owner_id")? != owner || !row.try_get::<bool, _>("live")? {
        Err("This match requires its live owner or explicit expired-owner adoption.".into())
    } else {
        Ok(())
    }
}
async fn locked_profiles(
    tx: &mut Transaction<'_, Postgres>,
    participants: &[ParticipantResult],
) -> StoreResult<BTreeMap<String, ProfileSummary>> {
    let mut ids: Vec<_> = participants
        .iter()
        .filter_map(|p| p.profile_id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    let mut profiles = BTreeMap::new();
    // All callers acquire profile locks in precisely this order, even across
    // different matches. No gameplay waits occur while these locks are held.
    for id in ids {
        let row = sqlx::query("SELECT * FROM career_profiles WHERE profile_id=$1 FOR UPDATE")
            .bind(&id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or("Participant profile not found.")?;
        profiles.insert(id, profile_row(&row)?);
    }
    Ok(profiles)
}
async fn append_participants(
    tx: &mut Transaction<'_, Postgres>,
    result: &MatchResult,
    old: &[ParticipantResult],
) -> StoreResult<BTreeMap<String, ProfileSummary>> {
    let old_ids: HashSet<_> = old.iter().map(|p| p.player_id).collect();
    let additions: Vec<_> = result
        .participants
        .iter()
        .filter(|p| !old_ids.contains(&p.player_id))
        .cloned()
        .collect();
    let profiles = locked_profiles(tx, &additions).await?;
    for (index, p) in additions.iter().enumerate() {
        sqlx::query("INSERT INTO career_participants(result_id,seat,player_id,profile_id) VALUES($1,$2,$3,$4)")
            .bind(&result.result_id).bind((old.len()+index) as i16).bind(p.player_id.to_string()).bind(&p.profile_id).execute(&mut **tx).await?;
        if let Some(id) = &p.profile_id {
            sqlx::query("INSERT INTO career_active_profiles(profile_id,result_id) VALUES($1,$2)")
                .bind(id)
                .bind(&result.result_id)
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(profiles)
}
fn validate_current_cohort(profiles: &BTreeMap<String, ProfileSummary>) -> StoreResult<()> {
    let first = profiles
        .values()
        .next()
        .ok_or("Rated allocation requires profiles.")?;
    let (minimum, maximum) = profiles
        .values()
        .fold((first.rating, first.rating), |(low, high), profile| {
            (low.min(profile.rating), high.max(profile.rating))
        });
    if profiles
        .values()
        .any(|profile| profile.newcomer() != first.newcomer())
        || maximum - minimum > crate::matchmaking::MAX_RATING_SPREAD
    {
        // Queue snapshots on another process can outlive a completed match.
        // Locks are held through allocation commit, so current classification
        // cannot change after this check and before seats become authoritative.
        return Err("Queued ratings changed. Clear the queue and try again.".into());
    }
    Ok(())
}
fn pool_connections(role: Option<&str>) -> u32 {
    if role == Some("match") { 1 } else { 4 }
}

fn roster_can_grow(result: &MatchResult) -> bool {
    !result.rated && result.ruleset != PUBLIC_CASUAL_RULESET
}

/// The trusted allocator/runtime chooses this marker only after checking actual
/// map/modifier settings. Persisted metadata still fails closed on malformed or
/// unauthenticated rosters. This policy grants progression, never competitive Elo.
fn validate_public_casual(result: &MatchResult) -> StoreResult<()> {
    let interrupted_recovery = result.outcome != MatchOutcome::Completed
        && result.unrated_reason.as_deref() == Some("server_interrupted");
    let bots = result.participants.iter().filter(|p| p.is_bot).count();
    if result.rated
        || (result.unrated_reason.as_deref() != Some("allocated_bots") && !interrupted_recovery)
        || result.map_profile != shared::map::ResolvedMap::default().map_profile
        || result.participants.len() != 10
        || bots == 0
        || bots == result.participants.len()
        || result
            .participants
            .iter()
            .filter(|p| p.team == Team::Green)
            .count()
            != 5
        || result
            .participants
            .iter()
            .any(|p| p.is_bot == p.profile_id.is_some())
    {
        return Err(
            "Public casual progression requires the approved allocated human/bot roster.".into(),
        );
    }
    Ok(())
}

fn normalize(mut result: MatchResult) -> StoreResult<MatchResult> {
    if result.result_id.is_empty()
        || result.result_id.len() > 128
        || result.result_id.chars().any(char::is_control)
        || result.server_epoch == 0
        || result.match_id == 0
        || result.participants.is_empty()
        || result.participants.len() > MAX_PARTICIPANTS
        || result.map_profile.is_empty()
        || result.map_profile.len() > 128
        || result.ruleset.is_empty()
        || result.ruleset.len() > 128
    {
        return Err("Invalid match identity, metadata or roster size.".into());
    }
    if result.ended_at_ms != 0 && result.ended_at_ms < result.started_at_ms {
        return Err("Match end precedes its start.".into());
    }
    let mut players = HashSet::new();
    let mut profiles = HashSet::new();
    for p in &mut result.participants {
        if p.player_id == 0 || !players.insert(p.player_id) {
            return Err("Duplicate or invalid participant identity.".into());
        }
        if let Some(id) = &p.profile_id {
            check_id(id)?;
            if !profiles.insert(id.clone()) {
                return Err("Profile occupies multiple seats.".into());
            }
        }
        normalize_nickname(&p.nickname)?;
        if [
            p.stats.damage_to_heroes,
            p.stats.damage_to_structures,
            p.stats.damage_to_creeps,
            p.stats.damage_taken,
        ]
        .iter()
        .any(|v| !v.is_finite() || *v < 0.0)
        {
            return Err("Damage totals must be finite and nonnegative.".into());
        }
        if p.character.len() > 256
            || p.avatar.as_ref().is_some_and(|s| s.len() > 256)
            || p.sprite_character.as_ref().is_some_and(|s| s.len() > 256)
        {
            return Err("Participant loadout is too large.".into());
        }
        p.rating = None;
        p.progression_xp_gained = 0;
    }
    result.saved = false;
    result.participants.sort_by_key(|p| p.player_id);
    if result.ruleset == PUBLIC_CASUAL_RULESET {
        validate_public_casual(&result)?;
    }
    if serde_json::to_vec(&result)
        .map_err(|_| "Invalid result JSON.")?
        .len()
        > MAX_JSON
    {
        return Err("Result payload exceeds storage limit.".into());
    }
    Ok(result)
}
fn normalize_terminal(result: MatchResult) -> StoreResult<MatchResult> {
    let result = normalize(result)?;
    if result.ended_at_ms == 0 || result.ended_at_ms < result.started_at_ms {
        return Err("Terminal result requires a valid end timestamp.".into());
    }
    match result.outcome {
        MatchOutcome::Completed => {
            if result.winner.is_none() {
                return Err("Completed match requires its authoritative winner.".into());
            }
        }
        _ => {
            if result.winner.is_some() || result.rated {
                return Err("Interrupted or abandoned matches cannot grant ranked wins.".into());
            }
        }
    }
    if result.rated {
        validate_rated(&result)?;
    }
    Ok(result)
}
fn validate_rated(result: &MatchResult) -> StoreResult<()> {
    if result
        .participants
        .iter()
        .any(|participant| participant.is_bot)
    {
        return Err("Rated matches cannot contain bot participants.".into());
    }
    let green = result
        .participants
        .iter()
        .filter(|p| p.team == Team::Green)
        .count();
    if result.ruleset != RULESET
        || result.unrated_reason.is_some()
        || result.participants.len() < 2
        || green * 2 != result.participants.len()
        || result.participants.iter().any(|p| p.profile_id.is_none())
    {
        return Err(
            "Rated matches require the approved ruleset and equal authenticated teams.".into(),
        );
    }
    Ok(())
}
fn same_person(a: &ParticipantResult, b: &ParticipantResult) -> bool {
    a.player_id == b.player_id
        && a.is_bot == b.is_bot
        && a.profile_id == b.profile_id
        && a.nickname == b.nickname
        && a.team == b.team
        && a.hero_class == b.hero_class
        && a.character == b.character
        && a.avatar == b.avatar
        && a.sprite_character == b.sprite_character
}
fn same_allocation(old: &MatchResult, new: &MatchResult, allow_add: bool) -> StoreResult<()> {
    if old.result_id != new.result_id
        || old.server_epoch != new.server_epoch
        || old.match_id != new.match_id
        || old.started_at_ms != new.started_at_ms
        || old.map_profile != new.map_profile
        || old.ruleset != new.ruleset
        || old.rated != new.rated
        || old.unrated_reason != new.unrated_reason
        || (!allow_add && old.participants.len() != new.participants.len())
        || old
            .participants
            .iter()
            .any(|a| !new.participants.iter().any(|b| same_person(a, b)))
    {
        Err("Result conflicts with its frozen allocation or participant identity.".into())
    } else {
        Ok(())
    }
}
fn same_terminal_allocation(old: &MatchResult, new: &MatchResult) -> StoreResult<()> {
    let mut comparable = new.clone();
    if new.outcome != MatchOutcome::Completed {
        comparable.rated = old.rated;
        comparable.unrated_reason = old.unrated_reason.clone();
    }
    same_allocation(old, &comparable, roster_can_grow(old))
}

#[cfg(test)]
#[path = "career_store_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "supporter_store_tests.rs"]
mod supporter_tests;
