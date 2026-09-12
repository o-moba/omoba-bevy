//! Real-UDP verification for authoritative cosmetic combat actions.

use std::time::{Duration, Instant};

use harness::navigation::BotNavigator;
use harness::{Bot, Character, PlayerActionKind, ServerPacket, ServerProcess, TargetId, Team};
use shared::navigation::Disc;

const TIMEOUT: Duration = Duration::from_secs(5);
const GROUND_Y: f32 = 0.5;

fn distance(a: &harness::PlayerState, b: &harness::PlayerState) -> f32 {
    ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

fn walk_into_range(observer: &mut Bot, caster: &Bot, observer_id: u64, caster_id: u64) {
    observer.set_speed_boost(true);
    caster.set_speed_boost(true);
    let mut routes: [BotNavigator; 2] = Default::default();
    let deadline = Instant::now() + Duration::from_secs(40);
    while Instant::now() < deadline {
        observer.ping();
        caster.ping();
        let Some(snapshot) = observer.recv_snapshot(deadline.min(Instant::now() + TIMEOUT)) else {
            continue;
        };
        let (Some(observer_state), Some(caster_state)) =
            (snapshot.player(observer_id), snapshot.player(caster_id))
        else {
            continue;
        };
        if distance(observer_state, caster_state) < 8.0 {
            return;
        }
        // The server clips live structure discs as well as authored terrain.
        // Follow waypoints from acknowledged positions instead of repeatedly
        // requesting the origin through the friendly midlane tower.
        let structures: Vec<_> = snapshot
            .structures()
            .iter()
            .filter(|s| s.hp > 0.0)
            .map(|s| Disc {
                center: [s.x, s.z],
                radius: if s.kind == "base_tower" { 3.2 } else { 1.3 },
            })
            .collect();
        for ((route, bot), state) in routes
            .iter_mut()
            .zip([&*observer, caster])
            .zip([observer_state, caster_state])
        {
            if let Some(next) = route.next([state.x, state.z], [0.0, 0.0], &structures) {
                bot.send_transform(
                    next[0],
                    GROUND_Y,
                    next[1],
                    (next[0] - state.x).atan2(next[1] - state.z),
                );
            }
        }
    }
    panic!("players did not enter authoritative Q cast range");
}

#[test]
fn two_clients_observe_sequential_accepted_casts_once_and_defaults_are_inert() {
    let server = ServerProcess::spawn();
    let mut observer = Bot::connect(server.addr());
    let mut caster = Bot::connect(server.addr());
    observer.join(Team::Green, Character::Ipfs);
    caster.join(Team::Blue, Character::Ipfs);

    let observer_id = observer.my_id(TIMEOUT);
    let caster_id = caster.my_id(TIMEOUT);
    let baseline = observer
        .wait_for_player(caster_id, |_| true, TIMEOUT)
        .expect("remote caster should replicate");
    assert_eq!(baseline.action_sequence, 0);
    assert_eq!(baseline.action_kind, PlayerActionKind::None);
    assert_eq!(baseline.action_slot, 0);

    walk_into_range(&mut observer, &caster, observer_id, caster_id);

    caster.cast(TargetId::player(observer_id));
    let first_remote = observer
        .wait_for_player(caster_id, |p| p.action_sequence > 0, TIMEOUT)
        .expect("observer should receive the first accepted cast");
    assert_eq!(first_remote.action_kind, PlayerActionKind::Attack);
    assert_eq!(first_remote.action_slot, 0);
    let first_sequence = first_remote.action_sequence;
    let first_local = caster
        .wait_for_player(caster_id, |p| p.action_sequence == first_sequence, TIMEOUT)
        .expect("caster should receive its own authoritative action");
    assert_eq!(first_local.action_kind, PlayerActionKind::Attack);

    // The immediate duplicate is rejected by cooldown and therefore cannot
    // manufacture a cosmetic event.
    caster.cast(TargetId::player(observer_id));
    std::thread::sleep(Duration::from_millis(150));
    let rejected = observer
        .latest_player(caster_id, TIMEOUT)
        .expect("snapshot after rejected cast");
    assert_eq!(rejected.action_sequence, first_sequence);

    std::thread::sleep(Duration::from_millis(450));
    caster.cast(TargetId::player(observer_id));
    let second_remote = observer
        .wait_for_player(caster_id, |p| p.action_sequence != first_sequence, TIMEOUT)
        .expect("observer should receive a distinct second accepted cast");
    assert_eq!(second_remote.action_sequence, first_sequence + 1);
    assert_eq!(second_remote.action_kind, PlayerActionKind::Attack);
    let second_local = caster
        .wait_for_player(
            caster_id,
            |p| p.action_sequence == second_remote.action_sequence,
            TIMEOUT,
        )
        .expect("caster should observe the same second action sequence");
    assert_eq!(second_local.action_slot, 0);
}

#[test]
fn legacy_and_unknown_action_fields_decode_safely() {
    let legacy = br#"{"type":"snapshot","your_id":7,"players":[{"id":7}]}"#;
    let packet: ServerPacket =
        serde_json::from_slice(legacy).expect("legacy snapshot should decode");
    let player = packet.player(7).expect("legacy player");
    assert_eq!(player.action_sequence, 0);
    assert_eq!(player.action_kind, PlayerActionKind::None);
    assert_eq!(player.action_slot, 0);

    let future = br#"{"type":"snapshot","your_id":7,"players":[{"id":7,"action_sequence":9,"action_kind":"future_action","action_slot":99}]}"#;
    let packet: ServerPacket =
        serde_json::from_slice(future).expect("unknown future action should decode");
    let player = packet.player(7).expect("future player");
    assert_eq!(player.action_sequence, 9);
    assert_eq!(player.action_kind, PlayerActionKind::None);
    assert_eq!(player.action_slot, 99);
}
