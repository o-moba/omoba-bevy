//! Bounded, anonymous comparison of client-declared controls in completed Ranked.
use super::*;
use shared::ranked::{InputKind, InputStatistics, InputStatisticsRow};

impl CareerStore {
    /// Samples the newest settled receipts (1..=10_000), then excludes unranked,
    /// unstarted, bot and proved-abandon seats. Missing/future controls stay
    /// unknown. The scan bound is explicit so a small sample is never presented
    /// as a whole-season population or hardware attestation.
    pub async fn input_statistics(&self, receipt_limit: u32) -> Result<InputStatistics, String> {
        if !(1..=10_000).contains(&receipt_limit) {
            return Err("Input statistics receipt limit must be between 1 and 10000.".into());
        }
        retry(|| async {
            let rows = sqlx::query(r#"
                WITH recent AS MATERIALIZED (
                    SELECT result FROM career_matches WHERE status='settled'
                    ORDER BY seq DESC LIMIT $1
                ), eligible AS MATERIALIZED (
                    SELECT result FROM recent
                    WHERE result->>'outcome'='completed'
                      AND result->>'rated'='true'
                      AND COALESCE((result->>'gameplay_started')::boolean,true)
                      AND result->>'winner' IN ('green','blue')
                ), seats AS (
                    SELECT CASE WHEN seat->>'input_kind' IN ('touch','mouse','mixed')
                           THEN seat->>'input_kind' ELSE 'unknown' END AS kind,
                           (seat->>'team'=result->>'winner') AS won
                    FROM eligible CROSS JOIN LATERAL jsonb_array_elements(result->'participants') AS seat
                    WHERE seat->>'profile_id' IS NOT NULL
                      AND NOT COALESCE((seat->>'is_bot')::boolean,false)
                      AND NOT COALESCE((seat->'participation'->>'abandoned')::boolean,false)
                ), counts AS (
                    SELECT kind,count(*) AS games,count(*) FILTER (WHERE won) AS wins
                    FROM seats GROUP BY kind
                )
                SELECT kinds.kind,COALESCE(counts.games,0)::bigint AS games,
                       COALESCE(counts.wins,0)::bigint AS wins,
                       (SELECT count(*) FROM recent) AS scanned,
                       (SELECT count(*) FROM eligible) AS eligible
                FROM unnest(ARRAY['touch','mouse','mixed','unknown']) WITH ORDINALITY AS kinds(kind,n)
                LEFT JOIN counts USING(kind) ORDER BY kinds.n
            "#).bind(i64::from(receipt_limit)).fetch_all(&self.pool).await?;
            let mut report = InputStatistics {
                receipt_limit,
                ..Default::default()
            };
            for row in rows {
                let count = |name| -> StoreResult<u64> {
                    u64::try_from(row.try_get::<i64,_>(name)?)
                        .map_err(|_| "Invalid input statistics count.".into())
                };
                report.scanned_receipts = count("scanned")?;
                report.completed_ranked_matches = count("eligible")?;
                let kind = match row.try_get::<&str,_>("kind")? {
                    "touch" => InputKind::Touch,
                    "mouse" => InputKind::Mouse,
                    "mixed" => InputKind::Mixed,
                    _ => InputKind::Unknown,
                };
                report.rows.push(InputStatisticsRow { kind, matches: count("games")?, wins: count("wins")? });
            }
            Ok(report)
        }).await
    }
}

/// Preserve checkpoint evidence across worker recovery, unknown legacy senders
/// and terminal spool retries without mutating append-only participant rows.
pub(super) fn preserve(previous: &MatchResult, next: &mut MatchResult) {
    for seat in &mut next.participants {
        if let Some(old) = previous
            .participants
            .iter()
            .find(|old| same_person(old, seat))
        {
            seat.input_kind = old.input_kind.observe(seat.input_kind);
        }
    }
}
