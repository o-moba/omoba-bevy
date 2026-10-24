//! Every match ends (LR-11) against the real server process: a running match
//! past `OMOBA_MATCH_MAX_DURATION_SECS` settles as Interrupted, logs
//! `MATCH_METRIC event=duration_capped`, and the server keeps serving a fresh
//! round. Worker reaping and the lobby port release are covered by the
//! server's unit tests (`server/src/match_lifecycle.rs`, `match_pool.rs`).

use std::time::{Duration, Instant};

use harness::{Bot, Character, GameState, ServerProcess, SnapshotView, Team};

const POLL_BUDGET: Duration = Duration::from_secs(15);

/// The match id of the next Running snapshot whose id satisfies `accept`.
fn running_match(bot: &mut Bot, what: &str, mut accept: impl FnMut(u64) -> bool) -> u64 {
    let deadline = Instant::now() + POLL_BUDGET;
    let mut last = None;
    while Instant::now() < deadline {
        bot.ping();
        if let Some(snapshot) = bot.recv_snapshot(Instant::now() + Duration::from_millis(200)) {
            let match_id = snapshot.meta().match_id;
            if matches!(snapshot.game_state(), GameState::Running) && accept(match_id) {
                return match_id;
            }
            last = Some((snapshot.game_state().clone(), match_id));
        }
    }
    panic!("timed out waiting for {what}; last state and match: {last:?}");
}

#[test]
fn a_match_past_the_duration_cap_ends_interrupted_and_the_server_moves_on() {
    let server = ServerProcess::spawn_with_env(&[
        ("OMOBA_MATCH_MODE", "dev"),
        ("OMOBA_MATCH_MAX_DURATION_SECS", "2"),
    ]);
    assert!(
        server
            .wait_for_output("Match duration cap: 2 s", Duration::from_secs(5))
            .is_some(),
        "the server must announce the configured cap"
    );
    let mut bot = Bot::connect(server.addr());
    bot.join(Team::Green, Character::Ipfs);
    let first = running_match(&mut bot, "the first running match", |_| true);

    let line = server
        .wait_for_output("MATCH_METRIC event=duration_capped", POLL_BUDGET)
        .expect("a capped match logs duration_capped");
    assert!(line.contains("outcome=Interrupted"), "{line}");
    assert!(line.contains(&format!("match={first} ")), "{line}");
    assert!(line.contains("cap_secs=2"), "{line}");

    // A standalone server starts a fresh round for the players still there.
    let next = running_match(&mut bot, "a fresh round after the cap", |id| id > first);
    assert!(next > first);
}
