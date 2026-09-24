//! Simulation-facing career integration. The backend owns every blocking I/O
//! operation; the tick only exchanges bounded worker commands and acknowledgements.
use crate::*;
use shared::career::{
    CareerRequest, CareerView, MatchOutcome, MatchResult, MatchStats, ParticipantResult, QueueView,
};
use std::collections::VecDeque;

const CAREER_SEND_INTERVAL: Duration = Duration::from_millis(250);
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(10);
const MAX_PENDING_RESULTS: usize = 8;
pub(crate) const RATED_RULESET: &str = "verdant-default-v1";

struct CareerRound {
    result: MatchResult,
    start_enqueued: bool,
    started_at: Instant,
    checkpoint_at: Instant,
    finalized: bool,
}

pub(crate) struct CareerRuntime {
    pub(crate) backend: Box<dyn CareerPort>,
    pub(crate) queue: matchmaking::Matchmaker,
    round: Option<CareerRound>,
    pending: VecDeque<MatchResult>,
    last_results: HashMap<u64, MatchResult>,
    dismissed: HashMap<u64, String>,
    errors: HashMap<SocketAddr, String>,
    blocked_starts: HashSet<u64>,
    last_sent: Option<Instant>,
    sequence: u64,
}

impl CareerRuntime {
    pub(crate) fn new(backend: Box<dyn CareerPort>) -> Self {
        Self {
            backend,
            queue: matchmaking::Matchmaker::default(),
            round: None,
            pending: VecDeque::new(),
            last_results: HashMap::new(),
            dismissed: HashMap::new(),
            errors: HashMap::new(),
            blocked_starts: HashSet::new(),
            last_sent: None,
            sequence: 0,
        }
    }
}

fn utc_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn participant(player: &ConnectedPlayer) -> ParticipantResult {
    ParticipantResult {
        is_bot: player.hero.identity.is_bot,
        player_id: player.hero.identity.id,
        profile_id: player
            .career_profile
            .as_ref()
            .map(|profile| profile.profile_id.clone()),
        nickname: player.career_profile.as_ref().map_or_else(
            || {
                if player.hero.identity.is_bot {
                    format!("Bot {}", player.hero.identity.id)
                } else {
                    format!("Guest {}", player.hero.identity.id)
                }
            },
            |profile| profile.nickname.clone(),
        ),
        team: player.hero.identity.team,
        hero_class: player.hero.identity.hero_class,
        character: serde_json::to_value(player.hero.identity.character)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "ipfs".into()),
        avatar: player.hero.identity.avatar.clone(),
        sprite_character: player.hero.identity.sprite_character.clone(),
        stats: MatchStats {
            final_level: player.hero.progress.level,
            ..Default::default()
        },
        disconnected: false,
        rating: None,
        progression_xp_gained: 0,
    }
}

/// Compare actual gameplay configuration, not an operator-supplied profile label.
fn approved_default_map(map: &shared::map::ResolvedMap) -> bool {
    let default = shared::map::ResolvedMap::default();
    map.geometry_id == default.geometry_id
        && map.map_profile == default.map_profile
        && map.structures.len() == default.structures.len()
        && map.structures.iter().all(|actual| {
            default.structures.iter().any(|expected| {
                actual.id == expected.id
                    && actual.key == expected.key
                    && actual.team == expected.team
                    && actual.lane == expected.lane
                    && actual.tier == expected.tier
                    && actual.position == expected.position
                    && actual.stats.max_hp == expected.stats.max_hp
                    && actual.stats.attack_range == expected.stats.attack_range
                    && actual.stats.attack_damage == expected.stats.attack_damage
                    && actual.stats.hero_damage_multiplier == expected.stats.hero_damage_multiplier
                    && actual.stats.attack_cooldown_ms == expected.stats.attack_cooldown_ms
            })
        })
}

// World snapshots use ordinary tick IDs. Career framing uses a separate key
// namespace while preserving the logical sequence inside the JSON envelope.
const CAREER_FRAME_NAMESPACE: u64 = 1 << 63;

fn clear_query_payload(view: &mut CareerView) {
    view.response_id = None;
    view.history.clear();
    view.history_loaded = false;
    view.history_next = None;
    view.detail = None;
    view.friends = None;
    view.visited_profile = None;
}

