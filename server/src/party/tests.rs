use std::net::SocketAddr;
use std::time::{Duration, Instant};

use shared::HeroClass;
use shared::map::Team;
use shared::match_service::MatchPreference;
use shared::party::{MAX_PARTY_SIZE, PartyCommand};
use shared::wire::{CharacterChoice, ClientPacket};

use super::*;
use crate::match_rules::{MatchConfig, MatchMode};
use crate::runtime::ports::{Clock, ManualClock, MemoryTransport};
use crate::{career_backend, runtime::ServerRuntime};

fn addr(index: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 58000 + index))
}

fn online(state: &mut PartyState, ids: &[u64], now: Instant) {
    for &id in ids {
        state.presence(id, addr(id as u16), &format!("P{id}"), None, None, now);
    }
}

fn no_friends() -> HashSet<String> {
    HashSet::new()
}

#[test]
fn invites_need_two_distinct_online_players() {
    let now = Instant::now();
    let mut state = PartyState::default();
    online(&mut state, &[1], now);
    assert_eq!(state.invite(1, 1, now), Err(PartyError::SelfInvite));
    assert_eq!(state.invite(1, 2, now), Err(PartyError::NotOnline));
    online(&mut state, &[2], now);
    assert!(state.invite(1, 2, now).is_ok());
    // The invitee sees it; the inviter's online list marks it.
    let view = state.view(2, now, |_| false, &no_friends());
    assert_eq!(view.invites.len(), 1);
    assert_eq!(view.invites[0].from_nickname, "P1");
    let view = state.view(1, now, |_| false, &no_friends());
    assert!(view.online.iter().any(|p| p.player_id == 2 && p.invited));
}

#[test]
fn accepting_forms_a_party_with_the_leader_first() {
    let now = Instant::now();
    let mut state = PartyState::default();
    online(&mut state, &[1, 2], now);
    let party = state.invite(1, 2, now).unwrap();
    assert_eq!(state.accept(2, party + 1, now), Err(PartyError::NoInvite));
    state.accept(2, party, now).unwrap();
    for viewer in [1, 2] {
        let view = state.view(viewer, now, |_| false, &no_friends());
        let info = view.party.expect("both see the party");
        assert_eq!(info.leader, 1);
        let ids: Vec<_> = info.members.iter().map(|m| m.player_id).collect();
        assert_eq!(ids, [1, 2]);
        assert!(info.members[0].leader && !info.members[1].leader);
        assert!(view.invites.is_empty());
    }
    assert_eq!(state.mates(2), [1]);
    assert_eq!(state.invite(1, 2, now), Err(PartyError::AlreadyMember));
}

#[test]
fn declined_and_expired_invites_leave_no_party_behind() {
    let now = Instant::now();
    let mut state = PartyState::default();
    online(&mut state, &[1, 2], now);
    let party = state.invite(1, 2, now).unwrap();
    state.decline(2, party);
    state.prune(now);
    assert!(state.view(1, now, |_| false, &no_friends()).party.is_none());

    let party = state.invite(1, 2, now).unwrap();
    let later = now + Duration::from_secs(shared::party::PARTY_INVITE_TTL_SECS + 1);
    online(&mut state, &[1, 2], later);
    assert_eq!(state.accept(2, party, later), Err(PartyError::NoInvite));
    state.prune(later);
    assert!(
        state
            .view(1, later, |_| false, &no_friends())
            .party
            .is_none()
    );
}

#[test]
fn leaving_passes_the_lead_and_only_the_leader_kicks() {
    let now = Instant::now();
    let mut state = PartyState::default();
    online(&mut state, &[1, 2, 3], now);
    let party = state.invite(1, 2, now).unwrap();
    state.accept(2, party, now).unwrap();
    state.invite(1, 3, now).unwrap();
    state.accept(3, party, now).unwrap();
    assert_eq!(state.kick(2, 3), Err(PartyError::NotLeader));
    state.leave(1);
    let info = state.view(2, now, |_| false, &no_friends()).party.unwrap();
    assert_eq!(info.leader, 2);
    state.kick(2, 3).unwrap();
    // A party of one with nothing pending dissolves.
    assert!(state.view(2, now, |_| false, &no_friends()).party.is_none());
    assert!(state.mates(3).is_empty());
}

