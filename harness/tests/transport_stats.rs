//! Load evidence must count actual received wire bytes, including packets that
//! are rejected by framing, JSON or snapshot-order checks.
use harness::{Bot, SnapshotView};
use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

#[test]
fn wire_counters_cover_raw_blocking_drained_and_rejected_packets_once() {
    let fixture = UdpSocket::bind("127.0.0.1:0").unwrap();
    fixture
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bot = Bot::connect(fixture.local_addr().unwrap());
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
}
