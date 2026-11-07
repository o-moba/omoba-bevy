use super::*;
use crate::career_backend::{
    MemoryCareer,
    consent::{Completion, Operation, Work},
};
use crate::career_port::CareerPort;
use omoba_career_store::career_store::ready::{Action, Member, Phase, Snapshot};
fn fixture(now: Instant) -> (MatchService, Vec<Waiting>, PathBuf, String, MemoryCareer) {
    let pool = crate::match_pool::Pool::for_test();
    let directory = pool.test_directory().to_path_buf();
    let mut service = tests::lobby_with(pool, &[]);
    service.enable_shared_ready(true);
    let mut entries: Vec<_> = (1..=10)
        .map(|i| tests::w(i, MatchPreference::Ranked, now))
        .collect();
    for entry in entries.iter_mut().take(2) {
        entry.party = Some((99, 2));
    }
    tests::enqueue_all(&mut service, &entries, 7, now);
    service.tick(now);
    let MatchService::Lobby(lobby) = &service else {
        unreachable!()
    };
    let id = lobby.ready.keys().next().unwrap().clone();
    let entries = lobby.ready[&id].entries.clone();
    (
        service,
        entries,
        directory,
        id,
        MemoryCareer::test_backend(9),
    )
}
fn work(service: &mut MatchService, id: &str, now: Instant) -> Work {
    let MatchService::Lobby(lobby) = service else {
        unreachable!()
    };
    lobby
        .ready
        .get_mut(id)
        .unwrap()
        .consent
        .as_mut()
        .unwrap()
        .next(now)
        .unwrap()
}
fn snapshot(entries: &[Waiting], id: &str, phase: Phase) -> Snapshot {
    let opened = unix_ms() as i64;
    Snapshot {
        check_id: id.into(),
        preference: MatchPreference::Ranked,
        phase,
        deadline_ms: opened + 15_000,
        opened_at_ms: Some(opened),
        members: entries
            .iter()
            .map(|e| Member {
                seat: omoba_career_store::career_store::ready::Seat {
                    profile_id: e.profile.profile_id.clone(),
                    session_id: e.session.clone(),
                },
                accepted: phase == Phase::Approved,
                dodged: false,
                event_id: "b".repeat(32),
                observed_at_ms: None,
                cooldown: None,
            })
            .collect(),
    }
}
fn commit(
    service: &mut MatchService,
    backend: &mut MemoryCareer,
    id: &str,
    entries: &[Waiting],
    phase: Phase,
    now: Instant,
) {
    let work = work(service, id, now);
    assert!(backend.submit_ready(work.clone()));
    backend.test_complete_ready(Completion {
        work,
        result: Ok(snapshot(entries, id, phase)),
    });
    service.poll_shared_ready(backend, now);
}
#[test]
fn real_lobby_never_exposes_or_launches_consent_before_durable_acknowledgements() {
    let now = Instant::now();
    let (mut service, entries, directory, id, mut backend) = fixture(now);
    assert!(matches!(
        service.view(&entries[0].profile.profile_id, &entries[0].session, now),
        Some(MatchServiceView::Waiting { .. })
    ));
    commit(
        &mut service,
        &mut backend,
        &id,
        &entries,
        Phase::Forming,
        now,
    );
    assert!(matches!(
        service.view(&entries[0].profile.profile_id, &entries[0].session, now),
        Some(MatchServiceView::Waiting { .. })
    ));
    commit(&mut service, &mut backend, &id, &entries, Phase::Open, now);
    assert!(matches!(
        service.view(&entries[0].profile.profile_id, &entries[0].session, now),
        Some(MatchServiceView::ReadyCheck {
            accepted_count: 0,
            ..
        })
    ));
    for entry in &entries {
        service.respond_ready(&entry.profile.profile_id, &entry.session, 7, &id, true, now);
    }
    service.tick(now + Duration::from_secs(1));
    let MatchService::Lobby(lobby) = &service else {
        unreachable!()
    };
    assert!(lobby.pool.slots.is_empty());
    assert!(lobby.ready[&id].accepted.is_empty());
    commit(
        &mut service,
        &mut backend,
        &id,
        &entries,
        Phase::Approved,
        now,
    );
    service.tick(now + Duration::from_secs(2));
    let MatchService::Lobby(lobby) = &service else {
        unreachable!()
    };
    assert_eq!(lobby.pool.slots.len(), 1);
    assert!(lobby.ready.is_empty());
    drop(service);
    let _ = std::fs::remove_dir_all(directory);
}
#[test]
fn real_lobby_cancels_published_consent_for_maintenance_without_penalty_and_keeps_fifo() {
    let now = Instant::now();
    let (mut service, entries, directory, id, mut backend) = fixture(now);
    commit(
        &mut service,
        &mut backend,
        &id,
        &entries,
        Phase::Forming,
        now,
    );
    commit(&mut service, &mut backend, &id, &entries, Phase::Open, now);
    let MatchService::Lobby(lobby) = &mut service else {
        unreachable!()
    };
    lobby.pause_ready_for_drain(now);
    let cancel = work(&mut service, &id, now);
    assert!(matches!(
        cancel.operation,
        Operation::Update(Action::Cancel)
    ));
    commit(
        &mut service,
        &mut backend,
        &id,
        &entries,
        Phase::Cancelled,
        now,
    );
    let MatchService::Lobby(lobby) = &service else {
        unreachable!()
    };
    assert!(lobby.ready.is_empty() && lobby.penalties.is_empty());
    assert_eq!(
        lobby.waiting.iter().map(|e| e.order).collect::<Vec<_>>(),
        entries.iter().map(|e| e.order).collect::<Vec<_>>()
    );
    assert!(lobby.waiting.iter().all(|e| e.queued == now));
    assert_eq!(
        lobby
            .waiting
            .iter()
            .filter(|e| e.party == Some((99, 2)))
            .count(),
        2
    );
    drop(service);
    let _ = std::fs::remove_dir_all(directory);
}
#[test]
fn real_lobby_applies_committed_dodge_without_creating_a_second_outbox_event() {
    let now = Instant::now();
    let (mut service, entries, directory, id, mut backend) = fixture(now);
    commit(
        &mut service,
        &mut backend,
        &id,
        &entries,
        Phase::Forming,
        now,
    );
    commit(&mut service, &mut backend, &id, &entries, Phase::Open, now);
    service.respond_ready(
        &entries[0].profile.profile_id,
        &entries[0].session,
        7,
        &id,
        false,
        now,
    );
    let work = work(&mut service, &id, now);
    assert!(backend.submit_ready(work.clone()));
    let mut closed = snapshot(&entries, &id, Phase::Cancelled);
    closed.members[0].dodged = true;
    closed.members[0].observed_at_ms = Some(unix_ms() as i64);
    closed.members[0].cooldown = Some(shared::career::PlayCooldown {
        until: (unix_ms() / 1000 + 60) as i64,
        strikes: 1,
    });
    backend.test_complete_ready(Completion {
        work,
        result: Ok(closed),
    });
    service.poll_shared_ready(&mut backend, now);
    let MatchService::Lobby(lobby) = &service else {
        unreachable!()
    };
    assert_eq!(lobby.waiting.len(), 9);
    assert_eq!(lobby.penalties.len(), 1);
    assert!(
        lobby.penalties[&entries[0].profile.profile_id]
            .pending_events
            .is_empty()
    );
    assert!(lobby.pool.slots.is_empty());
    assert!(
        lobby
            .waiting
            .iter()
            .all(|e| e.profile.profile_id != entries[0].profile.profile_id)
    );
    drop(service);
    let _ = std::fs::remove_dir_all(directory);
}
