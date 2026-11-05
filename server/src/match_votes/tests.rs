use super::*;
use crate::{
    match_allocation::{AllocatedHuman, Manifest, Worker, unix_ms},
    match_rules::{MatchConfig, MatchMode},
    match_service::MatchService,
};
use shared::{career::ProfileSummary, match_service::MatchPreference, wire::ClientPacket};

struct Game {
    rt: ServerRuntime,
    start: Instant,
    peers: Vec<SocketAddr>,
    directory: std::path::PathBuf,
}
impl Game {
    fn new() -> Self {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut rt = ServerRuntime::new(
            socket,
            MatchConfig {
                mode: MatchMode::Release,
                team_size: 5,
            },
        );
        rt.career.backend = Box::new(crate::career_backend::MemoryCareer::test_backend(
            rt.server_epoch,
        ));
        let start = Instant::now();
        let humans: Vec<_> = (0..10)
            .map(|i| AllocatedHuman {
                profile_id: format!("{:064x}", i + 1),
                session_id: format!("vote-seat-{i}"),
                team: if i % 2 == 0 { Team::Green } else { Team::Blue },
            })
            .collect();
        let manifest = Manifest {
            version: 1,
            allocation_id: "d".repeat(32),
            endpoint: "127.0.0.1:41000".into(),
            bind: "127.0.0.1:41000".into(),
            preference: MatchPreference::Ranked,
            humans: humans.clone(),
            join_deadline_ms: unix_ms() + 180_000,
            cohort: None,
        };
        manifest.validate().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "omoba-vote-test-{}-{}",
            std::process::id(),
            rt.server_epoch
        ));
        std::fs::create_dir_all(&directory).unwrap();
        rt.match_service = MatchService::Worker(Worker {
            manifest,
            directory: directory.clone(),
            recovery: false,
            origin_epoch: 0,
            aborted: false,
            booted_at: start,
            last_status: None,
            terminal_at: None,
            saw_human: false,
            released: Vec::new(),
        });
        let peers: Vec<_> = (0..10)
            .map(|i| format!("127.0.0.1:{}", 60500 + i).parse().unwrap())
            .collect();
        for (i, human) in humans.iter().enumerate() {
            rt.career.backend.test_authenticated(
                peers[i],
                ProfileSummary::new(human.profile_id.clone(), format!("Voter{i}")),
                &human.session_id,
            );
            rt.handle_packet(
                peers[i],
                ClientPacket::Join {
                    handheld: Default::default(),
                    prematch: false,
                    team: human.team,
                    character: Default::default(),
                    hero_class: shared::HeroClass::LEGACY[i / 2],
                    avatar: None,
                    sprite_character: None,
                    session_id: Some(human.session_id.clone()),
                    passport_ticket: None,
                },
                start,
            );
        }
        assert!(rt.allocated_humans_ready());
        assert!(!rt.begin_career_round(start));
        let result = rt.career_allocation_for_test().unwrap();
        assert!(result.rated);
        rt.career.backend.test_ack_start(&result.result_id);
        assert!(!rt.begin_career_round(start));
        rt.career.backend.test_ack_gameplay(&result.result_id);
        assert!(rt.begin_career_round(start));
        rt.world.game_state = GameState::Running;
        rt.match_started_at = Some(start);
        rt.observe_fair_play(start, false);
        Self {
            rt,
            start,
            peers,
            directory,
        }
    }
    fn active_at(&mut self, seconds: u64) {
        let now = self.start + Duration::from_secs(seconds);
        for &addr in &self.peers {
            if let Some(p) = self
                .rt
                .world
                .players
                .get(&addr)
                .filter(|_| !self.rt.session_recovery.is_afk(addr))
            {
                let packet = ClientPacket::Transform {
                    input_active: true,
                    x: p.hero.x,
                    y: p.hero.y,
                    z: p.hero.z,
                    yaw: 0.,
                    dash_sequence: p.hero.utility.dash_sequence,
                };
                self.rt.handle_packet(addr, packet, now);
            }
        }
        self.rt.observe_fair_play(now, false);
        self.rt.tick_match_votes(now);
    }
    fn vote(&mut self, peer: usize, kind: Kind, action: Action, generation: u64, seconds: u64) {
        self.rt.handle_packet(
            self.peers[peer],
            ClientPacket::MatchVote {
                server_epoch: self.rt.server_epoch,
                match_id: self.rt.match_id,
                generation,
                kind,
                action,
            },
            self.start + Duration::from_secs(seconds),
        );
    }
    fn leave(&mut self, peer: usize) {
        self.rt
            .handle_packet(self.peers[peer], ClientPacket::Leave, self.start);
        self.rt.observe_fair_play(self.start, false);
    }
}
impl Drop for Game {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn remake_requires_ninety_seconds_and_a_majority_of_own_remaining_humans() {
    let mut g = Game::new();
    g.leave(0);
    for second in 1..=89 {
        g.active_at(second);
    }
    g.vote(2, Kind::Remake, Action::Start, 1, 89);
    assert!(
        g.rt.match_vote_view(g.peers[2], g.start + Duration::from_secs(89))
            .unwrap()
            .active
            .is_none()
    );
    g.active_at(90);
    g.vote(2, Kind::Remake, Action::Start, 1, 90);
    let view =
        g.rt.match_vote_view(g.peers[2], g.start + Duration::from_secs(90))
            .unwrap();
    let active = view.active.unwrap();
    assert_eq!((active.yes, active.required, active.eligible), (1, 3, 4));
    assert!(!active.initiator_name.is_empty());
    g.vote(1, Kind::Remake, Action::Yes, 1, 90); // Opponent cannot add to Green's ballot.
    g.vote(2, Kind::Remake, Action::Yes, 1, 90); // Duplicate initiator ballot.
    g.vote(4, Kind::Remake, Action::Yes, 0, 90); // Stale generation.
    assert_eq!(
        g.rt.match_votes.teams[0]
            .active()
            .unwrap()
            .counts(
                &g.rt
                    .eligible_takeover_voters(Team::Green)
                    .into_iter()
                    .map(|(_, id)| id)
                    .collect()
            )
            .yes,
        1
    );
    g.vote(4, Kind::Remake, Action::Yes, 1, 90);
    assert!(g.rt.career_allocation_is_live());
    g.vote(6, Kind::Remake, Action::Yes, 1, 90);
    let saved = g.rt.career_allocation_for_test().unwrap();
    assert_eq!(saved.outcome, MatchOutcome::Remade);
    assert_eq!(saved.end_reason.as_deref(), Some("remake"));
    assert!(!saved.rated && saved.winner.is_none());
    assert!(g.rt.match_service.worker().unwrap().aborted);
    assert_eq!(
        g.rt.world.game_state,
        GameState::Running,
        "a remake must never fabricate a base victory"
    );
    g.vote(8, Kind::Surrender, Action::Start, 2, 600);
    assert_eq!(
        g.rt.career_allocation_for_test().unwrap(),
        saved,
        "terminal result is immutable"
    );
}

#[test]
fn surrender_opens_at_ten_minutes_and_four_unique_teammates_end_it_as_a_loss() {
    let mut g = Game::new();
    for second in 1..=599 {
        g.active_at(second);
    }
    g.vote(0, Kind::Surrender, Action::Start, 1, 599);
    assert!(g.rt.match_votes.teams[0].active().is_none());
    g.active_at(600);
    g.vote(0, Kind::Surrender, Action::Start, 1, 600);
    for i in [2, 4] {
        g.vote(i, Kind::Surrender, Action::Yes, 1, 600);
    }
    assert!(g.rt.career_allocation_is_live());
    g.vote(1, Kind::Surrender, Action::Yes, 1, 600);
    assert!(g.rt.career_allocation_is_live());
    g.vote(6, Kind::Surrender, Action::Yes, 1, 600);
    let saved = g.rt.career_allocation_for_test().unwrap();
    assert_eq!(saved.outcome, MatchOutcome::Completed);
    assert_eq!(saved.winner, Some(Team::Blue));
    assert!(saved.rated);
    assert_eq!(saved.end_reason.as_deref(), Some("surrender"));
    assert_eq!(
        g.rt.world.game_state,
        GameState::Victory { winner: Team::Blue }
    );
    assert!(!g.rt.match_service.worker().unwrap().aborted);
}

#[test]
fn remake_closes_at_exactly_three_minutes_and_old_epoch_votes_do_nothing() {
    let mut g = Game::new();
    g.leave(0);
    for second in 1..=179 {
        g.active_at(second);
    }
    let now = g.start + Duration::from_secs(179);
    assert!(
        g.rt.match_vote_view(g.peers[2], now)
            .unwrap()
            .remake
            .available
    );
    g.rt.handle_packet(
        g.peers[2],
        ClientPacket::MatchVote {
            server_epoch: g.rt.server_epoch + 1,
            match_id: g.rt.match_id,
            generation: 1,
            kind: Kind::Remake,
            action: Action::Start,
        },
        now,
    );
    assert!(g.rt.match_votes.teams[0].active().is_none());
    g.vote(2, Kind::Remake, Action::Start, 1, 179);
    g.active_at(180);
    assert!(g.rt.match_votes.teams[0].active().is_none());
    assert!(
        !g.rt
            .match_vote_view(g.peers[2], g.start + Duration::from_secs(180))
            .unwrap()
            .remake
            .available
    );
    g.vote(4, Kind::Remake, Action::Yes, 1, 180);
    assert!(g.rt.career_allocation_is_live());
}

#[test]
fn changed_authentication_cannot_cast_a_retained_seats_vote_or_read_its_team_view() {
    let mut g = Game::new();
    for second in 1..=600 {
        g.active_at(second);
    }
    g.vote(0, Kind::Surrender, Action::Start, 1, 600);
    g.rt.career.backend.test_authenticated(
        g.peers[2],
        ProfileSummary::new("f".repeat(64), "Stranger".into()),
        "different-session",
    );
    let now = g.start + Duration::from_secs(600);
    assert!(g.rt.match_vote_view(g.peers[2], now).is_none());
    g.vote(2, Kind::Surrender, Action::Yes, 1, 600);
    let eligible =
        g.rt.eligible_takeover_voters(Team::Green)
            .into_iter()
            .map(|(_, id)| id)
            .collect();
    assert_eq!(
        g.rt.match_votes.teams[0]
            .active()
            .unwrap()
            .counts(&eligible)
            .yes,
        1
    );
    assert!(g.rt.career_allocation_is_live());
}
