//! Ephemeral, recipient-filtered social traffic. No SQL or asset downloads.
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::map::Team;
use shared::social::{
    Entitlements, SocialChannel, SocialCommand, SocialEvent, SocialEventKind, SocialRequest,
    SocialTeam, SocialView, allowed_reactions, normalize_chat_text, reaction_allowed,
    validate_request,
};
use shared::wire::{GameState, ServerPacket};

use crate::runtime::ServerRuntime;

const MAX_EVENTS: usize = 32;
const MAX_SENDERS: usize = 512;
const HISTORY_TTL: Duration = Duration::from_secs(30);
const LIMITER_TTL: Duration = Duration::from_secs(120);
const SEND_INTERVAL: Duration = Duration::from_millis(250);
const SOCIAL_FRAME_NAMESPACE: u64 = 1 << 62;

struct RecordedEvent {
    event: SocialEvent,
    at: Instant,
}
struct Receipt {
    request: SocialRequest,
    error: Option<String>,
}
struct RateBudget {
    tokens: f32,
    updated: Instant,
}
impl RateBudget {
    fn consume(&mut self, now: Instant) -> bool {
        self.tokens = (self.tokens
            + now.saturating_duration_since(self.updated).as_secs_f32() / 2.0)
            .min(3.0);
        self.updated = now;
        if self.tokens < 1.0 {
            false
        } else {
            self.tokens -= 1.0;
            true
        }
    }
}

#[derive(Default)]
pub(super) struct SocialRuntime {
    epoch: u64,
    round: u64,
    next_event: u64,
    sequence: u64,
    last_sent: Option<Instant>,
    events: VecDeque<RecordedEvent>,
    receipts: HashMap<u64, Receipt>,
    // Account/session identity survives endpoint replacement and round changes.
    limits: HashMap<String, RateBudget>,
}

struct Sender {
    id: u64,
    nickname: String,
    team: SocialTeam,
    session: String,
    rate_key: String,
    alive: bool,
    requires_signature: bool,
    entitlements: Entitlements,
}

impl SocialRuntime {
    fn sync_round(&mut self, epoch: u64, round: u64, now: Instant) {
        if self.epoch != epoch || self.round != round {
            // Framing keys contain epoch + sequence, not match_id. Preserve
            // sequence through rematches so delayed fragments cannot collide.
            if self.epoch != epoch {
                self.sequence = 0;
            }
            self.epoch = epoch;
            self.round = round;
            self.next_event = 0;
            self.last_sent = None;
            self.events.clear();
            self.receipts.clear();
        }
        self.events
            .retain(|event| now.saturating_duration_since(event.at) < HISTORY_TTL);
        self.limits
            .retain(|_, limit| now.saturating_duration_since(limit.updated) < LIMITER_TTL);
    }

    fn submit(&mut self, sender: Sender, request: SocialRequest, verified: bool, now: Instant) {
        // A stale packet cannot overwrite a current-round acknowledgement.
        if request.server_epoch != self.epoch
            || request.match_id != self.round
            || request.session_id != sender.session
        {
            return;
        }
        // An unsigned packet must not poison an authenticated player's request
        // high-water mark (for example by submitting u64::MAX before a valid ID).
        if sender.requires_signature && !verified {
            return;
        }
        if let Some(receipt) = self.receipts.get(&sender.id) {
            if request.request_id <= receipt.request.request_id {
                // Repeated UDP delivery is acknowledged by the retained receipt;
                // it never emits or consumes another rate token, even if altered.
                return;
            }
        } else if self.receipts.len() >= MAX_SENDERS {
            return;
        }
        let error = self
            .accept(&sender, &request, verified, now)
            .err()
            .map(str::to_owned);
        self.receipts.insert(sender.id, Receipt { request, error });
    }