pub(crate) fn encode_career_datagrams(
    mut view: CareerView,
    server_epoch: u64,
    sequence: u64,
) -> Result<Vec<Vec<u8>>, String> {
    let encode = |view: &CareerView| {
        serde_json::to_vec(&ServerPacket::Career {
            server_epoch,
            sequence,
            career: view.clone(),
        })
        .map_err(|error| error.to_string())
    };
    let mut payload = encode(&view)?;
    let has_query_payload = view.history_loaded
        || view.detail.is_some()
        || view.friends.is_some()
        || view.visited_profile.is_some();
    if payload.len() > shared::transport::MAX_SNAPSHOT_BYTES
        && has_query_payload
        && view.last_result.is_some()
    {
        // Both pieces remain in the server cache and are repeated on alternate
        // updates. The client preserves absent result/query sections, including
        // request correlation. Never truncate a participant list or history page.
        if sequence.is_multiple_of(2) {
            view.last_result = None;
        } else {
            clear_query_payload(&mut view);
            view.loading = false;
        }
        payload = encode(&view)?;
    }
    if payload.len() > shared::transport::MAX_SNAPSHOT_BYTES {
        let response_id = view.response_id;
        clear_query_payload(&mut view);
        view.response_id = response_id;
        view.last_result = None;
        view.loading = false;
        view.error = Some("Career data exceeds the supported response size. Reopen the requested section or contact the server operator.".into());
        payload = encode(&view)?;
    }
    shared::transport::encode_snapshot(&payload, server_epoch, CAREER_FRAME_NAMESPACE | sequence)
        .map_err(|error| error.to_string())
}

impl ServerRuntime {
    pub(crate) fn career_worker_terminal(&self) -> bool {
        self.career
            .round
            .as_ref()
            .is_none_or(|r| r.finalized && (r.result.saved || !r.start_enqueued))
            && self.career.pending.is_empty()
    }
    pub(crate) fn career_worker_result_id(&self) -> Option<String> {
        self.career
            .round
            .as_ref()
            .map(|r| r.result.result_id.clone())
    }
    #[cfg(test)]
    pub(crate) fn career_allocation_for_test(&self) -> Option<MatchResult> {
        self.career.round.as_ref().map(|round| round.result.clone())
    }
    pub(crate) fn career_queue_enabled(&self) -> bool {
        !self.match_service.is_public() && self.rules.career_credit && self.career.backend.enabled()
    }
    pub(crate) fn career_flow_active(&self) -> bool {
        self.career.backend.enabled()
            || self
                .world
                .players
                .values()
                .any(|player| player.career_capable)
    }
    pub(crate) fn career_error(&mut self, addr: SocketAddr, message: impl Into<String>) {
        self.career.errors.insert(addr, message.into());
    }

    pub(crate) fn handle_career_request(
        &mut self,
        addr: SocketAddr,
        request: CareerRequest,
        now: Instant,
    ) {
        self.world.ensure_connected(addr, now);
        let player = self.world.players.get_mut(&addr).unwrap();
        player.career_capable = true;
        player.last_seen = now;
        let admitted_human = player.joined && !player.hero.identity.is_bot;
        // Protect an existing profile before a Challenge can clear its auth.
        // Repeat after handle creates a first guest-auth client: this flag is
        // also an identity guard and requires no persistent match allocation.
        self.career.backend.set_playing(addr, admitted_human);
        self.career.backend.handle(addr, request);
        self.career.backend.set_playing(addr, admitted_human);
        // Only verified signed cancellation completions change queue state.
        self.poll_career(now);
        self.career.last_sent = None;
        self.send_career_views(now);
    }

