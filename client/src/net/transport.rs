//! UDP I/O thread: socket setup, framing, decode and the channel bridge to the main thread.

use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender, TryRecvError, unbounded};
use std::{
    io,
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use shared::protocol::{PROTOCOL_VERSION, SnapshotOrder};
use shared::transport::{SnapshotAssembler, TransportError};
use shared::wire::{ClientPacket, ServerPacket};

use crate::persistence::ResolvedServerAddressForPrefs;
use crate::session_config::{
    T_RETRY, TRANSPORT_CONSECUTIVE_RECV_ERRORS, TRANSPORT_CONSECUTIVE_SEND_ERRORS,
};

use super::session::{ClientConnectionState, ClientSession};
use super::{offline, public_transport};

/// Bound both allocation-free candidate iteration and repeated socket creation.
const MAX_RESOLVED_SERVER_ADDRESSES: usize = 16;

const NETWORK_LOOP_SLEEP: Duration = Duration::from_millis(16);

/// Largest application payload that can be carried by one IPv4 UDP datagram.
const IPV4_UDP_MAX_PAYLOAD_BYTES: usize = 65_507;

/// Storage is deliberately larger than the legal payload ceiling so a valid
/// server datagram can never be silently truncated before JSON decoding.
const SERVER_DATAGRAM_RECEIVE_CAPACITY: usize = 65_536;

const _: () = assert!(SERVER_DATAGRAM_RECEIVE_CAPACITY > IPV4_UDP_MAX_PAYLOAD_BYTES);

const DECODE_ERROR_LOG_INTERVAL: Duration = Duration::from_secs(1);

/// Signal from the UDP thread to the Bevy main thread (failure detection §3 in spec).
#[derive(Debug, Clone, Copy)]
pub enum NetThreadSignal {
    /// Recv/send error streak exceeded fixed thresholds (P3 transport rule).
    TransportFailure,
    /// A framed reply uses a different application protocol. No payload was applied.
    ProtocolMismatch,
}

pub(in crate::net) type SharedGameplaySigner =
    Arc<Mutex<Option<crate::career_identity::GameplaySigner>>>;

#[derive(Resource)]
pub(in crate::net) struct NetworkChannels {
    pub(in crate::net) gameplay_signer: SharedGameplaySigner,
    pub(in crate::net) outgoing: Sender<ClientPacket>,
    pub(in crate::net) incoming: Receiver<ServerPacket>,
    pub(in crate::net) signals: Receiver<NetThreadSignal>,
}

pub(in crate::net) fn spawn_network_transport(
    commands: &mut Commands,
    client_session: &mut ClientSession,
    server_addr: String,
) {
    client_session.clear_join_attempt();
    client_session.snapshot_order = SnapshotOrder::default();
    client_session.server_addr_display.clone_from(&server_addr);
    if !client_session.ephemeral_endpoint {
        commands.insert_resource(ResolvedServerAddressForPrefs(server_addr.clone()));
    }
    let (outgoing_tx, outgoing_rx) = crossbeam_channel::bounded::<ClientPacket>(256);
    let (incoming_tx, incoming_rx) = crossbeam_channel::bounded::<ServerPacket>(8);
    let (signal_tx, signal_rx) = unbounded::<NetThreadSignal>();

    let gameplay_signer: SharedGameplaySigner = Arc::default();
    let signer_for_thread = gameplay_signer.clone();
    let addr_for_thread = server_addr;
    if client_session.is_offline() {
        commands.insert_resource(offline::LocalPractice::new(
            outgoing_rx,
            incoming_tx,
            signal_tx,
        ));
    } else {
        commands.remove_resource::<offline::LocalPractice>();
        thread::spawn(move || {
            run_udp_client(
                addr_for_thread,
                outgoing_rx,
                incoming_tx,
                signal_tx,
                signer_for_thread,
            );
        });
    }

    client_session.state = ClientConnectionState::WaitingForServer;
    client_session.waiting_since = Some(Instant::now());
    client_session.discard_incoming_snapshots = false;
    client_session.join_flow_committed = false;
    client_session.last_qualifying_snapshot_wall = None;

    commands.insert_resource(NetworkChannels {
        gameplay_signer,
        outgoing: outgoing_tx,
        incoming: incoming_rx,
        signals: signal_rx,
    });
}

/// Resolve exactly once on the existing network thread. Each address needs a
/// socket bound to its own IP family: an IPv4 wildcard cannot connect to IPv6.
/// UDP connect checks local routing, not server reachability; the normal snapshot
/// timeout still decides whether the selected server is actually responding.
fn connect_udp_server(server: impl ToSocketAddrs) -> io::Result<UdpSocket> {
    connect_resolved_udp_server(server.to_socket_addrs()?)
}

fn connect_resolved_udp_server(
    candidates: impl IntoIterator<Item = SocketAddr>,
) -> io::Result<UdpSocket> {
    let mut last_error = None;
    for address in candidates.into_iter().take(MAX_RESOLVED_SERVER_ADDRESSES) {
        if address.port() == 0 {
            last_error = Some(io::Error::new(
                io::ErrorKind::InvalidInput,
                "game server UDP port must be nonzero",
            ));
            continue;
        }
        let local = match address {
            SocketAddr::V4(_) => "0.0.0.0:0",
            SocketAddr::V6(_) => "[::]:0",
        };
        let connected = UdpSocket::bind(local).and_then(|socket| {
            socket.connect(address)?;
            Ok(socket)
        });
        match connected {
            Ok(socket) => return Ok(socket),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "game server resolved to no UDP addresses",
        )
    }))
}

