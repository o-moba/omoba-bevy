//! Headless gameplay scenarios driven against the real UDP server.
//!
//! Each test owns a fresh [`ServerProcess`] on a unique port (cleaned up on
//! drop), connects one or two [`Bot`]s, drives them over the wire, and asserts
//! on snapshot state. Scenarios poll with timeouts instead of fixed sleeps for
//! their assertions so they stay deterministic.
//!
//! Run with: `cargo test -p harness -- --test-threads=1`
//! (sequential keeps several spawned servers from competing for the CPU during
//! a cold build; each server already uses its own port so parallel is also
//! safe).

use std::time::{Duration, Instant};

use harness::{Bot, Character, ServerProcess, Team};
use harness::protocol::PlayerState;

// --- Constants mirrored from server balance (source of truth in
// `server/src/balance.rs`). Used only to shape inputs and pick thresholds. ---

/// `PLAYER_GROUND_Y` — fixed ground plane height the server snaps players to.
const GROUND_Y: f32 = 0.5;
/// `MAX_HP` — full player health.
const MAX_HP: f32 = 100.0;
/// `SPELL_CAST_RANGE` — horizontal cast range (plus a small hit radius). We
/// bring players closer than this before casting.
const SPELL_CAST_RANGE: f32 = 28.0;

/// Generous default for waiting on a specific snapshot condition.
const POLL_TIMEOUT: Duration = Duration::from_secs(3);

/// Horizontal (XZ) distance between two players.
fn xz_distance(a: &PlayerState, b: &PlayerState) -> f32 {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    (dx * dx + dz * dz).sqrt()
}

/// Brings two enemy bots from their far apart spawns to within cast range by
/// walking both toward the map origin (with the debug speed boost so it is
/// quick). Reads positions from `observer`'s snapshots. Panics on timeout.
fn walk_into_cast_range(observer: &mut Bot, mover: &Bot, observer_id: u64, mover_id: u64) {
    observer.set_speed_boost(true);
    mover.set_speed_boost(true);

    let target_gap = SPELL_CAST_RANGE - 4.0; // comfortably inside range
    let deadline = Instant::now() + Duration::from_secs(40);

    loop {
        assert!(
            Instant::now() < deadline,
            "players never converged into cast range"
        );

        // Drive both toward the origin; the server clamps each step.
        observer.send_transform(0.0, GROUND_Y, 0.0, 0.0);
        mover.send_transform(0.0, GROUND_Y, 0.0, 0.0);

        let Some(packet) = observer.recv_snapshot(Instant::now() + POLL_TIMEOUT) else {
            continue;
        };
        let (Some(a), Some(b)) = (packet.player(observer_id), packet.player(mover_id)) else {
            continue;
        };
        if xz_distance(a, b) <= target_gap {
            break;
        }
    }
}

/// Polls snapshots until a second player (id != `self_id`) appears and returns
/// its id. Panics on timeout.
fn poll_other_player_id(bot: &mut Bot, self_id: u64) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(5);
    while let Some(packet) = bot.recv_snapshot(deadline) {
        if let Some(other) = packet.players().iter().find(|p| p.id != self_id) {
            return other.id;
        }
    }
    panic!("a second player never appeared in a snapshot");
}

#[test]
fn join_produces_snapshot_with_player() {
    let server = ServerProcess::spawn();
    let mut bot = Bot::connect(server.addr());

    bot.join(Team::Green, Character::Ipfs);
    let id = bot.my_id(POLL_TIMEOUT);

    let me = bot
        .wait_for_player(id, |p| p.max_hp > 0.0 && p.hp >= p.max_hp, POLL_TIMEOUT)
        .expect("joined bot should appear in a snapshot at full hp");

    assert_eq!(me.id, id);
    assert!((me.hp - MAX_HP).abs() < 0.001, "expected full hp, got {}", me.hp);
    assert_eq!(me.team, Some(Team::Green));
}

