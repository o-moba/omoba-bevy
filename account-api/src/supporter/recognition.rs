//! Public recognition is separate from paid access and requires explicit profile consent.
use crate::*;
use serde::Deserialize;
use shared::career::valid_profile_id;

/// One unique, nonrefunded provider period credits one configured monthly purchase;
/// YooKassa donations credit each UTC calendar month with a donation once
/// (`portal.supporter_months`, portal schema 9). This aggregate deliberately includes
/// expired and already-paid future periods and never uses the bounded status/grants
/// response or divides provider dates by 30 days.
pub async fn get(app: &App, profile: &str) -> Result<Value> {
    let row = sqlx::query(
        "SELECT coalesce(p.recognition_visible,false) AS visible,
                coalesce(s.public_profile_enabled,false) AS public_profile_enabled,
                portal.supporter_months(c.profile_id) AS months_supported
         FROM career_profiles c
         LEFT JOIN portal.supporter_preferences p USING(profile_id)
         LEFT JOIN portal.profile_settings s USING(profile_id)
         WHERE c.profile_id=$1",
    )
    .bind(profile)
    .fetch_optional(&app.pool)
    .await?
    .ok_or_else(forbidden)?;
    recognition_dto(&row)
}

fn recognition_dto(row: &sqlx::postgres::PgRow) -> Result<Value> {
    Ok(json!({
        "months_supported": row.try_get::<i64,_>("months_supported")?,
        "visible": row.try_get::<bool,_>("visible")?,
        "public_profile_enabled": row.try_get::<bool,_>("public_profile_enabled")?,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preference {
    visible: bool,
}

/// The authenticated cosmetic session owns this profile; setting a boolean is idempotent.
pub async fn set(app: &App, profile: &str, body: &[u8]) -> Result<Value> {
    let preference: Preference = parse(body)?;
    let row = sqlx::query(
        "WITH updated AS (
            INSERT INTO portal.supporter_preferences(profile_id,aura,recognition_visible)
            VALUES($1,$3,$2)
            ON CONFLICT(profile_id) DO UPDATE
              SET recognition_visible=EXCLUDED.recognition_visible
            RETURNING profile_id,recognition_visible
         )
         SELECT u.recognition_visible AS visible,
                coalesce(s.public_profile_enabled,false) AS public_profile_enabled,
                portal.supporter_months(u.profile_id) AS months_supported
         FROM updated u LEFT JOIN portal.profile_settings s USING(profile_id)",
    )
    .bind(profile)
    .bind(preference.visible)
    // A placeholder until the first grant picks the highest unlocked style.
    .bind(super::PLACEHOLDER_AURA.id())
    .fetch_one(&app.pool)
    .await?;
    recognition_dto(&row)
}

#[derive(Debug, PartialEq, Eq)]
struct Page {
    limit: usize,
    cursor: Option<String>,
}
impl Page {
    fn parse(raw: Option<&str>) -> Result<Self> {
        let mut query = read::parameters(raw)?;
        if query
            .keys()
            .any(|key| !matches!(key.as_str(), "limit" | "cursor"))
        {
            return Err(invalid());
        }
        let limit = match query.remove("limit") {
            Some(value) => {
                // Reject signs, whitespace and noncanonical decimal spellings.
                let parsed = value.parse::<usize>().map_err(|_| invalid())?;
                if !(1..=50).contains(&parsed) || value != parsed.to_string() {
                    return Err(invalid());
                }
                parsed
            }
            None => 24,
        };
        let cursor = query.remove("cursor");
        if cursor.as_deref().is_some_and(|id| !valid_profile_id(id)) {
            return Err(invalid());
        }
        Ok(Self { limit, cursor })
    }
}

pub async fn directory(app: &App, raw: Option<&str>) -> Result<Value> {
    let page = Page::parse(raw)?;
    let current = now();
    // Consent, nickname, credit and active state share one MVCC snapshot. The
    // immutable profile ID gives stable keyset pagination without a spending rank.
    let rows = sqlx::query(
        "SELECT p.profile_id,p.nickname,credit.months_supported,credit.hero_until
         FROM portal.supporter_preferences pref
         JOIN portal.profile_settings settings USING(profile_id)
         JOIN career_profiles p USING(profile_id)
         CROSS JOIN LATERAL (
             SELECT portal.supporter_months(p.profile_id) AS months_supported,
                    max(valid_until) FILTER (WHERE valid_from<=$1 AND valid_until>$1) AS hero_until
             FROM portal.supporter_grants g
             WHERE g.profile_id=p.profile_id AND g.revoked_at IS NULL
         ) credit
         WHERE pref.recognition_visible AND settings.public_profile_enabled
           AND credit.months_supported>0 AND ($2::text IS NULL OR p.profile_id>$2)
         ORDER BY p.profile_id LIMIT $3",
    )
    .bind(current)
    .bind(&page.cursor)
    .bind((page.limit + 1) as i64)
    .fetch_all(&app.pool)
    .await?;
    let more = rows.len() > page.limit;
    let mut supporters = Vec::with_capacity(rows.len().min(page.limit));
    for row in rows.into_iter().take(page.limit) {
        let until: Option<i64> = row.try_get("hero_until")?;
        supporters.push(json!({
            "profile_id": row_text(&row,"profile_id")?,
            "nickname": row_text(&row,"nickname")?,
            "months_supported": row.try_get::<i64,_>("months_supported")?,
            "hero_active": until.is_some(),
            "hero_until": until,
        }));
    }
    let next_cursor = more.then(|| supporters.last().expect("nonempty page")["profile_id"].clone());
    Ok(
        json!({"supporters":supporters,"next_cursor":next_cursor,"data_as_of":timestamp(&app.pool,current).await?}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supporter_directory_accepts_only_bounded_canonical_pagination() {
        assert_eq!(
            Page::parse(None).unwrap(),
            Page {
                limit: 24,
                cursor: None
            }
        );
        let id = "a".repeat(64);
        assert_eq!(
            Page::parse(Some(&format!("limit=50&cursor={id}"))).unwrap(),
            Page {
                limit: 50,
                cursor: Some(id)
            }
        );
        for query in [
            "limit=0",
            "limit=51",
            "limit=-1",
            "limit=%2B1",
            "limit=01",
            "limit=1.0",
            "limit=",
            "limit=999999999999999999999999",
            "limit=1&limit=2",
            "cursor=",
            "cursor=abc",
            "cursor=%ff",
            "cursor=%",
            "sort=months",
            "profile_id=a",
            "limit",
            "limit=%201",
        ] {
            assert!(Page::parse(Some(query)).is_err(), "accepted {query}");
        }
        assert!(Page::parse(Some(&format!("cursor={}", "A".repeat(64)))).is_err());
    }

    #[test]
    fn recognition_toggle_requires_exact_boolean_only() {
        assert!(parse::<Preference>(br#"{"visible":true}"#).unwrap().visible);
        assert!(
            !parse::<Preference>(br#"{"visible":false}"#)
                .unwrap()
                .visible
        );
        for body in [
            r#"{}"#,
            r#"{"visible":null}"#,
            r#"{"visible":"true"}"#,
            r#"{"visible":true,"months_supported":20}"#,
            r#"{"visible":true,"profile_id":"someone-else"}"#,
            r#"{"visible":true,"visible":false}"#,
        ] {
            assert!(
                parse::<Preference>(body.as_bytes()).is_err(),
                "accepted {body}"
            );
        }
    }
}
