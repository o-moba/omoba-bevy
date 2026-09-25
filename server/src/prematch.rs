//! Opt-in server-owned draft. Legacy callers remain immediately ready and never
//! silently acquire a requirement to implement this protocol.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::prematch::{
    COUNTDOWN_MS, DraftPlayer, LOADING_TIMEOUT_MS, PrematchAction, PrematchPhase, PrematchRequest,
    PrematchSnapshot, Role,
};
use shared::wire::{ClientPacket, GameState};

use crate::entities::ConnectedPlayer;
use crate::formation::{joined_count, start_match_running};
use crate::match_rules::{MatchRules, RosterPolicy};
use crate::passport_admission;
use crate::runtime::ServerRuntime;
use crate::session::reset_player_round;

#[derive(Default)]
pub(super) struct DraftState {
    pub capable: bool,
    pub role: Role,
    pub locked: bool,
    pub loaded: bool,
    pub request_id: u64,
    pub acknowledged_request_id: u64,
    pub error: Option<String>,
}

#[derive(Default)]
pub(super) struct PrematchRuntime {
    generation: u64,
    roster: Vec<u64>,
    phase: Option<PrematchPhase>,
    deadline: Option<Instant>,
    // Bind asynchronous ownership checks to the original admitted identity.
    pending: HashMap<SocketAddr, (u64, u64)>,
}

impl ServerRuntime {
    #[cfg(test)]
    pub(crate) fn prematch_generation_for_test(&self) -> u64 {
        self.prematch.generation
    }

    pub(crate) fn prematch_required(&self) -> bool {
        self.match_started_at.is_none()
            && !matches!(self.world.game_state, GameState::Victory { .. })
            && self
                .world
                .players
                .values()
                .any(|p| p.joined && p.draft.capable)
    }

    pub(crate) fn invalidate_prematch_roster(&mut self, now: Instant) {
        if self.match_started_at.is_none() && self.prematch.phase.is_some() {
            self.prematch.roster.clear();
            self.reset_draft_generation(
                Some("The roster changed. Review your team and lock in again."),
                now,
            );
        }
    }

    fn reset_draft_generation(&mut self, message: Option<&str>, now: Instant) {
        self.invalidate_prematch_allocation(now);
        self.prematch.generation = self.prematch.generation.saturating_add(1);
        self.prematch.phase = Some(PrematchPhase::Draft);
        self.prematch.deadline = None;
        self.prematch.pending.clear();
        for player in self.world.players.values_mut().filter(|p| p.joined) {
            player.draft.locked = false;
            player.draft.loaded = false;
            player.draft.acknowledged_request_id = player.draft.request_id;
            player.draft.error = message.map(str::to_owned);
        }
    }

    /// Returns true while this module owns the formation boundary.
    pub(crate) fn tick_prematch(&mut self, now: Instant) -> bool {
        if !self.prematch_required() {
            self.prematch.phase = None;
            self.prematch.deadline = None;
            self.prematch.roster.clear();
            self.prematch.pending.clear();
            return false;
        }
        let mut roster: Vec<_> = self
            .world
            .players
            .values()
            .filter(|p| p.joined)
            .map(|p| p.hero.identity.id)
            .collect();
        roster.sort_unstable();
        if roster != self.prematch.roster {
            let first = self.prematch.roster.is_empty();
            self.prematch.roster = roster;
            self.reset_draft_generation(
                (!first).then_some("The roster changed. Review your team and lock in again."),
                now,
            );
        }
        let ready = joined_count(&self.world.players);
        let needed = match self.rules.prematch_roster {
            RosterPolicy::Present => ready.max(1),
            RosterPolicy::Full => self.rules.roster_size(),
        };
        let all = |test: fn(&DraftState) -> bool| {
            self.world
                .players
                .values()
                .filter(|p| p.joined && !p.hero.identity.is_bot && p.draft.capable)
                .all(|p| test(&p.draft))
        };
        match self.prematch.phase.unwrap_or(PrematchPhase::Draft) {
            PrematchPhase::Draft => {
                self.world.game_state = GameState::Forming { ready, needed };
                if ready >= needed && all(|d| d.locked) {
                    self.prematch.phase = Some(PrematchPhase::Countdown);
                    self.prematch.deadline = Some(now + Duration::from_millis(COUNTDOWN_MS.into()));
                    self.world.game_state = GameState::Starting {
                        countdown_ms: COUNTDOWN_MS,
                    };
                }
            }
            PrematchPhase::Countdown => {
                let remaining = remaining_ms(self.prematch.deadline, now);
                self.world.game_state = GameState::Starting {
                    countdown_ms: remaining,
                };
                if remaining == 0 {
                    self.prematch.phase = Some(PrematchPhase::Loading);
                    self.prematch.deadline =
                        Some(now + Duration::from_millis(LOADING_TIMEOUT_MS.into()));
                    self.world.game_state = GameState::Forming { ready, needed };
                }
            }
            PrematchPhase::Loading => {
                self.world.game_state = GameState::Forming { ready, needed };
                let all_loaded = all(|d| d.loaded);
                if remaining_ms(self.prematch.deadline, now) == 0 {
                    self.reset_draft_generation(Some(if all_loaded {
                        "Match start took too long. Lock in again to retry, or leave."
                    } else {
                        "Asset loading timed out. Retry your selection and lock in again, or leave."
                    }), now);
                } else if all_loaded {
                    // No gameplay, income or spawns run until durable start ACK.
                    if self.begin_career_round(now) {
                        start_match_running(&mut self.world, now);
                        self.track_round_start(now);
                        self.prematch.phase = None;
                    }
                }
            }
        }
        true
    }

