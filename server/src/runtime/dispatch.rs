//! Datagram receive loop and client packet dispatch.
use crate::*;

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
                self.complete_prematch_admission(
                    completion.addr,
                    request,
                    completion.allowed,
                    Instant::now(),
                );
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
                self.handle_packet_authorized(completion.addr, completion.packet, Instant::now());
            } else if let Some(player) = self
                .world
                .players
                .get_mut(&completion.addr)
                .filter(|player| !player.joined)
            {
                player.join_error = Some(shared::protocol::JoinRejection::AvatarNotAuthorized);
            }
        }
        let receive_started = Instant::now();
        for _ in 0..shared::public_transport::MAX_PACKETS_PER_TICK {
            if receive_started.elapsed()
                >= Duration::from_millis(shared::public_transport::RECEIVE_BUDGET_MILLIS)
            {
                break;
            }
            match self.socket.recv_from(&mut self.recv_buf) {
                Ok((len, addr)) => {
                    let now = Instant::now();
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
                                let _ = self.socket.send_to(&bytes, addr);
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
        if matches!(packet, ClientPacket::Leave) {
            // Public transport authenticated this command independently of the
            // account-action throttle. It is also the reliable local-exit path
            // when an immediately preceding CancelQueue datagram was dropped.
            if self.match_service.is_lobby()
                && self.career.backend.authenticated_session(addr).is_some()
                && let Some(profile) = self.career.backend.profile(addr)
            {
                self.match_service.cancel(&profile.profile_id);
            }
            self.leave_match(addr, now);
            return;
        }
        if matches!(packet, ClientPacket::RequestRematch) && self.career_flow_active() {
            self.career_play_again(addr, now);
            return;
        }
        if let ClientPacket::Practice { command } = packet {
            if let Some(player) = self.world.players.get_mut(&addr) {
                player.last_seen = now;
            }
            self.handle_practice_command(addr, command, now);
            return;
        }
        if let ClientPacket::Sandbox { request } = packet {
            self.initialize_sandbox_players();
            self.handle_sandbox(addr, request);
            if let Some(p) = self.world.players.get_mut(&addr) {
                p.last_seen = now;
            }
            return;
        }
        let wall_now = now;
        let now = self.sandbox.as_ref().map_or(now, |s| s.now);
        if self
            .sandbox
            .as_ref()
            .is_some_and(|s| s.config.environment.paused)
            && matches!(
                packet,
                ClientPacket::Transform { .. }
                    | ClientPacket::Cast { .. }
                    | ClientPacket::BasicAttack { .. }
                    | ClientPacket::Utility { .. }
            )
        {
            if let Some(p) = self.world.players.get_mut(&addr) {
                p.last_seen = wall_now;
            }
            return;
        }
        let combat_sandbox = self.sandbox_allowed();
        let targeting_qa = self.targeting_qa;
        let match_config = self.match_config;
        let match_id = self.match_id;
        let server_epoch = self.server_epoch;
        let world = &mut self.world;
        match packet {
            ClientPacket::Sandbox { .. } => unreachable!("handled above"),
            ClientPacket::Career { .. } | ClientPacket::Social { .. } => {
                unreachable!("handled before gameplay admission")
            }
            ClientPacket::Prematch { .. } => unreachable!("handled before gameplay admission"),
            ClientPacket::Leave => unreachable!("handled before the gameplay match"),
            ClientPacket::Hello { protocol_version } => {
                world.ensure_connected(addr, now);
                let player = world.players.get_mut(&addr).unwrap();
                player.last_seen = now;
                player.framed_snapshots = true;
                player.protocol_compatible = protocol_version == shared::protocol::PROTOCOL_VERSION;
                player.join_error = (!player.protocol_compatible)
                    .then_some(shared::protocol::JoinRejection::ProtocolMismatch);
            }
            ClientPacket::Transform {
                x,
                y,
                z,
                yaw,
                dash_sequence,
            } => {
                world.ensure_connected(addr, now);
                if let Some(player) = world.players.get_mut(&addr) {
                    player.last_seen = now;
                }
                if matches!(world.game_state, GameState::Running)
                    && let Some(player) = world.players.get_mut(&addr)
                    && player.hero.hp > 0.0
                    && dash_sequence == player.hero.utility.dash_sequence
                {
                    handle_transform_request_with_structures(
                        player,
                        &world.map_layout,
                        &world.structures,
                        x,
                        y,
                        z,
                        yaw,
                        now,
                    );
                }
            }
            ClientPacket::Cast { target, slot } => {
                world.ensure_connected(addr, now);
                if let Some(player) = world.players.get_mut(&addr) {
                    player.last_seen = now;
                }
                handle_cast_request(world, addr, target, slot, now);
            }
            ClientPacket::Utility {
                action,
                direction,
                server_epoch: requested_epoch,
                match_id: requested_match,
                request_id,
            } => {
                if requested_epoch != server_epoch || requested_match != match_id {
                    return;
                }
                if let Some(player) = world.players.get_mut(&addr) {
                    handle_utility_request(
                        player,
                        &world.map_layout,
                        &world.structures,
                        &world.game_state,
                        action,
                        direction,
                        request_id,
                        now,
                    );
                }
            }
            ClientPacket::BasicAttack {
                target,
                server_epoch: requested_epoch,
                match_id: requested_match,
                request_id,
            } => {
                if requested_epoch != server_epoch || requested_match != match_id {
                    return;
                }
                handle_basic_attack_request(world, addr, target, request_id, now);
            }
            ClientPacket::Join {
                prematch,
                team,
                character,
                hero_class,
                avatar,
                sprite_character,
                session_id,
                passport_ticket: _,
            } => {
                world.ensure_connected(addr, now);
                let player = world.players.get_mut(&addr).unwrap();
                player.last_seen = now;
                if !player.protocol_compatible {
                    return;
                }
                // A joined endpoint cannot rewrite its identity or loadout through Join.
                if player.joined {
                    player.join_error = None;
                    return;
                }
                let session_id = normalize_session_id(session_id);
                if !world.ensure_player_for_join(addr, session_id, now) {
                    world.players.get_mut(&addr).unwrap().join_error =
                        Some(shared::protocol::JoinRejection::SessionActive);
                    return;
                }
                // A reclaim is already joined: retain all authoritative round state.
                if let Some(player) = world.players.get_mut(&addr).filter(|player| player.joined) {
                    player.join_error = None;
                    self.register_career_participant(addr);
                    self.fill_practice_bots(now);
                    return;
                }
                // Team resolution: dev mode honors the client's
                // choice; release mode balances teams server-side
                // (rejoining players keep their original team).
                let assigned_team = allocated_team.or_else(|| match match_config.mode {
                    MatchMode::Practice => {
                        bots::assign_human_team(&world.players, match_config.team_size)
                    }
                    MatchMode::Dev if combat_sandbox => {
                        sandbox::assign_human_team(&world.players, &world.disconnected_sessions)
                    }
                    MatchMode::Dev if prematch => assign_reserved_release_team(
                        &world.players,
                        &world.disconnected_sessions,
                        match_config.team_size,
                    ),
                    MatchMode::Dev => (joined_count(&world.players)
                        + (world.disconnected_sessions.len() as u32)
                        < match_config.roster_size())
                    .then_some(team),
                    MatchMode::Release => {
                        let existing_team = world
                            .players
                            .get(&addr)
                            .filter(|player| player.joined)
                            .map(|player| player.hero.identity.team);
                        existing_team.or_else(|| {
                            assign_reserved_release_team(
                                &world.players,
                                &world.disconnected_sessions,
                                match_config.team_size,
                            )
                        })
                    }
                });
                let Some(assigned_team) = assigned_team else {
                    println!(
                        "Matchmaking: match is full ({} players) - join from {addr} rejected",
                        match_config.roster_size()
                    );
                    world.players.get_mut(&addr).unwrap().join_error =
                        Some(shared::protocol::JoinRejection::MatchFull);
                    return;
                };
                if match_config.mode == MatchMode::Practice {
                    bots::remove_replaced_bot(
                        &mut world.players,
                        &mut self.bots,
                        &mut self.combat_log.ledger,
                        assigned_team,
                    );
                }
                if let Some(player) = world.players.get_mut(&addr) {
                    player.join_error = None;
                    player.draft.capable = prematch;
                    handle_join_request_with_sprite(
                        player,
                        assigned_team,
                        character,
                        hero_class,
                        avatar.as_deref(),
                        sprite_character.as_deref(),
                        &world.map_layout,
                        now,
                    );
                }
                if targeting_qa {
                    targeting_qa::place_initial_join(&mut world.players, addr);
                }
                if world.players.values().any(|p| p.joined && p.draft.capable)
                    && self.match_started_at.is_none()
                {
                    world.game_state = GameState::Forming {
                        ready: joined_count(&world.players),
                        needed: match_config.roster_size(),
                    };
                } else {
                    advance_formation_on_join(world, match_config, now);
                }
            }
            ClientPacket::Ping => {
                world.ensure_connected(addr, now);
                if let Some(player) = world.players.get_mut(&addr) {
                    player.last_seen = now;
                }
            }
            ClientPacket::RequestRematch => {
                world.ensure_connected(addr, now);
                let mut joined = false;
                if let Some(player) = world.players.get_mut(&addr) {
                    player.last_seen = now;
                    joined = player.joined;
                }
                if joined && matches!(world.game_state, GameState::Victory { .. }) {
                    self.restart_round(now);
                }
            }
            ClientPacket::Practice { .. } => unreachable!("handled before gameplay admission"),
            ClientPacket::SetGodMode { enabled } => {
                // Development and local bot practice only; both are unrated.
                if !matches!(match_config.mode, MatchMode::Dev | MatchMode::Practice) {
                    return;
                }
                world.ensure_connected(addr, now);
                if let Some(player) = world.players.get_mut(&addr) {
                    player.last_seen = now;
                    if !player.joined {
                        return;
                    }
                    if player.modifiers.god_mode != enabled {
                        println!("Player {} god_mode={}", player.hero.identity.id, enabled);
                    }
                    player.modifiers.god_mode = enabled;
                    // The development toggle is invulnerability plus a full
                    // pool; a sandbox actor's resources stay its own setting.
                    if !combat_sandbox {
                        player.modifiers.infinite_resource = enabled;
                    }
                    if enabled {
                        player.hero.hp = player.hero.max_hp;
                        player.hero.mana = player.hero.max_mana;
                        player.timers.respawn_at = None;
                    }
                }
            }
            ClientPacket::SetSpeedBoost { enabled } => {
                if !matches!(match_config.mode, MatchMode::Dev | MatchMode::Practice) {
                    return;
                }
                world.ensure_connected(addr, now);
                if let Some(player) = world.players.get_mut(&addr) {
                    player.last_seen = now;
                    if !player.joined {
                        return;
                    }
                    let mult = if enabled { DEBUG_SPEED_MULTIPLIER } else { 1.0 };
                    if (player.modifiers.move_speed_mult - mult).abs() > f32::EPSILON {
                        println!("Player {} speed_boost={}", player.hero.identity.id, enabled);
                    }
                    player.modifiers.move_speed_mult = mult;
                }
            }
            ClientPacket::UpgradeSkill { slot } => {
                world.ensure_connected(addr, now);
                if let Some(player) = world.players.get_mut(&addr) {
                    player.last_seen = now;
                    if player.joined {
                        apply_skill_upgrade(player, slot);
                    }
                }
            }
            ClientPacket::BuyItem {
                item_id,
                request_id,
                match_id: requested_match,
                server_epoch: requested_epoch,
            } => {
                if requested_match != match_id || requested_epoch != server_epoch {
                    // A delayed datagram must not consume the new round's sequence.
                    return;
                }
                world.ensure_connected(addr, now);
                if let Some(player) = world.players.get_mut(&addr) {
                    player.last_seen = now;
                    handle_purchase(
                        player,
                        &world.map_layout,
                        &world.game_state,
                        &item_id,
                        request_id,
                        match_id,
                    );
                }
            }
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
