//! Datagram receive loop and client packet dispatch.
use std::io;
use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use shared::wire::ClientPacket;

use crate::runtime::ServerRuntime;
use crate::session::normalize_session_id;
use crate::snapshot::IPV4_UDP_MAX_PAYLOAD_BYTES;
use crate::{bots, passport_admission, public_transport};

/// Existing application bound for client -> server request datagrams.
pub(crate) const MAX_CLIENT_REQUEST_PAYLOAD_BYTES: usize = 8 * 1024;

/// Full receive storage lets the server identify and reject an oversized
/// request instead of accidentally accepting a valid truncated prefix.
pub(crate) const CLIENT_DATAGRAM_RECEIVE_CAPACITY: usize = 65_536;

const _: () = assert!(CLIENT_DATAGRAM_RECEIVE_CAPACITY > IPV4_UDP_MAX_PAYLOAD_BYTES);

impl ServerRuntime {
    pub(crate) fn receive_packets(&mut self) {
        for completion in self.passport_admissions.completed() {
            if let ClientPacket::Prematch { request } = completion.packet {
                let now = self.clock.now();
                self.complete_prematch_admission(completion.addr, request, completion.allowed, now);
                continue;
            }
            // Approval cannot change an already admitted loadout. A timed-out
            // endpoint is required to reconnect rather than resurrected here.
            if completion.allowed
                && self
                    .world
                    .players
                    .get(&completion.addr)
                    .is_some_and(|player| !player.joined)
            {
                let now = self.clock.now();
                self.handle_packet_authorized(completion.addr, completion.packet, now);
            } else if let Some(player) = self
                .world
                .players
                .get_mut(&completion.addr)
                .filter(|player| !player.joined)
            {
                player.join_error = Some(shared::protocol::JoinRejection::AvatarNotAuthorized);
            }
        }
        let receive_started = self.clock.now();
        for _ in 0..shared::public_transport::MAX_PACKETS_PER_TICK {
            if self.clock.now().saturating_duration_since(receive_started)
                >= Duration::from_millis(shared::public_transport::RECEIVE_BUDGET_MILLIS)
            {
                break;
            }
            match self.transport.recv(&mut self.recv_buf) {
                Ok((len, addr)) => {
                    let now = self.clock.now();
                    if len
                        > if self.match_service.is_public() {
                            shared::public_transport::MAX_PUBLIC_DATAGRAM_BYTES
                        } else {
                            MAX_CLIENT_REQUEST_PAYLOAD_BYTES
                        }
                    {
                        if let Some(suppressed) = self.invalid_request_diagnostic.record(now) {
                            eprintln!(
                                "Rejected oversized client datagram from {addr}: {len} bytes exceeds request limit {MAX_CLIENT_REQUEST_PAYLOAD_BYTES}; suppressed {suppressed} similar errors"
                            );
                        }
                        continue;
                    }
                    if self.match_service.is_public() {
                        match self.public_transport.receive(
                            addr,
                            &self.recv_buf[..len],
                            self.server_epoch,
                            self.match_id,
                            self.match_service.is_lobby(),
                            self.career.backend.gameplay_principal(addr),
                            now,
                        ) {
                            public_transport::Decision::Dispatch(packet) => {
                                self.handle_packet(addr, packet, now)
                            }
                            public_transport::Decision::Reply(bytes) => {
                                let _ = self.transport.send_to(&bytes, addr);
                            }
                            public_transport::Decision::Drop => {}
                        }
                        continue;
                    }
                    match serde_json::from_slice::<ClientPacket>(&self.recv_buf[..len]) {
                        Ok(packet) => self.handle_packet(addr, packet, now),
                        Err(error) => {
                            if let Some(suppressed) = self.invalid_request_diagnostic.record(now) {
                                eprintln!(
                                    "Invalid client datagram from {addr} ({len} bytes): {error}; suppressed {suppressed} similar errors"
                                );
                            }
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    eprintln!("Socket receive error: {error}");
                    break;
                }
            }
        }
    }

    pub(crate) fn handle_packet(&mut self, addr: SocketAddr, packet: ClientPacket, now: Instant) {
        // Internal bot actor addresses never accept network commands or identity claims.
        if bots::is_bot_address(addr) {
            return;
        }
        if let ClientPacket::Career { request } = packet {
            self.handle_career_request(addr, request, now);
            for (sender, request) in self.career.backend.take_social() {
                self.handle_social_request(sender, request, true, now);
            }
            return;
        }
        if let ClientPacket::Social { request } = packet {
            self.handle_social_request(addr, request, false, now);
            return;
        }
        self.career.backend.touch(addr);
        if let ClientPacket::Prematch { request } = packet {
            self.handle_prematch(addr, request, now);
            return;
        }
        if matches!(&packet, ClientPacket::Join { .. })
            && !self
                .world
                .players
                .get(&addr)
                .is_some_and(|player| player.joined)
        {
            self.world.ensure_connected(addr, now);
            // A reconnect may restore an old paid loadout. A client must not
            // bypass ticket verification by requesting a free avatar while
            // presenting the retained session id.
            if let ClientPacket::Join {
                avatar,
                session_id: Some(session),
                ..
            } = &packet
            {
                // Reclaim uses the normalized value too; whitespace must not
                // make this authorization check inspect a different session.
                let session = normalize_session_id(Some(session.clone()));
                let retained = self
                    .world
                    .players
                    .values()
                    .find(|player| session.is_some() && player.session_id == session)
                    .or_else(|| {
                        session
                            .as_ref()
                            .and_then(|session| self.world.disconnected_sessions.get(session))
                            .map(|saved| &saved.player)
                    })
                    .and_then(|player| player.hero.identity.avatar.as_deref());
                if retained.is_some_and(|slug| {
                    (slug.starts_with("ekza-")
                        || shared::avatar_definition(slug)
                            .is_some_and(|entry| entry.passport.is_some()))
                        && avatar.as_deref().map(str::trim) != Some(slug)
                }) {
                    self.world.ensure_connected(addr, now);
                    self.world.players.get_mut(&addr).unwrap().join_error =
                        Some(shared::protocol::JoinRejection::AvatarNotAuthorized);
                    return;
                }
            }
            match self.passport_admissions.begin(addr, &packet) {
                passport_admission::Admission::Free => {}
                passport_admission::Admission::Pending => return,
                passport_admission::Admission::Denied => {
                    self.world.ensure_connected(addr, now);
                    self.world.players.get_mut(&addr).unwrap().join_error =
                        Some(shared::protocol::JoinRejection::AvatarNotAuthorized);
                    return;
                }
            }
        }
        self.handle_packet_authorized(addr, packet, now);
    }

    pub(crate) fn handle_packet_authorized(
        &mut self,
        addr: SocketAddr,
        mut packet: ClientPacket,
        now: Instant,
    ) {
        if matches!(packet, ClientPacket::Join { .. })
            && !self.authorize_allocated_join(addr, &packet)
        {
            self.career_error(addr, "This match is reserved for its allocated roster.");
            return;
        }
        let allocated_team = self.allocated_team(addr, &packet);
        if self.match_service.worker().is_some() {
            if let ClientPacket::Join { prematch, .. } = &mut packet {
                *prematch = true;
            }
        }
        self.maintain_roster(now);
        if self
            .world
            .players
            .get(&addr)
            .is_some_and(|p| p.career_profile.is_some())
            && self.career.backend.authenticated_session(addr).is_none()
            && !matches!(packet, ClientPacket::Hello { .. })
        {
            return;
        }
        if matches!(packet, ClientPacket::Join { .. }) {
            if !self.authorize_career_join(addr, &packet, now) {
                return;
            }
            if !self.prepare_practice_join(addr, &packet, now) {
                return;
            }
            if self.career_queue_enabled() {
                self.join_career_queue(addr, packet, now);
                return;
            }
        }
        let wall_now = now;
        let now = self.sandbox.as_ref().map_or(now, |s| s.now);
        // Arm order is the pre-check order: Leave, the career rematch,
        // Practice and Sandbox run first on the wall clock and return before
        // the tail; then a paused sandbox swallows movement and combat; then
        // the per-variant handlers run on the (sandbox) simulation clock.
        let flow = match packet {
            ClientPacket::Leave => self.handle_leave(addr, wall_now),
            ClientPacket::RequestRematch if self.career_flow_active() => {
                self.handle_career_rematch(addr, wall_now)
            }
            ClientPacket::Practice { command } => self.handle_practice(addr, command, wall_now),
            ClientPacket::Sandbox { request } => {
                self.handle_sandbox_packet(addr, request, wall_now)
            }
            ClientPacket::Transform { .. }
            | ClientPacket::Cast { .. }
            | ClientPacket::BasicAttack { .. }
            | ClientPacket::Utility { .. }
                if self
                    .sandbox
                    .as_ref()
                    .is_some_and(|s| s.config.environment.paused) =>
            {
                if let Some(p) = self.world.players.get_mut(&addr) {
                    p.last_seen = wall_now;
                }
                ControlFlow::Break(())
            }
            ClientPacket::Hello { protocol_version } => {
                self.handle_hello(addr, protocol_version, now)
            }
            ClientPacket::Transform {
                x,
                y,
                z,
                yaw,
                dash_sequence,
            } => self.handle_transform(addr, x, y, z, yaw, dash_sequence, now),
            ClientPacket::Cast { target, slot } => self.handle_cast(addr, target, slot, now),
            ClientPacket::Utility {
                action,
                direction,
                server_epoch,
                match_id,
                request_id,
            } => self.handle_utility(
                addr,
                action,
                direction,
                server_epoch,
                match_id,
                request_id,
                now,
            ),
            ClientPacket::BasicAttack {
                target,
                server_epoch,
                match_id,
                request_id,
            } => self.handle_basic_attack(addr, target, server_epoch, match_id, request_id, now),
            ClientPacket::Join {
                prematch,
                team,
                character,
                hero_class,
                avatar,
                sprite_character,
                session_id,
                passport_ticket: _,
            } => self.handle_join(
                addr,
                allocated_team,
                prematch,
                team,
                character,
                hero_class,
                avatar,
                sprite_character,
                session_id,
                now,
            ),
            ClientPacket::Ping => self.handle_ping(addr, now),
            ClientPacket::RequestRematch => self.handle_request_rematch(addr, now),
            ClientPacket::SetGodMode { enabled } => self.handle_set_god_mode(addr, enabled, now),
            ClientPacket::SetSpeedBoost { enabled } => {
                self.handle_set_speed_boost(addr, enabled, now)
            }
            ClientPacket::UpgradeSkill { slot } => self.handle_upgrade_skill(addr, slot, now),
            ClientPacket::BuyItem {
                item_id,
                request_id,
                match_id,
                server_epoch,
            } => self.handle_buy_item(addr, item_id, request_id, match_id, server_epoch, now),
            ClientPacket::Career { .. }
            | ClientPacket::Social { .. }
            | ClientPacket::Prematch { .. } => {
                unreachable!("handled by handle_packet before gameplay admission")
            }
        };
        if flow.is_break() {
            return;
        }
        if let Some(p) = self.world.players.get_mut(&addr) {
            p.last_seen = wall_now;
        }
        self.initialize_sandbox_players();
        self.fill_practice_bots(now);
        self.tick_prematch(now);
        self.track_round_start(now);
        self.register_career_participant(addr);
    }
}
