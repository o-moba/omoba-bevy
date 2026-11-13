//! Shared queue counters and exact capacity-line invalidation, independent of admission.
use super::*;

#[derive(Default)]
struct WindowCounts {
    lows: Vec<i32>,
    tree: Vec<usize>,
    base: usize,
    spread: i32,
}
impl WindowCounts {
    fn new(ratings: &[i32], spread: i32) -> Self {
        let mut lows = Vec::new();
        let mut counts = Vec::new();
        let (mut start, mut end) = (0, 0);
        while start < ratings.len() {
            let low = ratings[start];
            while end < ratings.len()
                && i64::from(ratings[end]) <= i64::from(low) + i64::from(spread)
            {
                end += 1;
            }
            lows.push(low);
            counts.push(end - start);
            while start < ratings.len() && ratings[start] == low {
                start += 1;
            }
        }
        let base = counts.len().max(1).next_power_of_two();
        let mut tree = vec![0; base * 2];
        tree[base..base + counts.len()].copy_from_slice(&counts);
        for i in (1..base).rev() {
            tree[i] = tree[i * 2].max(tree[i * 2 + 1]);
        }
        Self {
            lows,
            tree,
            base,
            spread,
        }
    }
    fn peers(&self, rating: i32) -> usize {
        let low = i64::from(rating) - i64::from(self.spread);
        let mut left = self.lows.partition_point(|&r| i64::from(r) < low) + self.base;
        let mut right = self.lows.partition_point(|&r| r <= rating) + self.base;
        let mut best = 0;
        while left < right {
            if left & 1 == 1 {
                best = best.max(self.tree[left]);
                left += 1;
            }
            if right & 1 == 1 {
                right -= 1;
                best = best.max(self.tree[right]);
            }
            left /= 2;
            right /= 2;
        }
        best
    }
}
#[derive(Clone, Copy)]
struct UnitView {
    size: usize,
    invalid_ranked: bool,
    also_casual: bool,
}
struct Entry {
    index: usize,
    preference: MatchPreference,
    casual: bool,
    unit: Option<UnitView>,
}
struct Member {
    profile: String,
    preference: MatchPreference,
    queued: Instant,
}

#[derive(Default)]
pub(super) struct Snapshot {
    entries: HashMap<String, Entry>,
    parties: HashMap<u64, Vec<Member>>,
    windows: std::collections::BTreeMap<(i32, bool, Option<bool>), WindowCounts>,
    ranked: usize,
    casual: usize,
}
impl Snapshot {
    pub(super) fn new(waiting: &[Waiting], now: Instant) -> Self {
        let mut snapshot = Self::default();
        let mut ratings = [Vec::new(), Vec::new()];
        for (index, w) in waiting.iter().enumerate() {
            snapshot.entries.insert(
                w.profile.profile_id.clone(),
                Entry {
                    index,
                    preference: w.preference,
                    casual: w.casual_since.is_some(),
                    unit: None,
                },
            );
            snapshot.casual += usize::from(w.casual_since.is_some());
            if w.preference == MatchPreference::Ranked {
                snapshot.ranked += 1;
                ratings[usize::from(w.profile.newcomer())].push(w.profile.rating);
            }
            if let Some((party, _)) = w.party {
                snapshot.parties.entry(party).or_default().push(Member {
                    profile: w.profile.profile_id.clone(),
                    preference: w.preference,
                    queued: w.queued,
                });
            }
        }
        for values in &mut ratings {
            values.sort_unstable();
        }
        // Derive the finite windows from the shared policy instead of a second
        // hard-coded widening table.
        let mut policies = std::collections::BTreeSet::new();
        let mut add_policy = |age| {
            let p = CohortAllowance::after_wait(age);
            policies.insert((p.spread, p.merge_newcomers));
        };
        add_policy(Duration::ZERO);
        let steps = ((CohortAllowance::MAX.spread - CohortAllowance::BASE.spread)
            / CohortAllowance::WIDEN_STEP) as u32
            + 1;
        for step in 0..=steps {
            add_policy(
                CohortAllowance::WIDEN_AFTER + CohortAllowance::WIDEN_EVERY.saturating_mul(step),
            );
        }
        add_policy(CohortAllowance::MERGE_NEWCOMERS_AFTER);
        let mut merged = ratings.iter().flatten().copied().collect::<Vec<_>>();
        merged.sort_unstable();
        for (spread, merge) in policies {
            if merge {
                snapshot
                    .windows
                    .insert((spread, true, None), WindowCounts::new(&merged, spread));
            } else {
                for (bracket, values) in ratings.iter().enumerate() {
                    snapshot.windows.insert(
                        (spread, false, Some(bracket != 0)),
                        WindowCounts::new(values, spread),
                    );
                }
            }
        }
        for unit in units(waiting, now) {
            let view = UnitView {
                size: unit.len(),
                invalid_ranked: !same_cohort_with(
                    unit.iter().map(|&i| &waiting[i].profile),
                    CohortAllowance::BASE,
                ),
                also_casual: searches(waiting, &unit, MatchPreference::Casual),
            };
            for index in unit {
                snapshot
                    .entries
                    .get_mut(&waiting[index].profile.profile_id)
                    .unwrap()
                    .unit = Some(view);
            }
        }
        snapshot
    }

