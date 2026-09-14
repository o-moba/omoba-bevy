//! Bounded, server-owned queue policy. Networking, authentication, match
//! eligibility and durable settlement belong to the runtime/storage layer.
use std::{collections::HashSet, fmt, time::Instant};

use shared::{
    career::{MAX_PARTICIPANTS, ProfileSummary, QueueView, RatingChange, valid_profile_id},
    map::Team,
};

pub const MAX_WAITING: usize = 128;
pub const MAX_RATING_SPREAD: i32 = 300;
pub const MAX_RATING: i32 = 5000;
pub const ELO_K: i32 = 32;

#[derive(Clone, Debug)]
pub struct WaitingPlayer {
    pub player_id: u64,
    pub profile: ProfileSummary,
    pub queued_at: Instant,
    // Retained across countdown cancellation, including identical timestamps.
    order: u64,
}

#[derive(Clone, Debug)]
pub struct AssignedPlayer {
    pub waiting: WaitingPlayer,
    pub team: Team,
}

#[derive(Clone, Debug)]
pub struct MatchSelection {
    pub participants: Vec<AssignedPlayer>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueError {
    Full,
    DuplicateProfile,
    PlayerAlreadyQueued,
    InvalidPlayer,
    InvalidProfile,
    InvalidTeamSize,
    SelectionExists,
    InvalidSelection,
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full => "The waiting queue is full. Retry later.",
            Self::DuplicateProfile => "This profile already has a queue entry.",
            Self::PlayerAlreadyQueued => "This player is already queued under another profile.",
            Self::InvalidPlayer => "A queue player must have a nonzero server player ID.",
            Self::InvalidProfile => {
                "A queue profile needs a valid public identity and a rating from 0 to 5000."
            }
            Self::InvalidTeamSize => "Team size must be between 1 and 16.",
            Self::SelectionExists => "A different roster size is already reserved for countdown.",
            Self::InvalidSelection => {
                "Rating requires distinct players on two equal nonempty teams."
            }
        })
    }
}

impl std::error::Error for QueueError {}

/// Reserved countdown seats count toward the same bound as waiting entries.
/// This guarantees cancellation can restore every survivor without evicting
/// another player or expanding the queue.
#[derive(Default, Debug)]
pub struct Matchmaker {
    waiting: Vec<WaitingPlayer>,
    reserved: Option<MatchSelection>,
    next_order: u64,
}

