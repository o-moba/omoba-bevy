//! Account privacy and browser mutations. Receipts and effects commit together.
use crate::auth::Session;
use crate::*;
use serde::Deserialize;
use shared::career::{FriendAction, normalize_nickname, valid_profile_id};

async fn settings(app: &App, id: &str) -> Result<Value> {
    sqlx::query(
        "INSERT INTO portal.profile_settings(profile_id) VALUES($1) ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .execute(&app.pool)
    .await?;
    let row = sqlx::query("SELECT * FROM portal.profile_settings WHERE profile_id=$1")
        .bind(id)
        .fetch_one(&app.pool)
        .await?;
    Ok(
        json!({"public_profile_enabled":row.try_get::<bool,_>("public_profile_enabled")?,"public_stats_enabled":row.try_get::<bool,_>("public_stats_enabled")?,"discoverable":row.try_get::<bool,_>("discoverable")?,"locale":row_text(&row,"locale")?,"timezone":row_text(&row,"timezone")?,"version":row.try_get::<i64,_>("version")?.to_string()}),
    )
}
pub async fn me(app: &App, s: &Session) -> Result<Value> {
    let profile = app
        .career
        .profile(&s.profile_id)
        .await
        .map_err(game_error)?;
    Ok(
        json!({"profile":profile_dto(&profile),"settings":settings(app,&s.profile_id).await?,"rules":{"newcomer_matches":shared::career::NEWCOMER_MATCHES},"session":{"session_id":s.session_id}}),
    )
}
pub async fn player(app: &App, s: Option<&Session>, id: &str) -> Result<Value> {
    if !valid_profile_id(id) {
        return Err(forbidden());
    }
    let actor = s.map(|s| s.profile_id.as_str()).unwrap_or("");
    // Access flags and public fields come from one MVCC snapshot, avoiding privacy TOCTOU.
    let row=sqlx::query("SELECT p.profile_id,p.nickname,p.rating,p.rated_matches,p.matches_played,p.wins,p.losses,p.progression_xp::text AS xp,coalesce(s.public_profile_enabled,false) AS public,coalesce(s.public_stats_enabled,false) AS stats,($1=$2 OR EXISTS(SELECT 1 FROM career_friendships WHERE low_id=least($1,$2) AND high_id=greatest($1,$2))) AS contact FROM career_profiles p LEFT JOIN portal.profile_settings s USING(profile_id) WHERE p.profile_id=$2")
        .bind(actor).bind(id).fetch_optional(&app.pool).await?.ok_or_else(forbidden)?;
    let contact: bool = row.try_get("contact")?;
    if !contact && !row.try_get::<bool, _>("public")? {
        return Err(forbidden());
    }
    let mut p = json!({"profile_id":id,"nickname":row_text(&row,"nickname")?});
    if contact || row.try_get::<bool, _>("stats")? {
        p["rating"] = json!(row.try_get::<i32, _>("rating")?);
        for field in ["rated_matches", "matches_played", "wins", "losses"] {
            p[field] = json!(row.try_get::<i64, _>(field)?);
        }
        let xp = row_text(&row, "xp")?;
        let n = xp.parse::<u64>().map_err(|_| invalid())?;
        p["progression_xp"] = json!(xp);
        p["level"] = json!((1 + n / 1000).to_string());
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
    let q = crate::read::parameters(raw)?;
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
            let changed=sqlx::query("UPDATE portal.profile_settings SET public_profile_enabled=$2,public_stats_enabled=$3,discoverable=$4,locale=$5,timezone=$6,version=version+1 WHERE profile_id=$1 AND version=$7")
                .bind(&s.profile_id).bind(p.public_profile_enabled).bind(p.public_stats_enabled).bind(p.discoverable).bind(p.locale).bind(p.timezone).bind(version).execute(&mut *tx).await?.rows_affected();
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