    fn draft_request_valid(&self, addr: SocketAddr, request: &PrematchRequest) -> bool {
        self.prematch_required()
            && request.server_epoch == self.server_epoch
            && request.match_id == self.match_id
            && request.generation == self.prematch.generation
            && self.world.players.get(&addr).is_some_and(|p| {
                p.joined && p.draft.capable && p.protocol_compatible && !p.hero.identity.is_bot
            })
            && self.world.players.get(&addr).is_some_and(|p| {
                p.career_profile.is_none()
                    || self.career.backend.authenticated_session(addr).is_some()
            })
    }

    pub(crate) fn handle_prematch(
        &mut self,
        addr: SocketAddr,
        request: PrematchRequest,
        now: Instant,
    ) {
        self.maintain_roster(now);
        self.fill_practice_bots(now);
        self.tick_prematch(now);
        if !self.draft_request_valid(addr, &request) {
            return;
        }
        let player = self.world.players.get_mut(&addr).unwrap();
        player.last_seen = now;
        if request.request_id == 0 || request.request_id <= player.draft.request_id {
            return;
        }
        let previous_acknowledgment = player.draft.acknowledged_request_id;
        player.draft.request_id = request.request_id;
        player.draft.acknowledged_request_id = request.request_id;
        player.draft.error = None;
        let phase = self.prematch.phase;
        let allowed = match &request.action {
            PrematchAction::Select { .. } => {
                phase == Some(PrematchPhase::Draft) && !player.draft.locked
            }
            PrematchAction::Lock { .. } => phase == Some(PrematchPhase::Draft),
            PrematchAction::Loaded => phase == Some(PrematchPhase::Loading),
        };
        if !allowed {
            player.draft.error =
                Some("This selection is frozen. Wait for the match or the next draft.".into());
            return;
        }
        if let PrematchAction::Select { avatar, .. } = &request.action {
            // Re-selecting the already admitted exact avatar (e.g. changing an
            // intended role) needs no second single-use ownership ticket.
            if avatar.as_deref().map(str::trim) == player.hero.identity.avatar.as_deref() {
                self.apply_prematch(addr, request, now);
                return;
            }
            if self.passport_admissions.is_pending(addr) {
                player.draft.error = Some(
                    "Wait for avatar ownership verification, then retry your selection.".into(),
                );
                return;
            }
            let identity = player.hero.identity.id;
            let session = player.session_id.clone();
            let packet = ClientPacket::Prematch {
                request: request.clone(),
            };
            match self
                .passport_admissions
                .begin_with_session(addr, &packet, session.as_deref())
            {
                passport_admission::Admission::Denied => {
                    player.draft.error = Some(
                        "This avatar is unavailable or requires ownership verification.".into(),
                    );
                    return;
                }
                passport_admission::Admission::Pending => {
                    // An in-flight ownership check is not a final decision. A
                    // client retries the same ID until the accepted roster or
                    // rejection arrives; never tell it to issue another ticket.
                    player.draft.acknowledged_request_id = previous_acknowledgment;
                    self.prematch
                        .pending
                        .insert(addr, (identity, request.request_id));
                    return;
                }
                passport_admission::Admission::Free => {}
            }
        }
        self.apply_prematch(addr, request, now);
    }

