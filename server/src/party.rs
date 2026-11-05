//! Parties: players on this server who group up, see each other in the party
//! lobby and are seated on one team. [`PartyState`] is the pure rule set
//! (invites, membership, leader, launch); the `ServerRuntime` glue at the end
//! feeds it packets and presence and sends every subscriber its view.
//!
//! State is in memory on the server the players are connected to: the
//! standalone/practice server, or the public lobby. Match workers do not run
//! parties; members of a public match are "in match" while this lobby holds
//! their allocation and "away" otherwise, until they return to the lobby and
//! are rebound by their career profile.
//!
//! Sending: a presence heartbeat (every client, every 2 s) never forces a
//! broadcast. The periodic pass (every 500 ms) builds the online list once
//! ([`ViewContext`]) and derives each viewer's view from it. A state-changing
//! command answers its issuer at once, with the refusal code if it was
//! refused, and a successful invite also reaches the invitee at once;
//! everyone else sees the change on the next periodic pass.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::match_service::MatchPreference;
use shared::party::{
    MAX_PARTY_ONLINE, MAX_PARTY_SIZE, OnlinePlayer, PARTY_INVITE_TTL_SECS, PartyCommand, PartyInfo,
    PartyInvite, PartyLaunch, PartyMember, PartyView, normalize_party_nickname,
};
use shared::wire::ServerPacket;

use crate::runtime::ServerRuntime;

/// A subscriber that stops announcing itself is no longer "online".
const PRESENCE_TTL: Duration = Duration::from_secs(8);
/// A guest member that vanished is dropped after this long.
const GUEST_GRACE: Duration = Duration::from_secs(60);
/// A profile member may be in a public match on another process.
const PROFILE_GRACE: Duration = Duration::from_secs(45 * 60);
/// A launched party holds the practice draft for its missing members this long.
pub(crate) const LAUNCH_GATHER: Duration = Duration::from_secs(60);
pub(crate) const SEND_INTERVAL: Duration = Duration::from_millis(500);
/// A target that declined is not re-invited by the same sender for this long.
pub(crate) const DECLINE_COOLDOWN: Duration = Duration::from_secs(60);
/// Invites one sender may issue within [`INVITE_RATE_WINDOW`].
pub(crate) const INVITES_PER_WINDOW: usize = 10;
pub(crate) const INVITE_RATE_WINDOW: Duration = Duration::from_secs(60);
/// A viewer's friend list is re-read from the career view this often; the
/// career view is a clone, so it is not taken on every pass for every viewer.
const FRIENDS_CACHE_TTL: Duration = Duration::from_secs(2);
/// Party views are framed like social views, in their own tick namespace.
const PARTY_FRAME_NAMESPACE: u64 = (1 << 63) | (1 << 62);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartyError {
    NotOnline,
    SelfInvite,
    AlreadyMember,
    PartyFull,
    NoInvite,
    NotLeader,
    NoParty,
    /// The target declined this sender's invite less than a minute ago.
    InviteDeclined,
    /// The sender issued more than [`INVITES_PER_WINDOW`] invites in a minute.
    InviteRate,
}

impl PartyError {
    /// The stable code the client localizes (`PartyView::error`).
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::NotOnline => "not_online",
            Self::SelfInvite => "self_invite",
            Self::AlreadyMember => "already_member",
            Self::PartyFull => "party_full",
            Self::NoInvite => "no_invite",
            Self::NotLeader => "not_leader",
            Self::NoParty => "no_party",
            Self::InviteDeclined => "invite_declined",
            Self::InviteRate => "invite_rate",
        }
    }
}

#[derive(Debug, Clone)]
struct Presence {
    addr: SocketAddr,
    nickname: String,
    avatar: Option<String>,
    profile_id: Option<String>,
    last_seen: Instant,
}

#[derive(Debug, Clone)]
struct Member {
    player_id: u64,
    profile_id: Option<String>,
    nickname: String,
    avatar: Option<String>,
    away_since: Option<Instant>,
}

#[derive(Debug, Clone)]
struct Party {
    leader: u64,
    members: Vec<Member>,
    launch: Option<PartyLaunch>,
    launched_at: Option<Instant>,
}

