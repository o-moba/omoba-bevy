use super::*;
use omoba_career_store::career_store::ready::Member;
fn seats() -> Vec<Seat> {
    (1..=2)
        .map(|i| Seat {
            profile_id: format!("{i:064x}"),
            session_id: format!("session-{i}"),
        })
        .collect()
}
fn snapshot(phase: Phase) -> Snapshot {
    Snapshot {
        check_id: "a".repeat(32),
        preference: MatchPreference::Casual,
        phase,
        deadline_ms: 16_000,
        opened_at_ms: Some(1000),
        members: seats()
            .into_iter()
            .map(|seat| Member {
                seat,
                accepted: phase == Phase::Approved,
                dodged: false,
                event_id: "b".repeat(32),
                observed_at_ms: None,
                cooldown: None,
            })
            .collect(),
    }
}
fn committed(c: &mut Consent, phase: Phase, now: Instant) {
    let work = c.next(now).expect("pending operation");
    c.complete(
        Completion {
            work,
            result: Ok(snapshot(phase)),
        },
        now,
    );
}
fn opened(now: Instant) -> Consent {
    let mut c = Consent::new("a".repeat(32), MatchPreference::Casual, seats(), now);
    committed(&mut c, Phase::Forming, now);
    committed(&mut c, Phase::Open, now);
    c
}
#[test]
fn consent_is_hidden_until_both_reservation_and_publication_commit() {
    let now = Instant::now();
    let mut c = Consent::new("a".repeat(32), MatchPreference::Casual, seats(), now);
    assert!(!c.published() && !c.approved());
    let work = c.next(now).unwrap();
    assert!(matches!(work.operation, Operation::Reserve { .. }));
    assert_eq!(
        c.next(now).unwrap(),
        work,
        "channel retry is not an acknowledgement"
    );
    committed(&mut c, Phase::Forming, now);
    assert!(!c.published());
    assert!(matches!(
        c.next(now).unwrap().operation,
        Operation::Update(Action::Publish)
    ));
    committed(&mut c, Phase::Open, now);
    assert!(c.published() && !c.approved());
    assert_eq!(c.deadline_ms(), Some(16_000));
}
#[test]
fn in_flight_accept_followed_by_decline_cannot_launch_an_approved_snapshot() {
    let now = Instant::now();
    let mut c = opened(now);
    let s = seats();
    c.answer(&s[0].profile_id, &s[0].session_id, true);
    let work = c.next(now).unwrap();
    c.answer(&s[0].profile_id, &s[0].session_id, false);
    c.complete(
        Completion {
            work,
            result: Ok(snapshot(Phase::Approved)),
        },
        now,
    );
    assert!(!c.approved());
    assert!(matches!(
        c.next(now).unwrap().operation,
        Operation::Update(Action::Answers(ref answers)) if answers.len()==1&&!answers[0].accepted
    ));
    committed(&mut c, Phase::Cancelled, now);
    assert!(c.terminal() && !c.approved());
}
#[test]
fn answers_coalesce_per_bound_seat_without_counting_unsaved_clicks() {
    let now = Instant::now();
    let mut c = opened(now);
    let s = seats();
    for _ in 0..1000 {
        c.answer(&s[0].profile_id, &s[0].session_id, true);
    }
    c.answer(&s[1].profile_id, "another-session", true);
    assert_eq!(c.votes.len(), 1);
    assert!(!c.approved());
    assert!(
        c.snapshot
            .as_ref()
            .unwrap()
            .members
            .iter()
            .all(|m| !m.accepted)
    );
    committed(&mut c, Phase::Open, now);
    assert!(!c.approved());
}
#[test]
fn database_failure_retries_the_same_operation_and_cancel_survives_recovery() {
    let now = Instant::now();
    let mut c = Consent::new("a".repeat(32), MatchPreference::Casual, seats(), now);
    let work = c.next(now).unwrap();
    c.complete(
        Completion {
            work: work.clone(),
            result: Err("Career database unavailable.".into()),
        },
        now,
    );
    c.cancel();
    assert!(c.next(now).is_none());
    assert_eq!(c.next(now + Duration::from_secs(1)).unwrap(), work);
    committed(&mut c, Phase::Forming, now + Duration::from_secs(1));
    assert!(matches!(
        c.next(now + Duration::from_secs(1)).unwrap().operation,
        Operation::Update(Action::Cancel)
    ));
    assert!(!c.published() && !c.approved());
}
#[test]
fn a_full_command_queue_never_looks_like_committed_consent() {
    let now = Instant::now();
    let mut c = opened(now);
    let s = seats();
    c.answer(&s[0].profile_id, &s[0].session_id, true);
    let work = c.next(now).unwrap();
    for _ in 0..100 {
        assert_eq!(c.next(now).unwrap(), work);
        assert!(!c.approved());
    }
}
#[test]
fn foreign_or_stale_completion_cannot_replace_the_current_consent() {
    let now = Instant::now();
    let mut c = Consent::new("a".repeat(32), MatchPreference::Casual, seats(), now);
    let old = c.next(now).unwrap();
    committed(&mut c, Phase::Forming, now);
    let current = c.next(now).unwrap();
    c.complete(
        Completion {
            work: old,
            result: Ok(snapshot(Phase::Approved)),
        },
        now,
    );
    assert!(!c.published() && !c.approved());
    assert_eq!(c.next(now).unwrap(), current);
    committed(&mut c, Phase::Open, now);
    assert!(c.published());
}
#[test]
fn heartbeat_has_priority_over_votes_before_the_owner_lease_can_expire() {
    let now = Instant::now();
    let mut c = opened(now);
    let s = seats();
    c.answer(&s[0].profile_id, &s[0].session_id, true);
    let later = now + Duration::from_millis(600);
    assert!(matches!(
        c.next(later).unwrap().operation,
        Operation::Update(Action::Poll)
    ));
    committed(&mut c, Phase::Open, later);
    assert!(matches!(
        c.next(later).unwrap().operation,
        Operation::Update(Action::Answers(_))
    ));
}
#[test]
fn a_snapshot_with_another_roster_cancels_instead_of_granting_consent() {
    let now = Instant::now();
    let mut c = Consent::new("a".repeat(32), MatchPreference::Casual, seats(), now);
    let work = c.next(now).unwrap();
    let mut bad = snapshot(Phase::Approved);
    bad.members[0].seat.session_id = "wrong".into();
    c.complete(
        Completion {
            work,
            result: Ok(bad),
        },
        now,
    );
    assert!(!c.approved() && !c.published());
}
#[test]
fn a_known_reservation_refusal_does_not_hold_innocent_peers_until_cooldown_expiry() {
    let now = Instant::now();
    let mut c = Consent::new("a".repeat(32), MatchPreference::Casual, seats(), now);
    let work = c.next(now).unwrap();
    c.complete(
        Completion {
            work,
            result: Err(shared::career::error_code::DODGE_COOLDOWN.into()),
        },
        now,
    );
    assert!(c.terminal() && !c.approved());
    assert!(c.next(now + Duration::from_secs(100)).is_none());
}