impl Matchmaker {
    /// Number of waiting and reserved players; active matches are runtime-owned.
    pub fn len(&self) -> usize {
        self.waiting.len() + self.reserved.as_ref().map_or(0, |s| s.participants.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn selection(&self) -> Option<&MatchSelection> {
        self.reserved.as_ref()
    }

    /// Idempotent retries keep the original age and saved profile. Updating an
    /// entry explicitly requires cancellation/re-enqueue; a retry cannot alter
    /// a reserved roster's rating or newcomer classification.
    pub fn enqueue(
        &mut self,
        player_id: u64,
        profile: ProfileSummary,
        now: Instant,
    ) -> Result<bool, QueueError> {
        if player_id == 0 {
            return Err(QueueError::InvalidPlayer);
        }
        if !valid_profile_id(&profile.profile_id) || !(0..=MAX_RATING).contains(&profile.rating) {
            return Err(QueueError::InvalidProfile);
        }
        let entries = self.waiting.iter().chain(
            self.reserved
                .iter()
                .flat_map(|selection| selection.participants.iter().map(|p| &p.waiting)),
        );
        for entry in entries {
            if entry.profile.profile_id == profile.profile_id {
                return if entry.player_id == player_id {
                    Ok(false)
                } else {
                    Err(QueueError::DuplicateProfile)
                };
            }
            if entry.player_id == player_id {
                return Err(QueueError::PlayerAlreadyQueued);
            }
        }
        if self.len() >= MAX_WAITING {
            return Err(QueueError::Full);
        }
        self.waiting.push(WaitingPlayer {
            player_id,
            profile,
            queued_at: now,
            order: self.next_order,
        });
        self.next_order = self.next_order.saturating_add(1);
        Ok(true)
    }

    /// Removing a selected player invalidates the entire countdown selection.
    /// The runtime must demote its remaining selected heroes back to waiting.
    pub fn cancel(&mut self, player_id: u64) -> Option<WaitingPlayer> {
        if self.reserved.as_ref().is_some_and(|selection| {
            selection
                .participants
                .iter()
                .any(|player| player.waiting.player_id == player_id)
        }) {
            self.release_selection();
        }
        let index = self
            .waiting
            .iter()
            .position(|entry| entry.player_id == player_id)?;
        Some(self.waiting.remove(index))
    }

    /// Oldest feasible roster, then minimum possible difference in equal-team
    /// rating sums. Time affects priority only, never the compatibility limits.
    /// Calling this again during countdown returns the same frozen selection.
    pub fn reserve_match(
        &mut self,
        team_size: usize,
    ) -> Result<Option<MatchSelection>, QueueError> {
        let needed = roster_size(team_size)?;
        if let Some(selection) = &self.reserved {
            return if selection.participants.len() == needed {
                Ok(Some(selection.clone()))
            } else {
                Err(QueueError::SelectionExists)
            };
        }
        let Some(cohort) = oldest_feasible_cohort(&self.waiting, needed) else {
            return Ok(None);
        };
        let selected_ids: HashSet<_> = cohort.iter().map(|p| p.player_id).collect();
        let green_mask = balanced_team_mask(&cohort, team_size);
        let selection = MatchSelection {
            participants: cohort
                .into_iter()
                .enumerate()
                .map(|(index, waiting)| AssignedPlayer {
                    waiting,
                    team: if green_mask & (1u32 << index) != 0 {
                        Team::Green
                    } else {
                        Team::Blue
                    },
                })
                .collect(),
        };
        self.waiting
            .retain(|entry| !selected_ids.contains(&entry.player_id));
        self.reserved = Some(selection.clone());
        Ok(Some(selection))
    }

    /// Undo a countdown without resetting queue age or exceeding capacity.
    pub fn release_selection(&mut self) {
        if let Some(selection) = self.reserved.take() {
            self.waiting
                .extend(selection.participants.into_iter().map(|p| p.waiting));
        }
    }

    /// Transfer a full selection to the authoritative Running match. Later
    /// reconnects and its frozen result roster are maintained by the runtime.
    pub fn commit_selection(&mut self) -> Option<MatchSelection> {
        self.reserved.take()
    }

    pub fn view(&self, player_id: u64, team_size: usize, now: Instant) -> QueueView {
        if self.reserved.as_ref().is_some_and(|selection| {
            selection
                .participants
                .iter()
                .any(|player| player.waiting.player_id == player_id)
        }) {
            return QueueView::Selected;
        }
        let Some(entry) = self
            .waiting
            .iter()
            .find(|entry| entry.player_id == player_id)
        else {
            return QueueView::Idle;
        };
        let Ok(needed) = roster_size(team_size) else {
            return QueueView::Idle;
        };
        // Count the largest compatible rating window containing this player.
        // Merely counting everybody within +/-300 would admit a 600-point span.
        let compatible = self
            .waiting
            .iter()
            .filter(|low| {
                low.profile.newcomer() == entry.profile.newcomer()
                    && low.profile.rating <= entry.profile.rating
                    && rating_distance(low.profile.rating, entry.profile.rating)
                        <= MAX_RATING_SPREAD as i64
            })
            .map(|low| {
                self.waiting
                    .iter()
                    .filter(|candidate| {
                        candidate.profile.newcomer() == entry.profile.newcomer()
                            && candidate.profile.rating >= low.profile.rating
                            && rating_distance(low.profile.rating, candidate.profile.rating)
                                <= MAX_RATING_SPREAD as i64
                    })
                    .count()
            })
            .max()
            .unwrap_or(1)
            .min(needed);
        QueueView::Waiting {
            compatible: compatible as u32,
            needed: needed as u32,
            elapsed_secs: now.saturating_duration_since(entry.queued_at).as_secs(),
            newcomer: entry.profile.newcomer(),
        }
    }
}

fn roster_size(team_size: usize) -> Result<usize, QueueError> {
    if !(1..=MAX_PARTICIPANTS / 2).contains(&team_size) {
        return Err(QueueError::InvalidTeamSize);
    }
    Ok(team_size * 2)
}

fn priority(player: &WaitingPlayer) -> (Instant, u64, u64) {
    (player.queued_at, player.order, player.player_id)
}

fn rating_distance(a: i32, b: i32) -> i64 {
    (i64::from(a) - i64::from(b)).abs()
}

fn oldest_feasible_cohort(waiting: &[WaitingPlayer], needed: usize) -> Option<Vec<WaitingPlayer>> {
    if waiting.len() < needed {
        return None;
    }
    let mut best: Option<Vec<&WaitingPlayer>> = None;
    // Every feasible cohort has a minimum rating represented in this list.
    // Take the oldest N entries in each hard-width window, then compare whole
    // age sequences. This also serves a later feasible cohort when the queue's
    // first entry has no compatible peers.
    for low in waiting {
        let mut candidates: Vec<_> = waiting
            .iter()
            .filter(|entry| {
                entry.profile.newcomer() == low.profile.newcomer()
                    && entry.profile.rating >= low.profile.rating
                    && rating_distance(low.profile.rating, entry.profile.rating)
                        <= MAX_RATING_SPREAD as i64
            })
            .collect();
        if candidates.len() < needed {
            continue;
        }
        candidates.sort_unstable_by_key(|entry| priority(entry));
        candidates.truncate(needed);
        let earlier = best.as_ref().is_none_or(|current| {
            candidates
                .iter()
                .map(|p| priority(p))
                .cmp(current.iter().map(|p| priority(p)))
                .is_lt()
        });
        if earlier {
            best = Some(candidates);
        }
    }
    best.map(|entries| entries.into_iter().cloned().collect())
}

/// Exact cardinality-constrained subset sum. Subtracting the cohort's minimum
/// keeps the DP bounded by 17 * 4801 states at any permitted absolute rating.
/// A u32 records the chosen subset because a roster has at most 32 players.
fn balanced_team_mask(cohort: &[WaitingPlayer], team_size: usize) -> u32 {
    let minimum = cohort
        .iter()
        .map(|p| p.profile.rating)
        .min()
        .expect("nonempty cohort");
    let offsets: Vec<_> = cohort
        .iter()
        .map(|p| (i64::from(p.profile.rating) - i64::from(minimum)) as usize)
        .collect();
    let max_sum = team_size * MAX_RATING_SPREAD as usize;
    let width = max_sum + 1;
    let mut sums = vec![None; (team_size + 1) * width];
    sums[0] = Some(0u32);
    for (index, &offset) in offsets.iter().enumerate() {
        for count in (1..=team_size.min(index + 1)).rev() {
            for sum in (offset..=max_sum).rev() {
                let target = count * width + sum;
                if sums[target].is_none()
                    && let Some(previous) = sums[(count - 1) * width + sum - offset]
                {
                    sums[target] = Some(previous | (1u32 << index));
                }
            }
        }
    }
    let total = offsets.iter().sum::<usize>();
    (0..=max_sum)
        .filter_map(|sum| {
            sums[team_size * width + sum].map(|mask| ((sum * 2).abs_diff(total), mask))
        })
        .min_by_key(|&(gap, mask)| (gap, mask))
        .expect("an equal-size subset always exists")
        .1
}

/// Rating policy only: the caller must establish rated eligibility and settle
/// exactly once. It deliberately has no damage, kill, duration or XP input.
#[cfg(test)]
pub fn rating_changes(
    selection: &MatchSelection,
    winner: Team,
) -> Result<Vec<(u64, RatingChange)>, QueueError> {
    let players: Vec<_> = selection
        .participants
        .iter()
        .map(|p| (p.waiting.player_id, p.team, p.waiting.profile.rating))
        .collect();
    rating_changes_for_players(&players, winner)
}

pub fn rating_changes_for_players(
    players: &[(u64, Team, i32)],
    winner: Team,
) -> Result<Vec<(u64, RatingChange)>, QueueError> {
    if players.is_empty() || players.len() > MAX_PARTICIPANTS || !players.len().is_multiple_of(2) {
        return Err(QueueError::InvalidSelection);
    }
    let mut identities = HashSet::new();
    if players.iter().any(|&(id, _, rating)| {
        id == 0 || !(0..=MAX_RATING).contains(&rating) || !identities.insert(id)
    }) {
        return Err(QueueError::InvalidSelection);
    }
    let green_count = players
        .iter()
        .filter(|&&(_, team, _)| team == Team::Green)
        .count();
    if green_count != players.len() / 2 {
        return Err(QueueError::InvalidSelection);
    }
    let average = |team: Team| {
        players
            .iter()
            .filter(|&&(_, t, _)| t == team)
            .map(|&(_, _, rating)| f64::from(rating))
            .sum::<f64>()
            / green_count as f64
    };
    let losing_team = if winner == Team::Green {
        Team::Blue
    } else {
        Team::Green
    };
    let expected_win =
        1.0 / (1.0 + 10.0_f64.powf((average(losing_team) - average(winner)) / 400.0));
    let transfer = (f64::from(ELO_K) * (1.0 - expected_win)).round() as i32;
    // Fixed-K team outcome changes are symmetric away from the bounds. Clamp
    // each player independently so a zero-rated loser cannot prevent winners
    // from progressing. Bounds intentionally make some matches non-zero-sum;
    // report the actual persisted difference, never the unclamped request.
    Ok(players
        .iter()
        .map(|&(id, team, before)| {
            let requested = if team == winner { transfer } else { -transfer };
            let after = (before + requested).clamp(0, MAX_RATING);
            (
                id,
                RatingChange {
                    before,
                    after,
                    delta: after - before,
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::career::NEWCOMER_MATCHES;
    use std::time::Duration;

    fn profile(id: u64, rating: i32, rated_matches: u32) -> ProfileSummary {
        let mut profile = ProfileSummary::new(format!("{id:064x}"), format!("player-{id}"));
        profile.rating = rating;
        profile.rated_matches = rated_matches;
        profile
    }

    fn enqueue(queue: &mut Matchmaker, id: u64, rating: i32, experience: u32, now: Instant) {
        assert_eq!(
            queue.enqueue(id, profile(id, rating, experience), now),
            Ok(true)
        );
    }

    fn ids(selection: &MatchSelection) -> Vec<u64> {
        selection
            .participants
            .iter()
            .map(|p| p.waiting.player_id)
            .collect()
    }

    #[test]
    fn newcomer_and_huge_skill_gaps_never_widen_with_wait() {
        let now = Instant::now();
        let mut queue = Matchmaker::default();
        enqueue(&mut queue, 1, 1000, 0, now);
        enqueue(&mut queue, 2, 1000, NEWCOMER_MATCHES, now);
        enqueue(&mut queue, 3, 1600, 1, now);
        for seconds in [0, 60, 3600, 86_400] {
            assert!(queue.reserve_match(1).unwrap().is_none());
            assert_eq!(
                queue.view(1, 1, now + Duration::from_secs(seconds)),
                QueueView::Waiting {
                    compatible: 1,
                    needed: 2,
                    elapsed_secs: seconds,
                    newcomer: true,
                }
            );
        }
        enqueue(
            &mut queue,
            4,
            1300,
            NEWCOMER_MATCHES - 1,
            now + Duration::from_secs(86_401),
        );
        let found = queue.reserve_match(1).unwrap().unwrap();
        assert_eq!(ids(&found), vec![1, 4]);
        assert!(
            found
                .participants
                .iter()
                .all(|p| p.waiting.profile.newcomer())
        );
    }

    #[test]
    fn later_feasible_cohort_is_not_blocked_by_an_isolated_oldest_player() {
        let now = Instant::now();
        let mut queue = Matchmaker::default();
        enqueue(&mut queue, 1, 1000, 0, now);
        enqueue(&mut queue, 2, 2100, 300, now + Duration::from_secs(1));
        enqueue(&mut queue, 3, 2200, 20, now + Duration::from_secs(2));
        assert_eq!(ids(&queue.reserve_match(1).unwrap().unwrap()), vec![2, 3]);
        assert!(matches!(
            queue.view(1, 1, now),
            QueueView::Waiting { compatible: 1, .. }
        ));
    }

    #[test]
    fn oldest_feasible_cohort_compares_all_ages_not_just_adjacent_ratings() {
        let now = Instant::now();
        let mut queue = Matchmaker::default();
        enqueue(&mut queue, 1, 1000, 25, now);
        enqueue(&mut queue, 2, 1300, 25, now + Duration::from_secs(1));
        enqueue(&mut queue, 3, 1150, 25, now + Duration::from_secs(2));
        assert_eq!(ids(&queue.reserve_match(1).unwrap().unwrap()), vec![1, 2]);
    }

    #[test]
    fn compatible_count_uses_one_hard_window_not_plus_minus_spread() {
        let now = Instant::now();
        let mut queue = Matchmaker::default();
        for (id, rating) in [(1, 700), (2, 1000), (3, 1300), (4, 1301)] {
            enqueue(&mut queue, id, rating, 0, now);
        }
        assert!(queue.reserve_match(2).unwrap().is_none());
        assert_eq!(
            queue.view(2, 2, now),
            QueueView::Waiting {
                compatible: 2,
                needed: 4,
                elapsed_secs: 0,
                newcomer: true
            }
        );
    }

    #[test]
    fn account_and_player_uniqueness_retries_and_capacity_include_reservations() {
        let now = Instant::now();
        let mut queue = Matchmaker::default();
        for id in 1..=MAX_WAITING as u64 {
            enqueue(&mut queue, id, 1000, 0, now);
        }
        assert_eq!(
            queue.enqueue(1, profile(1, 2500, 100), now + Duration::from_secs(10)),
            Ok(false)
        );
        assert_eq!(
            queue.enqueue(999, profile(1, 1000, 0), now),
            Err(QueueError::DuplicateProfile)
        );
        assert_eq!(
            queue.enqueue(1, profile(999, 1000, 0), now),
            Err(QueueError::PlayerAlreadyQueued)
        );
        assert_eq!(
            queue.enqueue(999, profile(999, 1000, 0), now),
            Err(QueueError::Full)
        );
        let selected = queue.reserve_match(5).unwrap().unwrap();
        assert_eq!(selected.participants[0].waiting.profile.rating, 1000);
        assert_eq!(queue.len(), MAX_WAITING);
        assert_eq!(
            queue.enqueue(999, profile(999, 1000, 0), now),
            Err(QueueError::Full)
        );
        assert_eq!(
            ids(&queue.reserve_match(5).unwrap().unwrap()),
            ids(&selected)
        );
        queue.release_selection();
        assert_eq!(queue.len(), MAX_WAITING);
        assert_eq!(
            ids(&queue.reserve_match(5).unwrap().unwrap()),
            ids(&selected)
        );
        assert_eq!(queue.commit_selection().unwrap().participants.len(), 10);
        assert_eq!(queue.len(), MAX_WAITING - 10);
        assert_eq!(queue.enqueue(999, profile(999, 1000, 0), now), Ok(true));
    }

    #[test]
    fn countdown_dropout_requeues_survivors_with_original_age() {
        let now = Instant::now();
        let mut queue = Matchmaker::default();
        enqueue(&mut queue, 1, 1000, 0, now);
        enqueue(&mut queue, 2, 1000, 0, now + Duration::from_secs(1));
        let selected = queue.reserve_match(1).unwrap().unwrap();
        assert_eq!(queue.view(1, 1, now), QueueView::Selected);
        enqueue(&mut queue, 3, 1000, 0, now + Duration::from_secs(2));
        assert_eq!(
            queue.cancel(2).unwrap().queued_at,
            selected.participants[1].waiting.queued_at
        );
        assert!(queue.selection().is_none());
        assert_eq!(
            queue.view(1, 1, now + Duration::from_secs(20)),
            QueueView::Waiting {
                compatible: 2,
                needed: 2,
                elapsed_secs: 20,
                newcomer: true
            }
        );
        assert_eq!(ids(&queue.reserve_match(1).unwrap().unwrap()), vec![1, 3]);
        assert!(queue.cancel(999).is_none());
    }

    #[test]
    fn exact_equal_team_partition_beats_fill_order_and_matches_exhaustive_optimum() {
        let now = Instant::now();
        // Different rating arrangements catch greedy/snake-only partitions.
        for ratings in [
            [1000, 1010, 1090, 1120, 1160, 1230],
            [1000, 1000, 1001, 1002, 1299, 1300],
            [1200; 6],
        ] {
            let mut queue = Matchmaker::default();
            for (index, rating) in ratings.into_iter().enumerate() {
                enqueue(&mut queue, index as u64 + 1, rating, 0, now);
            }
            let selection = queue.reserve_match(3).unwrap().unwrap();
            let total = ratings.iter().map(|&r| i64::from(r)).sum::<i64>();
            let green = selection
                .participants
                .iter()
                .filter(|p| p.team == Team::Green)
                .map(|p| i64::from(p.waiting.profile.rating))
                .sum::<i64>();
            let best = (0u32..1 << 6)
                .filter(|mask| mask.count_ones() == 3)
                .map(|mask| {
                    let sum = ratings
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| mask & (1 << i) != 0)
                        .map(|(_, r)| i64::from(*r))
                        .sum::<i64>();
                    (sum * 2 - total).abs()
                })
                .min()
                .unwrap();
            assert_eq!((green * 2 - total).abs(), best);
            assert_eq!(
                selection
                    .participants
                    .iter()
                    .filter(|p| p.team == Team::Green)
                    .count(),
                3
            );
        }
    }

    #[test]
    fn maximum_roster_handles_large_absolute_ratings_with_bounded_offsets() {
        let now = Instant::now();
        let mut queue = Matchmaker::default();
        for id in 1..=32 {
            enqueue(&mut queue, id, MAX_RATING - 300 + (id as i32 * 7), 100, now);
        }
        let selection = queue.reserve_match(16).unwrap().unwrap();
        assert_eq!(selection.participants.len(), 32);
        assert_eq!(
            selection
                .participants
                .iter()
                .filter(|p| p.team == Team::Green)
                .count(),
            16
        );
        assert_eq!(
            rating_changes(&selection, Team::Green)
                .unwrap()
                .iter()
                .map(|(_, r)| r.delta)
                .sum::<i32>(),
            0
        );
        assert!(queue.commit_selection().is_some());
        assert!(queue.is_empty());
    }

    #[test]
    fn rating_is_team_outcome_only_symmetric_and_rewards_upsets() {
        let equal = [(1, Team::Green, 1000), (2, Team::Blue, 1000)];
        let win = rating_changes_for_players(&equal, Team::Green).unwrap();
        assert_eq!(
            win[0].1,
            RatingChange {
                before: 1000,
                after: 1016,
                delta: 16
            }
        );
        assert_eq!(
            win[1].1,
            RatingChange {
                before: 1000,
                after: 984,
                delta: -16
            }
        );
        let reversed = rating_changes_for_players(&equal, Team::Blue).unwrap();
        assert_eq!(win[0].1.delta, -reversed[0].1.delta);
        let unequal = [(1, Team::Green, 1000), (2, Team::Blue, 1300)];
        let upset = rating_changes_for_players(&unequal, Team::Green).unwrap();
        let expected = rating_changes_for_players(&unequal, Team::Blue).unwrap();
        assert!(upset[0].1.delta > 16 && upset[0].1.delta > expected[1].1.delta);
        assert_eq!(upset.iter().map(|(_, r)| r.delta).sum::<i32>(), 0);
    }

    #[test]
    fn rating_boundaries_clamp_independently_and_report_actual_non_zero_sum_deltas() {
        for players in [
            [(1, Team::Green, 0), (2, Team::Blue, 1)],
            [
                (1, Team::Green, MAX_RATING - 1),
                (2, Team::Blue, MAX_RATING),
            ],
        ] {
            for winner in [Team::Green, Team::Blue] {
                let changes = rating_changes_for_players(&players, winner).unwrap();
                assert_ne!(changes.iter().map(|(_, r)| r.delta).sum::<i32>(), 0);
                for (_, change) in changes {
                    assert_eq!(change.after - change.before, change.delta);
                    assert!((0..=MAX_RATING).contains(&change.after));
                }
            }
        }
        let zero_cohort = [(1, Team::Green, 0), (2, Team::Blue, 0)];
        let changes = rating_changes_for_players(&zero_cohort, Team::Green).unwrap();
        assert_eq!(changes[0].1.after, 16);
        assert_eq!(changes[0].1.delta, 16);
        assert_eq!(changes[1].1.after, 0);
        assert_eq!(changes[1].1.delta, 0);
        let upper_cohort = [(1, Team::Green, MAX_RATING), (2, Team::Blue, MAX_RATING)];
        let changes = rating_changes_for_players(&upper_cohort, Team::Green).unwrap();
        assert_eq!(changes[0].1.after, MAX_RATING);
        assert_eq!(changes[0].1.delta, 0);
        assert_eq!(changes[1].1.after, MAX_RATING - 16);
        assert_eq!(changes[1].1.delta, -16);
        for bad in [
            vec![],
            vec![(1, Team::Green, 1000)],
            vec![(1, Team::Green, 1000), (1, Team::Blue, 1000)],
            vec![(1, Team::Green, -1), (2, Team::Blue, 1000)],
            vec![(1, Team::Green, MAX_RATING + 1), (2, Team::Blue, 1000)],
            vec![(1, Team::Green, 1000), (2, Team::Green, 1000)],
        ] {
            assert_eq!(
                rating_changes_for_players(&bad, Team::Green),
                Err(QueueError::InvalidSelection)
            );
        }
    }

    #[test]
    fn invalid_input_never_mutates_queue() {
        let now = Instant::now();
        let mut queue = Matchmaker::default();
        assert_eq!(
            queue.enqueue(0, profile(1, 1000, 0), now),
            Err(QueueError::InvalidPlayer)
        );
        assert_eq!(
            queue.enqueue(1, profile(1, -1, 0), now),
            Err(QueueError::InvalidProfile)
        );
        assert_eq!(
            queue.enqueue(1, profile(1, MAX_RATING + 1, 0), now),
            Err(QueueError::InvalidProfile)
        );
        let mut bad = profile(1, 1000, 0);
        bad.profile_id = "claimed-nickname".into();
        assert_eq!(queue.enqueue(1, bad, now), Err(QueueError::InvalidProfile));
        assert!(queue.is_empty());
        assert!(matches!(
            queue.reserve_match(0),
            Err(QueueError::InvalidTeamSize)
        ));
        assert!(matches!(
            queue.reserve_match(17),
            Err(QueueError::InvalidTeamSize)
        ));
    }
}
