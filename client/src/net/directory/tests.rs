use super::*;
use crate::game_directory::{DirectoryTrust, PublicPin, admission::Admission};
use ed25519_dalek::{Signer, SigningKey};
use shared::{
    compatibility::ReleaseContract,
    game_directory::{Directory, Lobby, LobbyStatus, Policy, SignedDirectory},
    region::RegionId,
};
use std::{fs, net::UdpSocket};

const NOW: u64 = 1_800_000_000_000;
fn now() -> u64 {
    NOW
}
fn expired() -> u64 {
    NOW + 60_001
}
struct Fixture {
    root: PathBuf,
    sockets: [UdpSocket; 2],
    document: Directory,
}
impl Fixture {
    fn new() -> Self {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).unwrap();
        let suffix = bytes.iter().map(|n| format!("{n:02x}")).collect::<String>();
        let root = std::env::temp_dir().join(format!("omoba-routing-{suffix}"));
        fs::create_dir(&root).unwrap();
        let sockets = [
            UdpSocket::bind("127.0.0.1:0").unwrap(),
            UdpSocket::bind("127.0.0.1:0").unwrap(),
        ];
        let lobbies = sockets
            .iter()
            .enumerate()
            .map(|(i, socket)| Lobby {
                id: format!("lobby-{i}"),
                location: "loopback".into(),
                // Deliberately unresolvable: production transport must get the IP.
                host: format!("no-dns-{suffix}-{i}.dev.omoba.ru"),
                addresses: vec!["127.0.0.1".parse().unwrap()],
                port: socket.local_addr().unwrap().port(),
                contract: ReleaseContract::current(),
                status: LobbyStatus::Open,
                weight: 1,
            })
            .collect();
        let document = Directory {
            version: 1,
            region: RegionId::Ru,
            contour: "dev".into(),
            sequence: 1,
            issued_at_ms: NOW,
            expires_at_ms: NOW + 60_000,
            policy: Policy {
                min_release: ReleaseContract::current().release,
                maintenance: false,
                update_url: None,
                message: None,
            },
            lobbies,
        };
        Self {
            root,
            sockets,
            document,
        }
    }
    fn resolver(&self, ready: bool) -> Resolver {
        let key = SigningKey::from_bytes(&[7; 32]);
        let trust = DirectoryTrust::new(
            RegionId::Ru,
            "dev",
            vec![PublicPin {
                key_id: "fixture".into(),
                public_key: key.verifying_key().to_bytes(),
            }],
        )
        .unwrap();
        let mut admission = Admission::new(self.root.join("directory.json"), trust, None).unwrap();
        if ready {
            let mut signed = SignedDirectory {
                key_id: "fixture".into(),
                payload: serde_json::to_string(&self.document).unwrap(),
                signature: String::new(),
            };
            signed.signature = key
                .sign(&signed.signing_bytes().unwrap())
                .to_bytes()
                .iter()
                .map(|n| format!("{n:02x}"))
                .collect();
            admission
                .refresh_at(Ok(&serde_json::to_vec(&signed).unwrap()), NOW)
                .unwrap();
        }
        Resolver::new(admission).unwrap()
    }
    fn routing(&self, ready: bool) -> DirectoryRouting {
        let mut routing = DirectoryRouting::with_config(Ok(None));
        routing.active = true;
        routing.boot_pending = true;
        routing.clock = now;
        routing.resolver = Some(self.resolver(ready));
        routing.last_refresh = Some(Instant::now());
        routing
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn routing_app(routing: DirectoryRouting) -> App {
    let mut app = App::new();
    app.insert_resource(routing)
        .init_resource::<ClientSession>()
        .init_resource::<super::super::recovery::ResumeMatchState>()
        .add_message::<SessionUiCommand>()
        .add_systems(Update, poll_directory);
    app
}
fn lock_in() -> super::super::NetworkCommand {
    super::super::NetworkCommand::JoinPrematch {
        handheld: Default::default(),
        character: crate::team::CharacterChoice::Ipfs,
        hero_class: shared::HeroClass::Warrior,
        avatar: None,
        sprite_character: None,
    }
}
#[test]
fn pending_or_malformed_authority_never_opens_legacy_transport() {
    let fixture = Fixture::new();
    let mut app = routing_app(fixture.routing(false));
    app.update();
    assert!(
        !app.world()
            .contains_resource::<super::super::transport::NetworkChannels>()
    );
    assert_eq!(
        app.world().resource::<ClientSession>().directory_notice,
        Some(DirectoryNotice::Fetching)
    );
    let routing = DirectoryRouting::with_config(Err(DirectoryError::CacheInvalid));
    assert!(routing.configured());
    assert!(routing.resolver.is_none());
    let mut app = routing_app(routing);
    app.world_mut().resource_mut::<DirectoryRouting>().active = true;
    app.update();
    assert!(
        !app.world()
            .contains_resource::<super::super::transport::NetworkChannels>()
    );
    assert_eq!(
        app.world().resource::<ClientSession>().directory_notice,
        Some(DirectoryNotice::Unavailable)
    );
}
#[test]
fn boot_uses_signed_concrete_ip_despite_unresolvable_label() {
    let fixture = Fixture::new();
    let endpoint = fixture.sockets[0].local_addr().unwrap().to_string();
    let mut app = routing_app(fixture.routing(true));
    app.update();
    let session = app.world().resource::<ClientSession>();
    assert_eq!(session.server_addr(), endpoint);
    assert_eq!(session.directory_notice, None);
    assert!(
        app.world()
            .contains_resource::<super::super::transport::NetworkChannels>()
    );
    fixture.sockets[0]
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut packet = [0; 4096];
    let (length, peer) = fixture.sockets[0].recv_from(&mut packet).unwrap();
    assert!(length > 0);
    assert!(peer.ip().is_loopback());
}
#[test]
fn resume_selects_original_owner_before_new_search_policy_and_normalizes_hint() {
    let mut fixture = Fixture::new();
    fixture.document.policy.maintenance = true;
    let owner = &fixture.document.lobbies[1];
    let saved = super::super::recovery::SavedResume {
        allocation: shared::match_service::MatchAllocation {
            allocation_id: "a".repeat(32),
            endpoint: format!("{}:41000", owner.host),
            preference: Default::default(),
            team: shared::map::Team::Blue,
            human_count: 1,
            bot_count: 9,
            rated: false,
            join_deadline_ms: 1,
        },
        lobby: owner.display_endpoint(),
        session_id: "same-profile".into(),
        server_epoch: 7,
        match_id: 9,
        join: super::super::session::CommittedJoin::for_test(),
    };
    let mut routing = fixture.routing(true);
    routing.hint = Some(saved.clone());
    let mut app = routing_app(routing);
    app.update();
    let session = app.world().resource::<ClientSession>();
    assert_eq!(
        session.server_addr(),
        fixture.sockets[1].local_addr().unwrap().to_string()
    );
    assert_eq!(session.directory_notice, Some(DirectoryNotice::Maintenance));
    let hint = app
        .world()
        .resource::<super::super::recovery::ResumeMatchState>()
        .saved
        .as_ref()
        .unwrap();
    assert_eq!(hint.lobby, session.server_addr());
    assert_eq!(hint.allocation.endpoint, "127.0.0.1:41000");
    assert_eq!(hint.server_epoch, saved.server_epoch);
    assert_eq!(hint.join, saved.join);
}
#[test]
fn expired_worker_authority_keeps_pending_handoff_without_transport() {
    let fixture = Fixture::new();
    let mut routing = fixture.routing(true);
    let owner = &fixture.document.lobbies[1];
    routing.selected = Some(
        match routing
            .resolver
            .as_ref()
            .unwrap()
            .admission()
            .owner_at(NOW, &owner.display_endpoint())
        {
            Decision::Route(route) => route,
            _ => panic!("missing owner"),
        },
    );
    routing.boot_pending = false;
    assert_eq!(
        routing.worker("127.0.0.1:41000"),
        Some("127.0.0.1:41000".into())
    );
    routing.clock = expired;
    assert_eq!(routing.worker("127.0.0.1:41000"), None);
    routing.defer_worker("127.0.0.1:41000");
    assert!(routing.pending_handoff());
    let mut app = routing_app(routing);
    app.update();
    assert!(app.world().resource::<DirectoryRouting>().pending_handoff());
    assert!(
        !app.world()
            .contains_resource::<super::super::transport::NetworkChannels>()
    );
}
#[test]
fn maintenance_blocks_new_search_but_preserves_fifo_cancel_and_resume() {
    let mut fixture = Fixture::new();
    fixture.document.policy.maintenance = true;
    let mut routing = fixture.routing(true);
    routing.boot_pending = false;
    let mut session = ClientSession {
        state: super::super::ClientConnectionState::Connected,
        ..default()
    };
    let mut flow = crate::match_service::MatchServiceClient::default();
    let join = lock_in();
    assert!(routing.rejects(&join, &session, Some(&flow)));
    let find = super::super::NetworkCommand::Career(shared::career::CareerRequest::FindMatch {
        request_id: 1,
        preference: Default::default(),
        also_casual: false,
    });
    assert!(routing.rejects(&find, &session, Some(&flow)));
    let view = shared::career::CareerView {
        match_service: Some(shared::match_service::MatchServiceView::Idle),
        ..default()
    };
    assert!(flow.intercept_join(&view, "127.0.0.1:4000", &join));
    assert!(
        !routing.rejects(&find, &session, Some(&flow)),
        "same request must preserve FIFO"
    );
    assert!(!routing.rejects(
        &super::super::NetworkCommand::Career(shared::career::CareerRequest::CancelQueue),
        &session,
        Some(&flow)
    ));
    let launch = super::super::NetworkCommand::Party(shared::party::PartyCommand::Launch {
        preference: Default::default(),
    });
    assert!(routing.rejects(&launch, &session, Some(&flow)));
    routing.defer_worker("127.0.0.1:41000");
    assert!(
        routing.rejects(&join, &session, Some(&flow)),
        "worker Join must not leak into owning lobby"
    );
    routing.cancel_handoff();
    session.ephemeral_endpoint = true;
    assert!(
        !routing.rejects(&join, &session, Some(&flow)),
        "existing authorized arena is independent of new-search policy"
    );
}

#[test]
fn removed_owner_cannot_borrow_another_lobbys_worker_authority() {
    let mut fixture = Fixture::new();
    let mut routing = fixture.routing(true);
    let hint = fixture.document.lobbies[1].display_endpoint();
    routing.selected = Some(
        match routing
            .resolver
            .as_ref()
            .unwrap()
            .admission()
            .owner_at(NOW, &hint)
        {
            Decision::Route(route) => route,
            _ => panic!("missing owner"),
        },
    );
    fixture.document.sequence += 1;
    fixture.document.lobbies.pop();
    routing.resolver = Some(fixture.resolver(true));
    assert_eq!(
        routing.worker("127.0.0.1:41000"),
        None,
        "same IP in another lobby is not retained-owner authority"
    );
}

#[test]
fn command_dispatch_guard_runs_before_new_search_interception() {
    use super::super::{
        NetworkCommand, commands::send_network_commands, transport::NetworkChannels,
    };
    let mut fixture = Fixture::new();
    fixture.document.policy.maintenance = true;
    let mut routing = fixture.routing(true);
    routing.boot_pending = false;
    let (outgoing, packets) = crossbeam_channel::unbounded();
    let (_, incoming) = crossbeam_channel::unbounded();
    let (_, signals) = crossbeam_channel::unbounded();
    let mut app = App::new();
    let session = ClientSession {
        state: super::super::ClientConnectionState::Connected,
        ..default()
    };
    let mut career = crate::career::CareerClient::default();
    career.view.match_service = Some(shared::match_service::MatchServiceView::Idle);
    app.insert_resource(routing)
        .insert_resource(session)
        .insert_resource(career)
        .insert_resource(NetworkChannels {
            gameplay_signer: Default::default(),
            outgoing,
            incoming,
            signals,
        })
        .init_resource::<crate::persistence::ClientSessionId>()
        .init_resource::<crate::match_service::MatchServiceClient>()
        .add_message::<NetworkCommand>()
        .add_systems(Update, send_network_commands);
    app.world_mut().write_message(lock_in());
    app.update();
    assert!(
        !app.world()
            .resource::<crate::match_service::MatchServiceClient>()
            .active
    );
    assert!(packets.try_recv().is_err());
    fixture.document.sequence += 1;
    fixture.document.policy.maintenance = false;
    app.world_mut().resource_mut::<DirectoryRouting>().resolver = Some(fixture.resolver(true));
    app.world_mut().write_message(lock_in());
    app.update();
    assert!(
        app.world()
            .resource::<crate::match_service::MatchServiceClient>()
            .active
    );
    assert!(
        packets.try_recv().is_err(),
        "an accepted lock-in starts the ordinary service flow, not a direct arena Join"
    );
}

#[test]
fn lifecycle_retry_preserves_pending_allocation_until_authority_refreshes() {
    use super::super::{session::update_session_lifecycle, transport::NetworkChannels};
    let mut fixture = Fixture::new();
    let worker = UdpSocket::bind("127.0.0.1:0").unwrap();
    let endpoint = worker.local_addr().unwrap().to_string();
    let raw = format!(
        "{}:{}",
        fixture.document.lobbies[0].host,
        worker.local_addr().unwrap().port()
    );
    let owner = fixture.sockets[0].local_addr().unwrap().to_string();
    let mut routing = fixture.routing(true);
    routing.boot_pending = false;
    routing.selected = Some(
        match routing
            .resolver
            .as_ref()
            .unwrap()
            .admission()
            .owner_at(NOW, &owner)
        {
            Decision::Route(route) => route,
            _ => panic!("missing owner"),
        },
    );
    routing.clock = expired;
    let mut app = routing_app(routing);
    app.init_resource::<super::super::NetIncomingDisconnected>()
        .init_resource::<super::super::NetworkState>()
        .init_resource::<super::super::GameStateSnapshot>()
        .init_resource::<crate::team::TeamSelection>()
        .init_resource::<crate::camera::CameraState>()
        .init_resource::<crate::match_service::MatchServiceClient>()
        .add_systems(Update, update_session_lifecycle.after(poll_directory));
    let (outgoing, _packets) = crossbeam_channel::unbounded();
    let (_incoming, incoming) = crossbeam_channel::unbounded();
    let (_signals, signals) = crossbeam_channel::unbounded();
    app.insert_resource(NetworkChannels {
        gameplay_signer: Default::default(),
        outgoing,
        incoming,
        signals,
    });
    {
        let mut session = app.world_mut().resource_mut::<ClientSession>();
        session.state = super::super::ClientConnectionState::Connected;
        session.server_addr_display = owner.clone();
    }
    {
        let mut flow = app
            .world_mut()
            .resource_mut::<crate::match_service::MatchServiceClient>();
        flow.active = true;
        flow.lobby_addr = Some(owner.clone());
        flow.pending_join = Some(lock_in());
        flow.allocation = Some(shared::match_service::MatchAllocation {
            allocation_id: "b".repeat(32),
            endpoint: raw.clone(),
            preference: Default::default(),
            team: shared::map::Team::Blue,
            human_count: 1,
            bot_count: 9,
            rated: false,
            join_deadline_ms: 1,
        });
    }
    app.world_mut()
        .write_message(SessionUiCommand::ConnectAllocated(raw.clone()));
    app.update();
    assert_eq!(app.world().resource::<ClientSession>().server_addr(), owner);
    assert!(app.world().resource::<DirectoryRouting>().pending_handoff());
    app.world_mut().write_message(SessionUiCommand::Retry);
    app.update();
    assert!(
        app.world().resource::<DirectoryRouting>().pending_handoff(),
        "Retry must not lose an allocated seat"
    );
    assert_eq!(app.world().resource::<ClientSession>().server_addr(), owner);
    fixture.document.sequence += 1;
    fixture.document.expires_at_ms = NOW + 120_000;
    fixture.document.policy.maintenance = true;
    app.world_mut().resource_mut::<DirectoryRouting>().resolver = Some(fixture.resolver(true));
    app.update();
    assert!(!app.world().resource::<DirectoryRouting>().pending_handoff());
    let session = app.world().resource::<ClientSession>();
    assert!(session.ephemeral_endpoint);
    assert_eq!(session.server_addr(), endpoint);
    assert_eq!(session.directory_notice, None);
    let flow = app
        .world()
        .resource::<crate::match_service::MatchServiceClient>();
    assert_eq!(flow.lobby_addr.as_deref(), Some(owner.as_str()));
    assert_eq!(flow.allocation.as_ref().unwrap().endpoint, endpoint);
    assert_eq!(
        flow.allocation.as_ref().unwrap().allocation_id,
        "b".repeat(32)
    );
}

#[test]
fn returning_party_keeps_original_lobby_through_maintenance_and_weight_changes() {
    let mut fixture = Fixture::new();
    let owner = fixture.sockets[1].local_addr().unwrap().to_string();
    let mut routing = fixture.routing(true);
    routing.selected = Some(
        match routing
            .resolver
            .as_ref()
            .unwrap()
            .admission()
            .owner_at(NOW, &owner)
        {
            Decision::Route(route) => route,
            _ => panic!("missing owner"),
        },
    );
    fixture.document.sequence += 1;
    fixture.document.policy.maintenance = true;
    fixture.document.lobbies[1].status = LobbyStatus::Maintenance;
    fixture.document.lobbies[1].weight = 10_000;
    routing.resolver = Some(fixture.resolver(true));
    let mut session = ClientSession::default();
    routing.begin(&mut session, "existing-party", true);
    let mut app = routing_app(routing);
    app.insert_resource(session);
    app.update();
    assert_eq!(app.world().resource::<ClientSession>().server_addr(), owner);
    assert_eq!(
        app.world().resource::<ClientSession>().directory_notice,
        Some(DirectoryNotice::Maintenance)
    );
    assert_eq!(
        app.world().resource::<DirectoryRouting>().owned_lobby(),
        Some(owner)
    );
}
