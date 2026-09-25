//! Parties: players on this server who group up, see each other in the party
//! lobby and are seated on one team. [`PartyState`] is the pure rule set
//! (invites, membership, leader, launch); the `ServerRuntime` glue at the end
//! feeds it packets and presence and sends every subscriber its view.
//!
//! State is in memory on the server the players are connected to: the
//! standalone/practice server, or the public lobby. Match workers do not run
//! parties; members of a public match are "away" until they return to the
//! lobby and are rebound by their career profile.
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
const SEND_INTERVAL: Duration = Duration::from_millis(500);
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
    next_party_id: u64,
    launch_sequence: u64,
    sequence: u64,
    last_sent: Option<Instant>,
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
    }

    fn online(&self, player_id: u64, now: Instant) -> bool {
        self.presence
            .get(&player_id)
            .is_some_and(|p| now.saturating_duration_since(p.last_seen) < PRESENCE_TTL)
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

    pub(crate) fn decline(&mut self, player_id: u64, party_id: u64) {
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

    /// Expires invites and presence, marks vanished members away and drops
    /// them after their grace period.
    pub(crate) fn prune(&mut self, now: Instant) {
        self.invites.retain(|i| i.expires > now);
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

    pub(crate) fn view(
        &self,
        viewer: u64,
        now: Instant,
        in_match: impl Fn(u64) -> bool,
        friends: &HashSet<String>,
    ) -> PartyView {
        let own = self.party_id_of(viewer);
        let party = own.map(|party_id| {
            let party = &self.parties[&party_id];
            let mut members: Vec<_> = party
                .members
                .iter()
                .map(|m| PartyMember {
                    player_id: m.player_id,
                    nickname: m.nickname.clone(),
                    avatar: m.avatar.clone(),
                    leader: m.player_id == party.leader,
                    in_match: in_match(m.player_id),
                    away: m.away_since.is_some(),
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
        let mut online: Vec<_> = self
            .presence
            .iter()
            .filter(|(id, p)| {
                **id != viewer && now.saturating_duration_since(p.last_seen) < PRESENCE_TTL
            })
            .map(|(id, p)| OnlinePlayer {
                player_id: *id,
                nickname: p.nickname.clone(),
                avatar: p.avatar.clone(),
                friend: p
                    .profile_id
                    .as_ref()
                    .is_some_and(|pid| friends.contains(pid)),
                in_party: self
                    .party_of(*id)
                    .is_some_and(|party| party.members.len() > 1),
                in_match: in_match(*id),
                invited: own.is_some_and(|party_id| {
                    self.invites
                        .iter()
                        .any(|i| i.party_id == party_id && i.to == *id && i.expires > now)
                }),
            })
            .collect();
        online.sort_by(|a, b| {
            b.friend
                .cmp(&a.friend)
                .then_with(|| a.in_party.cmp(&b.in_party))
                .then_with(|| a.nickname.to_lowercase().cmp(&b.nickname.to_lowercase()))
                .then_with(|| a.player_id.cmp(&b.player_id))
        });
        online.truncate(MAX_PARTY_ONLINE);
        PartyView {
            you: viewer,
            party,
            invites,
            online,
        }
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
        let result = match command {
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
                self.party.decline(player_id, party_id);
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
        if let Err(error) = result {
            println!("Party: command from player {player_id} rejected: {error:?}");
        }
        // Answer promptly: the button press should show up at once.
        self.party.last_sent = None;
        self.send_party_views(now);
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

    fn send_party_views(&mut self, now: Instant) {
        if self
            .party
            .last_sent
            .is_some_and(|at| now.saturating_duration_since(at) < SEND_INTERVAL)
        {
            return;
        }
        self.party.last_sent = Some(now);
        self.party.sequence = self.party.sequence.saturating_add(1);
        let in_match: HashSet<u64> = self
            .world
            .players
            .values()
            .filter(|p| p.joined && !p.hero.identity.is_bot)
            .map(|p| p.hero.identity.id)
            .collect();
        let subscribers: Vec<_> = self.party.subscribers().collect();
        for (player_id, addr) in subscribers {
            let Some(player) = self.world.players.get(&addr) else {
                continue;
            };
            if player.hero.identity.id != player_id || !player.framed_snapshots {
                continue;
            }
            let friends: HashSet<String> = self
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
            let packet = ServerPacket::Party {
                server_epoch: self.server_epoch,
                sequence: self.party.sequence,
                party: self
                    .party
                    .view(player_id, now, |id| in_match.contains(&id), &friends),
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
                            if self.snapshot_send_diagnostic.record(now).is_some() {
                                eprintln!("Party send failed: {error}");
                            }
                            break;
                        }
                    }
                }
                Err(error) => {
                    if self.snapshot_send_diagnostic.record(now).is_some() {
                        eprintln!("Party framing failed: {error}");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