#[test]
fn a_party_never_outgrows_one_team() {
    let now = Instant::now();
    let mut state = PartyState::default();
    let ids: Vec<u64> = (1..=MAX_PARTY_SIZE as u64 + 1).collect();
    online(&mut state, &ids, now);
    let mut party = 0;
    for &id in &ids[1..MAX_PARTY_SIZE] {
        party = state.invite(1, id, now).unwrap();
        state.accept(id, party, now).unwrap();
    }
    assert_eq!(
        state.view(1, now, |_| false, &no_friends()).member_count(),
        MAX_PARTY_SIZE
    );
    assert_eq!(
        state.invite(1, ids[MAX_PARTY_SIZE], now),
        Err(PartyError::PartyFull)
    );
    assert!(party > 0);
}

#[test]
fn only_the_leader_launches_and_every_launch_is_new() {
    let now = Instant::now();
    let mut state = PartyState::default();
    online(&mut state, &[1, 2], now);
    assert_eq!(
        state.launch(1, MatchPreference::BotPractice, now),
        Err(PartyError::NoParty)
    );
    let party = state.invite(1, 2, now).unwrap();
    state.accept(2, party, now).unwrap();
    assert_eq!(
        state.launch(2, MatchPreference::BotPractice, now),
        Err(PartyError::NotLeader)
    );
    let first = state.launch(1, MatchPreference::BotPractice, now).unwrap();
    let second = state.launch(1, MatchPreference::Quick, now).unwrap();
    assert!(second.sequence > first.sequence);
    let seen = state.view(2, now, |_| false, &no_friends()).party.unwrap();
    assert_eq!(seen.launch, Some(second));
}

#[test]
fn a_vanished_guest_is_dropped_and_a_returning_profile_is_rebound() {
    let now = Instant::now();
    let mut state = PartyState::default();
    state.presence(1, addr(1), "Ann", None, Some("ann".into()), now);
    state.presence(2, addr(2), "Bob", None, None, now);
    state.presence(3, addr(3), "Cid", None, Some("cid".into()), now);
    let party = state.invite(1, 2, now).unwrap();
    state.accept(2, party, now).unwrap();
    state.invite(1, 3, now).unwrap();
    state.accept(3, party, now).unwrap();
    // Everyone but Ann goes quiet (Cid is in a public match elsewhere).
    // Every tick prunes: presence lapses first, then the guest's grace runs.
    let quiet = now + PRESENCE_TTL;
    state.presence(1, addr(1), "Ann", None, Some("ann".into()), quiet);
    state.prune(quiet);
    let later = quiet + GUEST_GRACE;
    state.presence(1, addr(1), "Ann", None, Some("ann".into()), later);
    state.prune(later);
    let view = state.view(1, later, |_| false, &no_friends());
    let info = view.party.unwrap();
    let ids: Vec<_> = info.members.iter().map(|m| m.player_id).collect();
    assert_eq!(ids, [1, 3], "the guest is gone, the profile is away");
    assert!(info.members[1].away);
    assert_eq!(state.present_size(party), 1);
    // Cid comes back to the lobby as a new endpoint and player id.
    state.presence(9, addr(9), "Cid", None, Some("cid".into()), later);
    assert_eq!(state.mates(1), [9]);
    assert_eq!(state.present_size(party), 2);
}

#[test]
fn only_shipped_roster_avatars_are_shown_to_others() {
    let now = Instant::now();
    let shipped = omoba_passport::avatars::avatar_roster()
        .first()
        .map(|a| a.slug.to_owned());
    let mut state = PartyState::default();
    state.presence(1, addr(1), "Ann", shipped.clone(), None, now);
    state.presence(
        2,
        addr(2),
        "Bob",
        Some("../../etc/passwd".into()),
        None,
        now,
    );
    let view = state.view(3, now, |_| false, &no_friends());
    let avatar = |id| {
        view.online
            .iter()
            .find(|p| p.player_id == id)
            .unwrap()
            .avatar
            .clone()
    };
    assert_eq!(avatar(1), shipped);
    assert_eq!(avatar(2), None);
}

#[test]
fn friends_are_listed_first() {
    let now = Instant::now();
    let mut state = PartyState::default();
    state.presence(1, addr(1), "Aaron", None, Some("a".into()), now);
    state.presence(2, addr(2), "Zed", None, Some("z".into()), now);
    let friends: HashSet<String> = ["z".to_owned()].into();
    let view = state.view(3, now, |_| false, &friends);
    assert_eq!(view.online[0].player_id, 2);
    assert!(view.online[0].friend && !view.online[1].friend);
}

// --- Runtime: two party members on a practice server share a team. ---

