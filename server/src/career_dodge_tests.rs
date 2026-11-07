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