pub(in crate::net) fn run_udp_client(
    server_addr: String,
    outgoing: Receiver<ClientPacket>,
    incoming: Sender<ServerPacket>,
    signals: Sender<NetThreadSignal>,
    gameplay_signer: SharedGameplaySigner,
) {
    println!("Connecting to server at {server_addr}");
    let socket = match connect_udp_server(server_addr.as_str()) {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("Failed to resolve/bind/connect UDP server {server_addr}: {error}");
            let _ = signals.send(NetThreadSignal::TransportFailure);
            return;
        }
    };

    if let Err(error) = socket.set_nonblocking(true) {
        eprintln!("Failed to set UDP client socket nonblocking: {error}");
        let _ = signals.send(NetThreadSignal::TransportFailure);
        return;
    }
    println!("UDP socket connected to {server_addr}; waiting for first snapshot");

    let mut public_transport = public_transport::PublicClientTransport::new();
    let mut assembler = SnapshotAssembler::default();
    let mut recv_buf = vec![0_u8; SERVER_DATAGRAM_RECEIVE_CAPACITY];
    let mut last_heartbeat_at = Instant::now();
    let mut last_receive_error_log_at: Option<Instant> = None;
    let mut last_decode_error_log_at: Option<Instant> = None;
    let mut suppressed_decode_errors = 0_u64;
    let mut first_snapshot_received = false;
    let mut consecutive_recv_errors: u32 = 0;
    let mut consecutive_send_errors: u32 = 0;
    let mut transport_failure_reported = false;

    let _ = udp_try_send(
        &socket,
        &mut public_transport,
        &gameplay_signer,
        &ClientPacket::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
        &mut consecutive_send_errors,
        &mut transport_failure_reported,
        &signals,
    );

    loop {
        for _ in 0..128 {
            match outgoing.try_recv() {
                Ok(packet) => {
                    udp_try_send(
                        &socket,
                        &mut public_transport,
                        &gameplay_signer,
                        &packet,
                        &mut consecutive_send_errors,
                        &mut transport_failure_reported,
                        &signals,
                    );
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        if last_heartbeat_at.elapsed() >= T_RETRY {
            assembler.expire(Instant::now());
            udp_try_send(
                &socket,
                &mut public_transport,
                &gameplay_signer,
                &ClientPacket::Hello {
                    protocol_version: PROTOCOL_VERSION,
                },
                &mut consecutive_send_errors,
                &mut transport_failure_reported,
                &signals,
            );
            last_heartbeat_at = Instant::now();
        }

        for _ in 0..128 {
            match socket.recv(&mut recv_buf) {
                Ok(len) => {
                    consecutive_recv_errors = 0;
                    if public_transport.handle_challenge(&socket, &recv_buf[..len]) {
                        continue;
                    }
                    let payload = match assembler.push(&recv_buf[..len], Instant::now()) {
                        Ok(Some(payload)) => payload,
                        Ok(None) => continue,
                        Err(TransportError::Version) => {
                            // Old framed servers are rejected before their JSON can
                            // reach admission. Surface that exact failure instead of
                            // waiting for a generic network timeout.
                            let _ = signals.send(NetThreadSignal::ProtocolMismatch);
                            return;
                        }
                        Err(error) => {
                            if last_decode_error_log_at
                                .is_none_or(|at: Instant| at.elapsed() >= T_RETRY)
                            {
                                warn!("Snapshot frame rejected: {error}");
                                last_decode_error_log_at = Some(Instant::now());
                            }
                            continue;
                        }
                    };
                    match forward_complete_server_datagram(&payload, &incoming) {
                        Ok(published) => {
                            if published {
                                public_transport.observed_server_packet();
                            }
                            if published && !first_snapshot_received {
                                println!(
                                    "First snapshot received from {server_addr}; connection is live"
                                );
                                first_snapshot_received = true;
                            }
                        }
                        Err(error) => {
                            let now = Instant::now();
                            if last_decode_error_log_at.is_none_or(|last| {
                                now.duration_since(last) >= DECODE_ERROR_LOG_INTERVAL
                            }) {
                                let suppressed = std::mem::take(&mut suppressed_decode_errors);
                                eprintln!(
                                    "Failed to decode complete server datagram ({len} bytes): \
                                     {error}; suppressed {suppressed} similar errors"
                                );
                                last_decode_error_log_at = Some(now);
                            } else {
                                suppressed_decode_errors =
                                    suppressed_decode_errors.saturating_add(1);
                            }
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    consecutive_recv_errors = consecutive_recv_errors.saturating_add(1);
                    let now = Instant::now();
                    if last_receive_error_log_at
                        .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
                    {
                        eprintln!("Client socket receive error: {error}");
                        last_receive_error_log_at = Some(now);
                    }
                    if consecutive_recv_errors >= TRANSPORT_CONSECUTIVE_RECV_ERRORS
                        && !transport_failure_reported
                    {
                        transport_failure_reported = true;
                        let _ = signals.send(NetThreadSignal::TransportFailure);
                    }
                    break;
                }
            }
        }

        thread::sleep(NETWORK_LOOP_SLEEP);
    }
}

pub(in crate::net) fn decode_server_packet(
    payload: &[u8],
) -> Result<ServerPacket, serde_json::Error> {
    serde_json::from_slice(payload)
}

/// Decode one complete received datagram and publish only the resulting whole
/// packet. Keeping this boundary shared by the socket loop and tests proves a
/// malformed/truncated JSON prefix can never enter snapshot staging.
pub(in crate::net) fn forward_complete_server_datagram(
    payload: &[u8],
    incoming: &Sender<ServerPacket>,
) -> io::Result<bool> {
    let packet = decode_server_packet(payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    match incoming.try_send(packet) {
        Ok(()) => Ok(true),
        // A stalled frame must not grow an unbounded snapshot queue.
        Err(crossbeam_channel::TrySendError::Full(_)) => Ok(false),
        Err(crossbeam_channel::TrySendError::Disconnected(_)) => Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "snapshot receiver closed",
        )),
    }
}

pub(in crate::net) fn send_packet(socket: &UdpSocket, packet: &ClientPacket) -> io::Result<()> {
    let payload = serde_json::to_vec(packet)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    validate_client_payload_size(payload.len())?;
    socket.send(&payload)?;
    Ok(())
}

fn validate_client_payload_size(payload_len: usize) -> io::Result<()> {
    if payload_len > IPV4_UDP_MAX_PAYLOAD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "client datagram is {} bytes; IPv4 UDP payload limit is {} bytes",
                payload_len, IPV4_UDP_MAX_PAYLOAD_BYTES
            ),
        ));
    }
    Ok(())
}

