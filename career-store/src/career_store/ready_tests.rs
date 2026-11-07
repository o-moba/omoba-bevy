use super::*;
use crate::career_store::ready::{Action, CLOSED, OWNED_ELSEWHERE, Phase, Seat};
use shared::match_service::MatchPreference;

fn seats(a: &ProfileSummary, b: &ProfileSummary) -> Vec<Seat> {
    vec![
        Seat {
            profile_id: a.profile_id.clone(),
            session_id: "seat-a".into(),
        },
        Seat {
            profile_id: b.profile_id.clone(),
            session_id: "seat-b".into(),
        },
    ]
}
fn answer(seat: &Seat, accepted: bool) -> Action {
    Action::Answer {
        profile_id: seat.profile_id.clone(),
        session_id: seat.session_id.clone(),
        accepted,
    }
}
async fn fixture() -> (Fixture, ProfileSummary, ProfileSummary, Vec<Seat>, String) {
    let f = Fixture::new().await;
    let a = f.profile(1).await;
    let b = f.profile(2).await;
    let roster = seats(&a, &b);
    let id = "a".repeat(32);
    (f, a, b, roster, id)
}
async fn expire(f: &Fixture, id: &str, healthy: bool) -> i64 {
    let at = queue_discipline_now(f).await as i64 - 1;
    sqlx::query("UPDATE career_ready_checks SET deadline_ms=$2,live_until_ms=$3 WHERE check_id=$1")
        .bind(id)
        .bind(at)
        .bind(if healthy { at + 3_000 } else { at - 1 })
        .execute(&f.store.pool)
        .await
        .unwrap();
    at
}
async fn event_count(f: &Fixture) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM career_queue_dodge_events")
        .fetch_one(&f.store.pool)
        .await
        .unwrap()
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_reserve_is_immutable_idempotent_and_excludes_other_hosts() {
    let (f, a, _, roster, id) = fixture().await;
    let other = f.another_owner().await;
    let one = f
        .store
        .ready_reserve(&id, MatchPreference::Casual, roster.clone())
        .await
        .unwrap();
    assert_eq!(one.phase, Phase::Forming);
    let mut reversed = roster.clone();
    reversed.reverse();
    assert_eq!(
        f.store
            .ready_reserve(&id, MatchPreference::Casual, reversed)
            .await
            .unwrap(),
        one
    );
    assert_eq!(
        other.ready_queue_access(&a.profile_id).await.unwrap_err(),
        OWNED_ELSEWHERE
    );
    assert!(
        other
            .access(&format!("{:064x}", 1), &a.profile_id)
            .await
            .unwrap()
            .key_active
    );
    assert!(other.ready_update(&id, Action::Publish).await.is_err());
    assert!(
        other
            .ready_reserve(&"b".repeat(32), MatchPreference::Casual, roster.clone())
            .await
            .is_err()
    );
    let mut conflict = roster;
    conflict[0].session_id = "another-session".into();
    assert!(
        f.store
            .ready_reserve(&id, MatchPreference::Casual, conflict)
            .await
            .is_err()
    );
    assert_eq!(event_count(&f).await, 0);
    f.close().await;
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_concurrent_overlapping_rosters_have_exactly_one_owner() {
    let (f, _, _, roster, id) = fixture().await;
    let other = f.another_owner().await;
    let second = "b".repeat(32);
    let (a, b) = tokio::join!(
        f.store
            .ready_reserve(&id, MatchPreference::Casual, roster.clone()),
        other.ready_reserve(&second, MatchPreference::Casual, roster)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM career_ready_checks")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(event_count(&f).await, 0);
    f.close().await;
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_publish_has_fifteen_seconds_and_only_unanimity_can_admit() {
    let (f, a, b, roster, id) = fixture().await;
    f.store
        .ready_reserve(&id, MatchPreference::Casual, roster.clone())
        .await
        .unwrap();
    let opened = f.store.ready_update(&id, Action::Publish).await.unwrap();
    assert_eq!(
        opened.deadline_ms - opened.opened_at_ms.unwrap(),
        15_000,
        "the database's published window is exactly15 seconds, independent of test scheduling"
    );
    assert_eq!(
        f.store
            .ready_update(&id, Action::Publish)
            .await
            .unwrap()
            .deadline_ms,
        opened.deadline_ms
    );
    let mut initial = starting(&result(&a, &b, 1001));
    initial.result_id = id.clone();
    initial.rated = false;
    initial.unrated_reason = Some("casual fixture".into());
    assert_eq!(f.store.start(initial.clone()).await.unwrap_err(), CLOSED);
    let one = f
        .store
        .ready_update(&id, answer(&roster[0], true))
        .await
        .unwrap();
    assert_eq!(one.phase, Phase::Open);
    assert_eq!(f.store.start(initial.clone()).await.unwrap_err(), CLOSED);
    let all = f
        .store
        .ready_update(&id, answer(&roster[1], true))
        .await
        .unwrap();
    assert_eq!(all.phase, Phase::Approved);
    f.store.start(initial.clone()).await.unwrap();
    f.store.start(initial).await.unwrap();
    assert_eq!(
        f.store.ready_update(&id, Action::Poll).await.unwrap().phase,
        Phase::Consumed
    );
    let claims: i64 = sqlx::query_scalar("SELECT count(*) FROM career_ready_claims")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(claims, 0);
    assert_eq!(event_count(&f).await, 0);
    f.close().await;
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_decline_commits_one_penalty_with_claim_release() {
    let (f, a, b, roster, id) = fixture().await;
    let other = f.another_owner().await;
    f.store
        .ready_reserve(&id, MatchPreference::Casual, roster.clone())
        .await
        .unwrap();
    f.store.ready_update(&id, Action::Publish).await.unwrap();
    let declined = f
        .store
        .ready_update(&id, answer(&roster[0], false))
        .await
        .unwrap();
    assert_eq!(declined.phase, Phase::Cancelled);
    assert_eq!(declined.members.iter().filter(|m| m.dodged).count(), 1);
    assert_eq!(
        f.store
            .ready_update(&id, answer(&roster[0], false))
            .await
            .unwrap(),
        declined
    );
    assert_eq!(event_count(&f).await, 1);
    assert_eq!(
        other
            .queue_access(&format!("{:064x}", 1), &a.profile_id)
            .await
            .unwrap_err(),
        shared::career::error_code::DODGE_COOLDOWN
    );
    assert!(
        other
            .queue_access(&format!("{:064x}", 2), &b.profile_id)
            .await
            .unwrap()
            .key_active
    );
    assert_eq!(f.store.profile(&a.profile_id).await.unwrap(), a);
    assert!(
        f.store
            .history(&a.profile_id, None)
            .await
            .unwrap()
            .0
            .is_empty()
    );
    f.close().await;
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_foreign_search_resolves_timeout_without_an_outbox_gap() {
    let (f, a, b, roster, id) = fixture().await;
    let other = f.another_owner().await;
    f.store
        .ready_reserve(&id, MatchPreference::Casual, roster.clone())
        .await
        .unwrap();
    f.store.ready_update(&id, Action::Publish).await.unwrap();
    f.store
        .ready_update(&id, answer(&roster[0], true))
        .await
        .unwrap();
    let at = expire(&f, &id, true).await;
    // No original-owner timeout delivery: the next host itself resolves the
    // durable answer set and commits discipline before answering new search.
    assert_eq!(
        other
            .queue_access(&format!("{:064x}", 2), &b.profile_id)
            .await
            .unwrap_err(),
        shared::career::error_code::DODGE_COOLDOWN
    );
    let closed = f
        .store
        .ready_update(&id, answer(&roster[1], true))
        .await
        .unwrap();
    assert_eq!(closed.phase, Phase::Expired);
    let missed = closed
        .members
        .iter()
        .find(|m| m.seat.profile_id == b.profile_id)
        .unwrap();
    assert!(!missed.accepted && missed.dodged);
    assert_eq!(missed.observed_at_ms, Some(at));
    assert_eq!(event_count(&f).await, 1);
    assert!(
        other
            .queue_access(&format!("{:064x}", 1), &a.profile_id)
            .await
            .unwrap()
            .key_active
    );
    assert!(
        other
            .queue_access(&format!("{:064x}", 2), &b.profile_id)
            .await
            .is_err()
    );
    assert_eq!(event_count(&f).await, 1);
    f.close().await;
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_failed_or_unpublished_owner_releases_without_blaming_players() {
    let (f, a, _, roster, id) = fixture().await;
    let other = f.another_owner().await;
    for (check, published) in [(&id, true), (&"b".repeat(32), false)] {
        f.store
            .ready_reserve(check, MatchPreference::Casual, roster.clone())
            .await
            .unwrap();
        if published {
            f.store.ready_update(check, Action::Publish).await.unwrap();
        }
        expire(&f, check, false).await;
        other.ready_queue_access(&a.profile_id).await.unwrap();
        assert_eq!(
            f.store
                .ready_update(check, Action::Poll)
                .await
                .unwrap()
                .phase,
            Phase::Cancelled
        );
    }
    assert_eq!(event_count(&f).await, 0);
    assert!(
        f.store
            .queue_dodge_cooldown(&a.profile_id)
            .await
            .unwrap()
            .is_none()
    );
    f.close().await;
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_cancel_and_old_answers_cannot_mutate_a_new_roster() {
    let (f, a, _, roster, id) = fixture().await;
    let other = f.another_owner().await;
    f.store
        .ready_reserve(&id, MatchPreference::Casual, roster.clone())
        .await
        .unwrap();
    f.store.ready_update(&id, Action::Publish).await.unwrap();
    f.store
        .ready_update(&id, answer(&roster[0], true))
        .await
        .unwrap();
    assert_eq!(
        f.store
            .ready_update(&id, Action::Cancel)
            .await
            .unwrap()
            .phase,
        Phase::Cancelled
    );
    let second = "b".repeat(32);
    let new = other
        .ready_reserve(&second, MatchPreference::Casual, roster.clone())
        .await
        .unwrap();
    assert_eq!(
        f.store
            .ready_update(&id, answer(&roster[1], false))
            .await
            .unwrap()
            .phase,
        Phase::Cancelled
    );
    assert_eq!(
        other
            .ready_reserve(&second, MatchPreference::Casual, roster)
            .await
            .unwrap(),
        new
    );
    assert_eq!(
        f.store.ready_queue_access(&a.profile_id).await.unwrap_err(),
        OWNED_ELSEWHERE
    );
    assert_eq!(event_count(&f).await, 0);
    f.close().await;
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_last_answer_and_start_decline_race_never_admits_a_rejected_roster() {
    let (f, a, b, roster, id) = fixture().await;
    f.store
        .ready_reserve(&id, MatchPreference::Casual, roster.clone())
        .await
        .unwrap();
    f.store.ready_update(&id, Action::Publish).await.unwrap();
    f.store
        .ready_update(&id, answer(&roster[0], true))
        .await
        .unwrap();
    let (yes, no) = tokio::join!(
        f.store.ready_update(&id, answer(&roster[1], true)),
        f.store.ready_update(&id, answer(&roster[1], false))
    );
    yes.unwrap();
    no.unwrap();
    assert_eq!(
        f.store.ready_update(&id, Action::Poll).await.unwrap().phase,
        Phase::Cancelled
    );
    let mut initial = starting(&result(&a, &b, 1002));
    initial.result_id = id;
    initial.rated = false;
    initial.unrated_reason = Some("casual fixture".into());
    assert_eq!(
        f.store.start(initial).await.unwrap_err(),
        shared::career::error_code::DODGE_COOLDOWN
    );
    assert_eq!(event_count(&f).await, 1);
    f.close().await;
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_invalid_answers_and_schema_upgrade_preserve_discipline() {
    let (f, a, _, roster, id) = fixture().await;
    let at = queue_discipline_now(&f).await;
    f.store
        .record_queue_dodge(&"f".repeat(32), &a.profile_id, at)
        .await
        .unwrap();
    sqlx::raw_sql("DROP TABLE career_ready_claims,career_ready_members,career_ready_checks; DELETE FROM career_schema_version WHERE version=12;")
        .execute(&f.store.pool).await.unwrap();
    assert!(
        CareerStore::runtime_from_pool(f.store.pool.clone())
            .await
            .is_err()
    );
    let upgraded = f.another_owner().await;
    assert!(
        upgraded
            .queue_dodge_cooldown(&a.profile_id)
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query(
        "UPDATE career_queue_penalties SET cooldown_until=clock_timestamp()-interval '1 second'",
    )
    .execute(&f.store.pool)
    .await
    .unwrap();
    assert!(
        upgraded
            .ready_reserve(&id, MatchPreference::Ranked, roster.clone())
            .await
            .is_err()
    );
    assert!(
        upgraded
            .ready_reserve("../escape", MatchPreference::Casual, roster.clone())
            .await
            .is_err()
    );
    let mut duplicate = roster.clone();
    duplicate[1] = duplicate[0].clone();
    assert!(
        upgraded
            .ready_reserve(&id, MatchPreference::Casual, duplicate)
            .await
            .is_err()
    );
    upgraded
        .ready_reserve(&id, MatchPreference::Casual, roster.clone())
        .await
        .unwrap();
    upgraded.ready_update(&id, Action::Publish).await.unwrap();
    let invalid = Action::Answer {
        profile_id: roster[0].profile_id.clone(),
        session_id: "wrong-session".into(),
        accepted: true,
    };
    expire(&f, &id, true).await;
    assert!(upgraded.ready_update(&id, invalid).await.is_err());
    assert_eq!(
        event_count(&f).await,
        1,
        "invalid seat causes no timeout side effects"
    );
    upgraded.ready_update(&id, Action::Cancel).await.unwrap();
    assert_eq!(event_count(&f).await, 1);
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_answer_batches_are_validated_atomically_and_preserve_decline_order() {
    use crate::career_store::ready::Answer;
    let (f, _, _, roster, id) = fixture().await;
    f.store
        .ready_reserve(&id, MatchPreference::Casual, roster.clone())
        .await
        .unwrap();
    f.store.ready_update(&id, Action::Publish).await.unwrap();
    let good = Answer {
        seat: roster[0].clone(),
        accepted: true,
    };
    let mut wrong = roster[1].clone();
    wrong.session_id = "wrong".into();
    assert!(
        f.store
            .ready_update(
                &id,
                Action::Answers(vec![
                    good.clone(),
                    Answer {
                        seat: wrong,
                        accepted: true
                    }
                ])
            )
            .await
            .is_err()
    );
    let unchanged = f.store.ready_update(&id, Action::Poll).await.unwrap();
    assert!(unchanged.members.iter().all(|m| !m.accepted));
    assert!(
        f.store
            .ready_update(&id, Action::Answers(vec![good.clone(), good.clone()]))
            .await
            .is_err()
    );
    let approved = f
        .store
        .ready_update(
            &id,
            Action::Answers(vec![
                good,
                Answer {
                    seat: roster[1].clone(),
                    accepted: true,
                },
            ]),
        )
        .await
        .unwrap();
    assert_eq!(approved.phase, Phase::Approved);
    let closed = f
        .store
        .ready_update(
            &id,
            Action::Answers(vec![
                Answer {
                    seat: roster[0].clone(),
                    accepted: false,
                },
                Answer {
                    seat: roster[1].clone(),
                    accepted: false,
                },
            ]),
        )
        .await
        .unwrap();
    assert_eq!(closed.phase, Phase::Cancelled);
    assert_eq!(closed.members.iter().filter(|m| m.dodged).count(), 1);
    assert!(
        closed
            .members
            .iter()
            .find(|m| m.seat == roster[0])
            .unwrap()
            .dodged
    );
    assert_eq!(event_count(&f).await, 1);
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_public_start_and_unknown_recovery_require_a_receipt() {
    let (f, a, b, _, id) = fixture().await;
    let mut initial = starting(&result(&a, &b, 2001));
    initial.result_id = id;
    initial.gameplay_started = false;
    initial.rated = false;
    initial.unrated_reason = Some("consent fixture".into());
    assert_eq!(
        f.store.start_public(initial.clone()).await.unwrap_err(),
        CLOSED
    );
    assert_eq!(
        f.store
            .ensure_public_allocation(initial.clone())
            .await
            .unwrap_err(),
        CLOSED
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM career_matches")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(f.store.profile(&a.profile_id).await.unwrap(), a);
    assert_eq!(f.store.profile(&b.profile_id).await.unwrap(), b);
    assert!(
        f.store
            .history(&a.profile_id, None)
            .await
            .unwrap()
            .0
            .is_empty()
    );
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_public_recovery_of_known_allocation_needs_no_second_vote() {
    let (f, a, b, roster, id) = fixture().await;
    f.store
        .ready_reserve(&id, MatchPreference::Casual, roster.clone())
        .await
        .unwrap();
    f.store.ready_update(&id, Action::Publish).await.unwrap();
    let mut initial = starting(&result(&a, &b, 2002));
    initial.result_id = id.clone();
    initial.gameplay_started = false;
    initial.rated = false;
    initial.unrated_reason = Some("consent fixture".into());
    assert_eq!(
        f.store
            .ensure_public_allocation(initial.clone())
            .await
            .unwrap_err(),
        CLOSED
    );
    f.store
        .ready_update(&id, answer(&roster[0], true))
        .await
        .unwrap();
    assert_eq!(
        f.store.start_public(initial.clone()).await.unwrap_err(),
        CLOSED
    );
    f.store
        .ready_update(&id, answer(&roster[1], true))
        .await
        .unwrap();
    f.store
        .ensure_public_allocation(initial.clone())
        .await
        .unwrap();
    f.store.start_public(initial.clone()).await.unwrap();
    // Disposable schema simulates future removal of a consumed ready receipt.
    // The committed immutable career allocation is the authority for Resume.
    sqlx::query("DELETE FROM career_ready_checks WHERE check_id=$1")
        .bind(&id)
        .execute(&f.store.pool)
        .await
        .unwrap();
    let at = queue_discipline_now(&f).await;
    f.store
        .record_queue_dodge(&"c".repeat(32), &a.profile_id, at)
        .await
        .unwrap();
    f.expire(&id).await;
    let other = f.another_owner().await;
    other
        .ensure_public_allocation(initial.clone())
        .await
        .unwrap();
    other.start_public(initial).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM career_matches")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(f.store.profile(&a.profile_id).await.unwrap(), a);
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_deleted_closed_receipt_never_becomes_public_standalone_admission() {
    let (f, a, b, roster, id) = fixture().await;
    f.store
        .ready_reserve(&id, MatchPreference::Casual, roster)
        .await
        .unwrap();
    f.store.ready_update(&id, Action::Cancel).await.unwrap();
    let mut initial = starting(&result(&a, &b, 2003));
    initial.result_id = id.clone();
    initial.gameplay_started = false;
    initial.rated = false;
    initial.unrated_reason = Some("consent fixture".into());
    for deleted in [false, true] {
        if deleted {
            sqlx::query("DELETE FROM career_ready_checks WHERE check_id=$1")
                .bind(&id)
                .execute(&f.store.pool)
                .await
                .unwrap();
        }
        assert_eq!(
            f.store.start_public(initial.clone()).await.unwrap_err(),
            CLOSED
        );
        assert_eq!(
            f.store
                .ensure_public_allocation(initial.clone())
                .await
                .unwrap_err(),
            CLOSED
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM career_matches")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(event_count(&f).await, 0);
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_unknown_public_recovery_rechecks_new_discipline_under_locks() {
    let (f, a, b, roster, id) = fixture().await;
    f.store
        .ready_reserve(&id, MatchPreference::Casual, roster.clone())
        .await
        .unwrap();
    f.store.ready_update(&id, Action::Publish).await.unwrap();
    for seat in &roster {
        f.store.ready_update(&id, answer(seat, true)).await.unwrap();
    }
    let at = queue_discipline_now(&f).await;
    f.store
        .record_queue_dodge(&"c".repeat(32), &a.profile_id, at)
        .await
        .unwrap();
    let mut initial = starting(&result(&a, &b, 2004));
    initial.result_id = id.clone();
    initial.gameplay_started = false;
    initial.rated = false;
    initial.unrated_reason = Some("consent fixture".into());
    assert_eq!(
        f.store.ensure_public_allocation(initial).await.unwrap_err(),
        shared::career::error_code::DODGE_COOLDOWN
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM career_matches")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        f.store.ready_update(&id, Action::Poll).await.unwrap().phase,
        Phase::Approved
    );
    f.close().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires real isolated PostgreSQL"]
async fn ready_ownership_public_ranked_requires_ranked_consent_full_roster_and_current_eligibility()
{
    let f = Fixture::new().await;
    let mut profiles = Vec::new();
    for n in 1..=10 {
        profiles.push(f.profile(n).await);
    }
    let roster: Vec<_> = profiles
        .iter()
        .enumerate()
        .map(|(i, p)| Seat {
            profile_id: p.profile_id.clone(),
            session_id: format!("ranked-seat-{i}"),
        })
        .collect();
    let casual = "c".repeat(32);
    f.store
        .ready_reserve(&casual, MatchPreference::Casual, roster[..2].to_vec())
        .await
        .unwrap();
    f.store
        .ready_update(&casual, Action::Publish)
        .await
        .unwrap();
    f.store
        .ready_update(
            &casual,
            Action::Answers(
                roster[..2]
                    .iter()
                    .map(|seat| crate::career_store::ready::Answer {
                        seat: seat.clone(),
                        accepted: true,
                    })
                    .collect(),
            ),
        )
        .await
        .unwrap();
    let mut initial = starting(&result(&profiles[0], &profiles[1], 2005));
    initial.result_id = casual.clone();
    initial.gameplay_started = false;
    assert_eq!(
        f.store.start_public(initial.clone()).await.unwrap_err(),
        CLOSED,
        "Casual consent cannot authorize a rated start"
    );
    f.store.ready_update(&casual, Action::Cancel).await.unwrap();
    let ranked = "d".repeat(32);
    f.store
        .ready_reserve(&ranked, MatchPreference::Ranked, roster.clone())
        .await
        .unwrap();
    f.store
        .ready_update(&ranked, Action::Publish)
        .await
        .unwrap();
    f.store
        .ready_update(
            &ranked,
            Action::Answers(
                roster
                    .iter()
                    .map(|seat| crate::career_store::ready::Answer {
                        seat: seat.clone(),
                        accepted: true,
                    })
                    .collect(),
            ),
        )
        .await
        .unwrap();
    initial.result_id = ranked.clone();
    assert_eq!(
        f.store.start_public(initial.clone()).await.unwrap_err(),
        CLOSED,
        "Ranked consent cannot authorize a truncated roster"
    );
    initial.participants = profiles
        .iter()
        .enumerate()
        .map(|(i, p)| {
            participant(
                p,
                (i + 1) as u64,
                if i < 5 { Team::Green } else { Team::Blue },
            )
        })
        .collect();
    sqlx::query("UPDATE career_profiles SET completed_public_matches=9 WHERE profile_id=$1")
        .bind(&profiles[0].profile_id)
        .execute(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(
        f.store
            .ensure_public_allocation(initial.clone())
            .await
            .unwrap_err(),
        shared::career::error_code::RANKED_LOCKED,
        "unknown recovery is a fresh admission, not a bypass"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM career_matches")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        f.store
            .ready_update(&ranked, Action::Poll)
            .await
            .unwrap()
            .phase,
        Phase::Approved
    );
    sqlx::query("UPDATE career_profiles SET completed_public_matches=10 WHERE profile_id=$1")
        .bind(&profiles[0].profile_id)
        .execute(&f.store.pool)
        .await
        .unwrap();
    f.store.ensure_public_allocation(initial).await.unwrap();
    assert_eq!(
        f.store
            .ready_update(&ranked, Action::Poll)
            .await
            .unwrap()
            .phase,
        Phase::Consumed
    );
    f.close().await;
}