    pub(crate) fn authorize_career_join(
        &mut self,
        addr: SocketAddr,
        packet: &ClientPacket,
        now: Instant,
    ) -> bool {
        let ClientPacket::Join { session_id, .. } = packet else {
            return true;
        };
        self.world.ensure_connected(addr, now);
        if self
            .career
            .blocked_starts
            .contains(&self.world.players[&addr].hero.identity.id)
        {
            return false;
        }
        let session = normalize_session_id(session_id.clone());
        let authenticated = self.career.backend.profile(addr);
        if (self.career_queue_enabled() || self.match_service.is_public())
            && authenticated.is_none()
        {
            self.career_error(
                addr,
                "Authenticate your saved profile before joining the ranked queue.",
            );
            return false;
        }
        if authenticated.is_some()
            && (session.is_none() || self.career.backend.authenticated_session(addr) != session)
        {
            self.career_error(
                addr,
                "The join session must match the signed profile session.",
            );
            return false;
        }
        let retained = session.as_ref().and_then(|token| {
            self.world
                .players
                .values()
                .find(|player| player.session_id.as_ref() == Some(token))
                .or_else(|| {
                    self.world
                        .disconnected_sessions
                        .get(token)
                        .map(|saved| &saved.player)
                })
        });
        let retained_id = retained.map(|player| player.hero.identity.id);
        if let Some(owner) = retained.and_then(|player| player.career_profile.as_ref())
            && authenticated.as_ref().map(|p| &p.profile_id) != Some(&owner.profile_id)
        {
            self.career_error(
                addr,
                "This reserved seat belongs to another authenticated profile.",
            );
            return false;
        }
        let current = &self.world.players[&addr];
        if let Some(owner) = &current.career_profile
            && authenticated.as_ref().map(|p| &p.profile_id) != Some(&owner.profile_id)
        {
            self.career_error(
                addr,
                "This endpoint already belongs to another match profile.",
            );
            return false;
        }
        if let Some(profile) = &authenticated {
            let conflicting = self
                .world
                .players
                .values()
                .chain(self.world.disconnected_sessions.values().map(|s| &s.player))
                .any(|player| {
                    player.hero.identity.id != current.hero.identity.id
                        && Some(player.hero.identity.id) != retained_id
                        && player
                            .career_profile
                            .as_ref()
                            .is_some_and(|p| p.profile_id == profile.profile_id)
                });
            let conflicting_frozen = !self.combat_log.ledger.is_frozen()
                && self.combat_log.ledger.snapshot().iter().any(|p| {
                    p.profile_id.as_ref() == Some(&profile.profile_id)
                        && p.player_id != current.hero.identity.id
                        && Some(p.player_id) != retained_id
                });
            if conflicting || conflicting_frozen {
                self.career_error(
                    addr,
                    if conflicting {
                        "This profile already owns a queue entry or a reserved match seat."
                    } else {
                        "This profile already participated in the current round. Wait for a new round before selecting another hero."
                    },
                );
                return false;
            }
        }
        if self.rules.career_flow
            && !self.career_queue_enabled()
            && !self.combat_log.ledger.is_frozen()
            && self.combat_log.ledger.is_started()
            && self.combat_log.ledger.snapshot().len() >= shared::career::MAX_PARTICIPANTS
            && !self.combat_log.ledger.snapshot().iter().any(|p| {
                p.player_id == current.hero.identity.id || Some(p.player_id) == retained_id
            })
        {
            self.career_error(addr, "This round has reached its participant limit.");
            return false;
        }
        if !current.joined {
            self.world.players.get_mut(&addr).unwrap().career_profile = authenticated;
        }
        self.career.errors.remove(&addr);
        true
    }

    pub(crate) fn join_career_queue(
        &mut self,
        addr: SocketAddr,
        packet: ClientPacket,
        now: Instant,
    ) {
        let ClientPacket::Join {
            prematch,
            team,
            character,
            hero_class,
            avatar,
            sprite_character,
            session_id,
            ..
        } = packet
        else {
            return;
        };
        let player = self.world.players.get_mut(&addr).unwrap();
        player.last_seen = now;
        if !player.protocol_compatible {
            return;
        }
        if !self
            .world
            .ensure_player_for_join(addr, normalize_session_id(session_id), now)
        {
            self.world.players.get_mut(&addr).unwrap().join_error =
                Some(shared::protocol::JoinRejection::SessionActive);
            return;
        }
        let player = &self.world.players[&addr];
        if player.joined {
            self.combat_log.ledger.update_player(
                player.hero.identity.id,
                player.hero.progress.level,
                false,
            );
            self.career.backend.set_playing(addr, true);
            return;
        }
        let id = player.hero.identity.id;
        if !matches!(
            self.career
                .queue
                .view(id, self.rules.team_size as usize, now),
            QueueView::Idle
        ) {
            return;
        }
        let Some(profile) = self.career.backend.profile(addr) else {
            return;
        };
        if let Err(error) = self.career.queue.enqueue(id, profile.clone(), now) {
            self.career_error(addr, error.to_string());
            return;
        }
        let player = self.world.players.get_mut(&addr).unwrap();
        player.career_profile = Some(profile);
        player.draft.capable = prematch;
        handle_join_request_with_sprite(
            player,
            team,
            character,
            hero_class,
            avatar.as_deref(),
            sprite_character.as_deref(),
            &self.world.map_layout,
            now,
        );
        player.joined = false;
        player.join_error = None;
        self.career.errors.remove(&addr);
        self.advance_career_queue(now);
    }

    fn cancel_career_entry(&mut self, player_id: u64, now: Instant) {
        let reserved: Vec<_> = self
            .career
            .queue
            .selection()
            .into_iter()
            .flat_map(|selection| {
                selection
                    .participants
                    .iter()
                    .map(|player| player.waiting.player_id)
            })
            .collect();
        self.career.queue.cancel(player_id);
        if reserved.contains(&player_id) {
            for (addr, player) in &mut self.world.players {
                if reserved.contains(&player.hero.identity.id) {
                    player.joined = false;
                    self.career.backend.set_playing(*addr, false);
                }
            }
            if self.match_started_at.is_none() {
                self.finish_career_round(MatchOutcome::Abandoned, None, now);
                self.restart_round(now);
            }
        }
    }

