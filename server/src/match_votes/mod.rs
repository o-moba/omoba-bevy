//! Server-owned remake/surrender; socket authentication determines the team.
use crate::runtime::ServerRuntime;
use shared::{
    career::MatchOutcome,
    map::Team,
    match_vote::{Action, Active, Eligibility, Kind, View},
    wire::GameState,
};
use std::{
    collections::BTreeSet,
    net::SocketAddr,
    time::{Duration, Instant},
};
mod policy;
#[cfg(test)]
mod tests;

const REMAKE_UNTIL: Duration = Duration::from_secs(180);
const SURRENDER_FROM: Duration = Duration::from_secs(600);

#[derive(Default)]
pub(crate) struct MatchVotes {
    round: Option<(u64, u64)>,
    teams: [policy::Slot; 2],
}
fn index(team: Team) -> usize {
    match team {
        Team::Green => 0,
        Team::Blue => 1,
    }
}
fn seconds(time: Duration) -> u64 {
    time.as_secs()
        .saturating_add(u64::from(time.subsec_nanos() > 0))
}

impl ServerRuntime {
    fn votes_running(&self) -> bool {
        self.match_started_at.is_some()
            && matches!(self.world.game_state, GameState::Running)
            && self
                .match_service
                .worker()
                .is_some_and(|w| !w.aborted && !w.recovery)
            && self.career_allocation_is_live()
    }

    fn vote_eligibility(&self, team: Team, kind: Kind, now: Instant) -> Eligibility {
        let elapsed = self
            .match_started_at
            .map_or(Duration::ZERO, |at| now.saturating_duration_since(at));
        let (available, starts_in_secs, code) = match kind {
            Kind::Surrender if elapsed < SURRENDER_FROM => {
                (false, seconds(SURRENDER_FROM - elapsed), "too_early")
            }
            Kind::Surrender => (true, 0, "available"),
            Kind::Remake if elapsed >= REMAKE_UNTIL => (false, 0, "window_closed"),
            Kind::Remake => {
                let absent = self
                    .world
                    .players
                    .values()
                    .chain(self.world.disconnected_sessions.values().map(|s| &s.player))
                    .any(|p| {
                        p.joined
                            && !p.hero.identity.is_bot
                            && p.hero.identity.team == team
                            && self.fair_play.currently_absent(p.hero.identity.id, now)
                    });
                (
                    absent,
                    0,
                    if absent {
                        "available"
                    } else {
                        "no_absent_teammate"
                    },
                )
            }
        };
        Eligibility {
            available,
            starts_in_secs,
            code: code.into(),
        }
    }

    pub(crate) fn match_vote_view(&self, addr: SocketAddr, now: Instant) -> Option<View> {
        if !self.votes_running() {
            return None;
        }
        let viewer = self.world.players.get(&addr)?;
        let team = viewer.hero.identity.team;
        let voters = self.eligible_takeover_voters(team);
        let (_, id) = voters.iter().find(|(candidate, _)| *candidate == addr)?;
        let eligible: BTreeSet<_> = voters.iter().map(|(_, id)| *id).collect();
        let slot = &self.match_votes.teams[index(team)];
        let current_round = self.match_votes.round == Some((self.server_epoch, self.match_id));
        let active = current_round.then(|| slot.active()).flatten().map(|vote| {
            let counts = vote.counts(&eligible);
            let initiator_name = self
                .world
                .players
                .values()
                .chain(self.world.disconnected_sessions.values().map(|s| &s.player))
                .find(|p| p.hero.identity.id == vote.initiator)
                .and_then(|p| p.career_profile.as_ref())
                .map_or_else(String::new, |p| p.nickname.clone());
            Active {
                kind: vote.kind,
                generation: vote.generation,
                initiator: vote.initiator,
                initiator_name,
                yes: counts.yes as u32,
                no: counts.no as u32,
                required: counts.required as u32,
                eligible: counts.eligible as u32,
                remaining_secs: seconds(vote.deadline.saturating_duration_since(now)),
                my_vote: vote.ballot(*id),
            }
        });
        Some(View {
            server_epoch: self.server_epoch,
            match_id: self.match_id,
            next_generation: if current_round {
                slot.next_generation()
            } else {
                Some(1)
            },
            cooldown_secs: if current_round {
                seconds(slot.cooldown(now))
            } else {
                0
            },
            remake: self.vote_eligibility(team, Kind::Remake, now),
            surrender: self.vote_eligibility(team, Kind::Surrender, now),
            active,
        })
    }

