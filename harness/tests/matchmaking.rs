//! Release-mode matchmaking scenarios against the real server (TASK-22).
//!
//! Runs the server with `OMOBA_MATCH_MODE=release` and `OMOBA_TEAM_SIZE=1`
//! so a full roster is 2 players (1v1) — release semantics, test-sized.

use std::time::{Duration, Instant};

use harness::{Bot, Character, GameState, ServerProcess, Team};

const POLL_BUDGET: Duration = Duration::from_secs(10);

/// Polls snapshots until the predicate on the match phase holds.
fn wait_for_state<F>(bot: &mut Bot, what: &str, mut predicate: F) -> GameState
where
    F: FnMut(&GameState) -> bool,
{
    let deadline = Instant::now() + POLL_BUDGET;
    let mut last_seen = GameState::Lobby;
    while Instant::now() < deadline {
        if let Some(snapshot) = bot.recv_snapshot(Instant::now() + Duration::from_millis(300)) {
            last_seen = snapshot.game_state().clone();
            if predicate(&last_seen) {
                return last_seen;
            }
        }
    }
    panic!("timed out waiting for {what}; last seen state: {last_seen:?}");
}

#[test]
fn release_mode_forms_and_starts_a_full_match() {
    let server =
        ServerProcess::spawn_with_env(&[("OMOBA_MATCH_MODE", "release"), ("OMOBA_TEAM_SIZE", "1")]);

    // First player queues: the match forms but must NOT start.
    let mut first = Bot::connect(server.addr());
    first.join(Team::Green, Character::Ipfs);
    let forming = wait_for_state(&mut first, "forming state after first join", |state| {
        matches!(state, GameState::Forming { .. })
    });
    assert_eq!(
        forming,
        GameState::Forming {
            ready: 1,
            needed: 2
        }
    );

    // Hold: a solo player never gets a running match in release mode.
    let hold_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < hold_deadline {
        if let Some(snapshot) = first.recv_snapshot(Instant::now() + Duration::from_millis(200)) {
            assert!(
                !matches!(snapshot.game_state(), GameState::Running),
                "solo player must not start a release-mode match"
            );
        }
        first.ping();
    }

    // Second player completes the roster: Starting countdown, then Running.
    let mut second = Bot::connect(server.addr());
    // Both bots ask for Green: the server must still balance to 1v1.
    second.join(Team::Green, Character::Ipfs);
    wait_for_state(
        &mut second,
        "starting countdown after full roster",
        |state| matches!(state, GameState::Starting { .. } | GameState::Running),
    );
    let running = wait_for_state(&mut second, "running after countdown", |state| {
        matches!(state, GameState::Running)
    });
    assert_eq!(running, GameState::Running);

    // Server-assigned teams are balanced despite identical requests.
    let deadline = Instant::now() + POLL_BUDGET;
    loop {
        let snapshot = second
            .recv_snapshot(Instant::now() + Duration::from_millis(300))
            .expect("snapshot while running");
        let players = snapshot.players();
        if players.len() == 2 {
            let greens = players
                .iter()
                .filter(|p| p.team == Some(Team::Green))
                .count();
            let blues = players
                .iter()
                .filter(|p| p.team == Some(Team::Blue))
                .count();
            assert_eq!((greens, blues), (1, 1), "teams must balance to 1v1");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for both players in a snapshot"
        );
    }
}