    pub(crate) fn advance_career_queue(&mut self, now: Instant) {
        if !self.career_queue_enabled() || matches!(self.world.game_state, GameState::Running) {
            return;
        }
        if matches!(self.world.game_state, GameState::Victory { .. }) {
            if self.career.queue.is_empty() {
                return;
            }
            self.record_match_metrics(now);
            self.restart_round(now);
        }
        if self.career.queue.selection().is_some() {
            return;
        }
        let Ok(Some(selection)) = self
            .career
            .queue
            .reserve_match(self.rules.team_size as usize)
        else {
            return;
        };
        for assigned in &selection.participants {
            let Some((addr, player)) = self
                .world
                .players
                .iter_mut()
                .find(|(_, player)| player.hero.identity.id == assigned.waiting.player_id)
            else {
                self.career.queue.release_selection();
                return;
            };
            if self
                .career
                .backend
                .profile(*addr)
                .is_none_or(|p| p.profile_id != assigned.waiting.profile.profile_id)
            {
                let id = assigned.waiting.player_id;
                self.cancel_career_entry(id, now);
                return;
            }
            player.hero.identity.team = assigned.team;
            player.career_profile = Some(assigned.waiting.profile.clone());
            player.joined = true;
            session::reset_player_round(player, &self.world.map_layout, now);
            self.career.backend.set_playing(*addr, true);
        }
        self.world.game_state = GameState::Forming {
            ready: selection.participants.len() as u32,
            needed: self.rules.roster_size(),
        };
    }

