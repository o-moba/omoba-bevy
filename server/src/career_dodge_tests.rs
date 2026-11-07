use super::*;

#[test]
fn dodge_submission_waits_for_sql_ack_and_retries_do_not_advance_the_barrier() {
    let mut backend = MemoryCareer::test_backend(9);
    let event = queue_dodges::Event::new(&"a".repeat(64), 1).unwrap();
    assert!(backend.record_queue_dodge(event.clone()));
    let generation = backend.record_generation;
    assert!(backend.record_queue_dodge(event.clone()));
    assert_eq!(backend.record_generation, generation);
    assert_eq!(backend.link.jobs.len(), 1);
    assert!(backend.take_queue_dodge_committed().is_empty());
    let mut conflict = event.clone();
    conflict.observed_at_ms += 1;
    assert!(!backend.record_queue_dodge(conflict));
    backend
        .link
        .replies
        .push_back(Reply::QueueDodgeCommitted(event.clone()));
    backend.poll();
    assert_eq!(backend.take_queue_dodge_committed(), vec![event.event_id]);
    assert!(backend.take_queue_dodge_committed().is_empty());
}
#[test]
fn a_dodge_invalidates_an_earlier_shutdown_barrier() {
    let mut backend = MemoryCareer::immediate(9);
    assert!(!backend.flush_outbox());
    backend.poll();
    assert!(backend.flush_outbox());
    let event = queue_dodges::Event::new(&"a".repeat(64), 1).unwrap();
    assert!(backend.record_queue_dodge(event.clone()));
    assert!(!backend.flush_outbox());
    backend.poll();
    assert!(backend.flush_outbox());
    assert_eq!(backend.take_queue_dodge_committed(), vec![event.event_id]);
}
#[test]
fn dodge_pending_and_undrained_ack_capacity_apply_backpressure_without_dropping_ids() {
    let mut backend = MemoryCareer::immediate(9);
    for _ in 0..MAX_JOBS {
        assert!(backend.record_queue_dodge(queue_dodges::Event::new(&"a".repeat(64), 1).unwrap()));
    }
    let next = queue_dodges::Event::new(&"a".repeat(64), 1).unwrap();
    assert!(!backend.record_queue_dodge(next.clone()));
    backend.poll();
    assert!(
        !backend.record_queue_dodge(next.clone()),
        "undrained acknowledgements must remain bounded"
    );
    assert_eq!(backend.take_queue_dodge_committed().len(), MAX_JOBS);
    assert!(backend.record_queue_dodge(next));
}
#[test]
fn disabled_or_full_links_never_claim_to_accept_a_dodge() {
    let mut disabled = MemoryCareer::over(9, MemoryLink::new(false, false));
    let event = queue_dodges::Event::new(&"a".repeat(64), 1).unwrap();
    assert!(!disabled.record_queue_dodge(event.clone()));
    let mut full = MemoryCareer::test_backend(9);
    for i in 0..MAX_JOBS {
        full.link.jobs.push_back(Job::Flush(i as u64));
    }
    assert!(!full.record_queue_dodge(event));
    assert!(full.queue_dodges.is_empty());
    assert_eq!(full.record_generation, 0);
}

#[test]
fn a_commit_ack_for_a_conflicting_payload_cannot_clear_the_producer_event() {
    let mut backend = MemoryCareer::test_backend(9);
    let committed = queue_dodges::Event::new(&"a".repeat(64), 1).unwrap();
    let mut expected = committed.clone();
    expected.profile_id = "b".repeat(64);
    assert!(backend.record_queue_dodge(expected.clone()));
    backend
        .link
        .replies
        .push_back(Reply::QueueDodgeCommitted(committed));
    backend.poll();
    assert!(backend.take_queue_dodge_committed().is_empty());
    assert_eq!(
        backend.queue_dodges.get(&expected.event_id),
        Some(&expected)
    );
}

