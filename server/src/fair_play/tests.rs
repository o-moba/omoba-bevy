use super::*;
use std::{fs, path::PathBuf, time::Duration};

struct Worker {
    rt: ServerRuntime,
    addr: SocketAddr,
    start: Instant,
    directory: PathBuf,
}

impl Worker {
    fn new() -> Self {
        let (mut rt, addr, join) = crate::match_allocation::runtime_tests::fixture();
        let directory = rt.match_service.worker().unwrap().directory.clone();
        fs::create_dir_all(&directory).unwrap();
        let start = Instant::now();
        rt.handle_packet(addr, join, start);
        assert!(!rt.begin_career_round(start));
        let id = rt.career_allocation_for_test().unwrap().result_id;
        rt.career.backend.test_ack_start(&id);
        assert!(!rt.begin_career_round(start));
        rt.career.backend.test_ack_gameplay(&id);
        assert!(rt.begin_career_round(start));
        rt.world.game_state = GameState::Running;
        rt.match_started_at = Some(start);
        rt.observe_fair_play(start, false);
        Self {
            rt,
            addr,
            start,
            directory,
        }
    }

    fn idle(&mut self, through: u64) {
        for second in 1..=through {
            let now = self.start + Duration::from_secs(second);
            self.rt.handle_packet(self.addr, ClientPacket::Ping, now);
            self.rt.observe_fair_play(now, false);
        }
    }

    fn transform(&self, x: f32, z: f32, input_active: bool) -> ClientPacket {
        ClientPacket::Transform {
            input_active,
            x,
            y: common::balance::PLAYER_GROUND_Y,
            z,
            yaw: 0.0,
            dash_sequence: self.rt.world.players[&self.addr].hero.utility.dash_sequence,
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Only this fixture's unique allocation directory, never a shared root.
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn heartbeat_only_human_gets_ai_at_ninety_seconds_and_new_input_reclaims_same_seat() {
    let mut w = Worker::new();
    let p = &w.rt.world.players[&w.addr];
    let identity = (
        p.hero.identity.id,
        p.hero.identity.team,
        p.hero.identity.hero_class,
        p.session_id.clone(),
    );
    let original_pose = (p.hero.x, p.hero.z);
    w.idle(89);
    assert!(!w.rt.session_recovery.is_afk(w.addr));
    let now = w.start + Duration::from_secs(90);
    w.rt.handle_packet(w.addr, ClientPacket::Ping, now);
    w.rt.observe_fair_play(now, false);
    assert!(w.rt.session_recovery.is_afk(w.addr));
    assert!(w.rt.bots.profile(w.addr).is_some());
    assert!(w.rt.career.backend.authenticated_session(w.addr).is_some());

    // Simulate the actor moving under AI while the old idle pose stays buffered.
    w.rt.world.players.get_mut(&w.addr).unwrap().hero.x += 4.0;
    let idle = w.transform(original_pose.0, original_pose.1, false);
    let reclaim_at = now + Duration::from_secs(1);
    w.rt.handle_packet(w.addr, idle, reclaim_at);
    w.rt.observe_fair_play(reclaim_at, false);
    assert!(
        w.rt.session_recovery.is_afk(w.addr),
        "AI motion is not human input"
    );
    assert!(w.rt.bots.profile(w.addr).is_some());
    // A client also reflects the NEW authoritative AI pose while idle. This
    // must not be mistaken for deliberate local input either.
    let reflected_x = w.rt.world.players[&w.addr].hero.x;
    let reflected = w.transform(reflected_x, original_pose.1, false);
    w.rt.handle_packet(w.addr, reflected, reclaim_at);
    assert!(w.rt.session_recovery.is_afk(w.addr));
    let active = w.transform(original_pose.0 + 0.5, original_pose.1, true);
    w.rt.handle_packet(w.addr, active, reclaim_at);
    assert!(!w.rt.session_recovery.is_afk(w.addr));
    assert!(w.rt.bots.profile(w.addr).is_none());
    let p = &w.rt.world.players[&w.addr];
    assert_eq!(
        (
            p.hero.identity.id,
            p.hero.identity.team,
            p.hero.identity.hero_class,
            p.session_id.clone()
        ),
        identity
    );
    let mut rows = w.rt.combat_log.ledger.snapshot();
    w.rt.fair_play.annotate(&mut rows);
    let participation = rows
        .iter()
        .find(|p| p.player_id == identity.0)
        .unwrap()
        .participation
        .unwrap();
    assert!(
        participation.abandoned,
        "reclaim cannot erase the established AFK strike"
    );
    assert_eq!(participation.afk_ms, 1000);
}

#[test]
fn ai_last_seen_cannot_keep_a_disconnected_afk_endpoint_alive() {
    let mut w = Worker::new();
    w.idle(90);
    assert!(w.rt.session_recovery.is_afk(w.addr));
    let id = w.rt.world.players[&w.addr].hero.identity.id;
    let later = w.start + Duration::from_secs(96);
    w.rt.world.players.get_mut(&w.addr).unwrap().last_seen = later;
    w.rt.maintain_roster(later);
    assert!(!w.rt.session_recovery.is_afk(w.addr));
    assert!(w.rt.bots.profile(w.addr).is_none());
    assert!(
        w.rt.world
            .players
            .iter()
            .any(|(addr, p)| crate::bots::is_bot_address(*addr) && p.hero.identity.id == id)
    );
}

#[test]
fn meaningful_packets_after_an_explicit_leave_do_not_auto_reclaim_the_actor() {
    let mut w = Worker::new();
    let id = w.rt.world.players[&w.addr].hero.identity.id;
    let pose = w.transform(1.0, 1.0, true);
    w.rt.handle_packet(w.addr, ClientPacket::Leave, w.start);
    w.rt.handle_packet(w.addr, pose, w.start + Duration::from_secs(1));
    assert!(
        w.rt.world
            .players
            .get(&w.addr)
            .is_none_or(|p| p.hero.identity.id != id || !p.joined)
    );
    assert!(
        w.rt.world
            .players
            .iter()
            .any(|(addr, p)| crate::bots::is_bot_address(*addr) && p.hero.identity.id == id)
    );
}

#[test]
fn all_afk_endpoints_cannot_hold_an_unattended_worker_until_the_hard_cap() {
    let mut w = Worker::new();
    w.idle(90);
    let now = w.start + Duration::from_secs(90);
    w.rt.enforce_match_lifecycle(now);
    w.rt.enforce_match_lifecycle(now + Duration::from_secs(211));
    assert!(w.rt.match_service.worker().unwrap().aborted);
    assert!(!w.rt.career_allocation_for_test().unwrap().rated);
    let result = w.rt.career_allocation_for_test().unwrap();
    assert_eq!(result.unrated_reason.as_deref(), Some("abandoned_reaped"));
    assert!(
        result
            .participants
            .iter()
            .any(|p| p.participation.is_some_and(|v| v.abandoned))
    );
}

#[test]
fn an_unplayed_later_draft_cannot_inherit_previous_round_participation() {
    let mut w = Worker::new();
    w.idle(90);
    assert!(!w.rt.fair_play.humans.is_empty());
    w.rt.match_started_at = None;
    w.rt.observe_fair_play(w.start + Duration::from_secs(91), false);
    assert!(w.rt.fair_play.humans.is_empty());
    assert!(w.rt.fair_play.round.is_none());
}