fn practice_runtime(size: u32) -> (ServerRuntime, ManualClock) {
    let clock = ManualClock::new(Instant::now());
    let rt = ServerRuntime::for_test(
        MemoryTransport::new(addr(0)),
        clock.clone(),
        career_backend::MemoryCareer::test_backend(7),
        MatchConfig {
            mode: MatchMode::Practice,
            team_size: size,
        },
    );
    (rt, clock)
}

fn join(session: &str, prematch: bool) -> ClientPacket {
    ClientPacket::Join {
        prematch,
        team: Team::Green,
        character: CharacterChoice::Cube,
        hero_class: HeroClass::Mage,
        avatar: None,
        sprite_character: None,
        session_id: Some(session.into()),
        passport_ticket: None,
    }
}

fn presence(rt: &mut ServerRuntime, index: u16, now: Instant) -> u64 {
    rt.handle_packet(
        addr(index),
        ClientPacket::Party {
            command: PartyCommand::Presence {
                nickname: format!("P{index}"),
                avatar: None,
            },
        },
        now,
    );
    rt.world.players[&addr(index)].hero.identity.id
}

fn party_of_two(rt: &mut ServerRuntime, now: Instant) {
    presence(rt, 1, now);
    let friend = presence(rt, 2, now);
    rt.handle_packet(
        addr(1),
        ClientPacket::Party {
            command: PartyCommand::Invite { player_id: friend },
        },
        now,
    );
    let party_id = rt
        .party
        .party_id_of(rt.world.players[&addr(1)].hero.identity.id);
    rt.handle_packet(
        addr(2),
        ClientPacket::Party {
            command: PartyCommand::Accept {
                party_id: party_id.unwrap(),
            },
        },
        now,
    );
}

fn team_of(rt: &ServerRuntime, index: u16) -> Team {
    let player = &rt.world.players[&addr(index)];
    assert!(player.joined);
    player.hero.identity.team
}

#[test]
fn party_mates_share_a_team_against_bots_on_a_practice_server() {
    let (mut rt, clock) = practice_runtime(5);
    let now = clock.now();
    party_of_two(&mut rt, now);
    rt.handle_packet(addr(1), join("a", false), now);
    rt.handle_packet(addr(2), join("b", false), now);
    assert_eq!(team_of(&rt, 1), team_of(&rt, 2));
    let bots_on = |team| {
        rt.world
            .players
            .values()
            .filter(|p| p.joined && p.hero.identity.is_bot && p.hero.identity.team == team)
            .count()
    };
    let ours = team_of(&rt, 1);
    let theirs = if ours == Team::Green {
        Team::Blue
    } else {
        Team::Green
    };
    assert_eq!(bots_on(ours), 3);
    assert_eq!(bots_on(theirs), 5);
}

#[test]
fn humans_without_a_party_still_balance_on_a_practice_server() {
    let (mut rt, clock) = practice_runtime(5);
    let now = clock.now();
    presence(&mut rt, 1, now);
    presence(&mut rt, 2, now);
    rt.handle_packet(addr(1), join("a", false), now);
    rt.handle_packet(addr(2), join("b", false), now);
    assert_ne!(team_of(&rt, 1), team_of(&rt, 2));
}

#[test]
fn a_launched_party_holds_the_draft_until_its_members_join() {
    let (mut rt, clock) = practice_runtime(5);
    let now = clock.now();
    party_of_two(&mut rt, now);
    rt.handle_packet(
        addr(1),
        ClientPacket::Party {
            command: PartyCommand::Launch {
                preference: MatchPreference::BotPractice,
            },
        },
        now,
    );
    // A member cannot launch for the party.
    let launch = rt
        .party
        .view(
            rt.world.players[&addr(1)].hero.identity.id,
            now,
            |_| false,
            &no_friends(),
        )
        .party
        .unwrap()
        .launch
        .unwrap();
    rt.handle_packet(
        addr(2),
        ClientPacket::Party {
            command: PartyCommand::Launch {
                preference: MatchPreference::Quick,
            },
        },
        now,
    );
    let after = rt
        .party
        .view(
            rt.world.players[&addr(2)].hero.identity.id,
            now,
            |_| false,
            &no_friends(),
        )
        .party
        .unwrap()
        .launch
        .unwrap();
    assert_eq!(after, launch);
    rt.handle_packet(addr(1), join("a", true), now);
    assert!(rt.party_gathering(now));
    // Past the gather window nobody is held any longer.
    assert!(!rt.party_gathering(now + LAUNCH_GATHER));
    rt.handle_packet(addr(2), join("b", true), now);
    assert!(!rt.party_gathering(now));
    assert_eq!(team_of(&rt, 1), team_of(&rt, 2));
}
