//! Sweep the shared cohort windows; exact packing only needs bounded oldest units.
use std::collections::{BTreeMap, BTreeSet};

use super::*;

struct Unit {
    original: usize,
    low: i32,
    high: i32,
    newcomer: bool,
    order: Vec<u64>,
    allowance: CohortAllowance,
}

fn order_key(waiting: &[Waiting], unit: &[usize]) -> Vec<u64> {
    let mut key: Vec<_> = unit.iter().map(|&i| waiting[i].order).collect();
    key.sort_unstable();
    key
}

/// Same-size units are interchangeable for team capacity. Retain enough oldest
/// units to fill ten seats; Casual also needs a separate bot-fill-due bucket.
/// Restore input order for the existing exact DP's stable tie handling.
pub(super) fn bounded_packing_units<'a>(
    waiting: &[Waiting],
    eligible: &[&'a Vec<usize>],
    allow_fill: bool,
    now: Instant,
) -> Vec<&'a Vec<usize>> {
    let mut ordered: Vec<_> = eligible
        .iter()
        .enumerate()
        .filter(|(_, unit)| (1..=5).contains(&unit.len()))
        .map(|(position, unit)| (order_key(waiting, unit), position, *unit))
        .collect();
    ordered.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut counts = [[0_usize; 2]; 6];
    let mut kept = Vec::new();
    for (_, position, unit) in ordered {
        let due = allow_fill
            && unit.iter().any(|&i| {
                waiting[i]
                    .casual_since
                    .is_some_and(|at| now.saturating_duration_since(at) >= CASUAL_BOT_FILL)
            });
        let count = &mut counts[unit.len()][usize::from(due)];
        if *count < 10 / unit.len() {
            *count += 1;
            kept.push((position, unit));
        }
    }
    kept.sort_unstable_by_key(|(position, _)| *position);
    kept.into_iter().map(|(_, unit)| unit).collect()
}

#[cfg(test)]
#[derive(Clone, Debug, Default)]
struct Work {
    units_seen: usize,
    groups: usize,
    unit_scans: usize,
    sweep_updates: usize,
    pack_calls: usize,
    largest_pack: usize,
}
#[cfg(test)]
thread_local! { static LAST_WORK: std::cell::RefCell<Work> = std::cell::RefCell::new(Work::default()); }

pub(super) fn select(waiting: &[Waiting], units: &[Vec<usize>], now: Instant) -> Option<Selection> {
    #[cfg(test)]
    let mut work = Work {
        units_seen: units.len(),
        ..Default::default()
    };
    let mut metadata = Vec::new();
    for (original, unit) in units.iter().enumerate() {
        if !searches(waiting, unit, MatchPreference::Ranked)
            || unit.is_empty()
            || unit.len() > 5
            || !same_cohort_with(
                unit.iter().map(|&i| &waiting[i].profile),
                CohortAllowance::BASE,
            )
        {
            continue;
        }
        let low = unit
            .iter()
            .map(|&i| waiting[i].profile.rating)
            .min()
            .unwrap();
        let high = unit
            .iter()
            .map(|&i| waiting[i].profile.rating)
            .max()
            .unwrap();
        let waited = unit
            .iter()
            .map(|&i| now.saturating_duration_since(waiting[i].queued))
            .max()
            .unwrap();
        metadata.push(Unit {
            original,
            low,
            high,
            newcomer: waiting[unit[0]].profile.newcomer(),
            order: order_key(waiting, unit),
            allowance: CohortAllowance::after_wait(waited),
        });
    }
    // Each anchor belongs to one of the actual finite after_wait policies.
    // Merged windows have no newcomer restriction; other windows have one.
    let mut groups: BTreeMap<(i32, bool, Option<bool>), Vec<usize>> = BTreeMap::new();
    for (index, unit) in metadata.iter().enumerate() {
        let policy = unit.allowance;
        groups
            .entry((
                policy.spread,
                policy.merge_newcomers,
                (!policy.merge_newcomers).then_some(unit.newcomer),
            ))
            .or_default()
            .push(index);
    }
    let mut best: Option<(Vec<usize>, CohortAllowance, usize)> = None;
    for ((spread, merge, bracket), mut anchors) in groups {
        #[cfg(test)]
        {
            work.groups += 1;
            work.unit_scans += metadata.len();
        }
        let eligible: Vec<_> = (0..metadata.len())
            .filter(|&i| bracket.is_none_or(|newcomer| metadata[i].newcomer == newcomer))
            .collect();
        let mut by_high = eligible.clone();
        by_high.sort_unstable_by_key(|&i| (metadata[i].high, metadata[i].original));
        let mut by_low = eligible;
        by_low.sort_unstable_by_key(|&i| (metadata[i].low, metadata[i].original));
        anchors.sort_unstable_by_key(|&i| (metadata[i].low, metadata[i].original));
        let mut active: [BTreeSet<(Vec<u64>, usize)>; 6] = std::array::from_fn(|_| BTreeSet::new());
        let (mut add, mut remove) = (0, 0);
        let mut windows = HashSet::new();
        for anchor_index in anchors {
            let anchor = &metadata[anchor_index];
            let high = i64::from(anchor.low) + i64::from(spread);
            while add < by_high.len() && i64::from(metadata[by_high[add]].high) <= high {
                let index = by_high[add];
                let unit = &metadata[index];
                active[units[unit.original].len()].insert((unit.order.clone(), index));
                add += 1;
                #[cfg(test)]
                {
                    work.sweep_updates += 1;
                }
            }
            while remove < by_low.len() && metadata[by_low[remove]].low < anchor.low {
                let index = by_low[remove];
                let unit = &metadata[index];
                active[units[unit.original].len()].remove(&(unit.order.clone(), index));
                remove += 1;
                #[cfg(test)]
                {
                    work.sweep_updates += 1;
                }
            }
            let anchor_unit = &units[anchor.original];
            // Equal-size anchors in one window: the original oldest dominates.
            if !windows.insert((anchor.low, anchor_unit.len())) {
                continue;
            }
            let mut chosen: Vec<_> = (1..=5)
                .flat_map(|size| {
                    active[size]
                        .iter()
                        .take(10 / size)
                        .map(|(_, index)| metadata[*index].original)
                })
                .collect();
            chosen.push(anchor.original);
            chosen.sort_unstable();
            chosen.dedup();
            debug_assert!(chosen.len() <= 23);
            #[cfg(test)]
            {
                work.pack_calls += 1;
                work.largest_pack = work.largest_pack.max(chosen.len());
            }
            let candidate: Vec<_> = chosen.iter().map(|&index| &units[index]).collect();
            let Some(members) = pack_units(waiting, &candidate, Some(anchor_unit), false, now)
            else {
                continue;
            };
            let better = best.as_ref().is_none_or(|(old, _, old_anchor)| {
                let order = members
                    .iter()
                    .map(|&i| waiting[i].order)
                    .cmp(old.iter().map(|&i| waiting[i].order));
                order.is_lt() || (order.is_eq() && anchor.original < *old_anchor)
            });
            if better {
                best = Some((
                    members,
                    CohortAllowance {
                        spread,
                        merge_newcomers: merge,
                    },
                    anchor.original,
                ));
            }
        }
    }
    #[cfg(test)]
    LAST_WORK.with(|last| *last.borrow_mut() = work);
    best.map(|(members, allowance, _)| Selection {
        members,
        queue: MatchPreference::Ranked,
        cohort: Some(allowance),
    })
}

#[cfg(test)]
mod tests;