    /// Structural queue changes may shift an old index. Validate membership
    /// before using it; only the short interval before the next snapshot scans.
    pub(super) fn index(&self, waiting: &[Waiting], profile: &str) -> Option<usize> {
        self.entries
            .get(profile)
            .map(|e| e.index)
            .filter(|&i| {
                waiting
                    .get(i)
                    .is_some_and(|w| w.profile.profile_id == profile)
            })
            .or_else(|| waiting.iter().position(|w| w.profile.profile_id == profile))
    }
    pub(super) fn view(
        &self,
        waiting: &[Waiting],
        index: usize,
        line: Option<CapacityLine>,
        now: Instant,
    ) -> MatchServiceView {
        let w = &waiting[index];
        let old = self.entries.get(&w.profile.profile_id);
        let own = old.and_then(|e| e.unit);
        let ranked = w.preference == MatchPreference::Ranked;
        if ranked && own.is_some_and(|u| u.invalid_ranked) {
            return MatchServiceView::Failed {
                code: RANKED_PARTY_COHORT.into(),
            };
        }
        let ranked_count = self.ranked.saturating_sub(usize::from(
            old.is_some_and(|e| e.preference == MatchPreference::Ranked),
        )) + usize::from(ranked);
        let casual_count = self
            .casual
            .saturating_sub(usize::from(old.is_some_and(|e| e.casual)))
            + usize::from(w.casual_since.is_some());
        let (humans, needed, searching) = match w.preference {
            MatchPreference::BotPractice => (
                own.map_or(1, |u| u.size),
                w.party.map_or(1, |(_, n)| n),
                None,
            ),
            MatchPreference::Casual => (casual_count, 10, Some(casual_count)),
            MatchPreference::Ranked => {
                let policy = CohortAllowance::after_wait(now.saturating_duration_since(w.queued));
                let key = (
                    policy.spread,
                    policy.merge_newcomers,
                    (!policy.merge_newcomers).then_some(w.profile.newcomer()),
                );
                let peers = self
                    .windows
                    .get(&key)
                    .map_or(0, |window| window.peers(w.profile.rating))
                    .max(1);
                (peers, 10, Some(ranked_count))
            }
        };
        MatchServiceView::Waiting {
            preference: w.preference,
            humans: humans.min(10) as u32,
            needed: needed as u32,
            elapsed_secs: now.saturating_duration_since(w.queued).as_secs(),
            bot_fill_after_secs: (w.preference == MatchPreference::Casual)
                .then_some(CASUAL_BOT_FILL.as_secs()),
            capacity_wait: line.is_some(),
            searching: searching.map(|n| n as u32),
            casual_offer_after_secs: ranked.then_some(RANKED_CASUAL_OFFER.as_secs()),
            also_casual: ranked && w.casual_since.is_some() && own.is_some_and(|u| u.also_casual),
            capacity_line: line,
            party_gathering: None,
        }
    }
    pub(super) fn gathering(
        &self,
        entry: &Waiting,
        roster: Option<&[(String, String)]>,
        opt_outs: Option<&HashMap<String, Instant>>,
        now: Instant,
    ) -> Option<shared::match_service::PartyGathering> {
        let (party, expected) = entry.party?;
        let members = self.parties.get(&party)?;
        let present: Vec<_> = members
            .iter()
            .filter(|w| w.preference == entry.preference)
            .collect();
        let other_queue = members.len() - present.len();
        let needed = expected
            .saturating_sub(other_queue)
            .max(1)
            .saturating_sub(present.len());
        let oldest = present.iter().map(|w| w.queued).min()?;
        let deadline = oldest + PARTY_GATHER_WINDOW;
        if needed == 0 || now >= deadline {
            return None;
        }
        let missing = roster
            .into_iter()
            .flatten()
            .filter(|(id, _)| !members.iter().any(|w| w.profile == *id))
            .filter(|(id, _)| !opt_outs.is_some_and(|out| out.contains_key(id)))
            .map(|(_, name)| name.clone())
            .take(needed)
            .collect();
        Some(shared::match_service::PartyGathering {
            missing,
            needed: needed as u32,
            remaining_secs: deadline.saturating_duration_since(now).as_secs().max(1),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct LineEntry {
    profile: String,
    order: u64,
    preference: MatchPreference,
    rating: i32,
    newcomer: bool,
    party: Option<(u64, usize)>,
    casual: bool,
    casual_due: bool,
    gathering_done: bool,
    allowance: (i32, bool),
    source: Option<(SourceIp, usize, bool)>,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) struct LineKey {
    entries: Vec<LineEntry>,
    arenas_per_ip: usize,
}
impl LineKey {
    pub(super) fn new(lobby: &Lobby, now: Instant) -> Self {
        let mut held = HashMap::<SourceIp, usize>::new();
        for ips in lobby.arena_sources.values() {
            for (i, ip) in ips.iter().enumerate() {
                if !ips[..i].contains(ip) {
                    *held.entry(*ip).or_default() += 1;
                }
            }
        }
        let mut contexts = HashMap::<SourceIp, (usize, bool)>::new();
        let entries = lobby
            .waiting
            .iter()
            .map(|w| {
                let age = now.saturating_duration_since(w.queued);
                let policy = if w.preference == MatchPreference::Ranked {
                    CohortAllowance::after_wait(age)
                } else {
                    CohortAllowance::BASE
                };
                let source = lobby.sources.get(&w.profile.profile_id).map(|ip| {
                    let (arenas, cooling) = *contexts.entry(*ip).or_insert_with(|| {
                        (held.get(ip).copied().unwrap_or(0), lobby.cooling(*ip, now))
                    });
                    (*ip, arenas, cooling)
                });
                LineEntry {
                    profile: w.profile.profile_id.clone(),
                    order: w.order,
                    preference: w.preference,
                    rating: w.profile.rating,
                    newcomer: w.profile.newcomer(),
                    party: w.party,
                    casual: w.casual_since.is_some(),
                    casual_due: w
                        .casual_since
                        .is_some_and(|t| now.saturating_duration_since(t) >= CASUAL_BOT_FILL),
                    gathering_done: w.party.is_some() && age >= PARTY_GATHER_WINDOW,
                    allowance: (policy.spread, policy.merge_newcomers),
                    source,
                }
            })
            .collect();
        Self {
            entries,
            arenas_per_ip: lobby.limits.arenas_per_ip,
        }
    }
}

#[cfg(test)]
mod tests;
