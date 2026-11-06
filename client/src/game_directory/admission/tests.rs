use super::*;
use crate::game_directory::{PublicPin, SignedDirectory};
use ed25519_dalek::{Signer, SigningKey};
use std::{
    fs,
    sync::{Arc, Barrier},
    time::{Duration, Instant},
};

const NOW: u64 = 1_800_000_000_000;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).unwrap();
        let suffix: String = random.iter().map(|n| format!("{n:02x}")).collect();
        let path = std::env::temp_dir().join(format!("omoba-directory-admission-{suffix}"));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn admission(&self) -> Admission {
        let trust = trust();
        let fallback = PinnedFallback::new(&trust, lobby("fallback", 105)).unwrap();
        Admission::new(self.0.join("directory.json"), trust, Some(fallback)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn trust() -> DirectoryTrust {
    DirectoryTrust::new(
        shared::region::RegionId::Ru,
        "production",
        vec![PublicPin {
            key_id: "fixture".into(),
            public_key: SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes(),
        }],
    )
    .unwrap()
}
fn lobby(id: &str, address: u8) -> Lobby {
    Lobby {
        id: id.into(),
        location: "moscow".into(),
        host: format!("{id}.omoba.ru"),
        addresses: vec![std::net::Ipv4Addr::new(212, 113, 117, address).into()],
        port: 4000,
        contract: ReleaseContract::current(),
        status: LobbyStatus::Open,
        weight: 1,
    }
}
fn document(sequence: u64) -> Directory {
    Directory {
        version: 1,
        region: shared::region::RegionId::Ru,
        contour: "production".into(),
        sequence,
        issued_at_ms: NOW,
        expires_at_ms: NOW + 60000,
        policy: Policy {
            min_release: ReleaseContract::current().release,
            maintenance: false,
            update_url: None,
            message: None,
        },
        lobbies: vec![lobby("one", 105), lobby("two", 106)],
    }
}
fn signed(document: &Directory) -> Vec<u8> {
    let mut envelope = SignedDirectory {
        key_id: "fixture".into(),
        payload: serde_json::to_string(document).unwrap(),
        signature: String::new(),
    };
    let signature = SigningKey::from_bytes(&[7; 32]).sign(&envelope.signing_bytes().unwrap());
    envelope.signature = signature
        .to_bytes()
        .iter()
        .map(|n| format!("{n:02x}"))
        .collect();
    serde_json::to_vec(&envelope).unwrap()
}
fn decision(admission: &Admission, now: u64) -> Decision {
    admission.decision_at(now, &ReleaseContract::current(), 0, None)
}
fn route(decision: Decision) -> Route {
    let Decision::Route(route) = decision else {
        panic!("expected concrete route, got {decision:?}")
    };
    route
}

#[test]
fn startup_cache_expiry_and_outage_use_only_valid_policy_or_pinned_addresses() {
    let f = Fixture::new();
    let mut a = f.admission();
    assert_eq!(decision(&a, NOW), Decision::Pending);
    assert_eq!(
        a.refresh_at(Err("offline"), NOW),
        Ok(Origin::PinnedFallback)
    );
    let fallback = route(decision(&a, NOW));
    assert_eq!(fallback.origin(), Origin::PinnedFallback);
    assert_eq!(fallback.sequence(), None);
    assert_eq!(fallback.endpoint().to_string(), "212.113.117.105:4000");
    assert_eq!(fallback.display(), r"fallback.omoba.ru:4000");
    a.refresh_at(Ok(&signed(&document(4))), NOW).unwrap();
    assert_eq!(route(decision(&a, NOW)).sequence(), Some(4));
    a.refresh_at(Err("offline"), NOW + 1).unwrap();
    assert_eq!(route(decision(&a, NOW + 1)).origin(), Origin::Cache);
    // Expiry is checked again for every decision without requiring a refresh.
    assert_eq!(
        route(decision(&a, NOW + 60000)).origin(),
        Origin::PinnedFallback
    );
    let mut restarted = f.admission();
    restarted.refresh_at(Err("offline"), NOW + 60000).unwrap();
    assert_eq!(
        route(decision(&restarted, NOW + 60000)).origin(),
        Origin::PinnedFallback
    );
    assert_eq!(
        restarted.floor.unwrap().directory.sequence,
        4,
        "expired rollback floor retained"
    );
}
#[test]
fn authenticated_closed_policies_do_not_escape_through_fallback_but_resume_is_retained() {
    let f = Fixture::new();
    let mut a = f.admission();
    a.refresh_at(Ok(&signed(&document(1))), NOW).unwrap();
    let owner = route(decision(&a, NOW)).retain();
    let original = owner.resume_route().clone();
    let mut doc = document(2);
    doc.policy.maintenance = true;
    doc.policy.message = Some("Проверка".into());
    a.refresh_at(Ok(&signed(&doc)), NOW).unwrap();
    assert!(matches!(decision(&a, NOW), Decision::Maintenance { .. }));
    assert_eq!(owner.resume_route(), &original);
    doc.sequence += 1;
    doc.policy.min_release = "999.0.0".into();
    a.refresh_at(Ok(&signed(&doc)), NOW).unwrap();
    assert!(matches!(decision(&a, NOW), Decision::UpdateRequired { .. }));
    doc.sequence += 1;
    doc.policy.min_release = ReleaseContract::current().release;
    doc.policy.maintenance = false;
    for lobby in &mut doc.lobbies {
        lobby.status = LobbyStatus::Full;
    }
    a.refresh_at(Ok(&signed(&doc)), NOW).unwrap();
    assert_eq!(decision(&a, NOW), Decision::Full);
    doc.sequence += 1;
    for lobby in &mut doc.lobbies {
        lobby.status = LobbyStatus::Open;
        lobby.contract.protocol += 1;
    }
    a.refresh_at(Ok(&signed(&doc)), NOW).unwrap();
    assert_eq!(decision(&a, NOW), Decision::Incompatible);
    assert_eq!(owner.resume_route().endpoint(), original.endpoint());
}
#[test]
fn party_affinity_never_switches_to_a_different_lobby_or_new_ip() {
    let f = Fixture::new();
    let mut a = f.admission();
    a.refresh_at(Ok(&signed(&document(1))), NOW).unwrap();
    let owner = route(decision(&a, NOW)).retain();
    let retained = a.decision_at(NOW, &ReleaseContract::current(), 1, Some(&owner));
    assert_eq!(route(retained).endpoint(), owner.resume_route().endpoint());
    let mut doc = document(2);
    doc.lobbies[0].status = LobbyStatus::Full;
    a.refresh_at(Ok(&signed(&doc)), NOW).unwrap();
    assert!(matches!(decision(&a, NOW), Decision::Route(_)));
    assert_eq!(
        a.decision_at(NOW, &ReleaseContract::current(), 1, Some(&owner)),
        Decision::Unavailable
    );
    doc.sequence += 1;
    doc.lobbies[0].status = LobbyStatus::Open;
    doc.lobbies[0].addresses = vec![std::net::Ipv4Addr::new(212, 113, 117, 107).into()];
    a.refresh_at(Ok(&signed(&doc)), NOW).unwrap();
    assert_eq!(
        a.decision_at(NOW, &ReleaseContract::current(), 0, Some(&owner)),
        Decision::Unavailable
    );
}
#[test]
fn tamper_and_replay_keep_current_durable_policy_while_equivocation_is_refused() {
    let f = Fixture::new();
    let mut a = f.admission();
    a.refresh_at(Ok(&signed(&document(8))), NOW).unwrap();
    a.refresh_at(Ok(b"untrusted bytes"), NOW).unwrap();
    assert_eq!(route(decision(&a, NOW)).sequence(), Some(8));
    a.refresh_at(Ok(&signed(&document(7))), NOW).unwrap();
    assert_eq!(route(decision(&a, NOW)).sequence(), Some(8));
    let mut other = document(8);
    other.policy.maintenance = true;
    assert_eq!(
        a.refresh_at(Ok(&signed(&other)), NOW),
        Err(DirectoryError::Equivocation)
    );
    assert_eq!(decision(&a, NOW), Decision::Unavailable);
    other.sequence = 9;
    a.refresh_at(Ok(&signed(&other)), NOW).unwrap();
    let mut restarted = f.admission();
    restarted
        .refresh_at(Ok(&signed(&document(8))), NOW)
        .unwrap();
    assert!(matches!(
        decision(&restarted, NOW),
        Decision::Maintenance { .. }
    ));
}
#[test]
fn corruption_and_backwards_clock_never_turn_into_an_open_fallback() {
    let f = Fixture::new();
    let mut a = f.admission();
    a.refresh_at(Ok(&signed(&document(8))), NOW).unwrap();
    assert_eq!(decision(&a, NOW - 30001), Decision::Unavailable);
    fs::write(&a.cache.path, b"corrupt history").unwrap();
    assert_eq!(
        a.refresh_at(Ok(&signed(&document(9))), NOW),
        Err(DirectoryError::CacheInvalid)
    );
    assert_eq!(decision(&a, NOW), Decision::Unavailable);
    assert_eq!(fs::read(&a.cache.path).unwrap(), b"corrupt history");
}
#[cfg(unix)]
#[test]
fn failed_new_maintenance_publication_fences_old_cache_until_durable_repair() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let mut a = f.admission();
    a.refresh_at(Ok(&signed(&document(8))), NOW).unwrap();
    let mut new = document(9);
    new.policy.maintenance = true;
    let bytes = signed(&new);
    fs::set_permissions(&f.0, fs::Permissions::from_mode(0o555)).unwrap();
    let failure = a.refresh_at(Ok(&bytes), NOW);
    fs::set_permissions(&f.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(failure, Err(DirectoryError::CacheIo));
    assert_eq!(decision(&a, NOW), Decision::Unavailable);
    assert_eq!(a.floor.as_ref().unwrap().directory.sequence, 9);
    assert_eq!(
        a.refresh_at(Err("offline"), NOW),
        Err(DirectoryError::Rollback)
    );
    assert_eq!(decision(&a, NOW), Decision::Unavailable);
    a.refresh_at(Ok(&bytes), NOW).unwrap();
    assert!(matches!(decision(&a, NOW), Decision::Maintenance { .. }));
}
#[test]
fn fallback_scope_and_concrete_destinations_are_validated_without_dns() {
    let trust = trust();
    let mut foreign = lobby("foreign", 105);
    foreign.host = r"lobby.omoba.io".into();
    assert!(PinnedFallback::new(&trust, foreign).is_err());
    let mut private = lobby("private", 105);
    private.addresses = vec![std::net::Ipv4Addr::LOCALHOST.into()];
    assert!(PinnedFallback::new(&trust, private).is_err());
    let other = DirectoryTrust::new(
        shared::region::RegionId::Global,
        "production",
        vec![PublicPin {
            key_id: "fixture".into(),
            public_key: SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes(),
        }],
    )
    .unwrap();
    let fallback = PinnedFallback::new(&trust, lobby("fallback", 105)).unwrap();
    let f = Fixture::new();
    assert!(Admission::new(f.0.join("cache.json"), other, Some(fallback)).is_err());
}
#[test]
fn async_refresh_has_one_worker_and_never_blocks_main_polling() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Resolver>();
    let f = Fixture::new();
    let mut resolver = Resolver::new(f.admission()).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let worker = barrier.clone();
    let bytes = signed(&document(1));
    assert!(resolver.start(
        move || {
            worker.wait();
            Ok(bytes)
        },
        || Ok(NOW)
    ));
    assert!(!resolver.start(|| panic!("second worker must not launch"), || Ok(NOW)));
    resolver.poll();
    assert_eq!(decision(resolver.admission(), NOW), Decision::Pending);
    barrier.wait();
    let deadline = Instant::now() + Duration::from_secs(5);
    while resolver.pending.is_some() && Instant::now() < deadline {
        resolver.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(resolver.pending.is_none());
    assert_eq!(
        route(decision(resolver.admission(), NOW)).sequence(),
        Some(1)
    );
}

#[test]
fn compatible_alternate_is_selected_and_missing_fallback_stays_unavailable() {
    let f = Fixture::new();
    let mut a = f.admission();
    let mut doc = document(1);
    doc.lobbies[0].contract.protocol += 1;
    a.refresh_at(Ok(&signed(&doc)), NOW).unwrap();
    assert_eq!(route(decision(&a, NOW)).lobby_id(), "two");
    let mut no_fallback = Admission::new(f.0.join("no-fallback.json"), trust(), None).unwrap();
    assert!(no_fallback.refresh_at(Err("offline"), NOW).is_err());
    assert_eq!(decision(&no_fallback, NOW), Decision::Unavailable);
}

#[test]
fn resume_revalidates_original_owner_without_new_admission_policy_or_selection() {
    let f = Fixture::new();
    let mut a = f.admission();
    assert_eq!(
        a.resume_at(NOW, "212.113.117.105:4000", "212.113.117.105:45600"),
        ResumeDecision::Pending
    );
    let mut doc = document(1);
    doc.policy.maintenance = true;
    doc.policy.min_release = "999.0.0".into();
    doc.lobbies[0].status = LobbyStatus::Maintenance;
    doc.lobbies[0].contract.protocol += 1;
    a.refresh_at(Ok(&signed(&doc)), NOW).unwrap();
    assert!(matches!(decision(&a, NOW), Decision::UpdateRequired { .. }));
    let ResumeDecision::Ready(resume) =
        a.resume_at(NOW, "212.113.117.105:4000", "212.113.117.105:45600")
    else {
        panic!("existing seat owner must not use new-admission selection")
    };
    assert_eq!(
        resume.lobby().endpoint().to_string(),
        "212.113.117.105:4000"
    );
    assert_eq!(resume.worker().to_string(), "212.113.117.105:45600");
    assert_eq!(resume.lobby().lobby_id(), "one");
}
#[test]
fn saved_resume_never_authorizes_foreign_hosts_ports_or_another_lobbys_worker() {
    let f = Fixture::new();
    let mut a = f.admission();
    a.refresh_at(Ok(&signed(&document(1))), NOW).unwrap();
    for (lobby, worker) in [
        ("212.113.117.105:4000", "212.113.117.106:45600"),
        ("212.113.117.105:4000", "77.246.105.57:45600"),
        ("212.113.117.105:4000", "127.0.0.1:45600"),
        ("212.113.117.105:4000", r"foreign.omoba.ru:45600"),
        ("212.113.117.105:4000", "212.113.117.105:0"),
        ("212.113.117.105:4001", "212.113.117.105:45600"),
        (r"foreign.omoba.ru:4000", "212.113.117.105:45600"),
    ] {
        assert_eq!(
            a.resume_at(NOW, lobby, worker),
            ResumeDecision::Unavailable,
            "{lobby}/{worker}"
        );
    }
    let ResumeDecision::Ready(route) = a.resume_at(
        NOW,
        "[::ffff:212.113.117.105]:4000",
        "[::ffff:212.113.117.105]:45600",
    ) else {
        panic!("canonical mapped addresses")
    };
    assert_eq!(route.worker().to_string(), "212.113.117.105:45600");
}
#[test]
fn resume_removal_expiry_and_corrupt_cache_cannot_reuse_old_directory_authority() {
    let f = Fixture::new();
    let mut a = f.admission();
    let mut doc = document(1);
    a.refresh_at(Ok(&signed(&doc)), NOW).unwrap();
    assert!(matches!(
        a.resume_at(NOW, "212.113.117.105:4000", "212.113.117.105:45600"),
        ResumeDecision::Ready(_)
    ));
    doc.sequence = 2;
    doc.lobbies.remove(0);
    a.refresh_at(Ok(&signed(&doc)), NOW).unwrap();
    assert_eq!(
        a.resume_at(NOW, "212.113.117.105:4000", "212.113.117.105:45600"),
        ResumeDecision::Unavailable,
        "current removal cannot be bypassed through baked fallback"
    );
    assert_eq!(
        a.resume_at(NOW + 60000, "212.113.117.106:4000", "212.113.117.106:45600"),
        ResumeDecision::Unavailable,
        "expired secondary owner is not the separately pinned fallback"
    );
    let ResumeDecision::Ready(fallback) =
        a.resume_at(NOW + 60000, "212.113.117.105:4000", "212.113.117.105:45600")
    else {
        panic!("separately pinned original fallback remains authorized")
    };
    assert_eq!(fallback.lobby().origin(), Origin::PinnedFallback);
    fs::write(&a.cache.path, b"corrupt").unwrap();
    assert!(a.refresh_at(Err("offline"), NOW + 60000).is_err());
    assert_eq!(
        a.resume_at(NOW + 60000, "212.113.117.105:4000", "212.113.117.105:45600"),
        ResumeDecision::Unavailable
    );
}
#[test]
fn legacy_and_multihomed_resume_hints_become_exact_authorized_ips_without_dns() {
    let f = Fixture::new();
    let mut a = f.admission();
    let mut doc = document(1);
    doc.lobbies[0]
        .addresses
        .push(std::net::Ipv4Addr::new(212, 113, 117, 108).into());
    a.refresh_at(Ok(&signed(&doc)), NOW).unwrap();
    let ResumeDecision::Ready(route) =
        a.resume_at(NOW, r"one.omoba.ru:4000", "212.113.117.108:45600")
    else {
        panic!("legacy owner label must retain actual authorized worker IP")
    };
    assert_eq!(route.lobby().endpoint().to_string(), "212.113.117.108:4000");
    let ResumeDecision::Ready(route) =
        a.resume_at(NOW, "212.113.117.108:4000", r"one.omoba.ru:45600")
    else {
        panic!("known label maps only to the original pinned owner")
    };
    assert_eq!(route.worker().to_string(), "212.113.117.108:45600");
}
