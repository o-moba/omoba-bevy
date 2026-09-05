//! Live lane-push scenario (TASK-23): a brain-driven bot on a real dev-mode
//! server measurably progresses along its lane from spawn toward the enemy
//! base, with every movement step accepted by the server's movement
//! authority (positions come back from snapshots, not local state).

use std::time::{Duration, Instant};

use harness::{
    Bot, Character, GameState, HeroClass, ServerProcess, Team,
    bot_ai::{BotBrain, Lane, WorldView, step_toward},
};

const TICK: Duration = Duration::from_millis(50);
const TEST_BUDGET: Duration = Duration::from_secs(12);
const MIN_PROGRESS: f32 = 20.0;

#[test]
fn brain_driven_bot_pushes_its_lane_on_a_live_server() {
    let server = ServerProcess::spawn(); // dev mode: match starts on join
    let mut bot = Bot::connect(server.addr());
    bot.join_with_loadout(Team::Green, Character::Ipfs, HeroClass::Warrior, None);
    let my_id = bot.my_id(Duration::from_secs(5));

    // Spawn position from the first snapshot that lists us.
    let me = bot
        .latest_player(my_id, Duration::from_secs(5))
        .expect("joined bot appears in snapshots");
    let spawn = (me.x, me.z);
    let mut brain = BotBrain::new(Lane::Mid, Team::Green, HeroClass::Warrior);
    brain.resync(spawn.0, spawn.1);

    let deadline = Instant::now() + TEST_BUDGET;
    let mut position = spawn;
    let mut running_seen = false;

    while Instant::now() < deadline {
        let Some(snapshot) = bot.recv_snapshot(Instant::now() + Duration::from_millis(200)) else {
            bot.ping();
            continue;
        };
        running_seen |= matches!(snapshot.game_state(), GameState::Running);
        let Some(me) = snapshot.player(my_id) else {
            continue;
        };
        // Server-accepted position is the source of truth.
        position = (me.x, me.z);
        let view = WorldView::from_snapshot(&snapshot, my_id, Team::Green);
        let decision = brain.decide(position.0, position.1, &view);
        if let Some(target) = decision.move_target {
            let (nx, nz, yaw) = step_toward(position, target, TICK.as_secs_f32());
            bot.send_transform(nx, 0.5, nz, yaw);
        }
        if let Some(target) = decision.cast {
            bot.cast_slot(target, 0);
        }
        std::thread::sleep(TICK);
    }

    assert!(running_seen, "dev-mode match should be running");
    let dx = position.0 - spawn.0;
    let dz = position.1 - spawn.1;
    let progress = (dx * dx + dz * dz).sqrt();
    // Green Mid pushes toward +x/+z (away base). Require real lane progress
    // accepted by the server, not just local intent.
    assert!(
        progress >= MIN_PROGRESS,
        "bot should push >= {MIN_PROGRESS} units along its lane, got {progress:.1} \
         (spawn {spawn:?} -> {position:?})"
    );
    assert!(
        dx > 0.0 && dz > 0.0,
        "green mid progress must head toward the enemy base, got delta ({dx:.1}, {dz:.1})"
    );
}
