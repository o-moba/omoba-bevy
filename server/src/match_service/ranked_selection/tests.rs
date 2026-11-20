use super::*;
use omoba_career_store::matchmaking::fits_cohort_with;

fn clock() -> Instant {
    Instant::now() + Duration::from_secs(1000)
}
fn waiter(
    id: u64,
    preference: MatchPreference,
    rating: i32,
    newcomer: bool,
    age: u64,
    now: Instant,
) -> Waiting {
    let queued = now - Duration::from_secs(age);
    Waiting {
        profile: ProfileSummary {
            ranked: None,
            completed_public_matches: 10,
            rating,
            rated_matches: if newcomer { 0 } else { 20 },
            ..ProfileSummary::new(format!("{id:064x}"), "Fixture".into())
        },
        session: format!("s{id}"),
        preference,
        queued,
        touched: now,
        order: id,
        party: None,
        casual_since: (preference == MatchPreference::Casual).then_some(queued),
    }
}
fn orders(waiting: &[Waiting], members: &[usize]) -> Vec<u64> {
    let mut out: Vec<_> = members.iter().map(|&i| waiting[i].order).collect();
    out.sort_unstable();
    out
}
fn due(waiting: &[Waiting], members: &[usize], now: Instant) -> bool {
    members.iter().any(|&i| {
        waiting[i]
            .casual_since
            .is_some_and(|t| now.saturating_duration_since(t) >= CASUAL_BOT_FILL)
    })
}
fn brute_ranked(
    waiting: &[Waiting],
    units: &[Vec<usize>],
    now: Instant,
) -> Option<(Vec<u64>, CohortAllowance)> {
    assert!(units.len() <= 10);
    let mut best: Option<(Vec<u64>, CohortAllowance, usize)> = None;
    for mask in 1usize..1 << units.len() {
        let picked: Vec<_> = units
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .collect();
        if picked.iter().map(|(_, u)| u.len()).sum::<usize>() != 10
            || picked.iter().any(|(_, u)| {
                !searches(waiting, u, MatchPreference::Ranked)
                    || !same_cohort_with(
                        u.iter().map(|&i| &waiting[i].profile),
                        CohortAllowance::BASE,
                    )
            })
            || !super::super::splits(&picked.iter().map(|(_, u)| u.len()).collect::<Vec<_>>())
        {
            continue;
        }
        let members: Vec<_> = picked.iter().flat_map(|(_, u)| u.iter().copied()).collect();
        for (anchor_index, anchor) in &picked {
            let low = anchor
                .iter()
                .map(|&i| &waiting[i])
                .min_by_key(|w| w.profile.rating)
                .unwrap();
            let age = anchor
                .iter()
                .map(|&i| now.saturating_duration_since(waiting[i].queued))
                .max()
                .unwrap();
            let policy = CohortAllowance::after_wait(age);
            if !members
                .iter()
                .all(|&i| fits_cohort_with(&low.profile, &waiting[i].profile, policy))
            {
                continue;
            }
            let order = orders(waiting, &members);
            if best.as_ref().is_none_or(|(old, _, old_anchor)| {
                order < *old || (order == *old && anchor_index < old_anchor)
            }) {
                best = Some((order, policy, *anchor_index));
            }
        }
    }
    best.map(|(members, policy, _)| (members, policy))
}
fn brute_pack(
    waiting: &[Waiting],
    units: &[Vec<usize>],
    mandatory: Option<usize>,
    allow_fill: bool,
    now: Instant,
) -> Option<Vec<u64>> {
    assert!(units.len() <= 10);
    let mut best: Option<Vec<u64>> = None;
    for mask in 1usize..1 << units.len() {
        if mandatory.is_some_and(|i| mask & (1 << i) == 0) {
            continue;
        }
        let chosen: Vec<_> = units
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, u)| u)
            .collect();
        let members: Vec<_> = chosen.iter().flat_map(|u| u.iter().copied()).collect();
        if members.len() > 10
            || (members.len() != 10 && !(allow_fill && due(waiting, &members, now)))
            || !super::super::splits(&chosen.iter().map(|u| u.len()).collect::<Vec<_>>())
        {
            continue;
        }
        let order = orders(waiting, &members);
        if best
            .as_ref()
            .is_none_or(|old| order.len() > old.len() || (order.len() == old.len() && order < *old))
        {
            best = Some(order);
        }
    }
    best
}
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
}