#[derive(Debug, Clone)]
struct Invite {
    party_id: u64,
    from: u64,
    to: u64,
    expires: Instant,
}

#[derive(Default)]
pub(crate) struct PartyState {
    presence: HashMap<u64, Presence>,
    parties: BTreeMap<u64, Party>,
    invites: Vec<Invite>,
    /// `(from, to, when)` of declined invites, for the re-invite cooldown.
    declined: Vec<(u64, u64, Instant)>,
    /// When each sender's recent invites went out, for the rate limit.
    invite_log: HashMap<u64, Vec<Instant>>,
    /// Each subscriber's accepted friends (profile ids), as of `Instant`.
    friends_cache: HashMap<SocketAddr, (Instant, HashSet<String>)>,
    next_party_id: u64,
    launch_sequence: u64,
    sequence: u64,
    last_sent: Option<Instant>,
    send_diagnostic: crate::runtime::RateLimitedDiagnostic,
}

/// One online player as every viewer sees it, before the viewer's own marks
/// (friend, invited) are applied.
struct OnlineRow {
    player: OnlinePlayer,
    profile_id: Option<String>,
    sort_name: String,
}

/// What every view of one pass shares: built once, read for each viewer.
pub(crate) struct ViewContext {
    now: Instant,
    in_match: HashSet<u64>,
    party_of: HashMap<u64, u64>,
    /// Every online player; a viewer's list is picked from it.
    online: Vec<OnlineRow>,
    /// `(party_id, invitee)` of every live invite.
    invited: HashSet<(u64, u64)>,
}

/// Only shipped roster avatars travel to other players; anything else (an
/// SDK model the others cannot load, garbage) shows the default model.
fn roster_avatar(avatar: Option<String>) -> Option<String> {
    avatar
        .map(|slug| slug.trim().to_owned())
        .filter(|slug| omoba_passport::avatars::avatar_definition(slug).is_some())
}

impl PartyState {
    pub(crate) fn presence(
        &mut self,
        player_id: u64,
        addr: SocketAddr,
        nickname: &str,
        avatar: Option<String>,
        profile_id: Option<String>,
        now: Instant,
    ) {
        let nickname = normalize_party_nickname(nickname);
        let avatar = roster_avatar(avatar);
        // A profile returning from a public match (a new endpoint and player
        // id) takes its old party seat back.
        if let Some(profile) = &profile_id
            && self.party_of(player_id).is_none()
        {
            let old = self.parties.values().find_map(|party| {
                party
                    .members
                    .iter()
                    .find(|m| m.profile_id.as_ref() == Some(profile) && m.player_id != player_id)
                    .map(|m| m.player_id)
            });
            if let Some(old) = old
                && !self.presence.contains_key(&old)
            {
                self.rebind(old, player_id);
            }
        }
        for party in self.parties.values_mut() {
            if let Some(member) = party.members.iter_mut().find(|m| m.player_id == player_id) {
                member.nickname = nickname.clone();
                member.avatar = avatar.clone();
                member.profile_id = profile_id.clone();
                member.away_since = None;
            }
        }
        self.presence.insert(
            player_id,
            Presence {
                addr,
                nickname,
                avatar,
                profile_id,
                last_seen: now,
            },
        );
    }

    fn rebind(&mut self, old: u64, new: u64) {
        for party in self.parties.values_mut() {
            if party.leader == old {
                party.leader = new;
            }
            for member in &mut party.members {
                if member.player_id == old {
                    member.player_id = new;
                }
            }
        }
        for invite in &mut self.invites {
            if invite.from == old {
                invite.from = new;
            }
            if invite.to == old {
                invite.to = new;
            }
        }
        for (from, to, _) in &mut self.declined {
            if *from == old {
                *from = new;
            }
            if *to == old {
                *to = new;
            }
        }
        if let Some(log) = self.invite_log.remove(&old) {
            self.invite_log.entry(new).or_default().extend(log);
        }
    }

    fn online(&self, player_id: u64, now: Instant) -> bool {
        self.presence
            .get(&player_id)
            .is_some_and(|p| now.saturating_duration_since(p.last_seen) < PRESENCE_TTL)
    }