    fn rating_eligibility(&self, roster: &[ParticipantResult]) -> Result<(), &'static str> {
        if self.sandbox.is_some() {
            return Err("Combat Sandbox is never eligible for career credit.");
        }
        if self.rules.local_results {
            return Err("Bot practice: local result only; no permanent career credit.");
        }
        if roster.iter().any(|participant| participant.is_bot) {
            return Err("Bot participants are not eligible for ranked play.");
        }
        if !self.career_queue_enabled() && self.match_service.worker().is_none() {
            return Err("Development or legacy guest match.");
        }
        if self.targeting_qa
            || self
                .world
                .players
                .values()
                .any(|p| p.joined && (p.modifiers.god_mode || p.modifiers.move_speed_mult != 1.0))
        {
            return Err("Development gameplay modifiers are enabled.");
        }
        if !approved_default_map(&self.world.map_config) {
            return Err("Custom map or gameplay tuning.");
        }
        if roster.len() != self.rules.roster_size() as usize
            || roster.iter().any(|p| p.profile_id.is_none())
        {
            return Err("The full authenticated roster was not present.");
        }
        let green = roster
            .iter()
            .filter(|p| p.team == shared::map::Team::Green)
            .count();
        if green != self.rules.team_size as usize {
            return Err("Teams are not balanced.");
        }
        if self.career.queue.selection().is_none() && self.match_service.worker().is_none() {
            return Err("Roster was not selected by the skill queue.");
        }
        Ok(())
    }

    /// Called at the final formation boundary. Career-backed rounds remain at
    /// Starting(0) until durable allocation ACK; no combat or income runs early.
    pub(crate) fn begin_career_round(&mut self, now: Instant) -> bool {
        if self.match_service.worker().is_some_and(|w| {
            w.recovery
                || w.cancelled()
                || crate::match_allocation::unix_ms() > w.manifest.join_deadline_ms
        }) && self.match_started_at.is_none()
        {
            return false;
        }
        if !self.allocated_humans_ready() {
            return false;
        }
        let practice = self.rules.local_results;
        let durable = !practice || self.match_service.worker().is_some();
        let public_casual = practice
            && self.match_service.worker().is_some()
            && approved_default_map(&self.world.map_config)
            && !self.targeting_qa
            && self.world.players.values().filter(|p| p.joined).all(|p| {
                !p.modifiers.god_mode
                    && p.modifiers.move_speed_mult == 1.0
                    && (p.hero.identity.is_bot || p.career_profile.is_some())
            });
        if durable && self.career.pending.len() >= MAX_PENDING_RESULTS {
            return false;
        }
        if self.career.round.is_none() {
            let mut roster: Vec<_> = self
                .world
                .players
                .values()
                .filter(|p| p.joined)
                .map(participant)
                .collect();
            roster.sort_by_key(|p| p.player_id);
            if roster.is_empty() {
                return true;
            } // Pure simulation fixtures.
            let eligibility = self.rating_eligibility(&roster);
            let result = MatchResult {
                result_id: self.career.backend.new_result_id(),
                server_epoch: self.server_epoch,
                match_id: self.match_id,
                started_at_ms: utc_ms(),
                ended_at_ms: 0,
                duration_ms: 0,
                map_profile: self.world.map_config.map_profile.clone(),
                ruleset: if public_casual {
                    "public-casual-v1"
                } else if practice {
                    "practice-bots-v1"
                } else if approved_default_map(&self.world.map_config) {
                    RATED_RULESET
                } else {
                    "custom-unrated-v1"
                }
                .into(),
                outcome: MatchOutcome::Interrupted,
                winner: None,
                rated: eligibility.is_ok(),
                unrated_reason: if public_casual {
                    Some("allocated_bots".into())
                } else {
                    eligibility.err().map(str::to_owned)
                },
                participants: roster,
                saved: false,
            };
            self.career.round = Some(CareerRound {
                result,
                start_enqueued: false,
                started_at: now,
                checkpoint_at: now,
                finalized: false,
            });
        }
        let round = self.career.round.as_mut().unwrap();
        if durable && self.career.backend.enabled() {
            if !round.start_enqueued {
                round.start_enqueued = self.career.backend.start(round.result.clone());
            }
            if !self.career.backend.started(&round.result.result_id) {
                return false;
            }
        }
        if !self.combat_log.ledger.is_started() {
            // Anyone who joined after the roster was drafted (practice bots
            // filled while a durable start was pending, a late seat) needs a
            // row before the first hit, or their kills and deaths vanish.
            for player in self.world.players.values().filter(|p| p.joined) {
                if !round
                    .result
                    .participants
                    .iter()
                    .any(|p| p.player_id == player.hero.identity.id)
                {
                    round.result.participants.push(participant(player));
                }
            }
            round.result.participants.sort_by_key(|p| p.player_id);
            if let Err(error) = self
                .combat_log
                .ledger
                .begin(round.result.participants.clone())
            {
                for (addr, player) in &self.world.players {
                    if player.joined {
                        self.career.errors.insert(*addr, error.into());
                    }
                }
                return false;
            }
            round.started_at = now;
            round.checkpoint_at = now;
            self.career.queue.commit_selection();
            for (addr, player) in &self.world.players {
                if player.joined && !player.hero.identity.is_bot {
                    self.career.backend.set_playing(*addr, true);
                }
            }
        }
        true
    }

    pub(crate) fn register_career_participant(&mut self, addr: SocketAddr) {
        let Some(player) = self.world.players.get(&addr).filter(|p| p.joined) else {
            return;
        };
        if !self.combat_log.ledger.is_started() {
            // The round is drafted but its statistics have not begun (for
            // example a durable start acknowledgment is still pending):
            // queue the seat so `begin_career_round` rosters it.
            if let Some(round) = self.career.round.as_mut()
                && !round
                    .result
                    .participants
                    .iter()
                    .any(|p| p.player_id == player.hero.identity.id)
            {
                round.result.participants.push(participant(player));
                round.result.participants.sort_by_key(|p| p.player_id);
            }
            return;
        }
        if self.combat_log.ledger.is_frozen() {
            return;
        }
        if let Err(error) = self.combat_log.ledger.register(participant(player)) {
            self.career.errors.insert(addr, error.into());
        }
        self.combat_log.ledger.update_player(
            player.hero.identity.id,
            player.hero.progress.level,
            false,
        );
        if !player.hero.identity.is_bot {
            self.career.backend.set_playing(addr, true);
        }
    }
    fn update_career_totals(&mut self) {
        for player in self
            .world
            .players
            .values()
            .filter(|p| p.joined)
            .chain(self.world.disconnected_sessions.values().map(|s| &s.player))
        {
            self.combat_log
                .ledger
                .update_earned_gold(player.hero.identity.id, player.economy.earned_gold);
        }
        for player in self.world.players.values().filter(|p| p.joined) {
            self.combat_log.ledger.update_player(
                player.hero.identity.id,
                player.hero.progress.level,
                false,
            );
        }
        for session in self.world.disconnected_sessions.values() {
            self.combat_log.ledger.update_player(
                session.player.hero.identity.id,
                session.player.hero.progress.level,
                true,
            );
        }
    }

    pub(crate) fn finish_career_round(
        &mut self,
        outcome: MatchOutcome,
        winner: Option<Team>,
        now: Instant,
    ) {
        if self
            .career
            .round
            .as_ref()
            .is_none_or(|round| round.finalized)
        {
            return;
        }
        self.update_career_totals();
        let participants = if self.combat_log.ledger.is_started() {
            Some(self.combat_log.ledger.freeze())
        } else {
            None
        };
        let round = self.career.round.as_mut().unwrap();
        round.finalized = true;
        round.result.outcome = outcome;
        round.result.winner = winner;
        round.result.ended_at_ms = utc_ms().max(round.result.started_at_ms);
        round.result.duration_ms = now
            .saturating_duration_since(round.started_at)
            .as_millis()
            .min(u64::MAX as u128) as u64;
        if let Some(participants) = participants {
            round.result.participants = participants;
        }
        if outcome != MatchOutcome::Completed {
            round.result.rated = false;
            round.result.winner = None;
            if !self.rules.local_results {
                round.result.unrated_reason = Some("The match did not finish normally.".into());
            }
        }
        for participant in &round.result.participants {
            self.career
                .last_results
                .insert(participant.player_id, round.result.clone());
        }
        if self.career.backend.enabled()
            && round.start_enqueued
            && !self.career.backend.settle(round.result.clone())
        {
            // Starts stop at the same bound, so one already-running finalization
            // always has space and cannot be discarded on worker saturation.
            self.career.pending.push_back(round.result.clone());
        }
    }

    pub(crate) fn checkpoint_career_round(&mut self, now: Instant) {
        if self.rules.local_results && self.match_service.worker().is_none() {
            self.update_career_totals();
            return;
        }
        if !matches!(self.world.game_state, GameState::Running) {
            return;
        }
        self.update_career_totals();
        let Some(round) = self.career.round.as_mut() else {
            return;
        };
        if round.finalized
            || now.saturating_duration_since(round.checkpoint_at) < CHECKPOINT_INTERVAL
        {
            return;
        }
        let mut checkpoint = round.result.clone();
        checkpoint.participants = self.combat_log.ledger.snapshot();
        checkpoint.duration_ms = now
            .saturating_duration_since(round.started_at)
            .as_millis()
            .min(u64::MAX as u128) as u64;
        if !self.career.backend.enabled() || self.career.backend.checkpoint(checkpoint) {
            round.checkpoint_at = now;
        }
    }

    /// A loading roster changed while durable allocation was pending. Keep the
    /// existing queue reservation, but retire this allocation before a new draft.
    pub(crate) fn invalidate_prematch_allocation(&mut self, now: Instant) {
        if self.match_started_at.is_none() && self.career.round.is_some() {
            self.finish_career_round(MatchOutcome::Abandoned, None, now);
            self.career.round = None;
        }
    }

    pub(crate) fn reset_career_round(&mut self) {
        let career_flow = self.career_flow_active();
        self.career.round = None;
        self.career.queue.release_selection();
        if career_flow && self.rules.career_flow {
            for (addr, player) in &mut self.world.players {
                player.joined = false;
                self.career.backend.set_playing(*addr, false);
            }
        }
    }

    /// A deliberate leave, as opposed to a timeout: the seat and any queue
    /// entry are released at once and nothing is kept for a session reclaim,
    /// so a guest can join again straight away with a new hero. Signed profiles
    /// still have at most one participant per round. The endpoint and its career
    /// authentication stay as they are.
    pub(crate) fn leave_match(&mut self, addr: SocketAddr, now: Instant) {
        let Some(player) = self.world.players.get_mut(&addr) else {
            return;
        };
        player.last_seen = now;
        let id = player.hero.identity.id;
        let level = player.hero.progress.level;
        let was_joined = player.joined;
        let session_id = player.session_id.take();
        if was_joined {
            self.combat_log
                .ledger
                .update_earned_gold(id, player.economy.earned_gold);
            player.timers.haste_expires_at = None;
            self.combat_log.ledger.update_player(id, level, true);
        }
        self.cancel_career_entry(id, now);
        self.career.blocked_starts.remove(&id);
        self.career.backend.set_playing(addr, false);
        if let Some(player) = self.world.players.get_mut(&addr) {
            player.joined = false;
            player.career_profile = None;
            player.join_error = None;
            if was_joined
                && self.combat_log.ledger.is_started()
                && !self.combat_log.ledger.is_frozen()
            {
                // A deliberate fresh admission can change hero and team and
                // resets gameplay progression. Never recycle its old round
                // identity: delayed damage still belongs to that retired row.
                // Timeout/session reclaim does not take this path.
                player.hero.identity.id = self.world.next_player_id;
                self.world.next_player_id += 1;
                if let Some(result) = self.career.last_results.remove(&id) {
                    self.career
                        .last_results
                        .insert(player.hero.identity.id, result);
                }
                if let Some(result_id) = self.career.dismissed.remove(&id) {
                    self.career
                        .dismissed
                        .insert(player.hero.identity.id, result_id);
                }
            }
        }
        if let Some(session_id) = session_id {
            self.world.disconnected_sessions.remove(&session_id);
        }
        if was_joined {
            self.invalidate_prematch_roster(now);
            println!(
                "MATCH_METRIC event=leave epoch={} match={} player={} elapsed_ms={}",
                self.server_epoch,
                self.match_id,
                id,
                self.elapsed_match_ms(now)
            );
        }
    }

    pub(crate) fn disconnect_career_player(&mut self, addr: SocketAddr, now: Instant) {
        if let Some(player) = self.world.players.get_mut(&addr) {
            self.combat_log
                .ledger
                .update_earned_gold(player.hero.identity.id, player.economy.earned_gold);
            player.timers.haste_expires_at = None;
        }
        if let Some(player) = self.world.players.get(&addr) {
            let id = player.hero.identity.id;
            self.combat_log
                .ledger
                .update_player(id, player.hero.progress.level, true);
            self.cancel_career_entry(id, now);
        }
        if let Some(player) = self.world.players.get(&addr) {
            self.career.blocked_starts.remove(&player.hero.identity.id);
        }
        self.career.backend.forget(addr);
        self.career.errors.remove(&addr);
    }

    pub(crate) fn poll_career(&mut self, now: Instant) {
        // Result delivery caches follow reclaimable identities. Durable pending
        // records remain independently owned by the worker/outbox after expiry.
        let retained_ids: HashSet<_> = self
            .world
            .players
            .values()
            .map(|p| p.hero.identity.id)
            .chain(
                self.world
                    .disconnected_sessions
                    .values()
                    .map(|s| s.player.hero.identity.id),
            )
            .collect();
        self.career
            .last_results
            .retain(|id, _| retained_ids.contains(id));
        self.career
            .dismissed
            .retain(|id, _| retained_ids.contains(id));
        self.career
            .errors
            .retain(|addr, _| self.world.players.contains_key(addr));
        self.career.backend.poll();
        let mut invalidated = Vec::new();
        for (addr, player) in &mut self.world.players {
            player.hero.identity.supporter_aura = if player.hero.identity.is_bot {
                None
            } else {
                self.career.backend.supporter_aura(*addr)
            };
            if player.career_profile.is_some()
                && self.career.backend.authenticated_session(*addr).is_none()
            {
                let queued = !matches!(
                    self.career.queue.view(
                        player.hero.identity.id,
                        self.rules.team_size as usize,
                        now
                    ),
                    QueueView::Idle
                );
                if player.joined || queued {
                    invalidated.push((*addr, player.hero.identity.id, player.hero.progress.level));
                }
                player.joined = false;
                player.hero.identity.supporter_aura = None;
            }
        }
        for (addr, player_id, level) in invalidated {
            self.combat_log.ledger.update_player(player_id, level, true);
            self.career.backend.set_playing(addr, false);
            // A selected endpoint is no longer authorized to hold a countdown
            // seat. Reuse normal cancellation so the remaining roster can queue.
            self.cancel_career_entry(player_id, now);
        }
        for (addr, request_id, preference) in self.career.backend.take_match_requests() {
            if let (Some(profile), Some(session)) = (
                self.career.backend.profile(addr),
                self.career.backend.authenticated_session(addr),
            ) {
                self.match_service
                    .enqueue(profile, session, request_id, preference, now);
            }
        }
        self.handle_rejected_career_start(now);
        for addr in self.career.backend.take_cancelled() {
            if let Some(profile) = self.career.backend.profile(addr) {
                self.match_service.cancel(&profile.profile_id);
            }
            if let Some(worker) = self.match_service.worker() {
                let member = self
                    .career
                    .backend
                    .profile(addr)
                    .zip(self.career.backend.authenticated_session(addr))
                    .is_some_and(|(profile, session)| {
                        worker
                            .manifest
                            .team(&profile.profile_id, &session)
                            .is_some()
                    });
                if self.match_started_at.is_none() && member {
                    let _ = crate::match_allocation::atomic_json(
                        &worker.directory.join("cancel.json"),
                        &true,
                    );
                }
            }
            if let Some(player) = self.world.players.get(&addr) {
                if player.joined && matches!(self.world.game_state, GameState::Running) {
                    continue;
                }
                let id = player.hero.identity.id;
                self.career.blocked_starts.remove(&id);
                self.career.errors.remove(&addr);
                self.cancel_career_entry(id, now);
                self.career.backend.set_playing(addr, false);
            }
        }
        while let Some(result) = self.career.pending.front() {
            if !self.career.backend.settle(result.clone()) {
                break;
            }
            self.career.pending.pop_front();
        }
        for saved in self.career.backend.take_settled() {
            for result in self.career.last_results.values_mut() {
                if result.result_id == saved.result_id {
                    *result = saved.clone();
                }
            }
            if let Some(round) = &mut self.career.round
                && round.result.result_id == saved.result_id
            {
                round.result = saved;
            }
        }
    }

    fn handle_rejected_career_start(&mut self, now: Instant) {
        if self.match_started_at.is_some() {
            return;
        }
        let Some((id, error, participants)) = self.career.round.as_ref().and_then(|round| {
            self.career
                .backend
                .start_rejected(&round.result.result_id)
                .map(|error| {
                    (
                        round.result.result_id.clone(),
                        error,
                        round.result.participants.clone(),
                    )
                })
        }) else {
            return;
        };
        // The worker confirms that its failed transaction was rolled back and
        // its Start retry removed. There is no durable match to finalize here.
        self.career.round = None;
        self.career.backend.forget_start(&id);
        self.career.queue.commit_selection();
        for participant in participants {
            self.career.queue.cancel(participant.player_id);
            self.career.blocked_starts.insert(participant.player_id);
            for (addr, player) in &mut self.world.players {
                if player.hero.identity.id == participant.player_id {
                    player.joined = false;
                    self.career.backend.set_playing(*addr, false);
                    self.career.errors.insert(*addr, format!("{error} Leave queue to clear this attempt, then retry after the other match ends."));
                }
            }
        }
        self.restart_round(now);
    }

    pub(crate) fn career_play_again(&mut self, addr: SocketAddr, now: Instant) {
        if self.match_service.is_public() {
            return;
        }
        let Some(player) = self.world.players.get_mut(&addr) else {
            return;
        };
        player.last_seen = now;
        let id = player.hero.identity.id;
        if player.joined && matches!(self.world.game_state, GameState::Running) {
            return;
        }
        if matches!(self.world.game_state, GameState::Victory { .. }) {
            self.record_match_metrics(now);
        }
        if let Some(result) = self.career.last_results.get(&id) {
            if self.rules.career_flow && self.career.backend.enabled() && !result.saved {
                self.career_error(
                    addr,
                    "Saving the result. Play again after storage acknowledges it.",
                );
                return;
            }
        } else {
            return;
        }
        if matches!(self.world.game_state, GameState::Victory { .. }) {
            self.restart_round(now);
        }
        if let Some(result) = self.career.last_results.remove(&id) {
            self.career.dismissed.insert(id, result.result_id);
        }
        self.career.errors.remove(&addr);
        let player = self.world.players.get_mut(&addr).unwrap();
        player.joined = false;
        let packet = ClientPacket::Join {
            prematch: player.draft.capable,
            team: player.hero.identity.team,
            character: player.hero.identity.character,
            hero_class: player.hero.identity.hero_class,
            avatar: player.hero.identity.avatar.clone(),
            sprite_character: player.hero.identity.sprite_character.clone(),
            session_id: player.session_id.clone(),
            passport_ticket: None,
        };
        // Cosmetics were already approved; this cannot replace the retained loadout.
        self.handle_packet_authorized(addr, packet, now);
    }

    pub(crate) fn career_view(&self, addr: SocketAddr, now: Instant) -> CareerView {
        let mut view = self.career.backend.view(addr);
        if let (Some(profile), Some(session)) = (
            self.career.backend.profile(addr),
            self.career.backend.authenticated_session(addr),
        ) {
            view.match_service = self.match_service.view(&profile.profile_id, &session, now);
            view.match_service_request_id =
                self.match_service.request_id(&profile.profile_id, &session);
        } else if self.match_service.is_lobby() {
            view.match_service = Some(shared::match_service::MatchServiceView::Idle);
        }
        if let Some(player) = self.world.players.get(&addr) {
            view.queue = if player.joined && matches!(self.world.game_state, GameState::Running) {
                QueueView::Playing
            } else {
                self.career
                    .queue
                    .view(player.hero.identity.id, self.rules.team_size as usize, now)
            };
            if let Some(result) = self.career.last_results.get(&player.hero.identity.id) {
                view.last_result = Some(result.clone());
            }
            if view.last_result.as_ref().is_some_and(|result| {
                self.career.dismissed.get(&player.hero.identity.id) == Some(&result.result_id)
            }) {
                view.last_result = None;
            }
            if let Some(round) = self.career.round.as_ref().filter(|round| !round.finalized)
                && self.match_started_at.is_none()
                && player.joined
            {
                view.error = self
                    .career
                    .backend
                    .start_error(&round.result.result_id)
                    .or_else(|| Some("Preparing durable match allocation…".into()));
            }
        }
        if let Some(error) = self.career.errors.get(&addr) {
            view.error = Some(error.clone());
        }
        view
    }

    pub(crate) fn send_career_views(&mut self, now: Instant) {
        if self
            .career
            .last_sent
            .is_some_and(|at| now.saturating_duration_since(at) < CAREER_SEND_INTERVAL)
        {
            return;
        }
        self.career.last_sent = Some(now);
        self.career.sequence = self.career.sequence.saturating_add(1);
        for (addr, player) in &self.world.players {
            if !player.career_capable {
                continue;
            }
            let result = encode_career_datagrams(
                self.career_view(*addr, now),
                self.server_epoch,
                self.career.sequence,
            )
            .and_then(|datagrams| {
                for bytes in datagrams {
                    let sent = self
                        .transport
                        .send_to(&bytes, *addr)
                        .map_err(|error| error.to_string())?;
                    if sent != bytes.len() {
                        return Err("Incomplete career datagram send".into());
                    }
                }
                Ok(())
            });
            if let Err(error) = result
                && let Some(suppressed) = self.snapshot_send_diagnostic.record(now)
            {
                eprintln!(
                    "Career transport failed for {addr}: {error}; suppressed {suppressed} similar errors"
                );
            }
        }
    }
}

#[test]
fn changed_hero_tower_damage_is_not_an_approved_default_map() {
    let mut map = shared::map::ResolvedMap::default();
    assert!(approved_default_map(&map));
    map.structures[0].stats.hero_damage_multiplier = 1.0;
    assert!(!approved_default_map(&map));
}