#[test]
fn sweeping_matches_an_independent_subset_oracle_with_parties_cohorts_and_wait_boundaries() {
    let now = clock();
    let mut rng = Rng(0x523900b1);
    let ages = [0, 60, 119, 120, 179, 180, 239, 240, 299, 300, 600];
    for case in 0..256 {
        let count = if case % 3 == 0 {
            6
        } else {
            1 + rng.next() as usize % 9
        };
        let mut waiting = Vec::new();
        for group in 0..count {
            let size = if case % 3 == 0 {
                [4, 4, 3, 2, 3, 2][group]
            } else {
                1 + rng.next() as usize % 5
            };
            let rating = if case % 3 == 0 {
                1000 + group as i32 * 7
            } else {
                700 + (rng.next() % 1200) as i32
            };
            let newcomer = case % 3 == 0 || rng.next().is_multiple_of(2);
            let age = ages[rng.next() as usize % ages.len()];
            let pref = if case % 3 == 0 || !rng.next().is_multiple_of(4) {
                MatchPreference::Ranked
            } else {
                MatchPreference::Casual
            };
            let incomplete = case % 3 != 0 && size < 5 && rng.next().is_multiple_of(7);
            for member in 0..size {
                let id = waiting.len() as u64 + 1;
                let mut w = waiter(id, pref, rating + member as i32 * 3, newcomer, age, now);
                if size > 1 {
                    w.party = Some((group as u64 + 1, size + usize::from(incomplete)));
                }
                if case % 11 == 0 {
                    w.order %= 8;
                }
                waiting.push(w);
            }
        }
        for i in (1..waiting.len()).rev() {
            let j = rng.next() as usize % (i + 1);
            waiting.swap(i, j);
        }
        let units = super::super::units(&waiting, now);
        let expected = brute_ranked(&waiting, &units, now);
        let actual = super::select(&waiting, &units, now)
            .map(|s| (orders(&waiting, &s.members), s.cohort.unwrap()));
        assert_eq!(actual, expected, "case {case}");
    }
}

#[test]
fn bounded_packing_matches_exhaustive_subsets_with_due_bits_and_a_mandatory_unit() {
    let now = clock();
    let mut rng = Rng(0xb07f11);
    for case in 0..160 {
        let count = 1 + rng.next() as usize % 9;
        let mut waiting = Vec::new();
        for group in 0..count {
            let size = 1 + rng.next() as usize % 5;
            let age = if rng.next().is_multiple_of(2) { 29 } else { 30 };
            for _ in 0..size {
                let mut w = waiter(
                    waiting.len() as u64 + 1,
                    MatchPreference::Casual,
                    1000,
                    true,
                    age,
                    now,
                );
                if size > 1 {
                    w.party = Some((group as u64 + 1, size));
                }
                waiting.push(w);
            }
        }
        let units = super::super::units(&waiting, now);
        let eligible: Vec<_> = units.iter().collect();
        for allow_fill in [false, true] {
            let mandatory = (case % 3 == 0).then(|| rng.next() as usize % units.len());
            let expected = brute_pack(&waiting, &units, mandatory, allow_fill, now);
            let actual = pack_units(
                &waiting,
                &eligible,
                mandatory.map(|i| &units[i]),
                allow_fill,
                now,
            )
            .map(|m| orders(&waiting, &m));
            assert_eq!(actual, expected, "case {case}, fill {allow_fill}");
        }
    }
}

