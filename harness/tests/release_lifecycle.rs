//! Real newly-built server process admission/reconnect probe. The server's
//! separate test-only UDP victory fixture covers deterministic combat/rematch.
use harness::ServerProcess;
use serde_json::{Value, json};
use shared::transport::{MAX_DATAGRAM_BYTES, SnapshotAssembler};
use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

struct Peer {
    socket: UdpSocket,
    assembler: SnapshotAssembler,
}
impl Peer {
    fn connect(server: std::net::SocketAddr) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.connect(server).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(150)))
            .unwrap();
        let peer = Self {
            socket,
            assembler: SnapshotAssembler::default(),
        };
        peer.send(json!({"type":"hello","protocol_version":shared::protocol::PROTOCOL_VERSION}));
        peer
    }
    fn send(&self, packet: Value) {
        self.socket
            .send(&serde_json::to_vec(&packet).unwrap())
            .unwrap();
    }
    fn join(&self, session: &str, team: &str, character: &str) {
        self.send(json!({"type":"join","team":team,"character":character,"session_id":session}));
    }
    fn snapshot(&mut self) -> Option<Value> {
        let mut buf = [0; 65_536];
        match self.socket.recv(&mut buf) {
            Ok(len) => {
                assert!(
                    len <= MAX_DATAGRAM_BYTES,
                    "native snapshot exceeded safe datagram budget"
                );
                self.assembler
                    .push(&buf[..len], Instant::now())
                    .unwrap()
                    .map(|payload| serde_json::from_slice(&payload).unwrap())
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                None
            }
            Err(e) => panic!("snapshot receive failed: {e}"),
        }
    }
}
fn wait(peer: &mut Peer, others: &[&Peer], predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(9);
    let mut last = None;
    while Instant::now() < deadline {
        peer.send(json!({"type":"ping"}));
        for other in others {
            other.send(json!({"type":"ping"}));
        }
        if let Some(snapshot) = peer.snapshot() {
            if predicate(&snapshot) {
                return snapshot;
            }
            last = Some(snapshot);
        }
    }
    panic!("snapshot condition not reached; last={last:?}");
}
fn own(snapshot: &Value) -> &Value {
    snapshot["players"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == snapshot["your_id"])
        .unwrap()
}

#[test]
fn launched_release_server_duplicate_reservation_conflict_and_reconnect_preserve_state() {
    let server =
        ServerProcess::spawn_with_env(&[("OMOBA_MATCH_MODE", "release"), ("OMOBA_TEAM_SIZE", "1")]);
    let mut first = Peer::connect(server.addr());
    let mut second = Peer::connect(server.addr());
    first.join("live-a", "green", "ipfs");
    second.join("live-b", "green", "wang");
    let admitted = wait(&mut first, &[&second], |s| {
        s["game_state"]["type"] == "running"
    });
    let id = admitted["your_id"].clone();
    let start = own(&admitted);
    first.send(
        json!({"type":"transform","x":start["x"].as_f64().unwrap()+0.5,
        "y":0.5,"z":start["z"],"yaw":0.75}),
    );
    let moved = wait(&mut first, &[&second], |s| {
        s["players"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == id && p["yaw"] == 0.75)
    });
    let before = own(&moved).clone();
    first.join("replacement-identity", "blue", "cube");
    let duplicate = wait(&mut first, &[&second], |s| {
        s["snapshot_tick"].as_u64() > moved["snapshot_tick"].as_u64()
    });
    assert_eq!(
        own(&duplicate),
        &before,
        "duplicate Join mutated authoritative gameplay"
    );
    let mut conflict = Peer::connect(server.addr());
    conflict.join("live-a", "blue", "cube");
    wait(&mut conflict, &[&first, &second], |s| {
        s["join_error"] == "session_active"
    });
    drop(conflict);
    drop(first);
    wait(&mut second, &[], |s| {
        s["players"].as_array().unwrap().len() == 1
    });
    let mut replacement = Peer::connect(server.addr());
    replacement.join("replacement", "green", "cube");
    wait(&mut replacement, &[&second], |s| {
        s["join_error"] == "match_full"
    });
    let mut reclaimed = Peer::connect(server.addr());
    reclaimed.join("live-a", "blue", "cube");
    let restored = wait(&mut reclaimed, &[&second], |s| {
        s["your_id"] == id && s["players"].as_array().unwrap().len() == 2
    });
    assert_eq!(
        own(&restored),
        &before,
        "reclaim reinitialized gameplay or loadout"
    );
    assert_eq!(restored["join_error"], Value::Null);
    assert_eq!(restored["match_id"], admitted["match_id"]);
    assert!(restored["snapshot_tick"].as_u64() > duplicate["snapshot_tick"].as_u64());
}