    pub(crate) fn complete_prematch_admission(
        &mut self,
        addr: SocketAddr,
        request: PrematchRequest,
        allowed: bool,
        now: Instant,
    ) {
        let expected = self.prematch.pending.remove(&addr);
        if expected
            != self
                .world
                .players
                .get(&addr)
                .map(|p| (p.hero.identity.id, request.request_id))
            || !self.draft_request_valid(addr, &request)
            || self.world.players[&addr].draft.request_id != request.request_id
        {
            return;
        }
        self.world
            .players
            .get_mut(&addr)
            .unwrap()
            .draft
            .acknowledged_request_id = request.request_id;
        if allowed {
            self.apply_prematch(addr, request, now);
        } else {
            self.world.players.get_mut(&addr).unwrap().draft.error =
                Some("This avatar is unavailable or requires ownership verification.".into());
        }
    }

    fn apply_prematch(&mut self, addr: SocketAddr, request: PrematchRequest, now: Instant) {
        // A verified asynchronous reply is still subject to the current stage.
        if !self.draft_request_valid(addr, &request) {
            return;
        }
        let player = self.world.players.get_mut(&addr).unwrap();
        match request.action {
            PrematchAction::Select {
                character,
                hero_class,
                avatar,
                sprite_character,
                role,
                ..
            } if self.prematch.phase == Some(PrematchPhase::Draft) && !player.draft.locked => {
                let normalized = omoba_passport::avatars::normalize_avatar_slug(avatar.as_deref());
                if avatar.is_some() && normalized.is_none() {
                    player.draft.error = Some(
                        "This avatar is no longer available. Choose another character.".into(),
                    );
                    return;
                }
                player.hero.identity.character = character;
                player.hero.identity.hero_class = hero_class;
                player.hero.identity.avatar = normalized.map(str::to_owned);
                player.hero.identity.sprite_character = Some(
                    shared::normalize_sprite_character_id(sprite_character.as_deref()).to_owned(),
                );
                player.draft.role = role;
                player.draft.loaded = false;
                reset_player_round(player, &self.world.map_layout, now);
            }
            PrematchAction::Lock { locked }
                if self.prematch.phase == Some(PrematchPhase::Draft) =>
            {
                // Never freeze a roster while a changed avatar is being checked.
                if locked && self.prematch.pending.contains_key(&addr) {
                    player.draft.error =
                        Some("Wait for avatar ownership verification before locking in.".into());
                    return;
                }
                player.draft.locked = locked;
            }
            PrematchAction::Loaded if self.prematch.phase == Some(PrematchPhase::Loading) => {
                player.draft.loaded = true
            }
            _ => {}
        }
        self.tick_prematch(now);
    }
}

fn remaining_ms(deadline: Option<Instant>, now: Instant) -> u32 {
    deadline.map_or(0, |d| {
        d.saturating_duration_since(now)
            .as_millis()
            .min(u128::from(u32::MAX)) as u32
    })
}

pub(super) fn snapshot(
    runtime: &PrematchRuntime,
    players: &HashMap<SocketAddr, ConnectedPlayer>,
    recipient: &ConnectedPlayer,
    rules: MatchRules,
    now: Instant,
) -> Option<PrematchSnapshot> {
    if !recipient.draft.capable || !recipient.joined {
        return None;
    }
    let phase = runtime.phase?;
    let mut roster: Vec<_> = players
        .values()
        .filter(|p| p.joined)
        .map(|p| DraftPlayer {
            player_id: p.hero.identity.id,
            nickname: p.career_profile.as_ref().map_or_else(
                || {
                    format!(
                        "{} {}",
                        if p.hero.identity.is_bot {
                            "Bot"
                        } else {
                            "Guest"
                        },
                        p.hero.identity.id
                    )
                },
                |v| v.nickname.clone(),
            ),
            team: p.hero.identity.team,
            character: p.hero.identity.character,
            hero_class: p.hero.identity.hero_class,
            avatar: p.hero.identity.avatar.clone(),
            sprite_character: p.hero.identity.sprite_character.clone(),
            role: p.draft.role,
            is_bot: p.hero.identity.is_bot,
            locked: p.hero.identity.is_bot || !p.draft.capable || p.draft.locked,
            loaded: p.hero.identity.is_bot || !p.draft.capable || p.draft.loaded,
        })
        .collect();
    roster.sort_by_key(|p| p.player_id);
    Some(PrematchSnapshot {
        generation: runtime.generation,
        phase,
        remaining_ms: remaining_ms(runtime.deadline, now),
        needed: match rules.prematch_roster {
            RosterPolicy::Present => players.values().filter(|p| p.joined).count().max(1) as u32,
            RosterPolicy::Full => rules.roster_size(),
        },
        players: roster,
        last_request_id: recipient.draft.acknowledged_request_id,
        error: recipient.draft.error.clone(),
    })
}

#[cfg(test)]
mod tests;
