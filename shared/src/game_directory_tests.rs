use super::*;

fn directory() -> Directory {
    Directory {
        version: 1,
        region: RegionId::Ru,
        contour: "production".into(),
        sequence: 4,
        issued_at_ms: 100_000,
        expires_at_ms: 200_000,
        policy: Policy {
            min_release: "0.45.0".into(),
            maintenance: false,
            update_url: Some("https://omoba.ru/download".into()),
            message: None,
        },
        lobbies: vec![Lobby {
            id: "ru-1".into(),
            location: "moscow".into(),
            host: "lobby.omoba.ru".into(),
            addresses: vec!["8.8.8.8".parse().unwrap()],
            port: 4000,
            contract: ReleaseContract::current(),
            status: LobbyStatus::Open,
            weight: 1,
        }],
    }
}
fn valid(d: &Directory) -> bool {
    d.validate_at(RegionId::Ru, "production", 150_000, 4)
        .is_ok()
}

#[test]
fn directory_window_scope_sequence_and_boundaries_are_checked_before_routing() {
    let d = directory();
    assert!(valid(&d));
    assert!(
        d.validate_at(RegionId::Global, "production", 150_000, 4)
            .is_err()
    );
    assert!(d.validate_at(RegionId::Ru, "dev", 150_000, 4).is_err());
    assert!(
        d.validate_at(RegionId::Ru, "production", 200_000, 4)
            .is_err()
    );
    assert!(
        d.validate_at(RegionId::Ru, "production", 69_999, 4)
            .is_err()
    );
    assert!(d.validate_at(RegionId::Ru, "production", 70_000, 4).is_ok());
    assert!(
        d.validate_at(RegionId::Ru, "production", 150_000, 5)
            .is_err()
    );
    for edit in [0, 1, 2, 3] {
        let mut bad = d.clone();
        match edit {
            0 => bad.version = 2,
            1 => bad.sequence = 0,
            2 => bad.expires_at_ms = bad.issued_at_ms,
            _ => bad.expires_at_ms = bad.issued_at_ms + MAX_TTL_MS + 1,
        }
        assert!(!valid(&bad));
    }
}
#[test]
fn malformed_duplicate_or_unroutable_entries_do_not_become_candidates() {
    for host in [
        "",
        "localhost",
        "127.0.0.1",
        "10.0.0.1",
        "100.64.0.1",
        "198.18.0.1",
        "192.0.0.2",
        "192.88.99.1",
        "127.00.0.1",
        "0x7f.0.0.1",
        "169.254.1.1",
        "0.0.0.0",
        "255.255.255.255",
        "224.0.0.1",
        "::1",
        "::127.0.0.1",
        "64:ff9b::7f00:1",
        "100::1",
        "::ffff:127.0.0.1",
        "fc00::1",
        "fe80::1",
        "2001:db8::1",
        "bad..host",
        "evil.test@omoba.ru",
        "host:4000",
        "a.local",
        "-bad.omoba.ru",
        "EXAMPLE.com",
    ] {
        let mut d = directory();
        d.lobbies[0].host = host.into();
        assert!(!valid(&d), "{host}");
    }
    let mut d = directory();
    d.lobbies.push(d.lobbies[0].clone());
    assert!(!valid(&d));
    d.lobbies[1].id = "ru-2".into();
    assert!(!valid(&d), "duplicate endpoint");
    d.lobbies[1].port = 4001;
    assert!(valid(&d));
    d.lobbies[1].weight = 0;
    assert!(!valid(&d));
    d.lobbies[1].weight = 1;
    d.lobbies[1].port = 0;
    assert!(!valid(&d));
    d.lobbies = vec![d.lobbies[0].clone(); MAX_LOBBIES + 1];
    assert!(!valid(&d));
    let mut local = directory();
    local.contour = "dev".into();
    local.lobbies[0].host = "::1".into();
    local.lobbies[0].addresses = vec!["::1".parse().unwrap()];
    assert!(local.validate_at(RegionId::Ru, "dev", 150_000, 4).is_ok());
    assert_eq!(local.lobbies[0].display_endpoint(), "[::1]:4000");
    assert_eq!(
        local.lobbies[0].network_endpoints(),
        vec!["[::1]:4000".parse::<SocketAddr>().unwrap()]
    );
    let mut bad = directory();
    bad.lobbies[0].addresses.clear();
    assert!(!valid(&bad));
    bad.lobbies[0].addresses = vec!["127.0.0.1".parse().unwrap()];
    assert!(!valid(&bad));
    let mut same_target = directory();
    let mut second = same_target.lobbies[0].clone();
    second.id = "another".into();
    second.host = "another.omoba.ru".into();
    same_target.lobbies.push(second);
    assert!(!valid(&same_target));
    assert_eq!(
        directory().lobbies[0].network_endpoints(),
        vec!["8.8.8.8:4000".parse::<SocketAddr>().unwrap()]
    );
}
#[test]
fn weighted_compatible_open_lobbies_are_selected_and_closed_policies_do_not_fall_back() {
    let mut d = directory();
    let client = ReleaseContract::current();
    let mut second = d.lobbies[0].clone();
    second.id = "ru-2".into();
    second.port = 4001;
    second.weight = 3;
    d.lobbies.push(second);
    let ids: Vec<_> = (0..4)
        .map(|e| match d.select(&client, e) {
            Selection::Lobby(l) => l.id.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(ids, ["ru-1", "ru-2", "ru-2", "ru-2"]);
    d.lobbies[0].contract.protocol += 1;
    assert!(matches!(d.select(&client,0),Selection::Lobby(l) if l.id=="ru-2"));
    d.lobbies[1].status = LobbyStatus::Full;
    assert_eq!(d.select(&client, 0), Selection::Full);
    d.lobbies[1].status = LobbyStatus::Maintenance;
    assert_eq!(d.select(&client, 0), Selection::Maintenance);
    d.lobbies[1].contract.catalog.push_str("-other");
    assert_eq!(d.select(&client, 0), Selection::Incompatible);
    d.policy.maintenance = true;
    assert_eq!(d.select(&client, 0), Selection::Maintenance);
    d.policy.min_release = "999.0.0".into();
    assert_eq!(d.select(&client, 0), Selection::UpdateRequired);
    d.lobbies.clear();
    assert!(valid(&d));
    d.policy.maintenance = false;
    assert!(!valid(&d));
}
#[test]
fn client_policy_and_wire_bounds_reject_ambiguous_documents() {
    let d = directory();
    let payload = serde_json::to_string(&d).unwrap();
    assert_eq!(Directory::parse(&payload).unwrap(), d);
    assert!(
        Directory::parse(&payload.replacen("\"version\":1", "\"version\":1,\"version\":1", 1))
            .is_err()
    );
    assert!(Directory::parse(&" ".repeat(MAX_PAYLOAD_BYTES + 1)).is_err());
    for url in [
        "http://omoba.ru/download",
        "https://evil@omoba.ru/download",
        "https://omoba.ru/#bad",
    ] {
        let mut bad = d.clone();
        bad.policy.update_url = Some(url.into());
        assert!(!valid(&bad));
    }
    let mut bad = d.clone();
    bad.policy.message = Some("bad\u{0000}".into());
    assert!(!valid(&bad));
    bad.policy.message = None;
    bad.policy.min_release = "latest".into();
    assert!(!valid(&bad));
    let envelope = SignedDirectory {
        key_id: "routing-1".into(),
        payload,
        signature: "0".repeat(128),
    };
    assert_eq!(
        SignedDirectory::parse(&serde_json::to_vec(&envelope).unwrap()).unwrap(),
        envelope
    );
    let mut altered = envelope.clone();
    altered.key_id = "routing-2".into();
    assert_ne!(
        altered.signing_bytes().unwrap(),
        envelope.signing_bytes().unwrap()
    );
    assert!(SignedDirectory::parse(&vec![b' '; MAX_WIRE_BYTES + 1]).is_err());
    let mut bad = envelope;
    bad.signature = "F".repeat(128);
    assert!(SignedDirectory::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
}
