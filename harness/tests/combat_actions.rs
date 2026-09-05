//! Real-UDP verification for authoritative cosmetic combat actions.

use std::time::{Duration, Instant};

use harness::{Bot, Character, PlayerActionKind, ServerPacket, ServerProcess, TargetId, Team};

const TIMEOUT: Duration = Duration::from_secs(5);
const GROUND_Y: f32 = 0.5;

fn distance(a: &harness::PlayerState, b: &harness::PlayerState) -> f32 {
    ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

fn walk_into_range(observer: &mut Bot, caster: &Bot, observer_id: u64, caster_id: u64) {
    observer.set_speed_boost(true);
    caster.set_speed_boost(true);
    let deadline = Instant::now() + Duration::from_secs(40);
    while Instant::now() < deadline {
        observer.send_transform(0.0, GROUND_Y, 0.0, 0.0);
        caster.send_transform(0.0, GROUND_Y, 0.0, 0.0);
        if let Some(snapshot) = observer.recv_snapshot(Instant::now() + TIMEOUT)
            && let (Some(observer_state), Some(caster_state)) =
                (snapshot.player(observer_id), snapshot.player(caster_id))
            && distance(observer_state, caster_state) < 8.0
        {
            return;
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
