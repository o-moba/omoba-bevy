//! A scripted bot client: one connected [`UdpSocket`] plus typed helpers for
//! every `ClientPacket` the harness needs, and polling helpers for reading
//! `Snapshot` packets back from the server.
//!
//! Network reads use a short socket timeout and the higher-level helpers
//! ([`Bot::recv_snapshot`], [`Bot::wait_for_player`]) poll until a deadline, so
//! tests stay deterministic without sleeping blindly. Network calls use
//! `expect` with descriptive messages rather than bare `unwrap`.

use std::{
    io::{self, ErrorKind},
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

use crate::protocol::{Character, ClientPacket, PlayerState, ServerPacket, TargetId, Team};

/// Per-recv socket timeout. Snapshots arrive every ~50ms, so this is short
/// enough to poll responsively and long enough to avoid busy-spinning.
const RECV_TIMEOUT: Duration = Duration::from_millis(200);

/// Max UDP datagram we will read. Snapshots can grow with entity count.
const MAX_DATAGRAM: usize = 64 * 1024;

/// A connected bot client.
pub struct Bot {
    socket: UdpSocket,
    /// Cached own player id, learned from the first snapshot.
    my_id: Option<u64>,
    /// Reusable receive buffer, allocated once instead of per `recv` call.
    recv_buf: Vec<u8>,
}

impl Bot {
    /// Connects a new bot to the given server address.
    pub fn connect(server_addr: SocketAddr) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("bot failed to bind a loopback socket");
        socket
            .connect(server_addr)
            .expect("bot failed to connect to the server address");
        socket
            .set_read_timeout(Some(RECV_TIMEOUT))
            .expect("bot failed to set a socket read timeout");
        Self {
            socket,
            my_id: None,
            recv_buf: vec![0u8; MAX_DATAGRAM],
        }
    }

    // --- Outbound packet helpers -------------------------------------------

    fn send(&self, packet: &ClientPacket) {
        let bytes = serde_json::to_vec(packet).expect("client packet should serialize to JSON");
        self.socket
            .send(&bytes)
            .expect("bot failed to send packet to the server");
    }

    /// Joins a team as the given character.
    pub fn join(&self, team: Team, character: Character) {
        self.send(&ClientPacket::Join {
            team,
            character,
            session_id: None,
        });
    }

    /// Sends a movement/orientation update.
    pub fn send_transform(&self, x: f32, y: f32, z: f32, yaw: f32) {
        self.send(&ClientPacket::Transform { x, y, z, yaw });
    }

    /// Casts the primary ability at a target.
    pub fn cast(&self, target: TargetId) {
        self.send(&ClientPacket::Cast { target });
    }

    /// Convenience: cast the primary ability at an enemy player by id.
    pub fn cast_player(&self, player_id: u64) {
        self.cast(TargetId::player(player_id));
    }

    /// Requests a skill upgrade for the given slot (0=Q, 1=W, 2=E, 3=R).
    pub fn upgrade_skill(&self, slot: u8) {
        self.send(&ClientPacket::UpgradeSkill { slot });
    }

    /// Toggles the debug invulnerability ("god mode") flag.
    pub fn set_god_mode(&self, enabled: bool) {
        self.send(&ClientPacket::SetGodMode { enabled });
    }

    /// Toggles the debug movement speed boost.
    pub fn set_speed_boost(&self, enabled: bool) {
        self.send(&ClientPacket::SetSpeedBoost { enabled });
    }

    /// Sends a keep-alive ping (also registers the bot with the server).
    pub fn ping(&self) {
        self.send(&ClientPacket::Ping);
    }

    // --- Inbound snapshot helpers ------------------------------------------

    /// Returns the **freshest** snapshot available before `deadline`.
    ///
    /// The server emits a snapshot every ~50ms, so a bot's socket buffer can
    /// hold a backlog of stale snapshots after it has been idle. This blocks
    /// for at least one snapshot, then drains any further buffered snapshots
    /// without blocking and returns the most recent one — so callers always
    /// assert against current server state, never lagged history.
    pub fn recv_snapshot(&mut self, deadline: Instant) -> Option<ServerPacket> {
        let mut latest = self.recv_one_blocking(deadline)?;

        // Drain anything already buffered to catch up to the newest snapshot.
        self.socket
            .set_nonblocking(true)
            .expect("bot failed to switch socket to non-blocking for draining");
        loop {
            match self.socket.recv(&mut self.recv_buf) {
                Ok(len) => {
                    if let Ok(packet) = serde_json::from_slice::<ServerPacket>(&self.recv_buf[..len])
                    {
                        latest = packet;
                    }
                }
                Err(error) if is_timeout(&error) => break,
                Err(error) => panic!("bot socket recv failed while draining: {error}"),
            }
        }
        // Restore blocking-with-timeout mode (the SO_RCVTIMEO set in `connect`
        // is independent of the non-blocking flag and is still in effect).
        self.socket
            .set_nonblocking(false)
            .expect("bot failed to restore blocking socket mode after draining");

        self.my_id.get_or_insert(latest.your_id());
        Some(latest)
    }

    /// Blocks until one snapshot arrives or `deadline` passes. Non-snapshot or
    /// unparsable datagrams are skipped.
    fn recv_one_blocking(&mut self, deadline: Instant) -> Option<ServerPacket> {
        loop {
            if Instant::now() >= deadline {
                return None;
            }
            match self.socket.recv(&mut self.recv_buf) {
                Ok(len) => {
                    if let Ok(packet) = serde_json::from_slice::<ServerPacket>(&self.recv_buf[..len])
                    {
                        return Some(packet);
                    }
                    // Unknown/garbage datagram: keep polling.
                }
                Err(error) if is_timeout(&error) => {
                    // No datagram this interval; loop until the deadline.
                }
                Err(error) => panic!("bot socket recv failed: {error}"),
            }
        }
    }

    /// Returns this bot's own player id, blocking (with a timeout) until the
    /// first snapshot is seen.
    pub fn my_id(&mut self, timeout: Duration) -> u64 {
        if let Some(id) = self.my_id {
            return id;
        }
        let deadline = Instant::now() + timeout;
        self.recv_snapshot(deadline)
            .map(|packet| packet.your_id())
            .expect("bot did not receive any snapshot within the timeout")
    }

    /// Polls snapshots until `predicate` holds for the player with `id`, then
    /// returns that player's state. Returns `None` on timeout.
    pub fn wait_for_player<F>(
        &mut self,
        id: u64,
        mut predicate: F,
        timeout: Duration,
    ) -> Option<PlayerState>
    where
        F: FnMut(&PlayerState) -> bool,
    {
        let deadline = Instant::now() + timeout;
        while let Some(packet) = self.recv_snapshot(deadline) {
            if let Some(player) = packet.player(id)
                && predicate(player)
            {
                return Some(player.clone());
            }
        }
        None
    }

    /// Reads the latest available player state for `id` within `timeout`.
    pub fn latest_player(&mut self, id: u64, timeout: Duration) -> Option<PlayerState> {
        self.wait_for_player(id, |_| true, timeout)
    }
}

/// Treats both `WouldBlock` (Unix `SO_RCVTIMEO`) and `TimedOut` (Windows) as a
/// "no datagram yet" signal.
fn is_timeout(error: &io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}