    fn accept(
        &mut self,
        sender: &Sender,
        request: &SocialRequest,
        verified: bool,
        now: Instant,
    ) -> Result<(), &'static str> {
        validate_request(request)?;
        if sender.requires_signature && !verified {
            return Err("Sign in before sending messages from this profile.");
        }
        if matches!(request.command, SocialCommand::Subscribe) {
            return Ok(());
        }
        if !self.limits.contains_key(&sender.rate_key) && self.limits.len() >= MAX_SENDERS {
            return Err("Chat is busy. Retry shortly.");
        }
        let limit = self
            .limits
            .entry(sender.rate_key.clone())
            .or_insert(RateBudget {
                tokens: 3.0,
                updated: now,
            });
        if !limit.consume(now) {
            return Err("Slow down: chat and reactions share a short cooldown.");
        }
        let kind = match &request.command {
            SocialCommand::Subscribe => unreachable!("subscription emits no chat event"),
            SocialCommand::Chat { channel, text } => SocialEventKind::Chat {
                channel: *channel,
                text: normalize_chat_text(text)?,
            },
            SocialCommand::Reaction { reaction_id } => {
                if !sender.alive {
                    return Err("React while your hero is alive in the match.");
                }
                if !reaction_allowed(reaction_id, &sender.entitlements) {
                    return Err("This reaction pack is not unlocked for this session.");
                }
                SocialEventKind::Reaction {
                    reaction_id: reaction_id.clone(),
                }
            }
        };
        self.next_event = self.next_event.saturating_add(1);
        self.events.push_back(RecordedEvent {
            at: now,
            event: SocialEvent {
                id: self.next_event,
                player_id: sender.id,
                nickname: sender.nickname.clone(),
                team: sender.team,
                age_ms: 0,
                kind,
            },
        });
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
        Ok(())
    }

    fn view(&self, sender: &Sender, now: Instant) -> SocialView {
        let receipt = self.receipts.get(&sender.id);
        SocialView {
            events: self
                .events
                .iter()
                .filter(|record| {
                    now.saturating_duration_since(record.at) < HISTORY_TTL
                        && (!matches!(
                            &record.event.kind,
                            SocialEventKind::Chat {
                                channel: SocialChannel::Team,
                                ..
                            }
                        ) || record.event.team == sender.team)
                })
                .map(|record| {
                    let mut event = record.event.clone();
                    event.age_ms = now
                        .saturating_duration_since(record.at)
                        .as_millis()
                        .min(u32::MAX as u128) as u32;
                    event
                })
                .collect(),
            request_id: receipt.map(|receipt| receipt.request.request_id),
            error: receipt.and_then(|receipt| receipt.error.clone()),
            allowed_reactions: allowed_reactions(&sender.entitlements),
        }
    }
}

fn social_team(team: Team) -> SocialTeam {
    match team {
        Team::Green => SocialTeam::Green,
        Team::Blue => SocialTeam::Blue,
    }
}

impl ServerRuntime {
    fn social_sender(&self, addr: SocketAddr) -> Option<Sender> {
        let player = self.world.players.get(&addr)?;
        if !player.joined || player.hero.identity.is_bot || !player.protocol_compatible {
            return None;
        }
        let session = player.session_id.clone()?;
        let profile = self
            .career
            .backend
            .profile(addr)
            .or_else(|| player.career_profile.clone());
        let rate_key = profile.as_ref().map_or_else(
            || format!("session:{session}"),
            |p| format!("profile:{}", p.profile_id),
        );
        Some(Sender {
            id: player.hero.identity.id,
            nickname: profile.as_ref().map_or_else(
                || format!("Guest {}", player.hero.identity.id),
                |p| p.nickname.clone(),
            ),
            team: social_team(player.hero.identity.team),
            session,
            rate_key,
            alive: player.hero.hp > 0.0 && matches!(self.world.game_state, GameState::Running),
            requires_signature: profile.is_some()
                || self.career.backend.authenticated_session(addr).is_some(),
            // Free starter pack works offline. Operator/Passport grants must be
            // installed by a trusted entitlement adapter, never a player packet.
            entitlements: Entitlements::default(),
        })
    }

    pub(super) fn handle_social_request(
        &mut self,
        addr: SocketAddr,
        request: SocialRequest,
        verified: bool,
        now: Instant,
    ) {
        self.social
            .sync_round(self.server_epoch, self.match_id, now);
        let Some(sender) = self.social_sender(addr) else {
            return;
        };
        if !matches!(
            self.world.game_state,
            GameState::Running | GameState::Victory { .. }
        ) {
            return;
        }
        if let Some(player) = self.world.players.get_mut(&addr) {
            player.last_seen = now;
        }
        self.social.submit(sender, request, verified, now);
    }

    pub(super) fn send_social_views(&mut self, now: Instant) {
        self.social
            .sync_round(self.server_epoch, self.match_id, now);
        if self
            .social
            .last_sent
            .is_some_and(|last| now.saturating_duration_since(last) < SEND_INTERVAL)
        {
            return;
        }
        self.social.last_sent = Some(now);
        self.social.sequence = self.social.sequence.saturating_add(1);
        for (addr, player) in &self.world.players {
            // Legacy framed clients have no Social packet decoder. Only a
            // scoped request opts a joined actor into this separate stream.
            if !player.framed_snapshots
                || !self.social.receipts.contains_key(&player.hero.identity.id)
            {
                continue;
            }
            let Some(sender) = self.social_sender(*addr) else {
                continue;
            };
            let packet = ServerPacket::Social {
                server_epoch: self.server_epoch,
                match_id: self.match_id,
                sequence: self.social.sequence,
                social: self.social.view(&sender, now),
            };
            let result = serde_json::to_vec(&packet)
                .map_err(|e| e.to_string())
                .and_then(|payload| {
                    shared::transport::encode_snapshot(
                        &payload,
                        self.server_epoch,
                        SOCIAL_FRAME_NAMESPACE | self.social.sequence,
                    )
                    .map_err(|e| e.to_string())
                });
            match result {
                Ok(datagrams) => {
                    for datagram in datagrams {
                        if let Err(error) = self.transport.send_to(&datagram, *addr) {
                            if self.snapshot_send_diagnostic.record(now).is_some() {
                                eprintln!("Social send failed: {error}");
                            }
                            break;
                        }
                    }
                }
                Err(error) => {
                    if self.snapshot_send_diagnostic.record(now).is_some() {
                        eprintln!("Social framing failed: {error}");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