    /// The endpoint an online player announced from.
    pub(crate) fn addr_of(&self, player_id: u64) -> Option<SocketAddr> {
        self.presence.get(&player_id).map(|p| p.addr)
    }

    pub(crate) fn party_id_of(&self, player_id: u64) -> Option<u64> {
        self.parties
            .iter()
            .find(|(_, party)| party.members.iter().any(|m| m.player_id == player_id))
            .map(|(id, _)| *id)
    }

    fn party_of(&self, player_id: u64) -> Option<&Party> {
        self.party_id_of(player_id).map(|id| &self.parties[&id])
    }

    /// The other members of `player_id`'s party.
    pub(crate) fn mates(&self, player_id: u64) -> Vec<u64> {
        self.party_of(player_id).map_or_else(Vec::new, |party| {
            party
                .members
                .iter()
                .map(|m| m.player_id)
                .filter(|id| *id != player_id)
                .collect()
        })
    }

    /// Members that are not away; a public queue waits for exactly these.
    pub(crate) fn present_size(&self, party_id: u64) -> usize {
        self.parties.get(&party_id).map_or(0, |party| {
            party
                .members
                .iter()
                .filter(|m| m.away_since.is_none())
                .count()
        })
    }

    fn member_from_presence(&self, player_id: u64) -> Member {
        let presence = &self.presence[&player_id];
        Member {
            player_id,
            profile_id: presence.profile_id.clone(),
            nickname: presence.nickname.clone(),
            avatar: presence.avatar.clone(),
            away_since: None,
        }
    }

    pub(crate) fn invite(&mut self, from: u64, to: u64, now: Instant) -> Result<u64, PartyError> {
        if from == to {
            return Err(PartyError::SelfInvite);
        }
        if !self.online(from, now) || !self.online(to, now) {
            return Err(PartyError::NotOnline);
        }
        if self.declined.iter().any(|(f, t, at)| {
            *f == from && *t == to && now.saturating_duration_since(*at) < DECLINE_COOLDOWN
        }) {
            return Err(PartyError::InviteDeclined);
        }
        let recent = self.invite_log.get(&from).map_or(0, |sent| {
            sent.iter()
                .filter(|at| now.saturating_duration_since(**at) < INVITE_RATE_WINDOW)
                .count()
        });
        if recent >= INVITES_PER_WINDOW {
            return Err(PartyError::InviteRate);
        }
        let party_id = match self.party_id_of(from) {
            Some(id) => id,
            None => {
                self.next_party_id += 1;
                let id = self.next_party_id;
                let leader = self.member_from_presence(from);
                self.parties.insert(
                    id,
                    Party {
                        leader: from,
                        members: vec![leader],
                        launch: None,
                        launched_at: None,
                    },
                );
                id
            }
        };
        let party = &self.parties[&party_id];
        if party.members.iter().any(|m| m.player_id == to) {
            return Err(PartyError::AlreadyMember);
        }
        if party.members.len() >= MAX_PARTY_SIZE {
            return Err(PartyError::PartyFull);
        }
        self.invites
            .retain(|i| !(i.party_id == party_id && i.to == to));
        self.invites.push(Invite {
            party_id,
            from,
            to,
            expires: now + Duration::from_secs(PARTY_INVITE_TTL_SECS),
        });
        self.invite_log.entry(from).or_default().push(now);
        Ok(party_id)
    }

    pub(crate) fn accept(
        &mut self,
        player_id: u64,
        party_id: u64,
        now: Instant,
    ) -> Result<(), PartyError> {
        let Some(index) = self
            .invites
            .iter()
            .position(|i| i.party_id == party_id && i.to == player_id && i.expires > now)
        else {
            return Err(PartyError::NoInvite);
        };
        if !self.parties.contains_key(&party_id) {
            self.invites.remove(index);
            return Err(PartyError::NoParty);
        }
        if self.parties[&party_id].members.len() >= MAX_PARTY_SIZE {
            return Err(PartyError::PartyFull);
        }
        if !self.online(player_id, now) {
            return Err(PartyError::NotOnline);
        }
        self.invites.remove(index);
        // One party at a time: accepting leaves the current one.
        if self.party_id_of(player_id).is_some() {
            self.leave(player_id);
        }
        let member = self.member_from_presence(player_id);
        if let Some(party) = self.parties.get_mut(&party_id) {
            party.members.push(member);
        }
        self.invites.retain(|i| i.to != player_id);
        Ok(())
    }