#[test]
fn older_not_due_parties_cannot_hide_the_party_that_allows_bot_fill() {
    let now = clock();
    let mut waiting = Vec::new();
    for group in 0..11 {
        for _ in 0..3 {
            let mut w = waiter(
                waiting.len() as u64 + 1,
                MatchPreference::Casual,
                1000,
                true,
                if group == 10 { 30 } else { 0 },
                now,
            );
            w.party = Some((group + 1, 3));
            waiting.push(w);
        }
    }
    let units = super::super::units(&waiting, now);
    let eligible: Vec<_> = units.iter().collect();
    let selected = pack_units(&waiting, &eligible, None, true, now).unwrap();
    assert_eq!(orders(&waiting, &selected), [1, 2, 3, 31, 32, 33]);
    assert!(bounded_packing_units(&waiting, &eligible, true, now).len() <= 44);
}

#[test]
fn a_younger_mandatory_anchor_survives_dominance_pruning() {
    let now = clock();
    let waiting: Vec<_> = (1..=51)
        .map(|id| waiter(id, MatchPreference::Ranked, 1000, true, 0, now))
        .collect();
    let units = super::super::units(&waiting, now);
    let eligible: Vec<_> = units.iter().collect();
    let selected = pack_units(&waiting, &eligible, Some(&units[50]), false, now).unwrap();
    assert_eq!(orders(&waiting, &selected), [1, 2, 3, 4, 5, 6, 7, 8, 9, 51]);
    assert!(bounded_packing_units(&waiting, &eligible, false, now).len() <= 22);
}

#[test]
fn sweep_rating_arithmetic_is_wide_at_both_integer_boundaries() {
    let now = clock();
    for low in [i32::MIN, i32::MAX - 9] {
        let waiting: Vec<_> = (0..10)
            .map(|i| {
                waiter(
                    i + 1,
                    MatchPreference::Ranked,
                    low + i as i32,
                    true,
                    600,
                    now,
                )
            })
            .collect();
        let units = super::super::units(&waiting, now);
        assert_eq!(
            super::select(&waiting, &units, now).unwrap().members.len(),
            10
        );
    }
}

#[test]
fn work_stays_bounded_at_1000_and_4096_waiters() {
    let now = clock();
    for players in [1000_usize, 4096] {
        for shape in ["same_rating", "distinct_ratings", "triples_and_one_solo"] {
            let mut waiting = Vec::new();
            for i in 0..players {
                let group = if shape == "triples_and_one_solo" {
                    i / 3
                } else {
                    i
                };
                let age = if shape == "same_rating" {
                    0
                } else {
                    [0, 120, 180, 240, 300][group % 5]
                };
                let rating = if shape == "same_rating" {
                    1000
                } else {
                    ((group * 7919) % 5001) as i32
                };
                let mut w = waiter(
                    i as u64 + 1,
                    MatchPreference::Ranked,
                    rating,
                    shape == "same_rating" || group % 2 == 0,
                    age,
                    now,
                );
                if shape == "triples_and_one_solo" && i < players - 1 {
                    w.party = Some((group as u64 + 1, 3));
                }
                waiting.push(w);
            }
            let start = Instant::now();
            let result = super::super::select(&waiting, now);
            let millis = start.elapsed().as_secs_f64() * 1000.0;
            let work = LAST_WORK.with(|w| w.borrow().clone());
            assert!(work.groups <= 9 && work.unit_scans <= 9 * work.units_seen);
            assert!(
                work.sweep_updates <= 18 * work.units_seen
                    && work.pack_calls <= work.units_seen
                    && work.largest_pack <= 23
            );
            assert_eq!(result.is_none(), shape == "triples_and_one_solo");
            println!(
                "SELECTION_BENCH {}",
                serde_json::json!({"players":players,"shape":shape,"elapsed_ms":millis,"units":work.units_seen,"groups":work.groups,"unit_scans":work.unit_scans,"sweep_updates":work.sweep_updates,"pack_calls":work.pack_calls,"largest_pack":work.largest_pack})
            );
        }
    }
}
