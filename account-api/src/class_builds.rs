//! Owner-scoped private drafts; shared gameplay rules validate every saved recipe.
//! Existing personal-settings consent does not grant publication or match admission.
use crate::auth::Session;
use crate::*;
use serde::Deserialize;
use shared::workshop::ClassBuildDocument;
use sqlx::postgres::PgRow;
use sqlx::types::Json as DbJson;

const MAX_BUILDS: i64 = 50;
const FIELDS: &str = "id,version,document,to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS created_at,to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS updated_at";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    document: ClassBuildDocument,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    expected_version: String,
    document: ClassBuildDocument,
}

fn version(raw: &str) -> Result<i64> {
    let value = raw.parse::<i64>().map_err(|_| invalid())?;
    if value < 1 || value == i64::MAX || value.to_string() != raw {
        return Err(invalid());
    }
    Ok(value)
}

fn identifier(value: &str) -> Result<()> {
    if !shared::career::valid_profile_id(value) {
        return Err(forbidden());
    }
    Ok(())
}

fn validate(document: &ClassBuildDocument) -> Result<()> {
    document
        .validate()
        .map_err(|_| Error(StatusCode::BAD_REQUEST, "invalid_class_build"))?;
    if serde_json::to_vec(document).map_err(|_| invalid())?.len() > 8192 {
        return Err(Error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large"));
    }
    Ok(())
}

async fn available(app: &App) -> Result<()> {
    let ready: bool = sqlx::query_scalar("SELECT to_regclass('portal.class_builds') IS NOT NULL")
        .fetch_one(&app.pool)
        .await?;
    if !ready {
        return Err(Error(
            StatusCode::SERVICE_UNAVAILABLE,
            "class_builds_unavailable",
        ));
    }
    Ok(())
}

fn dto(row: &PgRow) -> Result<Value> {
    Ok(json!({
        "id":row_text(row,"id")?,
        "version":row.try_get::<i64,_>("version")?.to_string(),
        "document":row.try_get::<DbJson<Value>,_>("document")?.0,
        "created_at":row_text(row,"created_at")?,
        "updated_at":row_text(row,"updated_at")?
    }))
}

pub async fn list(app: &App, session: &Session, query: Option<&str>) -> Result<Value> {
    if !read::parameters(query)?.is_empty() {
        return Err(invalid());
    }
    available(app).await?;
    let rows = sqlx::query(&format!("SELECT {FIELDS} FROM portal.class_builds WHERE profile_id=$1 ORDER BY updated_at DESC,id LIMIT 50"))
        .bind(&session.profile_id).fetch_all(&app.pool).await?;
    Ok(json!({"builds":rows.iter().map(dto).collect::<Result<Vec<_>>>()?}))
}

pub async fn get(app: &App, session: &Session, id: &str, query: Option<&str>) -> Result<Value> {
    identifier(id)?;
    if !read::parameters(query)?.is_empty() {
        return Err(invalid());
    }
    available(app).await?;
    let row = sqlx::query(&format!(
        "SELECT {FIELDS} FROM portal.class_builds WHERE id=$1 AND profile_id=$2"
    ))
    .bind(id)
    .bind(&session.profile_id)
    .fetch_optional(&app.pool)
    .await?
    .ok_or_else(forbidden)?;
    Ok(json!({"build":dto(&row)?}))
}

pub async fn mutate(
    app: &App,
    session: &Session,
    method: &str,
    id: Option<&str>,
    query: Option<&str>,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Value> {
    available(app).await?;
    if body.len() > 8192 {
        return Err(Error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large"));
    }
    let params = read::parameters(query)?;
    let (document, expected_version) = match (method, id) {
        ("POST", None) if params.is_empty() => {
            let request: Create = parse(body)?;
            validate(&request.document)?;
            (Some(request.document), None)
        }
        ("PATCH", Some(id)) if params.is_empty() => {
            identifier(id)?;
            let request: Update = parse(body)?;
            validate(&request.document)?;
            (
                Some(request.document),
                Some(version(&request.expected_version)?),
            )
        }
        ("DELETE", Some(id)) if params.len() == 1 && body.is_empty() => {
            identifier(id)?;
            let expected = params.get("version").ok_or_else(invalid)?;
            (None, Some(version(expected)?))
        }
        _ => return Err(invalid()),
    };
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(invalid)?;
    if !(16..=80).contains(&key.len())
        || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(invalid());
    }
    let path = id.map_or_else(
        || "me/class-builds".to_owned(),
        |id| format!("me/class-builds/{id}"),
    );
    let body_hash = crypto::hash(
        &serde_json::to_vec(&(
            &session.profile_id,
            method,
            &path,
            &document,
            expected_version,
        ))
        .map_err(|_| invalid())?,
    );
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,721946102))")
        .bind(format!("{}:{key}", session.session_id))
        .execute(&mut *tx)
        .await?;
    // Serialize the absent-row/quota case across all sessions belonging to this owner.
    sqlx::query("SELECT profile_id FROM career_profiles WHERE profile_id=$1 FOR UPDATE")
        .bind(&session.profile_id)
        .fetch_one(&mut *tx)
        .await?;
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal.web_sessions WHERE session_id=$1 AND profile_id=$2 AND authorization_version=2 AND NOT revoked AND expires_at>$3 AND last_seen>$3-86400)")
        .bind(&session.session_id).bind(&session.profile_id).bind(now()).fetch_one(&mut *tx).await?;
    if !active {
        return Err(Error(StatusCode::UNAUTHORIZED, "session_expired"));
    }
    if let Some(row) = sqlx::query("SELECT body_hash,response FROM portal.operation_receipts WHERE session_id=$1 AND operation_key=$2")
        .bind(&session.session_id).bind(key).fetch_optional(&mut *tx).await? {
        if row_text(&row,"body_hash")? != body_hash { return Err(Error(StatusCode::CONFLICT,"idempotency_conflict")); }
        return Ok(row.try_get::<DbJson<Value>,_>("response")?.0);
    }
    let response = if method == "POST" {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM portal.class_builds WHERE profile_id=$1")
                .bind(&session.profile_id)
                .fetch_one(&mut *tx)
                .await?;
        if count >= MAX_BUILDS {
            return Err(Error(StatusCode::CONFLICT, "class_build_limit"));
        }
        let id = crypto::random::<32>();
        let row = sqlx::query(&format!("INSERT INTO portal.class_builds(id,profile_id,document) VALUES($1,$2,$3) RETURNING {FIELDS}"))
            .bind(id).bind(&session.profile_id).bind(DbJson(document.as_ref().unwrap())).fetch_one(&mut *tx).await?;
        json!({"build":dto(&row)?})
    } else {
        let id = id.unwrap();
        let stored: i64 = sqlx::query_scalar(
            "SELECT version FROM portal.class_builds WHERE id=$1 AND profile_id=$2 FOR UPDATE",
        )
        .bind(id)
        .bind(&session.profile_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(forbidden)?;
        if Some(stored) != expected_version {
            return Err(conflict());
        }
        if method == "PATCH" {
            let row = sqlx::query(&format!("UPDATE portal.class_builds SET document=$3,version=version+1,updated_at=clock_timestamp() WHERE id=$1 AND profile_id=$2 RETURNING {FIELDS}"))
                .bind(id).bind(&session.profile_id).bind(DbJson(document.as_ref().unwrap())).fetch_one(&mut *tx).await?;
            json!({"build":dto(&row)?})
        } else {
            sqlx::query("DELETE FROM portal.class_builds WHERE id=$1 AND profile_id=$2")
                .bind(id)
                .bind(&session.profile_id)
                .execute(&mut *tx)
                .await?;
            json!({"status":"deleted","id":id})
        }
    };
    sqlx::query("INSERT INTO portal.operation_receipts(session_id,operation_key,body_hash,response,expires_at) VALUES($1,$2,$3,$4,$5)")
        .bind(&session.session_id).bind(key).bind(body_hash).bind(DbJson(&response)).bind(now()+8*86400).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO portal.audit_events(profile_id,event) VALUES($1,$2)")
        .bind(&session.profile_id)
        .bind(format!("class_build_{}", method.to_ascii_lowercase()))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(response)
}
