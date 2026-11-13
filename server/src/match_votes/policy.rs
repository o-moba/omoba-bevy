//! Per-team voting: stable roster IDs, bounded ballots, monotonic generations.
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

pub(super) const WINDOW: Duration = Duration::from_secs(30);
pub(super) const COOLDOWN: Duration = Duration::from_secs(120);
const MAX_VOTERS: usize = 32;

pub(super) use shared::match_vote::Kind;
fn required(kind: Kind, eligible: usize) -> usize {
    match kind {
        Kind::Remake => eligible / 2 + 1,
        Kind::Surrender => eligible.min(4),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Ignored,
    Pending,
    Passed(Kind),
    Failed,
}

#[derive(Debug)]
pub(super) struct Vote {
    pub kind: Kind,
    pub generation: u64,
    pub initiator: u64,
    pub deadline: Instant,
    ballots: BTreeMap<u64, bool>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Counts {
    pub yes: usize,
    pub no: usize,
    pub eligible: usize,
    pub required: usize,
}
impl Vote {
    pub fn counts(&self, eligible: &BTreeSet<u64>) -> Counts {
        let yes = eligible
            .iter()
            .filter(|id| self.ballots.get(id) == Some(&true))
            .count();
        let no = eligible
            .iter()
            .filter(|id| self.ballots.get(id) == Some(&false))
            .count();
        Counts {
            yes,
            no,
            eligible: eligible.len(),
            required: required(self.kind, eligible.len()),
        }
    }
    pub fn ballot(&self, voter: u64) -> Option<bool> {
        self.ballots.get(&voter).copied()
    }
}

#[derive(Debug, Default)]
pub(super) struct Slot {
    generation: u64,
    cooldown_until: Option<Instant>,
    active: Option<Vote>,
}
impl Slot {
    pub fn next_generation(&self) -> Option<u64> {
        self.generation.checked_add(1)
    }
    pub fn cooldown(&self, now: Instant) -> Duration {
        self.cooldown_until
            .map_or(Duration::ZERO, |until| until.saturating_duration_since(now))
    }
    pub fn active(&self) -> Option<&Vote> {
        self.active.as_ref()
    }

    /// The caller supplies eligible authenticated human IDs and an authoritative
    /// deadline, shortened to the remaining remake window when appropriate.
    pub fn start(
        &mut self,
        kind: Kind,
        generation: u64,
        voter: u64,
        eligible: &BTreeSet<u64>,
        now: Instant,
        deadline: Instant,
    ) -> Decision {
        if let result @ (Decision::Passed(_) | Decision::Failed) = self.tick(eligible, now) {
            return result;
        }
        if self.active.is_some()
            || self.cooldown(now) != Duration::ZERO
            || self.next_generation() != Some(generation)
            || !eligible.contains(&voter)
            || eligible.is_empty()
            || eligible.len() > MAX_VOTERS
            || deadline <= now
        {
            return Decision::Ignored;
        }
        self.generation = generation;
        self.active = Some(Vote {
            kind,
            generation,
            initiator: voter,
            deadline: deadline.min(now + WINDOW),
            ballots: BTreeMap::from([(voter, true)]),
        });
        self.tick(eligible, now)
    }

    pub fn vote(
        &mut self,
        kind: Kind,
        generation: u64,
        voter: u64,
        yes: bool,
        eligible: &BTreeSet<u64>,
        now: Instant,
    ) -> Decision {
        if let result @ (Decision::Passed(_) | Decision::Failed) = self.tick(eligible, now) {
            return result;
        }
        let Some(active) = self.active.as_mut() else {
            return Decision::Ignored;
        };
        if active.kind != kind
            || active.generation != generation
            || !eligible.contains(&voter)
            || active.ballots.contains_key(&voter)
            || active.ballots.len() >= MAX_VOTERS
        {
            return Decision::Ignored;
        }
        active.ballots.insert(voter, yes);
        self.tick(eligible, now)
    }

    pub fn tick(&mut self, eligible: &BTreeSet<u64>, now: Instant) -> Decision {
        let Some(active) = &self.active else {
            return Decision::Ignored;
        };
        if now >= active.deadline || eligible.is_empty() || eligible.len() > MAX_VOTERS {
            return self.fail(now);
        }
        let counts = active.counts(eligible);
        if counts.yes >= counts.required {
            let kind = active.kind;
            self.active = None;
            return Decision::Passed(kind);
        }
        if counts.eligible - counts.no < counts.required {
            return self.fail(now);
        }
        Decision::Pending
    }
    pub fn cancel(&mut self, now: Instant) -> Decision {
        if self.active.is_some() {
            self.fail(now)
        } else {
            Decision::Ignored
        }
    }
    fn fail(&mut self, now: Instant) -> Decision {
        self.active = None;
        self.cooldown_until = Some(now + COOLDOWN);
        Decision::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ids(n: u64) -> BTreeSet<u64> {
        (1..=n).collect()
    }
    fn start(slot: &mut Slot, kind: Kind, eligible: &BTreeSet<u64>, now: Instant) -> Decision {
        slot.start(
            kind,
            slot.next_generation().unwrap(),
            1,
            eligible,
            now,
            now + WINDOW,
        )
    }
    #[test]
    fn remake_needs_a_strict_majority_and_surrender_four_of_five() {
        let now = Instant::now();
        for (kind, voters, needed) in [
            (Kind::Remake, 4, 3),
            (Kind::Surrender, 5, 4),
            (Kind::Surrender, 3, 3),
        ] {
            let eligible = ids(voters);
            let mut slot = Slot::default();
            assert_eq!(start(&mut slot, kind, &eligible, now), Decision::Pending);
            for id in 2..=needed {
                assert_eq!(
                    slot.vote(kind, 1, id, true, &eligible, now),
                    if id == needed {
                        Decision::Passed(kind)
                    } else {
                        Decision::Pending
                    }
                );
            }
            assert!(slot.active().is_none());
            assert_eq!(
                slot.vote(kind, 1, needed, true, &eligible, now),
                Decision::Ignored
            );
        }
    }
    #[test]
    fn one_remaining_human_can_decide_for_the_remaining_team() {
        let now = Instant::now();
        for kind in [Kind::Remake, Kind::Surrender] {
            assert_eq!(
                start(&mut Slot::default(), kind, &ids(1), now),
                Decision::Passed(kind)
            );
        }
    }
    #[test]
    fn duplicate_wrong_kind_stranger_and_stale_generation_cannot_change_ballots() {
        let now = Instant::now();
        let eligible = ids(5);
        let mut slot = Slot::default();
        start(&mut slot, Kind::Surrender, &eligible, now);
        for (kind, generation, id) in [
            (Kind::Surrender, 1, 1),
            (Kind::Remake, 1, 2),
            (Kind::Surrender, 0, 2),
            (Kind::Surrender, 2, 2),
            (Kind::Surrender, 1, 99),
        ] {
            assert_eq!(
                slot.vote(kind, generation, id, false, &eligible, now),
                Decision::Ignored
            );
        }
        assert_eq!(
            slot.active().unwrap().counts(&eligible),
            Counts {
                yes: 1,
                no: 0,
                eligible: 5,
                required: 4
            }
        );
    }
    #[test]
    fn timeout_at_exactly_thirty_seconds_starts_exact_team_cooldown() {
        let now = Instant::now();
        let eligible = ids(5);
        let mut slot = Slot::default();
        start(&mut slot, Kind::Surrender, &eligible, now);
        assert_eq!(
            slot.tick(&eligible, now + WINDOW - Duration::from_nanos(1)),
            Decision::Pending
        );
        let failed = now + WINDOW;
        assert_eq!(
            slot.vote(Kind::Surrender, 1, 2, true, &eligible, failed),
            Decision::Failed
        );
        assert_eq!(slot.cooldown(failed), COOLDOWN);
        assert_eq!(
            start(
                &mut slot,
                Kind::Surrender,
                &eligible,
                failed + COOLDOWN - Duration::from_nanos(1)
            ),
            Decision::Ignored
        );
        assert_eq!(
            slot.start(
                Kind::Surrender,
                1,
                1,
                &eligible,
                failed + COOLDOWN,
                failed + COOLDOWN + WINDOW
            ),
            Decision::Ignored
        );
        assert_eq!(
            start(&mut slot, Kind::Surrender, &eligible, failed + COOLDOWN),
            Decision::Pending
        );
        assert_eq!(slot.active().unwrap().generation, 2);
    }
    #[test]
    fn impossible_quorum_fails_without_waiting_or_extending_a_replayed_cooldown() {
        let now = Instant::now();
        let eligible = ids(5);
        let mut slot = Slot::default();
        start(&mut slot, Kind::Surrender, &eligible, now);
        assert_eq!(
            slot.vote(Kind::Surrender, 1, 2, false, &eligible, now),
            Decision::Pending
        );
        assert_eq!(
            slot.vote(Kind::Surrender, 1, 3, false, &eligible, now),
            Decision::Failed
        );
        assert_eq!(
            slot.vote(
                Kind::Surrender,
                1,
                3,
                false,
                &eligible,
                now + Duration::from_secs(10)
            ),
            Decision::Ignored
        );
        assert_eq!(
            slot.cooldown(now + Duration::from_secs(10)),
            Duration::from_secs(110)
        );
    }
    #[test]
    fn disconnected_ballot_is_not_counted_but_cannot_be_changed_after_return() {
        let now = Instant::now();
        let all = ids(5);
        let mut slot = Slot::default();
        start(&mut slot, Kind::Surrender, &all, now);
        assert_eq!(
            slot.vote(Kind::Surrender, 1, 2, true, &all, now),
            Decision::Pending
        );
        let remaining = BTreeSet::from([1, 3, 4, 5]);
        assert_eq!(slot.tick(&remaining, now), Decision::Pending);
        assert_eq!(slot.active().unwrap().counts(&remaining).yes, 1);
        assert_eq!(
            slot.vote(Kind::Surrender, 1, 2, false, &all, now),
            Decision::Ignored
        );
        assert_eq!(slot.active().unwrap().ballot(2), Some(true));
    }
    #[test]
    fn empty_or_unbounded_roster_fails_closed_and_generations_never_wrap() {
        let now = Instant::now();
        let mut slot = Slot::default();
        assert_eq!(
            start(&mut slot, Kind::Surrender, &ids(0), now),
            Decision::Ignored
        );
        assert_eq!(
            start(&mut slot, Kind::Surrender, &ids(33), now),
            Decision::Ignored
        );
        slot.generation = u64::MAX;
        assert_eq!(slot.next_generation(), None);
        assert_eq!(
            slot.start(Kind::Remake, 0, 1, &ids(2), now, now + WINDOW),
            Decision::Ignored
        );
    }
    #[test]
    fn shortened_deadline_and_cancellation_are_terminal_for_that_generation() {
        let now = Instant::now();
        let mut slot = Slot::default();
        let eligible = ids(4);
        assert_eq!(
            slot.start(
                Kind::Remake,
                1,
                1,
                &eligible,
                now,
                now + Duration::from_secs(5)
            ),
            Decision::Pending
        );
        assert_eq!(
            slot.tick(&eligible, now + Duration::from_secs(5)),
            Decision::Failed
        );
        assert_eq!(
            slot.cancel(now + Duration::from_secs(10)),
            Decision::Ignored
        );
        assert_eq!(
            slot.cooldown(now + Duration::from_secs(10)),
            Duration::from_secs(115)
        );
    }
}
