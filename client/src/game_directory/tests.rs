// Routing fixture hosts use raw literals: the i18n scanner documents these
// as protocol data, never translation keys. Wire values stay unchanged.
use super::*;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};

const NOW: u64 = 1_800_000_000_000;
fn trust(region: RegionId, contour: &str) -> DirectoryTrust {
    DirectoryTrust::new(region, contour, vec![pin(7, "fixture")]).unwrap()
}
fn pin(seed: u8, id: &str) -> PublicPin {
    PublicPin {
        key_id: id.into(),
        public_key: SigningKey::from_bytes(&[seed; 32])
            .verifying_key()
            .to_bytes(),
    }
}
fn document(sequence: u64) -> Value {
    json!({"version":1,"region":"ru","contour":"production","sequence":sequence,"issued_at_ms":NOW,"expires_at_ms":NOW+60000,
        "policy":{"min_release":"0.45.0","maintenance":true,"update_url":null,"message":"Проверка ⚔"},"lobbies":[]})
}
fn signed(value: &Value, seed: u8, id: &str) -> Vec<u8> {
    let mut envelope = SignedDirectory {
        key_id: id.into(),
        payload: serde_json::to_string(value).unwrap(),
        signature: String::new(),
    };
    let sig = SigningKey::from_bytes(&[seed; 32]).sign(&envelope.signing_bytes().unwrap());
    envelope.signature = sig.to_bytes().iter().map(|n| format!("{n:02x}")).collect();
    serde_json::to_vec_pretty(&envelope).unwrap()
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let name = random
            .iter()
            .map(|n| format!("{n:02x}"))
            .collect::<String>();
        let dir = std::env::temp_dir().join(format!("omoba-directory-native-{name}"));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }
    fn cache(&self) -> DirectoryCache {
        DirectoryCache::new(
            self.0.join("directory.json"),
            trust(RegionId::Ru, "production"),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn authenticates_exact_payload_and_separate_pins_before_exposing_destinations() {
    let trust = trust(RegionId::Ru, "production");
    let wire = signed(&document(u64::MAX), 7, "fixture");
    assert_eq!(
        trust.verify_at(&wire, NOW, 1).unwrap().directory().sequence,
        u64::MAX
    );
    let mut envelope = SignedDirectory::parse(&wire).unwrap();
    envelope.payload.push(' ');
    assert!(
        trust
            .verify_at(&serde_json::to_vec(&envelope).unwrap(), NOW, 0)
            .is_err()
    );
    for (seed, id) in [(8, "fixture"), (7, "attacker")] {
        assert!(
            trust
                .verify_at(&signed(&document(1), seed, id), NOW, 0)
                .is_err()
        );
    }
    assert!(
        trust
            .verify_at(&vec![b'x'; MAX_WIRE_BYTES + 1], NOW, 0)
            .is_err()
    );
    assert!(
        DirectoryTrust::new(
            RegionId::Ru,
            "production",
            vec![pin(7, "fixture"), pin(8, "fixture")]
        )
        .is_err()
    );
    assert!(DirectoryTrust::new(RegionId::Ru, "production", vec![]).is_err());
}
#[test]
fn scope_time_and_known_foreign_labels_and_ips_are_checked_independently() {
    let trust = trust(RegionId::Ru, "production");
    for (field, value) in [
        ("region", json!("global")),
        ("contour", json!("dev")),
        ("expires_at_ms", json!(NOW)),
        ("issued_at_ms", json!(NOW + 30001)),
    ] {
        let mut doc = document(1);
        doc[field] = value;
        assert!(
            trust
                .verify_at(&signed(&doc, 7, "fixture"), NOW, 0)
                .is_err(),
            "{field}"
        );
    }
    let contract = shared::compatibility::ReleaseContract::current();
    for (host, ip) in [
        (r"lobby.omoba.io", "212.113.117.105"),
        (r"lobby.omoba.ru", "77.246.105.57"),
        (r"lobby.omoba.ru", "::ffff:77.246.105.57"),
        (r"lobby.dev.omoba.ru", "212.113.117.105"),
        (r"lobby.omoba.ru", "146.103.102.18"),
        (r"lobby.omoba.ru", "::ffff:146.103.102.18"),
    ] {
        let mut doc = document(1);
        doc["lobbies"] = json!([{"id":"ru-one","location":"moscow","host":host,"addresses":[ip],"port":4000,"contract":contract,"status":"open","weight":1}]);
        assert!(
            trust
                .verify_at(&signed(&doc, 7, "fixture"), NOW, 0)
                .is_err(),
            "{host}/{ip}"
        );
    }
    let mut doc = document(1);
    doc["contour"] = json!("dev");
    doc["lobbies"] = json!([{"id":"ru-one","location":"moscow","host":r"lobby.dev.omoba.ru","addresses":["212.113.117.105"],"port":4100,"contract":contract,"status":"open","weight":1}]);
    assert!(
        super::tests::trust(RegionId::Ru, "dev")
            .verify_at(&signed(&doc, 7, "fixture"), NOW, 0)
            .is_err()
    );
}
#[test]
fn expired_cache_survives_restart_as_a_rollback_and_equivocation_fence() {
    let f = Fixture::new();
    let cache = f.cache();
    assert!(cache.load_at(NOW).unwrap().is_none());
    let first = signed(&document(10), 7, "fixture");
    cache.accept_at(&first, NOW).unwrap();
    assert_eq!(fs::read(&cache.path).unwrap(), first);
    drop(cache);
    let cache = f.cache();
    assert!(cache.load_at(NOW + 60000).unwrap().is_none());
    assert!(cache.path.exists());
    let mut older = document(9);
    older["expires_at_ms"] = json!(NOW + 120000);
    assert_eq!(
        cache
            .accept_at(&signed(&older, 7, "fixture"), NOW + 60000)
            .unwrap_err(),
        DirectoryError::Rollback
    );
    let mut changed = document(10);
    changed["expires_at_ms"] = json!(NOW + 120000);
    assert_eq!(
        cache
            .accept_at(&signed(&changed, 7, "fixture"), NOW + 60000)
            .unwrap_err(),
        DirectoryError::Equivocation
    );
    let mut next = document(11);
    next["expires_at_ms"] = json!(NOW + 120000);
    cache
        .accept_at(&signed(&next, 7, "fixture"), NOW + 60000)
        .unwrap();
    assert_eq!(
        cache
            .load_at(NOW + 60000)
            .unwrap()
            .unwrap()
            .directory()
            .sequence,
        11
    );
}
#[test]
fn replay_reformatting_and_key_overlap_do_not_erase_policy_or_history() {
    let f = Fixture::new();
    let trust = DirectoryTrust::new(
        RegionId::Ru,
        "production",
        vec![pin(7, "fixture"), pin(8, "next")],
    )
    .unwrap();
    let cache = DirectoryCache::new(f.0.join("directory.json"), trust);
    let first = signed(&document(10), 7, "fixture");
    cache.accept_at(&first, NOW).unwrap();
    let envelope = SignedDirectory::parse(&first).unwrap();
    cache
        .accept_at(&serde_json::to_vec(&envelope).unwrap(), NOW)
        .unwrap();
    assert_eq!(
        fs::read(&cache.path).unwrap(),
        first,
        "outer envelope whitespace does not change signed payload"
    );
    cache
        .accept_at(&signed(&document(11), 8, "next"), NOW)
        .unwrap();
    assert_eq!(
        cache.accept_at(&first, NOW).unwrap_err(),
        DirectoryError::Rollback
    );
    assert!(matches!(
        cache
            .load_at(NOW)
            .unwrap()
            .unwrap()
            .directory()
            .select(&shared::compatibility::ReleaseContract::current(), 0),
        shared::game_directory::Selection::Maintenance
    ));
}
#[test]
fn corrupt_nonregular_and_oversize_cache_never_reset_sequence_to_zero() {
    let f = Fixture::new();
    let cache = f.cache();
    let wire = signed(&document(1), 7, "fixture");
    for bytes in [b"corrupt".to_vec(), vec![b'x'; MAX_WIRE_BYTES + 1]] {
        fs::write(&cache.path, bytes).unwrap();
        assert_eq!(
            cache.accept_at(&wire, NOW).unwrap_err(),
            DirectoryError::CacheInvalid
        );
        assert_eq!(
            cache.load_at(NOW).unwrap_err(),
            DirectoryError::CacheInvalid
        );
    }
    fs::remove_file(&cache.path).unwrap();
    fs::create_dir(&cache.path).unwrap();
    assert_eq!(
        cache.accept_at(&wire, NOW).unwrap_err(),
        DirectoryError::CacheInvalid
    );
    fs::remove_dir(&cache.path).unwrap();
    #[cfg(unix)]
    {
        let target = f.0.join("other.json");
        fs::write(&target, &wire).unwrap();
        std::os::unix::fs::symlink(target, &cache.path).unwrap();
        assert_eq!(
            cache.accept_at(&wire, NOW).unwrap_err(),
            DirectoryError::CacheInvalid
        );
    }
}
#[test]
fn independent_instances_cannot_update_a_locked_or_newer_cache() {
    if let Some(path) = std::env::var_os("OMOBA_DIRECTORY_CACHE_LOCK_PROBE") {
        let cache = DirectoryCache::new(path.into(), trust(RegionId::Ru, "production"));
        assert_eq!(
            cache
                .accept_at(&signed(&document(3), 7, "fixture"), NOW)
                .unwrap_err(),
            DirectoryError::CacheBusy
        );
        return;
    }
    let f = Fixture::new();
    let one = f.cache();
    let two = f.cache();
    let lock = one.lock().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "game_directory::tests::independent_instances_cannot_update_a_locked_or_newer_cache",
            "--nocapture",
        ])
        .env("OMOBA_DIRECTORY_CACHE_LOCK_PROBE", &one.path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > std::time::Duration::from_secs(10) {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("cache lock child deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "child stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        two.accept_at(&signed(&document(3), 7, "fixture"), NOW)
            .unwrap_err(),
        DirectoryError::CacheBusy
    );
    drop(lock);
    two.accept_at(&signed(&document(4), 7, "fixture"), NOW)
        .unwrap();
    assert_eq!(
        one.accept_at(&signed(&document(3), 7, "fixture"), NOW)
            .unwrap_err(),
        DirectoryError::Rollback
    );
    assert_eq!(one.load_at(NOW).unwrap().unwrap().directory().sequence, 4);
    assert_eq!(
        fs::read_dir(&f.0).unwrap().count(),
        2,
        "only cache and stable OS lock remain"
    );
}

#[test]
fn open_directory_exposes_only_the_verified_signed_concrete_destinations() {
    let mut doc = document(2);
    doc["policy"]["maintenance"] = json!(false);
    doc["lobbies"] = json!([{"id":"ru-one","location":"moscow","host":r"lobby.omoba.ru","addresses":["212.113.117.105"],"port":4000,"contract":shared::compatibility::ReleaseContract::current(),"status":"open","weight":1}]);
    let verified = trust(RegionId::Ru, "production")
        .verify_at(&signed(&doc, 7, "fixture"), NOW, 1)
        .unwrap();
    let shared::game_directory::Selection::Lobby(lobby) = verified
        .directory()
        .select(&shared::compatibility::ReleaseContract::current(), 0)
    else {
        panic!("expected compatible lobby")
    };
    assert_eq!(
        lobby.network_endpoints(),
        vec![
            "212.113.117.105:4000"
                .parse::<std::net::SocketAddr>()
                .unwrap()
        ]
    );
    assert_eq!(lobby.display_endpoint(), "lobby.omoba.ru:4000");
}

#[test]
fn invalid_remote_data_never_replaces_a_valid_newer_cache() {
    let f = Fixture::new();
    let cache = f.cache();
    let original = signed(&document(20), 7, "fixture");
    cache.accept_at(&original, NOW).unwrap();
    for bytes in [
        signed(&document(21), 8, "fixture"),
        b"corrupt".to_vec(),
        signed(&document(19), 7, "fixture"),
    ] {
        assert!(cache.accept_at(&bytes, NOW).is_err());
        assert_eq!(fs::read(&cache.path).unwrap(), original);
        assert_eq!(
            cache.load_at(NOW).unwrap().unwrap().directory().sequence,
            20
        );
    }
}

#[test]
fn a_backwards_clock_cannot_turn_known_policy_into_an_absent_cache() {
    let f = Fixture::new();
    let cache = f.cache();
    cache
        .accept_at(&signed(&document(3), 7, "fixture"), NOW)
        .unwrap();
    assert!(matches!(
        cache.load_at(NOW - 30001),
        Err(DirectoryError::Invalid(_))
    ));
    assert!(cache.load_at(NOW - 30000).unwrap().is_some());
    assert!(cache.load_at(NOW + 60000).unwrap().is_none());
}