    /// Drops the invite; its sender may not invite this player again for
    /// [`DECLINE_COOLDOWN`].
    pub(crate) fn decline(&mut self, player_id: u64, party_id: u64, now: Instant) {
        for invite in &self.invites {
            if invite.party_id == party_id && invite.to == player_id {
                self.declined.push((invite.from, player_id, now));
            }
        }
        self.invites
            .retain(|i| !(i.party_id == party_id && i.to == player_id));
    }

    pub(crate) fn leave(&mut self, player_id: u64) {
        let Some(party_id) = self.party_id_of(player_id) else {
            return;
        };
        let party = self.parties.get_mut(&party_id).unwrap();
        party.members.retain(|m| m.player_id != player_id);
        if party.leader == player_id
            && let Some(next) = party.members.first()
        {
            party.leader = next.player_id;
        }
        self.invites
            .retain(|i| !(i.party_id == party_id && i.from == player_id));
        self.dissolve_empty(party_id);
    }

    /// Takes a player out of presence, its party and every invite: a
    /// sanction hides it (LR-15), so nobody sees it online or can invite it.
    pub(crate) fn withdraw(&mut self, player_id: u64) {
        self.leave(player_id);
        self.presence.remove(&player_id);
        let invited: Vec<u64> = self
            .invites
            .iter()
            .filter(|i| i.to == player_id)
            .map(|i| i.party_id)
            .collect();
        self.invites.retain(|i| i.to != player_id);
        for party_id in invited {
            self.dissolve_empty(party_id);
        }
    }

    pub(crate) fn kick(&mut self, leader: u64, target: u64) -> Result<(), PartyError> {
        let party_id = self.party_id_of(leader).ok_or(PartyError::NoParty)?;
        if self.parties[&party_id].leader != leader {
            return Err(PartyError::NotLeader);
        }
        if leader == target || self.party_id_of(target) != Some(party_id) {
            return Err(PartyError::NoParty);
        }
        self.leave(target);
        Ok(())
    }

    pub(crate) fn launch(
        &mut self,
        player_id: u64,
        preference: MatchPreference,
        now: Instant,
    ) -> Result<PartyLaunch, PartyError> {
        let party_id = self.party_id_of(player_id).ok_or(PartyError::NoParty)?;
        let party = self.parties.get_mut(&party_id).unwrap();
        if party.leader != player_id {
            return Err(PartyError::NotLeader);
        }
        self.launch_sequence += 1;
        let launch = PartyLaunch {
            sequence: self.launch_sequence,
            preference,
        };
        party.launch = Some(launch);
        party.launched_at = Some(now);
        Ok(launch)
    }

    /// A party with one member and nothing pending is no party at all.
    fn dissolve_empty(&mut self, party_id: u64) {
        let Some(party) = self.parties.get(&party_id) else {
            return;
        };
        let pending = self.invites.iter().any(|i| i.party_id == party_id);
        if party.members.is_empty() || (party.members.len() == 1 && !pending) {
            self.parties.remove(&party_id);
            self.invites.retain(|i| i.party_id != party_id);
        }
    }