#[test]
fn a_fresh_shared_dodge_refusal_never_dispatches_a_queue_request() {
    let mut backend = MemoryCareer::test_backend(9);
    let addr = "127.0.0.1:51801".parse().unwrap();
    backend.test_authenticated(
        addr,
        ProfileSummary::new("a".repeat(64), "Player".into()),
        "session",
    );
    let nonce = backend.clients[&addr].auth.as_ref().unwrap().nonce.clone();
    backend.clients.get_mut(&addr).unwrap().queue_sequence = Some(1);
    backend.link.replies.push_back(Reply::QueueAccess {
        request: crate::career_port::MatchRequest {
            addr,
            request_id: 7,
            preference: shared::match_service::MatchPreference::Ranked,
            also_casual: false,
        },
        nonce,
        sequence: 1,
        access: Err(shared::career::error_code::DODGE_COOLDOWN.into()),
    });
    backend.poll();
    assert!(backend.take_match_requests().is_empty());
    assert_eq!(
        backend.view(addr).error_code.as_deref(),
        Some("dodge_cooldown")
    );
    assert_eq!(backend.view(addr).match_service_request_id, Some(7));
    assert_eq!(
        backend.view(addr).match_service,
        Some(shared::match_service::MatchServiceView::Failed {
            code: "dodge_cooldown".into()
        })
    );
    assert!(backend.authenticated_session(addr).is_some());
    let nonce = backend.clients[&addr].auth.as_ref().unwrap().nonce.clone();
    backend.clients.get_mut(&addr).unwrap().queue_sequence = Some(2);
    backend.link.replies.push_back(Reply::QueueAccess {
        request: crate::career_port::MatchRequest {
            addr,
            request_id: 8,
            preference: shared::match_service::MatchPreference::Ranked,
            also_casual: false,
        },
        nonce: nonce.clone(),
        sequence: 2,
        access: Ok(None),
    });
    // The old refusal arriving after this successful retry is stale.
    backend.link.replies.push_back(Reply::QueueAccess {
        request: crate::career_port::MatchRequest {
            addr,
            request_id: 7,
            preference: shared::match_service::MatchPreference::Ranked,
            also_casual: false,
        },
        nonce,
        sequence: 1,
        access: Err(shared::career::error_code::DODGE_COOLDOWN.into()),
    });
    backend.poll();
    assert!(backend.view(addr).match_service.is_none());
    assert!(backend.view(addr).match_service_request_id.is_none());
    assert_eq!(backend.take_match_requests()[0].request_id, 8);
}

fn ready_work(n: u64) -> consent::Work {
    consent::Work {
        check_id: format!("{n:032x}"),
        sequence: 1,
        operation: consent::Operation::Reserve {
            preference: shared::match_service::MatchPreference::Casual,
            seats: vec![omoba_career_store::career_store::ready::Seat {
                profile_id: "a".repeat(64),
                session_id: "session".into(),
            }],
        },
    }
}
#[test]
fn consent_channel_acceptance_is_bounded_and_an_ack_requires_the_full_original_command() {
    let mut backend = MemoryCareer::test_backend(9);
    let work = ready_work(1);
    assert!(backend.submit_ready(work.clone()));
    let generation = backend.record_generation;
    assert!(backend.submit_ready(work.clone()));
    assert_eq!(backend.record_generation, generation);
    assert_eq!(backend.link.jobs.len(), 1);
    let mut wrong = work.clone();
    wrong.operation =
        consent::Operation::Update(omoba_career_store::career_store::ready::Action::Cancel);
    backend
        .link
        .replies
        .push_back(Reply::Ready(consent::Completion {
            work: wrong,
            result: Err("unknown".into()),
        }));
    backend.poll();
    assert!(backend.take_ready_completions().is_empty());
    assert_eq!(backend.ready_jobs.len(), 1);
    backend
        .link
        .replies
        .push_back(Reply::Ready(consent::Completion {
            work: work.clone(),
            result: Err(STORE_UNAVAILABLE.into()),
        }));
    backend.poll();
    assert_eq!(backend.take_ready_completions().len(), 1);
    assert!(backend.submit_ready(work));
}
#[test]
fn undrained_consent_completions_apply_backpressure_instead_of_losing_an_ack() {
    let mut backend = MemoryCareer::test_backend(9);
    for n in 0..MAX_JOBS {
        let work = ready_work(n as u64);
        assert!(backend.submit_ready(work.clone()));
        backend
            .link
            .replies
            .push_back(Reply::Ready(consent::Completion {
                work,
                result: Err(STORE_UNAVAILABLE.into()),
            }));
    }
    backend.poll();
    assert!(!backend.submit_ready(ready_work(999)));
    assert_eq!(backend.take_ready_completions().len(), MAX_JOBS);
    // The test link still holds captured commands; draining those, as the
    // real worker does, makes room without inventing a completed transaction.
    backend.link.jobs.clear();
    assert!(backend.submit_ready(ready_work(999)));
}
