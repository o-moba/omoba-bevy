//! Immutable result reads and a replayable, idempotent statistics projection.
use crate::auth::Session;
use crate::*;
use shared::career::{MatchOutcome, MatchResult};
use sqlx::types::Json as DbJson;
use std::collections::BTreeMap;

pub fn parameters(raw: Option<&str>) -> Result<BTreeMap<String, String>> {
    fn decode(s: &str) -> Result<String> {
        let mut out = Vec::new();
        let mut b = s.bytes();
        while let Some(c) = b.next() {
            match c {
                b'+' => out.push(b' '),
                b'%' => {
                    let a = b.next().ok_or_else(invalid)?;
                    let d = b.next().ok_or_else(invalid)?;
                    let digits = [a, d];
                    let n = u8::from_str_radix(
                        std::str::from_utf8(&digits).map_err(|_| invalid())?,
                        16,
                    )
                    .map_err(|_| invalid())?;
                    out.push(n)
                }
                _ => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| invalid())
    }
    let mut out = BTreeMap::new();
    for item in raw.unwrap_or("").split('&').filter(|s| !s.is_empty()) {
        let (k, v) = item.split_once('=').ok_or_else(invalid)?;
        if out.insert(decode(k)?, decode(v)?).is_some() {
            return Err(invalid());
        }
    }
    Ok(out)
}
struct Filter {
    from: f64,
    class: Option<String>,
    rated: Option<bool>,
    outcome: Option<String>,
    limit: i64,
    cursor: Option<(u64, String)>,
}
impl Filter {
    async fn read(app: &App, s: &Session, raw: Option<&str>, stats: bool) -> Result<Self> {
        let q = parameters(raw)?;
        if q.keys().any(|k| {
            !matches!(
                k.as_str(),
                "days" | "hero_class" | "rated" | "outcome" | "limit" | "cursor"
            )
        }) {
            return Err(invalid());
        }
        let days = match q.get("days").map(String::as_str).unwrap_or("30") {
            "7" => 7,
            "30" => 30,
            "90" => 90,
            "all" => 0,
            _ => return Err(invalid()),
        };
        let from: f64 = if days == 0 {
            0.
        } else {
            sqlx::query_scalar("SELECT extract(epoch FROM ((date_trunc('day',clock_timestamp() AT TIME ZONE coalesce((SELECT timezone FROM portal.profile_settings WHERE profile_id=$1),'UTC'))-make_interval(days=>$2-1)) AT TIME ZONE coalesce((SELECT timezone FROM portal.profile_settings WHERE profile_id=$1),'UTC')))::double precision").bind(&s.profile_id).bind(days).fetch_one(&app.pool).await?
        };
        let class = q.get("hero_class").filter(|v| v.as_str() != "all").cloned();
        if class
            .as_ref()
            .is_some_and(|v| !matches!(v.as_str(), "warrior" | "ranger" | "mage" | "cleric"))
        {
            return Err(invalid());
        }
        let rated =
            match q
                .get("rated")
                .map(String::as_str)
                .unwrap_or(if stats { "true" } else { "all" })
            {
                "true" => Some(true),
                "false" => Some(false),
                "all" => None,
                _ => return Err(invalid()),
            };
        let outcome = q.get("outcome").filter(|v| v.as_str() != "all").cloned();
        if outcome
            .as_ref()
            .is_some_and(|v| !matches!(v.as_str(), "win" | "loss" | "interrupted" | "abandoned"))
        {
            return Err(invalid());
        }
        let limit = q
            .get("limit")
            .map(|v| v.parse::<i64>())
            .transpose()
            .map_err(|_| invalid())?
            .unwrap_or(20);
        if !(1..=50).contains(&limit) {
            return Err(invalid());
        }
        let cursor = if let Some(c) = q.get("cursor") {
            let (t, id) = c.split_once(':').ok_or_else(invalid)?;
            let n = t.parse::<u64>().map_err(|_| invalid())?;
            if n.to_string() != t
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(invalid());
            }
            Some((n, id.to_owned()))
        } else {
            None
        };
        Ok(Self {
            from,
            class,
            rated,
            outcome,
            limit,
            cursor,
        })
    }
}
pub async fn detail(app: &App, s: &Session, id: &str) -> Result<Value> {
    if id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(invalid());
    }
    let r = app
        .career
        .detail(&s.profile_id, id)
        .await
        .map_err(game_error)?;
    let mut value = serde_json::to_value(&r).map_err(|_| invalid())?;
    for field in [
        "server_epoch",
        "match_id",
        "started_at_ms",
        "ended_at_ms",
        "duration_ms",
    ] {
        value[field] = json!(value[field].as_u64().ok_or_else(invalid)?.to_string());
    }
    value["started_at"] = json!(timestamp(&app.pool, (r.started_at_ms / 1000) as i64).await?);
    value["ended_at"] = json!(timestamp(&app.pool, (r.ended_at_ms / 1000) as i64).await?);
    for (v, p) in value["participants"]
        .as_array_mut()
        .ok_or_else(invalid)?
        .iter_mut()
        .zip(&r.participants)
    {
        v["player_id"] = json!(p.player_id.to_string());
    }
    Ok(json!({"match":value}))
}
pub async fn history(app: &App, s: &Session, raw: Option<&str>) -> Result<Value> {
    let f = Filter::read(app, s, raw, false).await?;
    let rows=sqlx::query("SELECT m.result FROM career_matches m JOIN career_participants cp USING(result_id) CROSS JOIN LATERAL jsonb_array_elements(m.result->'participants') p WHERE cp.profile_id=$1 AND p->>'profile_id'=$1 AND m.status='settled' AND (m.result->>'ended_at_ms')::numeric >= $2::double precision*1000 AND ($3::text IS NULL OR p->>'hero_class'=$3) AND ($4::boolean IS NULL OR (m.result->>'rated')::boolean=$4) AND ($5::text IS NULL OR CASE WHEN $5 IN('win','loss') THEN m.result->>'outcome'='completed' AND m.result->>'winner' IS NOT NULL AND ((m.result->>'winner'=p->>'team')=($5='win')) ELSE m.result->>'outcome'=$5 END) AND ($6::text::numeric IS NULL OR ((m.result->>'ended_at_ms')::numeric,m.result_id)<($6::text::numeric,$7::text)) ORDER BY (m.result->>'ended_at_ms')::numeric DESC,m.result_id DESC LIMIT $8")
        .bind(&s.profile_id).bind(f.from).bind(&f.class).bind(f.rated).bind(&f.outcome).bind(f.cursor.as_ref().map(|c|c.0.to_string())).bind(f.cursor.as_ref().map(|c|&c.1)).bind(f.limit+1).fetch_all(&app.pool).await?;
    let more = rows.len() > f.limit as usize;
    let mut entries = Vec::new();
    let mut cursor = None;
    for row in rows.into_iter().take(f.limit as usize) {
        let r: DbJson<MatchResult> = row.try_get("result")?;
        let p = r
            .participants
            .iter()
            .find(|p| p.profile_id.as_deref() == Some(&s.profile_id))
            .ok_or_else(invalid)?;
        cursor = Some(format!("{}:{}", r.ended_at_ms, r.result_id));
        entries.push(json!({"result_id":r.result_id,"ended_at":timestamp(&app.pool,(r.ended_at_ms/1000) as i64).await?,"duration_ms":r.duration_ms.to_string(),"outcome":r.outcome,"won":r.winner.map(|w|w==p.team),"rated":r.rated,"hero_class":p.hero_class,"avatar":p.avatar,"kills":p.stats.kills,"deaths":p.stats.deaths,"assists":p.stats.assists,"damage_to_heroes":p.stats.damage_to_heroes,"rating":p.rating}));
    }
    Ok(
        json!({"matches":entries,"next_cursor":if more{cursor}else{None},"data_as_of":timestamp(&app.pool,now()).await?}),
    )
}
pub async fn project(app: &App, limit: i64) -> Result<usize> {
    let mut tx = app.pool.begin().await?;
    // A transaction lock elects one projector; no allocation-sequence watermark can skip late settlement.
    let elected: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(721946103)")
        .fetch_one(&mut *tx)
        .await?;
    if !elected {
        return Ok(0);
    }
    let rows=sqlx::query("SELECT m.result FROM career_matches m LEFT JOIN portal.projected_results p USING(result_id) WHERE m.status='settled' AND p.result_id IS NULL ORDER BY m.seq LIMIT $1").bind(limit.clamp(1,256)).fetch_all(&mut *tx).await?;
    let count = rows.len();
    for row in rows {
        let r: DbJson<MatchResult> = row.try_get("result")?;
        for p in &r.participants {
            if p.is_bot || p.profile_id.is_none() {
                continue;
            }
            if !p.stats.damage_to_heroes.is_finite() {
                return Err(invalid());
            }
            sqlx::query("INSERT INTO portal.player_match_facts(result_id,player_id,profile_id,ended_at,duration_ms,hero_class,outcome,rated,won,kills,deaths,assists,damage_to_heroes,minion_last_hits,jungle_last_hits,rating_before,rating_after,rating_delta) VALUES($1,$2,$3,to_timestamp($4),$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18) ON CONFLICT(result_id,player_id) DO NOTHING")
                .bind(&r.result_id).bind(p.player_id.to_string()).bind(&p.profile_id).bind(r.ended_at_ms as f64/1000.).bind(r.duration_ms as f64)
                .bind(serde_json::to_value(p.hero_class).map_err(|_|invalid())?.as_str().ok_or_else(invalid)?)
                .bind(serde_json::to_value(r.outcome).map_err(|_|invalid())?.as_str().ok_or_else(invalid)?)
                .bind(r.rated).bind(if r.outcome==MatchOutcome::Completed{r.winner.map(|w|w==p.team)}else{None})
                .bind(i64::from(p.stats.kills)).bind(i64::from(p.stats.deaths)).bind(i64::from(p.stats.assists)).bind(p.stats.damage_to_heroes).bind(i64::from(p.stats.minion_last_hits)).bind(i64::from(p.stats.jungle_last_hits))
                .bind(p.rating.as_ref().map(|v|v.before)).bind(p.rating.as_ref().map(|v|v.after)).bind(p.rating.as_ref().map(|v|v.delta)).execute(&mut *tx).await?;
        }
        let bytes = serde_json::to_vec(&r.0).map_err(|_| invalid())?;
        sqlx::query(
            "INSERT INTO portal.projected_results(result_id,version,source_hash) VALUES($1,1,$2)",
        )
        .bind(&r.result_id)
        .bind(crypto::hash(&bytes))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(count)
}
fn aggregate(row: &sqlx::postgres::PgRow) -> Result<Value> {
    let matches: i64 = row.try_get("matches")?;
    let wins: i64 = row.try_get("wins")?;
    let losses: i64 = row.try_get("losses")?;
    let kills: i64 = row.try_get("kills")?;
    let deaths: i64 = row.try_get("deaths")?;
    let assists: i64 = row.try_get("assists")?;
    let duration: f64 = row.try_get("duration")?;
    let damage: f64 = row.try_get("damage")?;
    Ok(
        json!({"matches":matches,"wins":wins,"losses":losses,"kills":kills,"deaths":deaths,"assists":assists,"kda":if matches>0{Some((kills+assists) as f64/deaths.max(1) as f64)}else{None},"win_rate":if wins+losses>0{Some(wins as f64/(wins+losses) as f64)}else{None},"damage_per_minute":if duration>0.{Some(damage/(duration/60000.))}else{None},"minion_last_hits":row.try_get::<i64,_>("minions")?,"jungle_last_hits":row.try_get::<i64,_>("jungle")?}),
    )
}
pub async fn statistics(app: &App, s: &Session, raw: Option<&str>) -> Result<Value> {
    let f = Filter::read(app, s, raw, true).await?;
    let predicate = " FROM portal.player_match_facts WHERE profile_id=$1 AND ended_at>=to_timestamp($2) AND ($3::text IS NULL OR hero_class=$3) AND ($4::boolean IS NULL OR rated=$4) AND ($5::text IS NULL OR CASE WHEN $5 IN('win','loss') THEN outcome='completed' AND won=($5='win') ELSE outcome=$5 END)";
    let sql = format!(
        "SELECT hero_class,count(*)::bigint AS matches,count(*) FILTER(WHERE won)::bigint AS wins,count(*) FILTER(WHERE NOT won)::bigint AS losses,coalesce(sum(kills),0)::bigint AS kills,coalesce(sum(deaths),0)::bigint AS deaths,coalesce(sum(assists),0)::bigint AS assists,coalesce(sum(duration_ms),0)::double precision AS duration,coalesce(sum(damage_to_heroes),0)::double precision AS damage,coalesce(sum(minion_last_hits),0)::bigint AS minions,coalesce(sum(jungle_last_hits),0)::bigint AS jungle{predicate} GROUP BY GROUPING SETS ((),(hero_class)) ORDER BY hero_class NULLS FIRST"
    );
    let mut tx = app.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    // One expensive analytics request per account across API replicas.
    let elected: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1,721946104))")
            .bind(&s.profile_id)
            .fetch_one(&mut *tx)
            .await?;
    if !elected {
        return Err(Error(StatusCode::TOO_MANY_REQUESTS, "statistics_busy"));
    }
    let rows = sqlx::query(&sql)
        .bind(&s.profile_id)
        .bind(f.from)
        .bind(&f.class)
        .bind(f.rated)
        .bind(&f.outcome)
        .fetch_all(&mut *tx)
        .await?;
    let mut summary = Value::Null;
    let mut classes = Vec::new();
    for row in rows {
        let mut a = aggregate(&row)?;
        if let Some(c) = row.try_get::<Option<String>, _>("hero_class")? {
            a["hero_class"] = json!(c);
            classes.push(a);
        } else {
            summary = a;
        }
    }
    let rating_sql = format!(
        "SELECT result_id,to_char(ended_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS ended_at,rating_before,rating_after,rating_delta{predicate} AND rating_before IS NOT NULL ORDER BY ended_at DESC,result_id DESC LIMIT 1001"
    );
    let rows = sqlx::query(&rating_sql)
        .bind(&s.profile_id)
        .bind(f.from)
        .bind(&f.class)
        .bind(f.rated)
        .bind(&f.outcome)
        .fetch_all(&mut *tx)
        .await?;
    let truncated = rows.len() > 1000;
    let mut rating = Vec::new();
    for row in rows.into_iter().take(1000) {
        rating.push(json!({"result_id":row_text(&row,"result_id")?,"ended_at":row_text(&row,"ended_at")?,"before":row.try_get::<i32,_>("rating_before")?,"after":row.try_get::<i32,_>("rating_after")?,"delta":row.try_get::<i32,_>("rating_delta")?}));
    }
    rating.reverse();
    let pending:i64=sqlx::query_scalar("SELECT count(*) FROM career_matches m JOIN career_participants c USING(result_id) LEFT JOIN portal.projected_results p USING(result_id) WHERE c.profile_id=$1 AND m.status='settled' AND p.result_id IS NULL").bind(&s.profile_id).fetch_one(&mut *tx).await?;
    let asof:Option<String>=sqlx::query_scalar("SELECT to_char(max(p.projected_at) AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') FROM portal.projected_results p JOIN career_participants c USING(result_id) WHERE c.profile_id=$1").bind(&s.profile_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(
        json!({"summary":summary,"classes":classes,"rating":rating,"rating_truncated":truncated,"projection_status":if pending>0{"catching_up"}else{"current"},"pending_results":pending,"data_as_of":asof}),
    )
}
pub fn releases(app: &App) -> Result<Value> {
    let Some(path) = &app.config.release_file else {
        return Ok(json!({"releases":[]}));
    };
    let meta = std::fs::metadata(path)
        .map_err(|_| Error(StatusCode::SERVICE_UNAVAILABLE, "catalog_unavailable"))?;
    if meta.len() > 131072 {
        return Err(invalid());
    }
    let source = std::fs::read(path)
        .map_err(|_| Error(StatusCode::SERVICE_UNAVAILABLE, "catalog_unavailable"))?;
    let value: Value = parse(&source)?;
    let items = value
        .get("releases")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if items.len() > 32 {
        return Err(invalid());
    }
    for item in items {
        let url = item
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        let uri: axum::http::Uri = url.parse().map_err(|_| invalid())?;
        if uri.scheme_str() != Some("https")
            || uri.host().is_none()
            || uri.authority().is_some_and(|a| a.as_str().contains('@'))
            || uri.query().is_some()
        {
            return Err(invalid());
        }
        if crypto::decode::<32>(
            item.get("sha256")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?,
        )
        .is_none()
        {
            return Err(invalid());
        }
        let size = item
            .get("size_bytes")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?
            .parse::<u64>()
            .map_err(|_| invalid())?;
        if size == 0 {
            return Err(invalid());
        }
        for field in [
            "platform",
            "architecture",
            "version",
            "published_at",
            "instructions",
            "test_status",
        ] {
            if item
                .get(field)
                .and_then(Value::as_str)
                .is_none_or(|s| s.is_empty() || s.len() > 2000)
            {
                return Err(invalid());
            }
        }
    }
    Ok(json!({"releases":items}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_is_strict() {
        assert_eq!(
            parameters(Some("query=%D0%B8%D0%B3%D1%80%D0%B0+1")).unwrap()["query"],
            "игра 1"
        );
        for bad in ["a=%", "a=1&a=2", "a=%ff", "missing"] {
            assert!(parameters(Some(bad)).is_err());
        }
    }
}