    /// Expires invites, presence, decline cooldowns and invite rate records,
    /// marks vanished members away and drops them after their grace period.
    pub(crate) fn prune(&mut self, now: Instant) {
        self.invites.retain(|i| i.expires > now);
        self.declined
            .retain(|(_, _, at)| now.saturating_duration_since(*at) < DECLINE_COOLDOWN);
        self.invite_log.retain(|_, sent| {
            sent.retain(|at| now.saturating_duration_since(*at) < INVITE_RATE_WINDOW);
            !sent.is_empty()
        });
        self.presence
            .retain(|_, p| now.saturating_duration_since(p.last_seen) < PRESENCE_TTL);
        let mut gone = Vec::new();
        for party in self.parties.values_mut() {
            for member in &mut party.members {
                if self.presence.contains_key(&member.player_id) {
                    member.away_since = None;
                    continue;
                }
                let since = *member.away_since.get_or_insert(now);
                let grace = if member.profile_id.is_some() {
                    PROFILE_GRACE
                } else {
                    GUEST_GRACE
                };
                if now.saturating_duration_since(since) >= grace {
                    gone.push(member.player_id);
                }
            }
        }
        for player_id in gone {
            self.leave(player_id);
        }
        let ids: Vec<_> = self.parties.keys().copied().collect();
        for id in ids {
            self.dissolve_empty(id);
        }
    }

    /// A launched party is still gathering: some of its members sit in the
    /// match and others, still online, have not joined yet.
    pub(crate) fn gathering(&self, now: Instant, joined: impl Fn(u64) -> bool) -> bool {
        self.parties.values().any(|party| {
            party
                .launched_at
                .is_some_and(|at| now.saturating_duration_since(at) < LAUNCH_GATHER)
                && party.members.iter().any(|m| joined(m.player_id))
                && party
                    .members
                    .iter()
                    .any(|m| m.away_since.is_none() && !joined(m.player_id))
        })
    }

