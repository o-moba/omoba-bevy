//! Account privacy and browser mutations. Receipts and effects commit together.
use crate::auth::Session;
use crate::read::parameters;
use crate::*;
use serde::Deserialize;
use shared::career::{FriendAction, normalize_nickname, valid_profile_id};

async fn settings(app: &App, id: &str) -> Result<Value> {
    let row = sqlx::query("SELECT * FROM portal.profile_settings WHERE profile_id=$1")
        .bind(id)
        .fetch_optional(&app.pool)
        .await?;
    let row = match row {
        Some(row) => row,
        None => {
            // Most account refreshes are reads. Create defaults only on the
            // first visit, tolerating another replica creating them at once.
            sqlx::query(
                "INSERT INTO portal.profile_settings(profile_id) VALUES($1) ON CONFLICT DO NOTHING",
            )
            .bind(id)
            .execute(&app.pool)
            .await?;
            sqlx::query("SELECT * FROM portal.profile_settings WHERE profile_id=$1")
                .bind(id)
                .fetch_one(&app.pool)
                .await?
        }
    };
    Ok(
        json!({"public_profile_enabled":row.try_get::<bool,_>("public_profile_enabled")?,"public_stats_enabled":row.try_get::<bool,_>("public_stats_enabled")?,"discoverable":row.try_get::<bool,_>("discoverable")?,"leaderboard_listed":row.try_get::<bool,_>("leaderboard_listed")?,"locale":row_text(&row,"locale")?,"timezone":row_text(&row,"timezone")?,"version":row.try_get::<i64,_>("version")?.to_string()}),
    )
}
pub async fn me(app: &App, s: &Session) -> Result<Value> {
    let profile = player(app, Some(s), &s.profile_id).await?["profile"].clone();
    Ok(
        json!({"profile":profile,"settings":settings(app,&s.profile_id).await?,"rules":{"newcomer_matches":shared::career::NEWCOMER_MATCHES},"session":{"session_id":s.session_id},"social":social::linked(app,&s.profile_id).await?}),
    )
}
pub async fn player(app: &App, s: Option<&Session>, id: &str) -> Result<Value> {
    if !valid_profile_id(id) {
        return Err(forbidden());
    }
    let actor = s.map(|s| s.profile_id.as_str()).unwrap_or("");
    // Access flags and public fields come from one MVCC snapshot, avoiding privacy TOCTOU.
    let row=sqlx::query("SELECT p.*,p.progression_xp::text AS xp,coalesce(s.public_profile_enabled,false) AS public,coalesce(s.public_stats_enabled,false) AS stats,($1=$2 OR EXISTS(SELECT 1 FROM career_friendships WHERE low_id=least($1,$2) AND high_id=greatest($1,$2))) AS contact,
        coalesce(pref.recognition_visible,false) AS supporter_visible,credit.months_supported,credit.hero_until,history.entries AS ranked_archive
        FROM career_profiles p LEFT JOIN portal.profile_settings s USING(profile_id)
        LEFT JOIN portal.supporter_preferences pref USING(profile_id)
        CROSS JOIN LATERAL (SELECT count(*) AS months_supported,max(valid_until) FILTER (WHERE valid_from<=$3 AND valid_until>$3) AS hero_until FROM portal.supporter_grants g WHERE g.profile_id=p.profile_id AND g.revoked_at IS NULL) credit
        CROSS JOIN LATERAL (SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY a.season_id DESC),'[]'::jsonb) AS entries FROM (
          SELECT ar.season_id,se.started_at_ms,se.ends_at_ms,ar.rating,ar.rated_matches,ar.wins,ar.losses,ar.completed_matches,ar.badge
          FROM career_ranked_archives ar JOIN career_ranked_seasons se USING(season_id)
          WHERE ar.profile_id=p.profile_id ORDER BY ar.season_id DESC LIMIT 10
        ) a) history
        WHERE p.profile_id=$2")
        .bind(actor).bind(id).bind(now()).fetch_optional(&app.pool).await?.ok_or_else(forbidden)?;
    let contact: bool = row.try_get("contact")?;
    if !contact && !row.try_get::<bool, _>("public")? {
        return Err(forbidden());
    }
    let mut p = json!({"profile_id":id,"nickname":row_text(&row,"nickname")?});
    if contact || row.try_get::<bool, _>("stats")? {
        p["rating"] = json!(row.try_get::<i32, _>("rating")?);
        p["ranked"] = json!(CareerStore::ranked_profile_from_row(&row).map_err(game_error)?);
        p["ranked_archive"] = row.try_get::<Value, _>("ranked_archive")?;
        if actor == id {
            p["completed_public_matches"] =
                json!(row.try_get::<i64, _>("completed_public_matches")?);
        }
        for field in [
            "rated_matches",
            "matches_played",
            "wins",
            "losses",
            "abandons",
        ] {
            p[field] = json!(row.try_get::<i64, _>(field)?);
        }
        let xp = row_text(&row, "xp")?;
        let n = xp.parse::<u64>().map_err(|_| invalid())?;
        p["progression_xp"] = json!(xp);
        p["level"] = json!((1 + n / 1000).to_string());
    }
    if row.try_get::<bool, _>("public")?
        && row.try_get::<bool, _>("supporter_visible")?
        && row.try_get::<i64, _>("months_supported")? > 0
    {
        let until: Option<i64> = row.try_get("hero_until")?;
        p["supporter"] = json!({
            "months_supported":row.try_get::<i64,_>("months_supported")?,
            "hero_active":until.is_some(),
            "hero_until":until,
        });
    }
    Ok(json!({"profile":p,"access":if contact{"contact"}else{"public"}}))
}
pub async fn friends(app: &App, s: &Session) -> Result<Value> {
    let (view, versions) = app
        .career
        .friends_with_versions(&s.profile_id)
        .await
        .map_err(game_error)?;
    let convert = |items: Vec<shared::career::FriendProfile>, presence: bool| -> Vec<Value> {
        items.into_iter().map(|f|{
        let mut v=json!({"profile":profile_dto(&f.profile),"etag":versions.get(&f.profile.profile_id)});
        if presence {v["presence"]=json!(f.presence);}v
    }).collect()
    };
    Ok(
        json!({"friends":convert(view.friends,true),"incoming":convert(view.incoming,false),"outgoing":convert(view.outgoing,false),"data_as_of":timestamp(&app.pool,now()).await?}),
    )
}
pub async fn search(app: &App, s: &Session, raw: Option<&str>) -> Result<Value> {
    let q = parameters(raw)?;
    if q.len() != 1 {
        return Err(invalid());
    }
    let query = q.get("query").ok_or_else(invalid)?.trim();
    if query.contains('#') {
        return match app.career.lookup_player(query).await {
            Ok(player) => Ok(json!({"players":[player]})),
            Err(e) if e.starts_with("Player not found") => Ok(json!({"players":[]})),
            Err(e) => Err(game_error(e)),
        };
    }
    if valid_profile_id(query) {
        return match player(app, Some(s), query).await {
            Ok(p) => Ok(json!({"players":[p["profile"].clone()]})),
            Err(Error(StatusCode::NOT_FOUND, _)) => Ok(json!({"players":[]})),
            Err(e) => Err(e),
        };
    }
    if query.chars().count() < 2 || query.len() > 80 {
        return Err(invalid());
    }
    let prefix = format!(
        "{}%",
        query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let rows=sqlx::query("SELECT p.profile_id,p.nickname FROM career_profiles p JOIN portal.profile_settings s USING(profile_id) WHERE s.public_profile_enabled AND s.discoverable AND p.nickname ILIKE $1 ESCAPE '\' ORDER BY lower(p.nickname),p.profile_id LIMIT 20")
        .bind(prefix).fetch_all(&app.pool).await?;
    let mut players = Vec::new();
    for row in rows {
        players.push(json!({"profile_id":row_text(&row,"profile_id")?,"nickname":row_text(&row,"nickname")?}));
    }
    Ok(json!({"players":players}))
}
fn ladder_ranked(row: &sqlx::postgres::PgRow) -> Result<shared::ranked::SeasonProfile> {
    let mut ranked = CareerStore::ranked_profile_from_row(row).map_err(game_error)?;
    // Listing the current rating does not publish private historical awards.
    ranked.latest_badge = None;
    Ok(ranked)
}

/// The current-season rating ladder and caller's position under the same filter.
pub async fn leaderboard(app: &App, s: &Session, raw: Option<&str>) -> Result<Value> {
    let q = parameters(raw)?;
    if q.keys().any(|k| k != "limit") {
        return Err(invalid());
    }
    let limit = q
        .get("limit")
        .map(|v| v.parse::<i64>())
        .transpose()
        .map_err(|_| invalid())?
        .unwrap_or(100);
    if !(1..=100).contains(&limit) {
        return Err(invalid());
    }
    // Use this season's calibration and tie-break count, matching the schema10
    // partial index. Lifetime totals remain separate response fields.
    // A profile without a settings row is listed.
    let eligible = |profile: &str, settings: &str| {
        format!(
            "{profile}.season_rated_matches>={} AND coalesce({settings}.leaderboard_listed,true)",
            shared::career::NEWCOMER_MATCHES
        )
    };
    let top = format!(
        "SELECT p.* FROM career_profiles p LEFT JOIN portal.profile_settings s USING(profile_id) WHERE {} ORDER BY p.rating DESC,p.season_rated_matches DESC,p.profile_id LIMIT $1",
        eligible("p", "s")
    );
    // The caller's rank counts the listed, eligible profiles ordered strictly
    // before it: higher rating, then more rated matches, then a smaller id.
    let mine = format!(
        "SELECT me.*,coalesce(s.leaderboard_listed,true) AS listed,CASE WHEN {} THEN (SELECT count(*)+1 FROM career_profiles o LEFT JOIN portal.profile_settings os USING(profile_id) WHERE {} AND (o.rating>me.rating OR (o.rating=me.rating AND (o.season_rated_matches>me.season_rated_matches OR (o.season_rated_matches=me.season_rated_matches AND o.profile_id<me.profile_id))))) END AS rank FROM career_profiles me LEFT JOIN portal.profile_settings s USING(profile_id) WHERE me.profile_id=$1",
        eligible("me", "s"),
        eligible("o", "os")
    );
    // Both statements read one MVCC snapshot, so the rank matches the entries.
    let mut tx = app.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let rows = sqlx::query(&top).bind(limit).fetch_all(&mut *tx).await?;
    let me = sqlx::query(&mine)
        .bind(&s.profile_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(forbidden)?;
    tx.commit().await?;
    let mut entries = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        entries.push(json!({
            "rank": index as i64 + 1,
            "profile_id": row_text(row, "profile_id")?,
            "nickname": row_text(row, "nickname")?,
            "rating": row.try_get::<i32, _>("rating")?,
            "rated_matches": row.try_get::<i64, _>("rated_matches")?,
            "ranked": ladder_ranked(row)?,
            "wins": row.try_get::<i64, _>("wins")?,
            "losses": row.try_get::<i64, _>("losses")?,
        }));
    }
    Ok(json!({
        "entries": entries,
        "me": {
            "rank": me.try_get::<Option<i64>, _>("rank")?,
            "rating": me.try_get::<i32, _>("rating")?,
            "rated_matches": me.try_get::<i64, _>("rated_matches")?,
            "ranked": ladder_ranked(&me)?,
            "listed": me.try_get::<bool, _>("listed")?,
        },
        "min_rated_matches": shared::career::NEWCOMER_MATCHES,
        "data_as_of": timestamp(&app.pool, now()).await?,
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nickname {
    nickname: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FriendMutation {
    target_profile_id: String,
    action: FriendAction,
    expected_etag: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    public_profile_enabled: bool,
    public_stats_enabled: bool,
    discoverable: bool,
    leaderboard_listed: bool,
    locale: String,
    timezone: String,
    expected_version: String,
}
pub async fn mutate(
    app: &App,
    s: &Session,
    path: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Value> {
    let key = headers
        .get("idempotency-key")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(invalid)?;
    if !(16..=80).contains(&key.len())
        || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(invalid());
    }
    let payload: Value = parse(body)?;
    let hash =
        crypto::hash(&serde_json::to_vec(&(&s.profile_id, path, &payload)).map_err(|_| invalid())?);
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,721946102))")
        .bind(format!("{}:{key}", s.session_id))
        .execute(&mut *tx)
        .await?;
    if let Some(row)=sqlx::query("SELECT body_hash,response FROM portal.operation_receipts WHERE session_id=$1 AND operation_key=$2").bind(&s.session_id).bind(key).fetch_optional(&mut *tx).await?{
        if row_text(&row,"body_hash")?!=hash{return Err(Error(StatusCode::CONFLICT,"idempotency_conflict"));}
        return Ok(row.try_get::<sqlx::types::Json<Value>,_>("response")?.0);
    }
    match path {
        "me/friend-actions" => {
            let f: FriendMutation = parse(body)?;
            if f.action != FriendAction::Request
                && f.expected_etag
                    .as_ref()
                    .is_none_or(|v| v.len() > 160 || v.is_empty())
            {
                return Err(invalid());
            }
            omoba_career_store::career_store::CareerStore::apply_friend_action(
                &mut tx,
                &s.profile_id,
                &f.target_profile_id,
                f.action,
                f.expected_etag.as_deref(),
            )
            .await
            .map_err(game_error)?;
        }
        "me/profile" => {
            let n: Nickname = parse(body)?;
            let nickname = normalize_nickname(&n.nickname).map_err(|_| invalid())?;
            // A moderator's name reset locks renames until it ends (LR-15).
            let locked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM career_sanctions WHERE profile_id=$1 AND kind='name_reset' AND lifted_at IS NULL AND (expires_at IS NULL OR expires_at > clock_timestamp()))")
                .bind(&s.profile_id)
                .fetch_one(&mut *tx)
                .await?;
            if locked {
                return Err(Error(StatusCode::FORBIDDEN, "name_locked"));
            }
            sqlx::query("UPDATE career_profiles SET nickname=$2 WHERE profile_id=$1")
                .bind(&s.profile_id)
                .bind(nickname)
                .execute(&mut *tx)
                .await
                .map_err(|e| {
                    if e.as_database_error().and_then(|e| e.constraint())
                        == Some("career_player_handle")
                    {
                        Error(StatusCode::CONFLICT, "handle_taken")
                    } else {
                        e.into()
                    }
                })?;
        }
        "me/settings" => {
            let p: Preferences = parse(body)?;
            let version = p.expected_version.parse::<i64>().map_err(|_| invalid())?;
            if version.to_string() != p.expected_version
                || version < 1
                || (!p.public_profile_enabled && (p.public_stats_enabled || p.discoverable))
                || !matches!(p.locale.as_str(), "ru" | "en")
                || p.timezone.len() > 80
            {
                return Err(invalid());
            }
            let valid: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_timezone_names WHERE name=$1)")
                    .bind(&p.timezone)
                    .fetch_one(&mut *tx)
                    .await?;
            if !valid {
                return Err(invalid());
            }
            sqlx::query(
                "INSERT INTO portal.profile_settings(profile_id) VALUES($1) ON CONFLICT DO NOTHING",
            )
            .bind(&s.profile_id)
            .execute(&mut *tx)
            .await?;
            let changed=sqlx::query("UPDATE portal.profile_settings SET public_profile_enabled=$2,public_stats_enabled=$3,discoverable=$4,locale=$5,timezone=$6,leaderboard_listed=$8,version=version+1 WHERE profile_id=$1 AND version=$7")
                .bind(&s.profile_id).bind(p.public_profile_enabled).bind(p.public_stats_enabled).bind(p.discoverable).bind(p.locale).bind(p.timezone).bind(version).bind(p.leaderboard_listed).execute(&mut *tx).await?.rows_affected();
            if changed != 1 {
                return Err(conflict());
            }
        }
        _ => return Err(forbidden()),
    }
    let response = json!({"status":"updated"});
    sqlx::query("INSERT INTO portal.operation_receipts(session_id,operation_key,body_hash,response,expires_at) VALUES($1,$2,$3,$4,$5)").bind(&s.session_id).bind(key).bind(hash).bind(sqlx::types::Json(&response)).bind(now()+8*86400).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO portal.audit_events(profile_id,event) VALUES($1,$2)")
        .bind(&s.profile_id)
        .bind(path)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(response)
}