#[test]
fn god_mode_keeps_player_alive_under_damage() {
    let server = ServerProcess::spawn();
    let mut victim = Bot::connect(server.addr()); // A (green)
    let attacker = Bot::connect(server.addr()); // B (blue)

    victim.join(Team::Green, Character::Ipfs);
    attacker.join(Team::Blue, Character::Ipfs);

    let victim_id = victim.my_id(POLL_TIMEOUT);

    // Learn the attacker's id from the victim's snapshot: it lists all players,
    // so the non-victim id is the attacker. Poll until both have joined.
    let attacker_id = poll_other_player_id(&mut victim, victim_id);

    walk_into_cast_range(&mut victim, &attacker, victim_id, attacker_id);

    // --- Phase 1: god mode ON -> attacker casts for ~2s -> hp must hold. ---
    victim.set_god_mode(true);
    let phase1_end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < phase1_end {
        attacker.cast_player(victim_id);
        victim.ping(); // keep-alive so the idle victim does not time out
        if let Some(p) = victim.latest_player(victim_id, Duration::from_millis(200)) {
            assert!(
                p.hp >= MAX_HP - 0.001,
                "god mode should prevent all damage, but hp fell to {}",
                p.hp
            );
        }
    }

    // --- Phase 2 (control): god mode OFF -> the SAME casts must draw blood. ---
    victim.set_god_mode(false);
    let phase2_end = Instant::now() + Duration::from_secs(6);
    let mut took_damage = false;
    while Instant::now() < phase2_end {
        attacker.cast_player(victim_id);
        if let Some(p) = victim.latest_player(victim_id, Duration::from_millis(200))
            && p.hp < MAX_HP - 0.5
        {
            took_damage = true;
            break;
        }
    }
    assert!(
        took_damage,
        "control case failed: without god mode the victim never lost hp, \
         so the god-mode assertion would be meaningless"
    );
}

#[test]
fn speed_boost_widens_movement_clamp() {
    let server = ServerProcess::spawn();

    // Two same-team bots so neither interferes with the other's movement.
    let mut slow = Bot::connect(server.addr());
    let mut fast = Bot::connect(server.addr());
    slow.join(Team::Green, Character::Ipfs);
    fast.join(Team::Green, Character::Ipfs);

    let slow_id = slow.my_id(POLL_TIMEOUT);
    let fast_id = fast.my_id(POLL_TIMEOUT);
    fast.set_speed_boost(true);

    // Identical drive loops; only the speed-boost flag differs.
    let slow_adv = drive_forward(&mut slow, slow_id);
    let fast_adv = drive_forward(&mut fast, fast_id);

    // The unboosted run is clamped well short of the huge requested step.
    assert!(slow_adv > 0.0, "unboosted player should still advance");
    assert!(
        slow_adv < 50.0,
        "unboosted run should be clamped far below the requested step, got {slow_adv}"
    );
    // The boosted run accepts a wider clamp and reaches meaningfully further.
    assert!(
        fast_adv > slow_adv * 1.5,
        "speed boost should widen the clamp: boosted={fast_adv} vs unboosted={slow_adv}"
    );
}

/// Sends a large +x transform repeatedly at a fixed cadence and returns how far
/// the server actually let the player advance along x.
fn drive_forward(bot: &mut Bot, id: u64) -> f32 {
    let start = bot
        .latest_player(id, POLL_TIMEOUT)
        .expect("player should be present before driving");
    let start_x = start.x;
    let start_z = start.z;

    for _ in 0..30 {
        // Request a far target so the server is always the limiting factor.
        bot.send_transform(start_x + 1000.0, GROUND_Y, start_z, 0.0);
        // Input-pacing sleep (not an assertion wait): spaces transforms so each
        // is a distinct tick. The boosted/unboosted ratio is load-insensitive —
        // both runs use this same cadence, so they scale together.
        std::thread::sleep(Duration::from_millis(40));
    }

    let end = bot
        .latest_player(id, POLL_TIMEOUT)
        .expect("player should still be present after driving");
    end.x - start_x
}

#[test]
fn upgrade_skill_without_points_is_noop() {
    let server = ServerProcess::spawn();
    let mut bot = Bot::connect(server.addr());

    bot.join(Team::Green, Character::Ipfs);
    let id = bot.my_id(POLL_TIMEOUT);

    // Fresh player: zero skill points, all ranks at the base value of 1.
    let before = bot
        .latest_player(id, POLL_TIMEOUT)
        .expect("player snapshot");
    assert_eq!(before.skill_points, 0, "fresh player should have no points");
    assert_eq!(before.ranks, [1; 4], "fresh player ranks should all be 1");

    // Requesting an upgrade with no points must be a no-op on the server.
    bot.upgrade_skill(0);
    // Settle sleep (not an assertion wait): let several snapshots flow so any
    // erroneous state change would surface before we re-read.
    std::thread::sleep(Duration::from_millis(300));
    let after = bot
        .latest_player(id, POLL_TIMEOUT)
        .expect("player snapshot");

    assert_eq!(after.ranks[0], 1, "rank must not rise without a skill point");
    assert_eq!(after.skill_points, 0, "skill points must stay at zero");

    // TODO: the real consume-and-cap path (spend a point -> rank rises and is
    // deducted, capped at MAX_SKILL_RANK -> projectile hits harder) depends on
    // long-form leveling and belongs in a server-side unit test (see the
    // skill-point/rank tests in `server/src/main.rs`).
}