    pub(crate) fn subscribers(&self) -> impl Iterator<Item = (u64, SocketAddr)> + '_ {
        self.presence.iter().map(|(id, p)| (*id, p.addr))
    }

    /// Player ids (online players and party members, including away ones)
    /// whose career profile is among `profiles`: on the public lobby, the
    /// profiles holding a live match allocation.
    pub(crate) fn players_with_profiles<'a>(
        &self,
        profiles: impl IntoIterator<Item = &'a str>,
    ) -> HashSet<u64> {
        let profiles: HashSet<&str> = profiles.into_iter().collect();
        if profiles.is_empty() {
            return HashSet::new();
        }
        let has =
            |profile: &Option<String>| profile.as_deref().is_some_and(|p| profiles.contains(p));
        self.presence
            .iter()
            .filter(|(_, p)| has(&p.profile_id))
            .map(|(id, _)| *id)
            .chain(
                self.parties
                    .values()
                    .flat_map(|party| &party.members)
                    .filter(|m| has(&m.profile_id))
                    .map(|m| m.player_id),
            )
            .collect()
    }

    /// The shared part of every view of one pass.
    pub(crate) fn view_context(&self, now: Instant, in_match: HashSet<u64>) -> ViewContext {
        let party_of: HashMap<u64, u64> = self
            .parties
            .iter()
            .flat_map(|(id, party)| party.members.iter().map(move |m| (m.player_id, *id)))
            .collect();
        let online = self
            .presence
            .iter()
            .filter(|(_, p)| now.saturating_duration_since(p.last_seen) < PRESENCE_TTL)
            .map(|(id, p)| OnlineRow {
                player: OnlinePlayer {
                    player_id: *id,
                    nickname: p.nickname.clone(),
                    avatar: p.avatar.clone(),
                    friend: false,
                    in_party: party_of
                        .get(id)
                        .is_some_and(|party_id| self.parties[party_id].members.len() > 1),
                    in_match: in_match.contains(id),
                    invited: false,
                },
                profile_id: p.profile_id.clone(),
                sort_name: p.nickname.to_lowercase(),
            })
            .collect();
        let invited = self
            .invites
            .iter()
            .filter(|i| i.expires > now)
            .map(|i| (i.party_id, i.to))
            .collect();
        ViewContext {
            now,
            in_match,
            party_of,
            online,
            invited,
        }
    }

    /// One viewer's view: its party, its invites and the online list with
    /// its friends marked and listed first, capped at [`MAX_PARTY_ONLINE`].
    pub(crate) fn view_in(
        &self,
        viewer: u64,
        context: &ViewContext,
        friends: &HashSet<String>,
    ) -> PartyView {
        let now = context.now;
        let own = context.party_of.get(&viewer).copied();
        let party = own.map(|party_id| {
            let party = &self.parties[&party_id];
            let mut members: Vec<_> = party
                .members
                .iter()
                .map(|m| {
                    let in_match = context.in_match.contains(&m.player_id);
                    PartyMember {
                        player_id: m.player_id,
                        nickname: m.nickname.clone(),
                        avatar: m.avatar.clone(),
                        leader: m.player_id == party.leader,
                        in_match,
                        // A member in a match elsewhere is not "away".
                        away: m.away_since.is_some() && !in_match,
                    }
                })
                .collect();
            members.sort_by_key(|m| !m.leader);
            PartyInfo {
                party_id,
                leader: party.leader,
                members,
                launch: party.launch,
            }
        });
        let invites = self
            .invites
            .iter()
            .filter(|i| i.to == viewer && i.expires > now)
            .filter_map(|i| {
                let from = self
                    .parties
                    .get(&i.party_id)?
                    .members
                    .iter()
                    .find(|m| m.player_id == i.from)?;
                Some(PartyInvite {
                    party_id: i.party_id,
                    from_player_id: i.from,
                    from_nickname: from.nickname.clone(),
                    expires_in_secs: i.expires.saturating_duration_since(now).as_secs(),
                })
            })
            .collect();
        // Pick the viewer's rows by key first; only the listed ones are cloned.
        let mut picks: Vec<(usize, bool, bool)> = context
            .online
            .iter()
            .enumerate()
            .filter(|(_, row)| row.player.player_id != viewer)
            .map(|(index, row)| {
                let friend = row
                    .profile_id
                    .as_ref()
                    .is_some_and(|pid| friends.contains(pid));
                let invited = own.is_some_and(|party_id| {
                    context.invited.contains(&(party_id, row.player.player_id))
                });
                (index, friend, invited)
            })
            .collect();
        picks.sort_by(|a, b| {
            let (ra, rb) = (&context.online[a.0], &context.online[b.0]);
            b.1.cmp(&a.1)
                .then_with(|| ra.player.in_party.cmp(&rb.player.in_party))
                .then_with(|| ra.sort_name.cmp(&rb.sort_name))
                .then_with(|| ra.player.player_id.cmp(&rb.player.player_id))
        });
        picks.truncate(MAX_PARTY_ONLINE);
        let online = picks
            .into_iter()
            .map(|(index, friend, invited)| OnlinePlayer {
                friend,
                invited,
                ..context.online[index].player.clone()
            })
            .collect();
        PartyView {
            you: viewer,
            party,
            invites,
            online,
            error: None,
        }
    }

    /// A single view (tests): the context is built for it alone.
    #[cfg(test)]
    pub(crate) fn view(
        &self,
        viewer: u64,
        now: Instant,
        in_match: impl Fn(u64) -> bool,
        friends: &HashSet<String>,
    ) -> PartyView {
        let ids: HashSet<u64> = self
            .presence
            .keys()
            .copied()
            .chain(
                self.parties
                    .values()
                    .flat_map(|party| party.members.iter().map(|m| m.player_id)),
            )
            .filter(|id| in_match(*id))
            .collect();
        self.view_in(viewer, &self.view_context(now, ids), friends)
    }
}

impl ServerRuntime {
    /// Parties live on the server players browse from; a match worker only
    /// runs its frozen roster.
    pub(crate) fn parties_enabled(&self) -> bool {
        self.match_service.worker().is_none()
    }