    pub(crate) fn tick_match_votes(&mut self, now: Instant) {
        let key = (self.server_epoch, self.match_id);
        if !self.votes_running() {
            self.match_votes = Default::default();
            return;
        }
        if self.match_votes.round != Some(key) {
            self.match_votes = MatchVotes {
                round: Some(key),
                ..Default::default()
            };
        }
        for team in [Team::Green, Team::Blue] {
            let eligible = self
                .eligible_takeover_voters(team)
                .into_iter()
                .map(|(_, id)| id)
                .collect();
            let invalid = self.match_votes.teams[index(team)]
                .active()
                .is_some_and(|v| !self.vote_eligibility(team, v.kind, now).available);
            let slot = &mut self.match_votes.teams[index(team)];
            let decision = if invalid {
                slot.cancel(now)
            } else {
                slot.tick(&eligible, now)
            };
            if let policy::Decision::Passed(kind) = decision {
                self.resolve_match_vote(team, kind, now);
                break;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handle_match_vote(
        &mut self,
        addr: SocketAddr,
        epoch: u64,
        match_id: u64,
        generation: u64,
        kind: Kind,
        action: Action,
        now: Instant,
    ) {
        if epoch != self.server_epoch || match_id != self.match_id || !self.votes_running() {
            return;
        }
        self.observe_fair_play(now, false);
        self.tick_match_votes(now);
        if !self.votes_running() {
            return;
        }
        let Some(player) = self.world.players.get(&addr) else {
            return;
        };
        let team = player.hero.identity.team;
        let voters = self.eligible_takeover_voters(team);
        let Some((_, id)) = voters.iter().find(|(candidate, _)| *candidate == addr) else {
            return;
        };
        let eligible = voters.iter().map(|(_, id)| *id).collect();
        let allowed = self.vote_eligibility(team, kind, now).available;
        let started = self.match_started_at.unwrap();
        let deadline = match kind {
            Kind::Remake => (now + policy::WINDOW).min(started + REMAKE_UNTIL),
            Kind::Surrender => now + policy::WINDOW,
        };
        let slot = &mut self.match_votes.teams[index(team)];
        let decision = match action {
            Action::Start if allowed => slot.start(kind, generation, *id, &eligible, now, deadline),
            Action::Start => policy::Decision::Ignored,
            Action::Yes | Action::No => {
                slot.vote(kind, generation, *id, action == Action::Yes, &eligible, now)
            }
        };
        if let policy::Decision::Passed(kind) = decision {
            self.resolve_match_vote(team, kind, now);
        }
        // Only an accepted ballot changes the view. Invalid/replayed commands
        // must not turn one signed datagram into a broadcast to the whole roster.
        if decision != policy::Decision::Ignored {
            self.career_refresh_views(now);
        }
    }

    fn resolve_match_vote(&mut self, team: Team, kind: Kind, now: Instant) {
        if !self.votes_running() {
            return;
        }
        match kind {
            Kind::Remake => {
                self.finish_career_round_with_reason(
                    MatchOutcome::Remade,
                    None,
                    now,
                    Some("remake"),
                );
                if let crate::match_service::MatchService::Worker(worker) = &mut self.match_service
                {
                    worker.aborted = true;
                }
            }
            Kind::Surrender => {
                let winner = match team {
                    Team::Green => Team::Blue,
                    Team::Blue => Team::Green,
                };
                self.world.game_state = GameState::Victory { winner };
                self.victory_at = Some(now);
                self.finish_career_round_with_reason(
                    MatchOutcome::Completed,
                    Some(winner),
                    now,
                    Some("surrender"),
                );
            }
        }
        println!(
            "MATCH_METRIC event=team_vote_passed epoch={} match={} team={team:?} kind={kind:?}",
            self.server_epoch, self.match_id
        );
        self.match_votes = Default::default();
    }
}