#[test]
fn all_answers_are_one_bounded_command_even_when_the_tick_is_delayed() {
    let now = Instant::now();
    let mut c = opened(now);
    let s = seats();
    for seat in &s {
        c.answer(&seat.profile_id, &seat.session_id, true);
    }
    let later = now + Duration::from_secs(1);
    assert!(matches!(
        c.next(later).unwrap().operation,
        Operation::Update(Action::Poll)
    ));
    committed(&mut c, Phase::Open, later + Duration::from_secs(1));
    let work = c.next(later + Duration::from_secs(1)).unwrap();
    assert!(
        matches!(&work.operation,Operation::Update(Action::Answers(answers)) if answers.len()==s.len())
    );
    c.complete(
        Completion {
            work,
            result: Ok(snapshot(Phase::Approved)),
        },
        later + Duration::from_secs(2),
    );
    assert!(c.approved());
}
#[test]
fn an_observed_decline_cannot_be_overwritten_by_a_later_accept() {
    let now = Instant::now();
    let mut c = opened(now);
    let s = seats();
    c.answer(&s[0].profile_id, &s[0].session_id, false);
    c.answer(&s[0].profile_id, &s[0].session_id, true);
    assert!(
        matches!(c.next(now).unwrap().operation,Operation::Update(Action::Answers(ref answers)) if answers.len()==1&&!answers[0].accepted)
    );
    assert!(!c.approved());
}
