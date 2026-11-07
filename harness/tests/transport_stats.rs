//! Load evidence must count actual received wire bytes, including packets that
//! are rejected by framing, JSON or snapshot-order checks.
use harness::{Bot, SnapshotView};
use std::{
    net::UdpSocket,
    thread,
    time::{Duration, Instant},
};

fn late_datagram_does_not_extend_poll_deadline(raw: bool) {
    let fixture = UdpSocket::bind("127.0.0.1:0").unwrap();
    fixture
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bot = Bot::connect(fixture.local_addr().unwrap());
    bot.send_raw(b"fixture");
    let (_, peer) = fixture.recv_from(&mut [0; 64]).unwrap();
    let packet = if raw {
        b"late".to_vec()
    } else {
        serde_json::to_vec(&serde_json::json!({
            "type":"snapshot", "players":[], "your_id":3,
            "protocol_version":shared::protocol::PROTOCOL_VERSION,
            "server_epoch":7, "match_id":8, "snapshot_tick":1,
        }))
        .unwrap()
    };
    let deadline = Instant::now() + Duration::from_millis(20);
    let sender = thread::spawn(move || {
        thread::sleep(
            (deadline + Duration::from_millis(80)).saturating_duration_since(Instant::now()),
        );
        fixture.send_to(&packet, peer).unwrap();
    });
    let missed_deadline = if raw {
        bot.recv_raw_datagram(deadline).is_some()
    } else {
        bot.recv_snapshot(deadline).is_some()
    };
    sender.join().unwrap();
    assert!(
        !missed_deadline,
        "20ms poll consumed a datagram sent80ms after its deadline"
    );
    // The late packet remains available for a subsequent poll; limiting one
    // idle peer must neither starve other peers nor throw away future input.
    let next = Instant::now() + Duration::from_secs(2);
    if raw {
        assert_eq!(bot.recv_raw_datagram(next), Some(b"late".to_vec()));
    } else {
        assert_eq!(bot.recv_snapshot(next).unwrap().meta().snapshot_tick, 1);
    }
}

#[test]
fn raw_idle_poll_honors_short_deadline() {
    late_datagram_does_not_extend_poll_deadline(true);
}

#[test]
fn snapshot_idle_poll_honors_short_deadline() {
    late_datagram_does_not_extend_poll_deadline(false);
}

#[test]
fn wire_counters_cover_raw_blocking_drained_and_rejected_packets_once() {
    let fixture = UdpSocket::bind("127.0.0.1:0").unwrap();
    fixture
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bot = Bot::connect(fixture.local_addr().unwrap());
    assert_eq!(bot.snapshot_meta(), None);
    bot.send_raw(b"fixture");
    let mut buffer = [0; 64];
    let (_, peer) = fixture.recv_from(&mut buffer).unwrap();
    assert_eq!(bot.transport_stats().received_bytes, 0);

    fixture.send_to(b"raw", peer).unwrap();
    assert_eq!(
        bot.recv_raw_datagram(Instant::now() + Duration::from_secs(2)),
        Some(b"raw".to_vec())
    );
    let before_timeout = bot.transport_stats();
    assert!(
        bot.recv_raw_datagram(Instant::now() + Duration::from_millis(20))
            .is_none()
    );
    assert_eq!(bot.transport_stats(), before_timeout);

    fixture.send_to(b"OMB1", peer).unwrap();
    fixture.send_to(b"?", peer).unwrap();
    assert!(
        bot.recv_snapshot(Instant::now() + Duration::from_millis(20))
            .is_none()
    );
    assert_eq!(bot.transport_stats().framing_errors, 1);
    assert_eq!(bot.transport_stats().decode_errors, 1);

    let packets = [1, 2].map(|tick| {
        serde_json::to_vec(&serde_json::json!({
            "type":"snapshot", "players":[], "your_id":3,
            "protocol_version":shared::protocol::PROTOCOL_VERSION,
            "server_epoch":7, "match_id":8, "snapshot_tick":tick,
        }))
        .unwrap()
    });
    for bytes in &packets {
        fixture.send_to(bytes, peer).unwrap();
    }
    let latest = bot
        .recv_snapshot(Instant::now() + Duration::from_secs(2))
        .unwrap();
    assert_eq!(latest.meta().snapshot_tick, 2);
    let stats = bot.transport_stats();
    assert_eq!(stats.received_datagrams, 5);
    assert_eq!(
        stats.received_bytes,
        (3 + 4 + 1 + packets.iter().map(Vec::len).sum::<usize>()) as u64
    );
    assert_eq!(stats.accepted_snapshots, 2);
    assert_eq!(bot.snapshot_meta(), Some(latest.meta()));

    fixture.send_to(&packets[1], peer).unwrap();
    assert!(
        bot.recv_snapshot(Instant::now() + Duration::from_millis(20))
            .is_none()
    );
    let after_replay = bot.transport_stats();
    assert_eq!(after_replay.received_datagrams, 6);
    assert_eq!(
        after_replay.received_bytes,
        stats.received_bytes + packets[1].len() as u64
    );
    assert_eq!(after_replay.accepted_snapshots, 2);
    assert_eq!(bot.snapshot_meta(), Some(latest.meta()));

    // A missed broadcast advances producer ticks without inventing an accepted
    // snapshot. The diagnostic must retain that gap, rather than count polls.
    let gap = serde_json::to_vec(&serde_json::json!({
        "type":"snapshot", "players":[], "your_id":3,
        "protocol_version":shared::protocol::PROTOCOL_VERSION,
        "server_epoch":7, "match_id":8, "snapshot_tick":5,
    }))
    .unwrap();
    fixture.send_to(&gap, peer).unwrap();
    bot.recv_snapshot(Instant::now() + Duration::from_secs(2))
        .unwrap();
    assert_eq!(bot.transport_stats().accepted_snapshots, 3);
    assert_eq!(bot.snapshot_meta().unwrap().snapshot_tick, 5);
}