    pub(crate) fn handle_party(&mut self, addr: SocketAddr, command: PartyCommand, now: Instant) {
        if !self.parties_enabled() {
            return;
        }
        self.world.ensure_connected(addr, now);
        let player = self.world.players.get_mut(&addr).unwrap();
        player.last_seen = now;
        let player_id = player.hero.identity.id;
        // A heartbeat changes nothing another viewer must see right now; the
        // periodic pass carries it. Every other command is answered at once.
        let answer_now = !matches!(command, PartyCommand::Presence { .. });
        let invitee = match command {
            PartyCommand::Invite { player_id } => Some(player_id),
            _ => None,
        };
        let result = match command {
            // An account or matchmaking ban hides the player (LR-15); the
            // presence also carries a moderator's name reset to others.
            PartyCommand::Presence { .. }
                if shared::moderation::hides_presence(
                    &self.career.backend.sanctions(addr),
                    crate::moderation::unix_now(),
                ) =>
            {
                self.party.withdraw(player_id);
                Ok(())
            }
            PartyCommand::Presence { nickname, avatar } => {
                let profile = self.career.backend.profile(addr);
                let nickname = profile.as_ref().map_or(nickname, |p| p.nickname.clone());
                self.party.presence(
                    player_id,
                    addr,
                    &nickname,
                    avatar,
                    profile.map(|p| p.profile_id),
                    now,
                );
                Ok(())
            }
            PartyCommand::Invite { player_id: to } => {
                self.party.invite(player_id, to, now).map(|_| ())
            }
            PartyCommand::Accept { party_id } => self.party.accept(player_id, party_id, now),
            PartyCommand::Decline { party_id } => {
                self.party.decline(player_id, party_id, now);
                Ok(())
            }
            PartyCommand::Leave => {
                self.party.leave(player_id);
                Ok(())
            }
            PartyCommand::Kick { player_id: target } => self.party.kick(player_id, target),
            PartyCommand::Launch { preference } => {
                self.party.launch(player_id, preference, now).map(|_| ())
            }
        };
        let error = result.err();
        if let Some(error) = error {
            println!("Party: command from player {player_id} rejected: {error:?}");
        }
        if !answer_now {
            return;
        }
        // The button press shows up at once for the one who pressed it, with
        // the refusal if any; a fresh invite shows up for the invitee too.
        let context = self.party_context(now);
        self.send_party_view_to(player_id, addr, &context, error.map(PartyError::code), now);
        if error.is_none()
            && let Some(to) = invitee
            && let Some(to_addr) = self.party.addr_of(to)
        {
            self.send_party_view_to(to, to_addr, &context, None, now);
        }
    }

    /// The same-team seat for a joining human whose party mate is already
    /// seated, while that team still has room for another human.
    pub(crate) fn party_team(&self, addr: SocketAddr) -> Option<shared::map::Team> {
        let id = self.world.players.get(&addr)?.hero.identity.id;
        let mates = self.party.mates(id);
        let team = self
            .world
            .players
            .values()
            .find(|p| p.joined && !p.hero.identity.is_bot && mates.contains(&p.hero.identity.id))?
            .hero
            .identity
            .team;
        let humans = self
            .world
            .players
            .values()
            .chain(self.world.disconnected_sessions.values().map(|s| &s.player))
            .filter(|p| p.joined && !p.hero.identity.is_bot && p.hero.identity.team == team)
            .count() as u32;
        (humans < self.rules.team_size).then_some(team)
    }

    /// A practice draft waits (bounded) for the rest of a launched party.
    pub(crate) fn party_gathering(&self, now: Instant) -> bool {
        let joined: HashSet<u64> = self
            .world
            .players
            .values()
            .filter(|p| p.joined && !p.hero.identity.is_bot)
            .map(|p| p.hero.identity.id)
            .collect();
        self.party.gathering(now, |id| joined.contains(&id))
    }

    pub(crate) fn queue_party_roster(
        &self,
        addr: SocketAddr,
    ) -> Option<(u64, Vec<(String, String)>)> {
        let id = self.world.players.get(&addr)?.hero.identity.id;
        let party_id = self.party.party_id_of(id)?;
        let party = self.party.parties.get(&party_id)?;
        let members = party
            .members
            .iter()
            .filter(|m| m.away_since.is_none())
            .map(|m| {
                (
                    m.profile_id
                        .clone()
                        .unwrap_or_else(|| format!("guest:{}", m.player_id)),
                    m.nickname.clone(),
                )
            })
            .collect();
        Some((party_id, members))
    }

    /// Party tag for a public queue entry: (party id, members to wait for).
    pub(crate) fn party_tag(&self, addr: SocketAddr) -> Option<(u64, usize)> {
        let id = self.world.players.get(&addr)?.hero.identity.id;
        let party_id = self.party.party_id_of(id)?;
        Some((party_id, self.party.present_size(party_id).max(1)))
    }

