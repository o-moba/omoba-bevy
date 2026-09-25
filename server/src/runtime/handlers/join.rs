//! `Join`: session reclaim, team assignment per `rules.team_assignment`, the
//! hero loadout and the formation step it triggers.
use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::time::Instant;

use shared::HeroClass;
use shared::map::Team;
use shared::wire::{CharacterChoice, GameState};

use crate::formation::{advance_formation_on_join, joined_count};
use crate::match_rules::TeamAssignment;
use crate::runtime::ServerRuntime;
use crate::session::{
    assign_reserved_release_team, handle_join_request_with_sprite, normalize_session_id,
};
use crate::{bots, sandbox, targeting_qa};

impl ServerRuntime {
    /// `allocated_team` is the worker allocation's seat for this endpoint,
    /// resolved by the dispatcher before the join normalisation.
    pub(in crate::runtime) fn handle_join(
        &mut self,
        addr: SocketAddr,
        allocated_team: Option<Team>,
        prematch: bool,
        team: Team,
        character: CharacterChoice,
        hero_class: HeroClass,
        avatar: Option<String>,
        sprite_character: Option<String>,
        session_id: Option<String>,
        now: Instant,
    ) -> ControlFlow<()> {
        let combat_sandbox = self.sandbox_allowed();
        let targeting_qa = self.targeting_qa;
        let rules = self.rules;
        // A party mate already seated pulls this human onto the same team
        // (while it has human room). A free client choice is left alone.
        let party_team = self.party_team(addr).filter(|_| {
            !matches!(rules.team_assignment, TeamAssignment::ClientChoice)
                || (prematch && !combat_sandbox)
        });
        let world = &mut self.world;
        world.ensure_connected(addr, now);
        let player = world.players.get_mut(&addr).unwrap();
        player.last_seen = now;
        if !player.protocol_compatible {
            return ControlFlow::Break(());
        }
        // A joined endpoint cannot rewrite its identity or loadout through Join.
        if player.joined {
            player.join_error = None;
            return ControlFlow::Break(());
        }
        let session_id = normalize_session_id(session_id);
        if !world.ensure_player_for_join(addr, session_id, now) {
            world.players.get_mut(&addr).unwrap().join_error =
                Some(shared::protocol::JoinRejection::SessionActive);
            return ControlFlow::Break(());
        }
        // A reclaim is already joined: retain all authoritative round state.
        if let Some(player) = world.players.get_mut(&addr).filter(|player| player.joined) {
            player.join_error = None;
            self.register_career_participant(addr);
            self.fill_practice_bots(now);
            return ControlFlow::Break(());
        }
        // Team resolution: `ClientChoice` (dev) honors the client's
        // choice; `Balanced` (release) balances teams server-side
        // (rejoining players keep their original team);
        // `PracticeSeat` takes a bot's seat.
        let assigned_team = allocated_team
            .or(party_team)
            .or_else(|| match rules.team_assignment {
                TeamAssignment::PracticeSeat => {
                    bots::assign_human_team(&world.players, rules.team_size)
                }
                TeamAssignment::ClientChoice if combat_sandbox => {
                    sandbox::assign_human_team(&world.players, &world.disconnected_sessions)
                }
                TeamAssignment::ClientChoice if prematch => assign_reserved_release_team(
                    &world.players,
                    &world.disconnected_sessions,
                    rules.team_size,
                ),
                TeamAssignment::ClientChoice => (joined_count(&world.players)
                    + (world.disconnected_sessions.len() as u32)
                    < rules.roster_size())
                .then_some(team),
                TeamAssignment::Balanced => {
                    let existing_team = world
                        .players
                        .get(&addr)
                        .filter(|player| player.joined)
                        .map(|player| player.hero.identity.team);
                    existing_team.or_else(|| {
                        assign_reserved_release_team(
                            &world.players,
                            &world.disconnected_sessions,
                            rules.team_size,
                        )
                    })
                }
            });
        let Some(assigned_team) = assigned_team else {
            println!(
                "Matchmaking: match is full ({} players) - join from {addr} rejected",
                rules.roster_size()
            );
            world.players.get_mut(&addr).unwrap().join_error =
                Some(shared::protocol::JoinRejection::MatchFull);
            return ControlFlow::Break(());
        };
        if rules.fills_with_bots {
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
                needed: rules.roster_size(),
            };
        } else {
            advance_formation_on_join(world, rules, now);
        }
        ControlFlow::Continue(())
    }
}