fn udp_try_send(
    socket: &UdpSocket,
    public_transport: &mut public_transport::PublicClientTransport,
    gameplay_signer: &SharedGameplaySigner,
    packet: &ClientPacket,
    consecutive_send_errors: &mut u32,
    transport_failure_reported: &mut bool,
    signals: &Sender<NetThreadSignal>,
) -> bool {
    match public_transport.send(socket, packet, gameplay_signer) {
        Ok(()) => {
            *consecutive_send_errors = 0;
            true
        }
        Err(error) => {
            *consecutive_send_errors = consecutive_send_errors.saturating_add(1);
            eprintln!("Client UDP send error: {error}");
            if *consecutive_send_errors >= TRANSPORT_CONSECUTIVE_SEND_ERRORS
                && !*transport_failure_reported
            {
                *transport_failure_reported = true;
                let _ = signals.send(NetThreadSignal::TransportFailure);
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::test_fixtures::*;
    use std::cell::Cell;

    #[test]
    fn old_8_kib_boundary_decodes_complete_trailing_entities() {
        for size in [8_191, 8_192, 8_193] {
            let payload = exact_size_snapshot_fixture(size, size as u64);
            assert_fixture_sentinel(&payload, size as u64);
        }
    }

    #[test]
    fn representative_populated_snapshot_larger_than_8_kib_decodes_completely() {
        let payload = populated_snapshot_fixture();
        assert!(
            payload.len() > 8 * 1024,
            "representative snapshot unexpectedly shrank to {} bytes",
            payload.len()
        );
        let packet = decode_server_packet(&payload).expect("populated snapshot should decode");
        let ServerPacket::Snapshot {
            players,
            projectiles,
            structures,
            minions,
            neutrals,
            rematch_in_secs,
            ..
        } = packet
        else {
            panic!("expected snapshot");
        };
        assert_eq!(players.len(), 10);
        assert_eq!(projectiles.len(), 4);
        assert_eq!(structures.last().map(|structure| structure.id), Some(8));
        assert_eq!(minions.last().map(|minion| minion.id), Some(18));
        assert_eq!(neutrals.len(), 5);
        assert_eq!(rematch_in_secs, Some(4242));
    }

    #[test]
    fn near_ipv4_limit_decodes_without_prefix_truncation() {
        let payload = exact_size_snapshot_fixture(IPV4_UDP_MAX_PAYLOAD_BYTES, 65_507);
        assert_fixture_sentinel(&payload, 65_507);
    }

    #[test]
    fn outbound_payload_guard_uses_ipv4_udp_ceiling() {
        assert!(validate_client_payload_size(IPV4_UDP_MAX_PAYLOAD_BYTES).is_ok());
        let error = validate_client_payload_size(IPV4_UDP_MAX_PAYLOAD_BYTES + 1).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn loopback_receives_8191_8192_8193_and_malformed_between_good_datagrams() {
        let receiver = UdpSocket::bind("127.0.0.1:0").expect("bind receiver");
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("set receiver timeout");
        let sender = UdpSocket::bind("127.0.0.1:0").expect("bind sender");
        sender.connect(receiver.local_addr().unwrap()).unwrap();

        let datagrams = [
            exact_size_snapshot_fixture(8_191, 1),
            exact_size_snapshot_fixture(8_192, 2),
            exact_size_snapshot_fixture(8_193, 3),
            br#"{"type":"snapshot","your_id":7,"players":["#.to_vec(),
            exact_size_snapshot_fixture(9_000, 4),
        ];
        for payload in &datagrams {
            assert_eq!(sender.send(payload).expect("loopback send"), payload.len());
        }

        let mut receive_storage = vec![0_u8; SERVER_DATAGRAM_RECEIVE_CAPACITY];
        let mut decoded_sentinels = Vec::new();
        for expected_len in datagrams.iter().map(Vec::len) {
            let len = receiver
                .recv(&mut receive_storage)
                .expect("loopback receive should complete");
            assert_eq!(len, expected_len);
            if let Ok(ServerPacket::Snapshot {
                rematch_in_secs, ..
            }) = decode_server_packet(&receive_storage[..len])
            {
                decoded_sentinels.push(rematch_in_secs.expect("fixture sentinel"));
            }
        }
        assert_eq!(decoded_sentinels, [1, 2, 3, 4]);
    }

    #[test]
    fn platform_near_limit_loopback_behavior_is_explicit() {
        let receiver = UdpSocket::bind("127.0.0.1:0").expect("bind receiver");
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("set receiver timeout");
        let sender = UdpSocket::bind("127.0.0.1:0").expect("bind sender");
        sender.connect(receiver.local_addr().unwrap()).unwrap();
        let payload = exact_size_snapshot_fixture(IPV4_UDP_MAX_PAYLOAD_BYTES, 55);

        match sender.send(&payload) {
            Ok(len) => {
                assert_eq!(len, payload.len());
                let mut storage = vec![0_u8; SERVER_DATAGRAM_RECEIVE_CAPACITY];
                let received = receiver
                    .recv(&mut storage)
                    .expect("receive near-limit payload");
                assert_eq!(received, payload.len());
                assert_fixture_sentinel(&storage[..received], 55);
            }
            Err(error) if cfg!(target_os = "macos") => {
                // The task's Darwin runner was measured at a 9,216-byte
                // send ceiling. errno 40 (EMSGSIZE) is that lower kernel
                // sender limit, not receive truncation; this change does not
                // claim to raise or bypass it.
                assert_eq!(
                    error.raw_os_error(),
                    Some(40),
                    "unexpected macOS error: {error}"
                );
            }
            Err(error) => panic!("legal IPv4 UDP payload failed on loopback: {error}"),
        }
    }

    fn assert_loopback_round_trip(bind_address: &str) {
        let server = match UdpSocket::bind(bind_address) {
            Ok(server) => server,
            // Containers without an IPv6 stack cannot bind `[::1]`; that is a
            // property of the host, not of the resolver under test.
            Err(error) if error.raw_os_error() == Some(libc_eafnosupport()) => {
                eprintln!("skipping {bind_address}: {error}");
                return;
            }
            Err(error) => panic!("bind loopback server {bind_address}: {error}"),
        };
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let server_address = server.local_addr().unwrap();
        let client = connect_udp_server(server_address).expect("connect matching IP family");
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        assert_eq!(client.peer_addr().unwrap(), server_address);
        assert_eq!(
            client.local_addr().unwrap().is_ipv6(),
            server_address.is_ipv6()
        );

        client.send(b"phone hello").unwrap();
        let mut payload = [0u8; 32];
        let (received, peer) = server.recv_from(&mut payload).unwrap();
        assert_eq!(&payload[..received], b"phone hello");
        server.send_to(b"match reply", peer).unwrap();
        let received = client.recv(&mut payload).unwrap();
        assert_eq!(&payload[..received], b"match reply");
    }

    #[test]
    fn ipv4_loopback_uses_ipv4_socket_and_round_trips() {
        assert_loopback_round_trip("127.0.0.1:0");
    }

    #[test]
    fn ipv6_loopback_uses_ipv6_socket_and_round_trips() {
        assert_loopback_round_trip("[::1]:0");
    }

    /// `EAFNOSUPPORT` on Linux and macOS; Windows reports `WSAEAFNOSUPPORT`.
    fn libc_eafnosupport() -> i32 {
        if cfg!(windows) { 10047 } else { 97 }
    }

    struct StubResolver<'a> {
        calls: &'a Cell<usize>,
        addresses: Vec<SocketAddr>,
        error: Option<io::ErrorKind>,
    }

    impl ToSocketAddrs for StubResolver<'_> {
        type Iter = std::vec::IntoIter<SocketAddr>;

        fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
            self.calls.set(self.calls.get() + 1);
            match self.error {
                Some(kind) => Err(io::Error::new(kind, "resolution failed")),
                None => Ok(self.addresses.clone().into_iter()),
            }
        }
    }

    #[test]
    fn resolves_once_and_tries_next_candidate_after_invalid_port() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let calls = Cell::new(0);
        let resolver = StubResolver {
            calls: &calls,
            addresses: vec!["[::1]:0".parse().unwrap(), address],
            error: None,
        };
        let client = connect_udp_server(resolver).unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(client.peer_addr().unwrap(), address);
    }

    #[test]
    fn empty_and_failed_resolution_surface_as_errors_without_retrying_dns() {
        let calls = Cell::new(0);
        let empty = StubResolver {
            calls: &calls,
            addresses: vec![],
            error: None,
        };
        assert_eq!(
            connect_udp_server(empty).unwrap_err().kind(),
            io::ErrorKind::AddrNotAvailable
        );
        assert_eq!(calls.get(), 1);
        let failed = StubResolver {
            calls: &calls,
            addresses: vec![],
            error: Some(io::ErrorKind::NotFound),
        };
        let error = connect_udp_server(failed).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(error.to_string(), "resolution failed");
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn candidate_failures_preserve_last_error_and_iteration_is_bounded() {
        let attempted = Cell::new(0);
        let candidates = std::iter::repeat("127.0.0.1:0".parse::<SocketAddr>().unwrap())
            .inspect(|_| attempted.set(attempted.get() + 1));
        let error = connect_resolved_udp_server(candidates).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(attempted.get(), MAX_RESOLVED_SERVER_ADDRESSES);
    }

    #[test]
    fn social_packet_roundtrip_preserves_match_binding_and_remains_distinct_from_snapshot() {
        let packet = ServerPacket::Social {
            server_epoch: 12,
            match_id: 3,
            sequence: 7,
            social: shared::social::SocialView::default(),
        };
        let bytes = serde_json::to_vec(&packet).unwrap();
        assert!(matches!(
            decode_server_packet(&bytes).unwrap(),
            ServerPacket::Social {
                server_epoch: 12,
                match_id: 3,
                sequence: 7,
                ..
            }
        ));
        let request = shared::social::SocialRequest {
            request_id: 1,
            server_epoch: 12,
            match_id: 3,
            session_id: "session".into(),
            command: shared::social::SocialCommand::Subscribe,
        };
        let json = serde_json::to_value(ClientPacket::Social { request }).unwrap();
        assert_eq!(json["type"], "social");
        assert_eq!(json["request"]["command"]["kind"], "subscribe");
    }
}