    pub(crate) fn tick_party(&mut self, now: Instant) {
        if !self.parties_enabled() {
            return;
        }
        self.party.prune(now);
        self.send_party_views(now);
    }

    /// Who is in a match: humans seated on this server (standalone,
    /// practice) and, on the public lobby, the profiles this lobby handed a
    /// live allocation (they play on a worker, so nobody is seated here).
    fn party_context(&self, now: Instant) -> ViewContext {
        let mut in_match: HashSet<u64> = self
            .world
            .players
            .values()
            .filter(|p| p.joined && !p.hero.identity.is_bot)
            .map(|p| p.hero.identity.id)
            .collect();
        in_match.extend(
            self.party
                .players_with_profiles(self.match_service.assigned_profiles()),
        );
        self.party.view_context(now, in_match)
    }

    /// The viewer's accepted friends from its career view, re-read every
    /// [`FRIENDS_CACHE_TTL`]: the career view is a clone, too heavy to take
    /// twice a second for every subscriber.
    fn refresh_friends(&mut self, addr: SocketAddr, now: Instant) {
        let fresh = self
            .party
            .friends_cache
            .get(&addr)
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) < FRIENDS_CACHE_TTL);
        if fresh {
            return;
        }
        let friends = self
            .career
            .backend
            .view(addr)
            .friends
            .map(|f| {
                f.friends
                    .into_iter()
                    .map(|f| f.profile.profile_id)
                    .collect()
            })
            .unwrap_or_default();
        self.party.friends_cache.insert(addr, (now, friends));
    }

    /// The periodic pass: every subscriber's view, from one shared context.
    fn send_party_views(&mut self, now: Instant) {
        if self
            .party
            .last_sent
            .is_some_and(|at| now.saturating_duration_since(at) < SEND_INTERVAL)
        {
            return;
        }
        self.party.last_sent = Some(now);
        let subscribers: Vec<_> = self.party.subscribers().collect();
        let live: HashSet<SocketAddr> = subscribers.iter().map(|(_, addr)| *addr).collect();
        self.party
            .friends_cache
            .retain(|addr, _| live.contains(addr));
        let context = self.party_context(now);
        for (player_id, addr) in subscribers {
            self.send_party_view_to(player_id, addr, &context, None, now);
        }
    }

    /// One viewer's view, now; `error` only on the reply to a refused command.
    fn send_party_view_to(
        &mut self,
        player_id: u64,
        addr: SocketAddr,
        context: &ViewContext,
        error: Option<&str>,
        now: Instant,
    ) {
        let Some(player) = self.world.players.get(&addr) else {
            return;
        };
        if player.hero.identity.id != player_id
            || !player.framed_snapshots
            || !crate::runtime::network_endpoint(addr, player)
        {
            return;
        }
        self.refresh_friends(addr, now);
        let friends = &self.party.friends_cache[&addr].1;
        let mut view = self.party.view_in(player_id, context, friends);
        view.error = error.map(str::to_owned);
        self.party.sequence = self.party.sequence.saturating_add(1);
        let packet = ServerPacket::Party {
            server_epoch: self.server_epoch,
            sequence: self.party.sequence,
            party: view,
        };
        let result = serde_json::to_vec(&packet)
            .map_err(|e| e.to_string())
            .and_then(|payload| {
                shared::transport::encode_snapshot(
                    &payload,
                    self.server_epoch,
                    PARTY_FRAME_NAMESPACE | self.party.sequence,
                )
                .map_err(|e| e.to_string())
            });
        match result {
            Ok(datagrams) => {
                for datagram in datagrams {
                    if let Err(error) = self.transport.send_to(&datagram, addr) {
                        if let Some(suppressed) = self.party.send_diagnostic.record(now) {
                            eprintln!(
                                "Party send failed for {addr}: {error}; suppressed {suppressed} similar errors"
                            );
                        }
                        break;
                    }
                }
            }
            Err(error) => {
                if let Some(suppressed) = self.party.send_diagnostic.record(now) {
                    eprintln!(
                        "Party framing failed for {addr}: {error}; suppressed {suppressed} similar errors"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
