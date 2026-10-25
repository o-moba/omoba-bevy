//! One public lobby schedules independent, immutable game workers.
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use omoba_career_store::matchmaking::{fits_cohort, same_cohort};
use shared::career::ProfileSummary;
use shared::match_service::{InMatch, MatchAllocation, MatchPreference, MatchServiceView};
use shared::wire::{GameState, ServerPacket};

use crate::admission::{Limits, PRACTICE_ABANDON_WINDOW, SourceIp};
use crate::match_allocation::{AllocatedHuman, Manifest, Phase, Status, Worker, unix_ms};
use crate::runtime::ServerRuntime;

const MAX_WAITING: usize = 128;
/// Casual PvP waits this long for more humans, then bots fill the empty seats
/// (owner default O-2).
const CASUAL_BOT_FILL: Duration = Duration::from_secs(30);
/// After this long a Ranked search may also search Casual PvP, keeping its
/// place in Ranked (owner default O-1).
const RANKED_CASUAL_OFFER: Duration = Duration::from_secs(300);
/// A Ranked party whose members are not one rated cohort (LR-08).
const RANKED_PARTY_COHORT: &str = "ranked_party_cohort";
/// A roster whose every human comes from one IP that already holds
/// `arenas_per_ip` arenas alone (LR-16).
const ARENA_LIMIT: &str = "arena_limit";
/// vs Bots from an IP that left too many vs Bots allocations unplayed (LR-16).
const PRACTICE_COOLDOWN: &str = "practice_cooldown";

/// What one `FindMatch` asks for (LR-08).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Search {
    pub preference: MatchPreference,
    /// A Ranked search that also searches Casual PvP. Honoured once the
    /// offer is due; a retry without it withdraws it.
    pub also_casual: bool,
}

impl From<MatchPreference> for Search {
    fn from(preference: MatchPreference) -> Self {
        Self {
            preference,
            also_casual: false,
        }
    }
}

#[derive(Clone)]
struct Waiting {
    profile: ProfileSummary,
    session: String,
    preference: MatchPreference,
    queued: Instant,
    touched: Instant,
    order: u64,
    /// (party id, members the party waits for). Refreshed on every retry.
    party: Option<(u64, usize)>,
    /// Since when this entry searches Casual PvP: a Casual entry from the
    /// start, a Ranked entry once it took the Casual offer. The Casual bot
    /// fill counts from here.
    casual_since: Option<Instant>,
}

impl Waiting {
    /// Mirrors a Ranked entry's request to also search Casual PvP, once the
    /// offer is due. A Casual entry always searches Casual.
    fn also_casual(&mut self, also_casual: bool, now: Instant) {
        if self.preference != MatchPreference::Ranked {
            return;
        }
        if !also_casual {
            self.casual_since = None;
        } else if now.saturating_duration_since(self.queued) >= RANKED_CASUAL_OFFER {
            self.casual_since.get_or_insert(now);
        }
    }
}
pub(crate) struct Lobby {
    pool: crate::match_pool::Pool,
    waiting: Vec<Waiting>,
    assignments: HashMap<String, String>,
    errors: HashMap<String, String>,
    requests: HashMap<String, (String, u64, Instant)>,
    order: u64,
    last_poll: Option<Instant>,
    /// Per-source limits (LR-16), and what they track in memory only: the IP
    /// of each profile's latest FindMatch, the IPs of each live allocation
    /// this lobby made, and when each IP left a vs Bots allocation unplayed.
    limits: Limits,
    sources: HashMap<String, SourceIp>,
    arena_sources: HashMap<String, Vec<SourceIp>>,
    abandons: HashMap<SourceIp, VecDeque<Instant>>,
}

impl Lobby {
    fn new(pool: crate::match_pool::Pool, limits: Limits) -> Self {
        Self {
            pool,
            waiting: Vec::new(),
            assignments: HashMap::new(),
            errors: HashMap::new(),
            requests: HashMap::new(),
            order: 0,
            last_poll: None,
            limits,
            sources: HashMap::new(),
            arena_sources: HashMap::new(),
            abandons: HashMap::new(),
        }
    }

    /// Live arenas `ip` holds alone: every human of the allocation came from it.
    fn arenas_held(&self, ip: SourceIp) -> usize {
        self.arena_sources
            .iter()
            .filter(|(id, ips)| {
                ips.as_slice() == [ip]
                    && self
                        .pool
                        .slots
                        .get(*id)
                        .is_some_and(|slot| !slot.terminal())
            })
            .count()
    }

    /// Whether `ip` cools down after leaving too many vs Bots allocations
    /// unplayed within [`PRACTICE_ABANDON_WINDOW`].
    fn cooling(&self, ip: SourceIp, now: Instant) -> bool {
        self.abandons.get(&ip).is_some_and(|times| {
            let recent = times
                .iter()
                .filter(|at| now.saturating_duration_since(**at) < PRACTICE_ABANDON_WINDOW)
                .count();
            recent >= self.limits.practice_abandons
                && times.back().is_some_and(|last| {
                    now.saturating_duration_since(*last) < self.limits.practice_cooldown
                })
        })
    }

    /// Why a roster of `members` (indices into `waiting`) for `queue` may not
    /// get an arena now, as a lobby failure code.
    fn refusal(
        &self,
        members: &[usize],
        queue: MatchPreference,
        now: Instant,
    ) -> Option<&'static str> {
        let mut ips: Vec<SourceIp> = members
            .iter()
            .filter_map(|&i| {
                self.sources
                    .get(&self.waiting[i].profile.profile_id)
                    .copied()
            })
            .collect();
        ips.sort();
        ips.dedup();
        if queue == MatchPreference::BotPractice && ips.iter().any(|ip| self.cooling(*ip, now)) {
            return Some(PRACTICE_COOLDOWN);
        }
        match ips.as_slice() {
            [ip] if self.arenas_held(*ip) >= self.limits.arenas_per_ip => Some(ARENA_LIMIT),
            _ => None,
        }
    }

    /// Records the vs Bots allocations that ended without their match ever
    /// running, and forgets the IPs of every ended allocation.
    fn track_abandons(&mut self, now: Instant) {
        let pool = &self.pool;
        let abandons = &mut self.abandons;
        self.arena_sources.retain(|id, ips| {
            let Some(slot) = pool.slots.get(id) else {
                return false;
            };
            if !slot.terminal() {
                return true;
            }
            if slot.manifest.preference == MatchPreference::BotPractice && !slot.ran() {
                for ip in ips.iter() {
                    abandons.entry(*ip).or_default().push_back(now);
                }
            }
            false
        });
        let keep = PRACTICE_ABANDON_WINDOW.max(self.limits.practice_cooldown);
        abandons.retain(|_, times| {
            while times
                .front()
                .is_some_and(|at| now.saturating_duration_since(*at) >= keep)
            {
                times.pop_front();
            }
            !times.is_empty()
        });
    }
}
#[derive(Default)]
pub(crate) enum MatchService {
    #[default]
    Standalone,
    Lobby(Lobby),
    Worker(Worker),
}
impl MatchService {
    pub fn is_public(&self) -> bool {
        !matches!(self, Self::Standalone)
    }
    pub fn is_lobby(&self) -> bool {
        matches!(self, Self::Lobby(_))
    }
    pub fn can_observe(&self, session: &str, profile: &str) -> bool {
        self.worker()
            .is_none_or(|w| !w.recovery && w.manifest.team(profile, session).is_some())
    }
    pub fn worker(&self) -> Option<&Worker> {
        if let Self::Worker(w) = self {
            Some(w)
        } else {
            None
        }
    }
    /// `bound` is this process's bound UDP address (`SERVER_ADDR`).
    pub fn from_env(bound: std::net::SocketAddr, limits: Limits) -> Result<Self, String> {
        match std::env::var("OMOBA_SERVER_ROLE").ok().as_deref() {
            None | Some("") | Some("standalone") => Ok(Self::Standalone),
            Some("lobby") => {
                let pool = crate::match_pool::Pool::from_env(bound)?;
                let assignments = pool
                    .slots
                    .iter()
                    .flat_map(|(id, slot)| {
                        slot.manifest
                            .humans
                            .iter()
                            .map(move |h| (h.profile_id.clone(), id.clone()))
                    })
                    .collect();
                Ok(Self::Lobby(Lobby {
                    assignments,
                    ..Lobby::new(pool, limits)
                }))
            }
            Some("match") => {
                let path = PathBuf::from(
                    std::env::var_os("OMOBA_MATCH_ALLOCATION")
                        .ok_or("Match worker needs OMOBA_MATCH_ALLOCATION")?,
                );
                let manifest = Manifest::read(&path)?;
                let recovery = std::env::var("OMOBA_MATCH_RECOVERY").as_deref() == Ok("1");
                let origin_epoch = if recovery {
                    path.parent()
                        .and_then(|p| std::fs::read(p.join("status.json")).ok())
                        .and_then(|bytes| serde_json::from_slice::<Status>(&bytes).ok())
                        .map_or(0, |s| s.server_epoch)
                } else {
                    0
                };
                Ok(Self::Worker(Worker {
                    manifest,
                    directory: path
                        .parent()
                        .ok_or("Missing allocation directory")?
                        .to_path_buf(),
                    recovery,
                    origin_epoch,
                    aborted: false,
                    booted_at: Instant::now(),
                    last_status: None,
                    terminal_at: None,
                }))
            }
            Some(_) => Err("Invalid OMOBA_SERVER_ROLE".into()),
        }
    }
    /// `source` is the IP the signed FindMatch came from (LR-16).
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue(
        &mut self,
        profile: ProfileSummary,
        session: String,
        request_id: u64,
        search: Search,
        party: Option<(u64, usize)>,
        source: SourceIp,
        now: Instant,
    ) {
        let preference = search.preference;
        let Self::Lobby(lobby) = self else {
            return;
        };
        if lobby.requests.len() >= 512 && !lobby.requests.contains_key(&profile.profile_id) {
            return;
        }
        lobby.requests.insert(
            profile.profile_id.clone(),
            (session.clone(), request_id, now),
        );
        lobby.errors.remove(&profile.profile_id);
        lobby.sources.insert(profile.profile_id.clone(), source);
        // A new request can arrive between worker settlement and the lobby's
        // periodic pruning. Never bind it to the completed allocation.
        if let Some(slot) = lobby
            .assignments
            .get(&profile.profile_id)
            .and_then(|id| lobby.pool.slots.get_mut(id))
        {
            slot.refresh_status();
        }
        if lobby
            .assignments
            .get(&profile.profile_id)
            .is_some_and(|id| lobby.pool.slots.get(id).is_none_or(|slot| slot.terminal()))
        {
            lobby.assignments.remove(&profile.profile_id);
        }
        // A live allocation is either still being handed off or, once its
        // match runs, a seat this profile left: the view offers Resume and
        // the next FindMatch after the match ends queues (LR-13).
        if lobby.assignments.contains_key(&profile.profile_id) {
            return;
        }
        if let Some(existing) = lobby
            .waiting
            .iter_mut()
            .find(|w| w.profile.profile_id == profile.profile_id)
        {
            if existing.session == session && existing.preference == preference {
                existing.touched = now;
                existing.party = party;
                existing.also_casual(search.also_casual, now);
            } else {
                lobby
                    .errors
                    .insert(profile.profile_id, "already_queued".into());
            }
            return;
        }
        if lobby.waiting.len() >= MAX_WAITING {
            lobby.errors.insert(profile.profile_id, "queue_full".into());
            return;
        }
        // vs Bots allocates at once: refuse it here rather than after queueing.
        if preference == MatchPreference::BotPractice && party.is_none() {
            let refusal = if lobby.cooling(source, now) {
                Some(PRACTICE_COOLDOWN)
            } else {
                (lobby.arenas_held(source) >= lobby.limits.arenas_per_ip).then_some(ARENA_LIMIT)
            };
            if let Some(code) = refusal {
                lobby.errors.insert(profile.profile_id, code.into());
                return;
            }
        }
        lobby.order = lobby.order.saturating_add(1);
        lobby.waiting.push(Waiting {
            profile,
            session,
            preference,
            queued: now,
            touched: now,
            order: lobby.order,
            party,
            casual_since: (preference == MatchPreference::Casual).then_some(now),
        });
    }
    pub fn cancel(&mut self, profile: &str) {
        let Self::Lobby(lobby) = self else {
            return;
        };
        lobby.waiting.retain(|w| w.profile.profile_id != profile);
        lobby.errors.remove(profile);
        if let Some(id) = lobby.assignments.get(profile) {
            lobby.pool.cancel(id);
        }
    }
    pub fn request_id(&self, profile: &str, session: &str) -> Option<u64> {
        if let Self::Lobby(l) = self {
            l.requests
                .get(profile)
                .filter(|(s, _, _)| s == session)
                .map(|(_, id, _)| *id)
        } else {
            None
        }
    }
    pub fn view(&self, profile: &str, session: &str, now: Instant) -> Option<MatchServiceView> {
        let Self::Lobby(lobby) = self else {
            return None;
        };
        if let Some(id) = lobby.assignments.get(profile) {
            if let Some(slot) = lobby.pool.slots.get(id) {
                if let Some(h) = slot
                    .manifest
                    .humans
                    .iter()
                    .find(|h| h.profile_id == profile && h.session_id == session)
                {
                    let Some(status) = slot.ready_status() else {
                        return Some(MatchServiceView::Allocating);
                    };
                    // Every allocated human joins before a worker's match
                    // starts, so a running match on the lobby view is one this
                    // profile left: it can resume the seat, not be handed a
                    // fresh match (LR-13).
                    let in_match = (status.phase == Phase::Running).then(|| InMatch {
                        ends_within_secs: status
                            .ends_by_ms
                            .map(|end| end.saturating_sub(unix_ms()).div_ceil(1000)),
                    });
                    return Some(MatchServiceView::Assigned {
                        allocation: MatchAllocation {
                            allocation_id: id.clone(),
                            endpoint: slot.manifest.endpoint.clone(),
                            preference: slot.manifest.preference,
                            team: h.team,
                            human_count: slot.manifest.humans.len() as u32,
                            bot_count: 10 - slot.manifest.humans.len() as u32,
                            // Only Ranked is rated, when the match is eligible.
                            rated: slot.manifest.preference.rated(),
                            join_deadline_ms: slot.manifest.join_deadline_ms,
                        },
                        in_match,
                    });
                }
                return Some(MatchServiceView::Failed {
                    code: "profile_has_another_session".into(),
                });
            }
        }
        if let Some(code) = lobby.errors.get(profile) {
            return Some(MatchServiceView::Failed { code: code.clone() });
        }
        if let Some(index) = lobby
            .waiting
            .iter()
            .position(|w| w.profile.profile_id == profile)
        {
            return Some(waiting_view(&lobby.waiting, index, lobby.pool.full(), now));
        }
        Some(MatchServiceView::Idle)
    }
    pub fn tick(&mut self, now: Instant) {
        let Self::Lobby(lobby) = self else {
            return;
        };
        if lobby
            .last_poll
            .is_some_and(|at| now.duration_since(at) < Duration::from_secs(1))
        {
            return;
        }
        lobby.last_poll = Some(now);
        lobby.pool.poll();
        lobby.assignments.retain(|profile, id| {
            if let Some(slot) = lobby.pool.slots.get(id) {
                if !slot.terminal() {
                    return true;
                }
                // A played match that ended (finished, capped or reaped) frees
                // its seat silently; the next FindMatch queues (LR-13).
                if !slot.ran()
                    && slot
                        .status
                        .as_ref()
                        .is_some_and(|s| s.phase == Phase::Failed)
                {
                    lobby
                        .errors
                        .insert(profile.clone(), "allocation_cancelled_or_expired".into());
                }
            }
            false
        });
        expire_waiting(&mut lobby.waiting, now);
        lobby.track_abandons(now);
        lobby.requests.retain(|profile, (_, _, touched)| {
            now.saturating_duration_since(*touched) < Duration::from_secs(120)
                || lobby.assignments.contains_key(profile)
                || lobby
                    .waiting
                    .iter()
                    .any(|w| &w.profile.profile_id == profile)
        });
        lobby
            .errors
            .retain(|profile, _| lobby.requests.contains_key(profile));
        lobby
            .sources
            .retain(|profile, _| lobby.requests.contains_key(profile));
        // At most four OS process launches per scheduling pass.
        for _ in 0..4 {
            if lobby.pool.full() {
                break;
            }
            let Some((indices, queue)) = select(&lobby.waiting, now) else {
                break;
            };
            if let Some(code) = lobby.refusal(&indices, queue, now) {
                let refused: HashSet<_> = indices
                    .iter()
                    .map(|&i| lobby.waiting[i].profile.profile_id.clone())
                    .collect();
                for profile in &refused {
                    lobby.errors.insert(profile.clone(), code.into());
                }
                lobby
                    .waiting
                    .retain(|w| !refused.contains(&w.profile.profile_id));
                continue;
            }
            let selected: Vec<_> = indices.iter().map(|&i| lobby.waiting[i].clone()).collect();
            let teams = split_teams(&selected);
            let humans = selected
                .iter()
                .zip(teams)
                .map(|(w, team)| AllocatedHuman {
                    profile_id: w.profile.profile_id.clone(),
                    session_id: w.session.clone(),
                    team,
                })
                .collect();
            let mut bytes = [0_u8; 16];
            if getrandom::fill(&mut bytes).is_err() {
                break;
            }
            let id = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let manifest = Manifest {
                version: 1,
                allocation_id: id,
                endpoint: String::new(),
                bind: String::new(),
                preference: queue,
                humans,
                join_deadline_ms: unix_ms() + 180_000,
            };
            match lobby.pool.allocate(manifest) {
                Ok(id) => {
                    let mut ips: Vec<SourceIp> = selected
                        .iter()
                        .filter_map(|w| lobby.sources.get(&w.profile.profile_id).copied())
                        .collect();
                    ips.sort();
                    ips.dedup();
                    lobby.arena_sources.insert(id.clone(), ips);
                    for w in &selected {
                        lobby
                            .assignments
                            .insert(w.profile.profile_id.clone(), id.clone());
                    }
                    let ids: HashSet<_> = selected.iter().map(|w| &w.profile.profile_id).collect();
                    lobby
                        .waiting
                        .retain(|w| !ids.contains(&w.profile.profile_id));
                }
                Err(error) => {
                    for w in &selected {
                        lobby
                            .errors
                            .insert(w.profile.profile_id.clone(), error.clone());
                    }
                    break;
                }
            }
        }
    }
}
fn expire_waiting(waiting: &mut Vec<Waiting>, now: Instant) {
    waiting.retain(|w| now.saturating_duration_since(w.touched) < Duration::from_secs(15));
}

/// This unit searches `queue`: its own queue, and Casual PvP too for a Ranked
/// unit whose every member took the Casual offer.
fn searches(waiting: &[Waiting], unit: &[usize], queue: MatchPreference) -> bool {
    match queue {
        MatchPreference::Casual => unit.iter().all(|&i| waiting[i].casual_since.is_some()),
        _ => unit.iter().all(|&i| waiting[i].preference == queue),
    }
}

/// The searching screen's view of one queued entry.
fn waiting_view(
    waiting: &[Waiting],
    index: usize,
    capacity_wait: bool,
    now: Instant,
) -> MatchServiceView {
    let w = &waiting[index];
    let units = units(waiting);
    let own = units.iter().find(|unit| unit.contains(&index));
    let ranked = w.preference == MatchPreference::Ranked;
    // The lobby never seats a mixed party in Ranked: say so instead of
    // waiting forever (LR-08).
    if ranked && own.is_some_and(|unit| !same_cohort(unit.iter().map(|&i| &waiting[i].profile))) {
        return MatchServiceView::Failed {
            code: RANKED_PARTY_COHORT.into(),
        };
    }
    let count = |queue| {
        waiting
            .iter()
            .enumerate()
            .filter(|(i, _)| searches(waiting, &[*i], queue))
            .count()
    };
    let (humans, needed, searching) = match w.preference {
        MatchPreference::BotPractice => {
            let size = w.party.map_or(1, |(_, size)| size);
            (own.map_or(1, Vec::len), size, None)
        }
        // The largest rated cohort this player can be seated in.
        MatchPreference::Ranked => {
            let peers = waiting
                .iter()
                .filter(|low| {
                    low.preference == w.preference && fits_cohort(&low.profile, &w.profile)
                })
                .map(|low| {
                    waiting
                        .iter()
                        .filter(|other| {
                            other.preference == w.preference
                                && fits_cohort(&low.profile, &other.profile)
                        })
                        .count()
                })
                .max()
                .unwrap_or(1);
            (peers, 10, Some(count(MatchPreference::Ranked)))
        }
        MatchPreference::Casual => {
            let all = count(MatchPreference::Casual);
            (all, 10, Some(all))
        }
    };
    MatchServiceView::Waiting {
        preference: w.preference,
        humans: humans.min(10) as u32,
        needed: needed as u32,
        elapsed_secs: now.saturating_duration_since(w.queued).as_secs(),
        bot_fill_after_secs: (w.preference == MatchPreference::Casual)
            .then_some(CASUAL_BOT_FILL.as_secs()),
        capacity_wait,
        searching: searching.map(|n| n as u32),
        casual_offer_after_secs: ranked.then_some(RANKED_CASUAL_OFFER.as_secs()),
        // For a party, only once every member took the offer.
        also_casual: ranked
            && own.is_some_and(|unit| searches(waiting, unit, MatchPreference::Casual)),
    }
}
/// Queue units: a party's queued members travel together, everyone else
/// alone. A party is a unit only once every member it waits for has queued
/// with the same preference. Units are in queue order.
fn units(waiting: &[Waiting]) -> Vec<Vec<usize>> {
    let mut parties: std::collections::BTreeMap<u64, Vec<usize>> = Default::default();
    let mut out = Vec::new();
    for (i, w) in waiting.iter().enumerate() {
        match w.party {
            Some((id, _)) => parties.entry(id).or_default().push(i),
            None => out.push(vec![i]),
        }
    }
    for members in parties.into_values() {
        let need = members
            .iter()
            .filter_map(|&i| waiting[i].party.map(|(_, n)| n))
            .max()
            .unwrap_or(1);
        let preference = waiting[members[0]].preference;
        if members.len() >= need
            && members.len() <= 5
            && members.iter().all(|&i| waiting[i].preference == preference)
        {
            out.push(members);
        }
    }
    out.sort_by_key(|unit| unit.iter().map(|&i| waiting[i].order).min());
    out
}

/// Units of these sizes can be seated on two teams of at most five.
fn splits(sizes: &[usize]) -> bool {
    let total: usize = sizes.iter().sum();
    total <= 10
        && (0_u32..1 << sizes.len()).any(|mask| {
            let green: usize = sizes
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, s)| s)
                .sum();
            green <= 5 && total - green <= 5
        })
}

/// Team per selected entry: parties stay together, head counts as even as
/// possible, then rating sums as close as possible. A lone party (bot
/// practice) plays Green.
fn split_teams(selected: &[Waiting]) -> Vec<shared::map::Team> {
    use shared::map::Team;
    let units = units(selected);
    // Every selected entry belongs to exactly one unit; an incomplete party
    // cannot have been selected, but seat any stray entry on its own.
    let mut covered = vec![false; selected.len()];
    let mut units: Vec<Vec<usize>> = units;
    for unit in &units {
        for &i in unit {
            covered[i] = true;
        }
    }
    units.extend(
        (0..selected.len())
            .filter(|&i| !covered[i])
            .map(|i| vec![i]),
    );
    let rating = |unit: &Vec<usize>| -> i64 {
        unit.iter()
            .map(|&i| i64::from(selected[i].profile.rating))
            .sum()
    };
    let mut best: Option<((usize, i64, bool), u32)> = None;
    for mask in 0_u32..1 << units.len() {
        let (mut counts, mut sums) = ([0_usize; 2], [0_i64; 2]);
        for (u, unit) in units.iter().enumerate() {
            let side = usize::from(mask & (1 << u) == 0);
            counts[side] += unit.len();
            sums[side] += rating(unit);
        }
        if counts[0] > 5 || counts[1] > 5 {
            continue;
        }
        let key = (
            counts[0].abs_diff(counts[1]),
            (sums[0] - sums[1]).abs(),
            counts[0] < counts[1],
        );
        if best.is_none_or(|(k, _)| key < k) {
            best = Some((key, mask));
        }
    }
    let mask = best.map_or(0, |(_, mask)| mask);
    let mut teams = vec![Team::Green; selected.len()];
    for (u, unit) in units.iter().enumerate() {
        for &i in unit {
            teams[i] = if mask & (1 << u) != 0 {
                Team::Green
            } else {
                Team::Blue
            };
        }
    }
    teams
}

/// The next allocation and its queue (LR-08), or `None`.
///
/// - vs Bots: the oldest practice unit, at once.
/// - Ranked: ten humans of one rated cohort (the career store's policy, so
///   the store never rejects the roster), never filled with bots. A Ranked
///   roster goes before a Casual one, which keeps a Ranked player's place
///   while it also searches Casual.
/// - Casual PvP: every unit that searches Casual, whatever its rating or
///   newcomer cohort; once anyone has searched Casual for
///   [`CASUAL_BOT_FILL`], bots fill the empty seats.
///
/// Among rosters of one queue the oldest wins.
fn select(waiting: &[Waiting], now: Instant) -> Option<(Vec<usize>, MatchPreference)> {
    let units = units(waiting);
    if let Some(unit) = units
        .iter()
        .find(|unit| searches(waiting, unit, MatchPreference::BotPractice))
    {
        return Some((unit.clone(), MatchPreference::BotPractice));
    }
    let older = |a: &Vec<usize>, b: &Vec<usize>| {
        a.iter()
            .map(|&i| waiting[i].order)
            .cmp(b.iter().map(|&i| waiting[i].order))
            .is_lt()
    };
    for queue in [MatchPreference::Ranked, MatchPreference::Casual] {
        let mut best: Option<Vec<usize>> = None;
        for anchor in units.iter().filter(|unit| searches(waiting, unit, queue)) {
            let low = anchor
                .iter()
                .map(|&i| &waiting[i])
                .min_by_key(|w| w.profile.rating)
                .expect("units are never empty");
            // Ranked admits a unit only when its every member fits the
            // cohort that starts at the anchor's lowest rating.
            let fits = |unit: &Vec<usize>| {
                queue != MatchPreference::Ranked
                    || unit
                        .iter()
                        .all(|&i| fits_cohort(&low.profile, &waiting[i].profile))
            };
            if !fits(anchor) {
                continue;
            }
            let mut cohort: Vec<&Vec<usize>> = vec![anchor];
            for unit in &units {
                if std::ptr::eq(unit, anchor) || !searches(waiting, unit, queue) || !fits(unit) {
                    continue;
                }
                let mut sizes: Vec<usize> = cohort.iter().map(|u| u.len()).collect();
                sizes.push(unit.len());
                if splits(&sizes) {
                    cohort.push(unit);
                }
            }
            let mut members: Vec<usize> = cohort.iter().flat_map(|u| u.iter().copied()).collect();
            members.sort_by_key(|&i| waiting[i].order);
            let ready = members.len() == 10
                || (queue == MatchPreference::Casual
                    && members.iter().any(|&i| {
                        waiting[i].casual_since.is_some_and(|since| {
                            now.saturating_duration_since(since) >= CASUAL_BOT_FILL
                        })
                    }));
            if ready && best.as_ref().is_none_or(|b| older(&members, b)) {
                best = Some(members);
            }
        }
        if let Some(members) = best {
            return Some((members, queue));
        }
    }
    None
}
impl ServerRuntime {
    pub(crate) fn send_lobby_snapshots(&mut self, now: Instant) {
        if now.saturating_duration_since(self.last_bootstrap_at) < Duration::from_secs(1) {
            return;
        }
        self.last_bootstrap_at = now;
        self.snapshot_tick = self.snapshot_tick.saturating_add(1);
        for (addr, player) in &self.world.players {
            if !self.public_transport.validated(*addr, now)
                || (!self.match_service.is_lobby()
                    && self.career.backend.gameplay_principal(*addr).is_some())
            {
                continue;
            }
            let packet = ServerPacket::Snapshot {
                vision: None,
                sandbox: None,
                debug_access: None,
                match_mode: if self.match_service.is_lobby() {
                    "lobby"
                } else {
                    self.rules.mode_id()
                }
                .into(),
                geometry_id: self.world.map_config.geometry_id.clone(),
                map_profile: self.world.map_config.map_profile.clone(),
                meta: shared::protocol::SnapshotMeta::new(
                    self.server_epoch,
                    self.match_id,
                    self.snapshot_tick,
                ),
                join_error: None,
                your_id: player.hero.identity.id,
                players: Vec::new(),
                scoreboard: None,
                prematch: None,
                skill_effects: Vec::new(),
                projectiles: Vec::new(),
                combat_events: Vec::new(),
                structures: Vec::new(),
                minions: Vec::new(),
                neutrals: Vec::new(),
                team_buffs: Vec::new(),
                forest_pickups: Vec::new(),
                game_state: GameState::Lobby,
                rematch_in_secs: None,
            };
            if let Ok(bytes) = serde_json::to_vec(&packet) {
                let _ = self.transport.send_to(&bytes, *addr);
            }
        }
    }
    pub(crate) fn tick_match_service(&mut self, now: Instant) {
        // Only explicit FindMatch retries renew queue intent. An authenticated
        // Home heartbeat must not keep an abandoned queue alive after packet loss.
        self.match_service.tick(now);
        let Some(worker) = self.match_service.worker() else {
            return;
        };
        let recovery = worker.recovery;
        let abort = worker.aborted
            || recovery
            || (self.match_started_at.is_none()
                && (worker.cancelled() || unix_ms() > worker.manifest.join_deadline_ms));
        if abort {
            self.finish_career_round(shared::career::MatchOutcome::Abandoned, None, now);
        }
        let terminal = self.career_worker_terminal();
        let result_id = self.career_worker_result_id();
        let phase = if recovery {
            Phase::Recovering
        } else if abort {
            if terminal {
                Phase::Failed
            } else {
                Phase::Settling
            }
        } else if matches!(self.world.game_state, GameState::Running) {
            Phase::Running
        } else if matches!(self.world.game_state, GameState::Victory { .. }) {
            if terminal {
                Phase::Finished
            } else {
                Phase::Settling
            }
        } else if self.world.players.values().any(|p| p.joined) {
            Phase::Forming
        } else {
            Phase::Ready
        };
        // A player who left sees this on the lobby (LR-13).
        let ends_by_ms = (phase == Phase::Running)
            .then(|| self.latest_match_end(now))
            .flatten()
            .map(|end| {
                let left = end.saturating_duration_since(now).as_millis();
                unix_ms().saturating_add(u64::try_from(left).unwrap_or(u64::MAX))
            });
        let MatchService::Worker(worker) = &mut self.match_service else {
            return;
        };
        if worker.recovery
            && now.saturating_duration_since(worker.booted_at) > Duration::from_secs(120)
        {
            let pending = std::fs::read_dir(worker.directory.join("outbox")).is_ok_and(|entries| {
                entries
                    .flatten()
                    .any(|e| e.path().extension().is_some_and(|x| x == "json"))
            });
            if !pending
                && self
                    .career
                    .backend
                    .recovery_confirmed_since(worker.booted_at + Duration::from_secs(100))
            {
                let status = Status {
                    allocation_id: worker.manifest.allocation_id.clone(),
                    server_epoch: if worker.origin_epoch == 0 {
                        self.server_epoch
                    } else {
                        worker.origin_epoch
                    },
                    heartbeat_ms: unix_ms(),
                    phase: Phase::Failed,
                    result_id,
                    ends_by_ms: None,
                };
                if crate::match_allocation::atomic_json(
                    &worker.directory.join("status.json"),
                    &status,
                )
                .is_ok()
                {
                    std::process::exit(0);
                }
                return;
            }
        }
        if worker
            .last_status
            .is_none_or(|at| now.duration_since(at) >= Duration::from_secs(1))
        {
            let status = Status {
                allocation_id: worker.manifest.allocation_id.clone(),
                server_epoch: if worker.origin_epoch == 0 {
                    self.server_epoch
                } else {
                    worker.origin_epoch
                },
                heartbeat_ms: unix_ms(),
                phase: phase.clone(),
                result_id,
                ends_by_ms,
            };
            if crate::match_allocation::atomic_json(&worker.directory.join("status.json"), &status)
                .is_ok()
            {
                worker.last_status = Some(now);
            }
        }
        if matches!(phase, Phase::Finished | Phase::Failed) {
            let since = worker.terminal_at.get_or_insert(now);
            if now.saturating_duration_since(*since) >= Duration::from_secs(30) {
                std::process::exit(0);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::*;
    /// One player's own IP.
    fn home() -> SourceIp {
        SourceIp::of(std::net::SocketAddr::from(([203, 0, 113, 1], 40000)))
    }
    fn w(id: u64, p: MatchPreference, now: Instant) -> Waiting {
        Waiting {
            profile: ProfileSummary::new(format!("{id:064x}"), "Player".into()),
            session: format!("s{id}"),
            preference: p,
            queued: now,
            touched: now,
            order: id,
            party: None,
            casual_since: (p == MatchPreference::Casual).then_some(now),
        }
    }
    /// The indices `select` picks, for a queue that must form.
    fn picked(waiting: &[Waiting], now: Instant, queue: MatchPreference) -> Vec<usize> {
        let (members, picked) = select(waiting, now).expect("a roster forms");
        assert_eq!(picked, queue);
        members
    }
    #[test]
    fn abandoned_queue_expires_without_explicit_find_match_retry() {
        let now = Instant::now();
        let mut waiting = vec![w(1, MatchPreference::Ranked, now)];
        expire_waiting(&mut waiting, now + Duration::from_secs(14));
        assert_eq!(waiting.len(), 1);
        // Runtime heartbeats do not touch queue intent. Only enqueue/retry does.
        expire_waiting(&mut waiting, now + Duration::from_secs(15));
        assert!(waiting.is_empty());
        waiting.push(w(2, MatchPreference::Ranked, now));
        waiting[0].touched = now + Duration::from_secs(14);
        expire_waiting(&mut waiting, now + Duration::from_secs(15));
        assert_eq!(waiting.len(), 1);
    }

    /// LR-08: Casual bot-fills after 30 s; Ranked never bot-fills, however
    /// long it waits and however many humans short it is.
    #[test]
    fn casual_waits_thirty_seconds_and_ranked_never_bot_fills() {
        let now = Instant::now();
        let q = vec![w(1, MatchPreference::Casual, now)];
        assert!(select(&q, now + Duration::from_secs(29)).is_none());
        assert_eq!(
            picked(&q, now + CASUAL_BOT_FILL, MatchPreference::Casual),
            vec![0]
        );
        let ranked: Vec<_> = (1..=9)
            .map(|i| w(i, MatchPreference::Ranked, now))
            .collect();
        assert!(select(&ranked, now + Duration::from_secs(99_999)).is_none());
    }
    #[test]
    fn full_queues_and_practice_allocate_immediately() {
        let now = Instant::now();
        for queue in [MatchPreference::Ranked, MatchPreference::Casual] {
            let q: Vec<_> = (1..=10).map(|i| w(i, queue, now)).collect();
            assert_eq!(picked(&q, now, queue).len(), 10);
        }
        assert_eq!(
            picked(
                &[w(1, MatchPreference::BotPractice, now)],
                now,
                MatchPreference::BotPractice
            ),
            vec![0]
        );
    }

    /// LR-08 acceptance: one newcomer and one veteran in Casual get one
    /// allocation with both after 30 s; their cohorts no longer split them
    /// into two bot games.
    #[test]
    fn casual_groups_a_newcomer_and_a_veteran_before_bots_fill() {
        let now = Instant::now();
        let mut veteran = w(2, MatchPreference::Casual, now + Duration::from_secs(5));
        veteran.profile.rating = 1400;
        veteran.profile.rated_matches = 40;
        let q = vec![w(1, MatchPreference::Casual, now), veteran];
        assert!(q[0].profile.newcomer() && !q[1].profile.newcomer());
        assert!(select(&q, now + Duration::from_secs(29)).is_none());
        let both = picked(&q, now + CASUAL_BOT_FILL, MatchPreference::Casual);
        assert_eq!(both, vec![0, 1]);
        let chosen: Vec<_> = both.iter().map(|&i| q[i].clone()).collect();
        let manifest = manifest_for(&chosen, MatchPreference::Casual);
        assert_eq!(manifest.humans.len(), 2);
        assert_eq!(manifest.validate(), Ok(()));
    }

    /// LR-08 acceptance: a party {1000 newcomer, 1400 veteran} with 8 solo
    /// players never yields a rated roster the store would reject, whichever
    /// cohort the solos are in. Ranked tells the party why; Casual seats all
    /// ten unrated.
    #[test]
    fn a_mixed_party_never_yields_a_rated_roster_the_store_rejects() {
        let now = Instant::now();
        for solos_rating in [1000, 1400] {
            for solos_veterans in [false, true] {
                let party = |id, rating, rated_matches| {
                    let mut entry = in_party(w(id, MatchPreference::Ranked, now), 9, 2);
                    entry.profile.rating = rating;
                    entry.profile.rated_matches = rated_matches;
                    entry
                };
                let mut q = vec![party(1, 1000, 0), party(2, 1400, 40)];
                for id in 3..=10 {
                    let mut solo = w(id, MatchPreference::Ranked, now);
                    solo.profile.rating = solos_rating;
                    solo.profile.rated_matches = if solos_veterans { 40 } else { 0 };
                    q.push(solo);
                }
                let later = now + Duration::from_secs(99_999);
                assert!(
                    select(&q, later).is_none(),
                    "{solos_rating} {solos_veterans}"
                );
                assert_eq!(
                    waiting_view(&q, 0, false, now),
                    MatchServiceView::Failed {
                        code: RANKED_PARTY_COHORT.into()
                    }
                );
                // Every Ranked roster the lobby can still form passes the
                // store's own check.
                for id in 11..=12 {
                    let mut solo = w(id, MatchPreference::Ranked, now);
                    solo.profile.rating = solos_rating;
                    solo.profile.rated_matches = if solos_veterans { 40 } else { 0 };
                    q.push(solo);
                }
                let (members, queue) = select(&q, later).expect("ten solos form Ranked");
                assert_eq!(queue, MatchPreference::Ranked);
                assert!(!members.contains(&0) && !members.contains(&1));
                assert!(same_cohort(members.iter().map(|&i| &q[i].profile)));

                // The same ten in Casual PvP: one unrated roster.
                q.truncate(10);
                for entry in &mut q {
                    entry.preference = MatchPreference::Casual;
                    entry.casual_since = Some(now);
                }
                let all = picked(&q, now, MatchPreference::Casual);
                assert_eq!(all.len(), 10);
                let chosen: Vec<_> = all.iter().map(|&i| q[i].clone()).collect();
                let teams = split_teams(&chosen);
                assert_eq!(teams[0], teams[1], "the party plays together");
                assert!(
                    !manifest_for(&chosen, MatchPreference::Casual)
                        .preference
                        .rated()
                );
            }
        }
    }

    /// O-1: after five minutes a Ranked search may also search Casual PvP.
    /// It keeps its place in Ranked: a full Ranked roster still takes it,
    /// and otherwise Casual bot-fills 30 s after the offer was taken.
    #[test]
    fn ranked_offers_casual_after_five_minutes_and_keeps_its_place() {
        let now = Instant::now();
        let mut service = lobby_with(crate::match_pool::Pool::for_test(), &[]);
        let directory = match &service {
            MatchService::Lobby(lobby) => lobby.pool.test_directory().to_path_buf(),
            _ => unreachable!(),
        };
        let ranked = w(1, MatchPreference::Ranked, now);
        let (profile, session) = (&ranked.profile.profile_id, &ranked.session);
        let find = |service: &mut MatchService, also_casual, at| {
            service.enqueue(
                ranked.profile.clone(),
                session.clone(),
                1,
                Search {
                    preference: MatchPreference::Ranked,
                    also_casual,
                },
                None,
                home(),
                at,
            );
        };
        let view = |service: &MatchService, at| match service.view(profile, session, at) {
            Some(MatchServiceView::Waiting {
                searching,
                casual_offer_after_secs,
                also_casual,
                bot_fill_after_secs,
                ..
            }) => (
                searching,
                casual_offer_after_secs,
                also_casual,
                bot_fill_after_secs,
            ),
            other => panic!("expected a wait, got {other:?}"),
        };
        find(&mut service, false, now);
        assert_eq!(view(&service, now), (Some(1), Some(300), false, None));
        // Too early: the lobby ignores the request.
        let early = now + Duration::from_secs(299);
        find(&mut service, true, early);
        assert!(!view(&service, early).2);
        let offered = now + RANKED_CASUAL_OFFER;
        find(&mut service, true, offered);
        assert!(view(&service, offered).2);
        let MatchService::Lobby(lobby) = &mut service else {
            unreachable!()
        };
        assert!(select(&lobby.waiting, offered + Duration::from_secs(29)).is_none());
        // A full Ranked roster goes first and keeps this player's place.
        let mut q = lobby.waiting.clone();
        q.extend((2..=10).map(|i| w(i, MatchPreference::Ranked, offered)));
        let ranked_roster = picked(&q, offered + CASUAL_BOT_FILL, MatchPreference::Ranked);
        assert!(ranked_roster.contains(&0));
        // A Casual player meets it in Casual PvP.
        lobby.waiting.push(w(11, MatchPreference::Casual, offered));
        assert_eq!(
            picked(
                &lobby.waiting,
                offered + CASUAL_BOT_FILL,
                MatchPreference::Casual
            ),
            vec![0, 1]
        );
        // Casual counts it among its searchers; withdrawing ends that.
        let casual_count = |lobby: &Lobby| match waiting_view(&lobby.waiting, 1, false, offered) {
            MatchServiceView::Waiting { searching, .. } => searching,
            other => panic!("{other:?}"),
        };
        assert_eq!(casual_count(lobby), Some(2));
        find(&mut service, false, offered);
        let MatchService::Lobby(lobby) = &service else {
            unreachable!()
        };
        assert_eq!(casual_count(lobby), Some(1));
        let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
    }

    /// A Ranked party searches Casual only once every member took the offer.
    #[test]
    fn a_ranked_party_also_searches_casual_only_together() {
        let now = Instant::now();
        let mut q = vec![
            in_party(w(1, MatchPreference::Ranked, now), 4, 2),
            in_party(w(2, MatchPreference::Ranked, now), 4, 2),
        ];
        let offered = now + RANKED_CASUAL_OFFER;
        q[0].also_casual(true, offered);
        let late = offered + Duration::from_secs(999);
        assert!(select(&q, late).is_none());
        let also = |q: &[Waiting]| match waiting_view(q, 0, false, late) {
            MatchServiceView::Waiting { also_casual, .. } => also_casual,
            other => panic!("{other:?}"),
        };
        assert!(!also(&q), "one member alone does not move the party");
        q[1].also_casual(true, offered);
        assert!(also(&q));
        assert_eq!(picked(&q, late, MatchPreference::Casual), vec![0, 1]);
    }

    /// LR-08: the lobby reports `rated` from the queue, not the head count.
    #[test]
    fn ten_humans_are_rated_only_in_ranked() {
        let now = Instant::now();
        for queue in [MatchPreference::Casual, MatchPreference::Ranked] {
            let pool = crate::match_pool::Pool::for_test();
            let directory = pool.test_directory().to_path_buf();
            let mut service = lobby_with(pool, &[]);
            let entries: Vec<_> = (1..=10).map(|i| w(i, queue, now)).collect();
            for entry in &entries {
                service.enqueue(
                    entry.profile.clone(),
                    entry.session.clone(),
                    1,
                    queue.into(),
                    None,
                    home(),
                    now,
                );
            }
            service.tick(now);
            let MatchService::Lobby(lobby) = &service else {
                unreachable!()
            };
            assert!(lobby.waiting.is_empty(), "{queue:?}");
            let (id, slot) = lobby.pool.slots.iter().next().unwrap();
            assert_eq!(slot.manifest.preference, queue);
            assert_eq!(slot.manifest.humans.len(), 10);
            let receipt = Status {
                allocation_id: id.clone(),
                server_epoch: 7,
                heartbeat_ms: unix_ms(),
                phase: Phase::Ready,
                result_id: None,
                ends_by_ms: None,
            };
            lobby.pool.write_status_for_test(id, &receipt);
            let Some(MatchServiceView::Assigned { allocation, .. }) =
                service.view(&entries[0].profile.profile_id, &entries[0].session, now)
            else {
                panic!("expected a handoff");
            };
            assert_eq!(allocation.preference, queue);
            assert_eq!((allocation.human_count, allocation.bot_count), (10, 0));
            assert_eq!(allocation.rated, queue == MatchPreference::Ranked);
            let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
        }
    }
    fn in_party(mut entry: Waiting, party: u64, size: usize) -> Waiting {
        entry.party = Some((party, size));
        entry
    }

    #[test]
    fn a_party_practices_together_once_every_member_queued() {
        let now = Instant::now();
        let mut q = vec![in_party(w(1, MatchPreference::BotPractice, now), 7, 2)];
        assert!(select(&q, now).is_none(), "the friend has not queued yet");
        q.push(w(2, MatchPreference::BotPractice, now));
        // A stranger's practice is its own allocation.
        assert_eq!(picked(&q, now, MatchPreference::BotPractice), vec![1]);
        q.remove(1);
        q.push(in_party(w(3, MatchPreference::BotPractice, now), 7, 2));
        let together = picked(&q, now, MatchPreference::BotPractice);
        assert_eq!(together, vec![0, 1]);
        let chosen: Vec<_> = together.iter().map(|&i| q[i].clone()).collect();
        let teams = split_teams(&chosen);
        assert_eq!(teams, vec![shared::map::Team::Green; 2]);
    }

    fn manifest_for(chosen: &[Waiting], queue: MatchPreference) -> Manifest {
        let teams = split_teams(chosen);
        Manifest {
            version: 1,
            allocation_id: "a".repeat(32),
            endpoint: "127.0.0.1:41000".into(),
            bind: "127.0.0.1:41000".into(),
            preference: queue,
            humans: chosen
                .iter()
                .zip(teams)
                .map(|(w, team)| AllocatedHuman {
                    profile_id: w.profile.profile_id.clone(),
                    session_id: w.session.clone(),
                    team,
                })
                .collect(),
            join_deadline_ms: 0,
        }
    }

    #[test]
    fn compatibility_dispatch_works_in_every_role_without_allocating_a_player() {
        use crate::runtime::ports::{ManualClock, MemoryTransport};
        use shared::compatibility::{CompatibilityProbe, ReleaseContract};
        let now = Instant::now();
        let pool = crate::match_pool::Pool::for_test();
        let directory = pool.test_directory().to_path_buf();
        let worker = Worker {
            manifest: manifest_for(
                &[w(1, MatchPreference::BotPractice, now)],
                MatchPreference::BotPractice,
            ),
            directory: directory.clone(),
            recovery: false,
            origin_epoch: 0,
            aborted: false,
            booted_at: now,
            last_status: None,
            terminal_at: None,
        };
        let lobby = Lobby::new(pool, Limits::default());
        for role in [
            MatchService::Standalone,
            MatchService::Lobby(lobby),
            MatchService::Worker(worker),
        ] {
            let transport = MemoryTransport::new("127.0.0.1:4000".parse().unwrap());
            let mut runtime = ServerRuntime::for_test(
                transport.clone(),
                ManualClock::new(now),
                crate::career_backend::MemoryCareer::disabled(7),
                crate::match_rules::MatchConfig::dev(),
            );
            runtime.match_service = role;
            let mut client = ReleaseContract::current();
            client.protocol += 1; // Must reply even when gameplay packets would be rejected.
            let probe = CompatibilityProbe::new(client, "ab".repeat(16));
            transport.push_inbound("127.0.0.1:60100".parse().unwrap(), probe.request());
            runtime.receive_packets();
            let sent = transport.take_outbound();
            assert_eq!(sent.len(), 1);
            assert_eq!(probe.accept(&sent[0].1), Some(ReleaseContract::current()));
            assert!(runtime.world.players.is_empty());
        }
        let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
    }

    #[test]
    fn fresh_queue_request_cannot_reuse_settling_or_terminal_allocation() {
        let now = Instant::now();
        let entry = w(1, MatchPreference::BotPractice, now);
        let mut pool = crate::match_pool::Pool::for_test();
        let id = pool
            .allocate(manifest_for(std::slice::from_ref(&entry), entry.preference))
            .unwrap();
        let mut receipt = Status {
            allocation_id: id.clone(),
            server_epoch: 7,
            heartbeat_ms: unix_ms(),
            phase: Phase::Running,
            result_id: Some("finished-round".into()),
            ends_by_ms: None,
        };
        pool.slots.get_mut(&id).unwrap().status = Some(receipt.clone());
        // Worker writes between lobby polls. Its cache deliberately stays Running.
        receipt.phase = Phase::Settling;
        pool.write_status_for_test(&id, &receipt);
        let mut service = MatchService::Lobby(Lobby {
            assignments: HashMap::from([(entry.profile.profile_id.clone(), id.clone())]),
            order: 1,
            last_poll: Some(now),
            ..Lobby::new(pool, Limits::default())
        });
        assert!(
            matches!(
                service.view(&entry.profile.profile_id, &entry.session, now),
                Some(MatchServiceView::Allocating)
            ),
            "settling workers must never accept a fresh handoff"
        );
        if let MatchService::Lobby(lobby) = &mut service {
            assert_eq!(
                lobby.pool.slots[&id].status.as_ref().unwrap().phase,
                Phase::Running
            );
            receipt.phase = Phase::Failed;
            lobby.pool.write_status_for_test(&id, &receipt);
        }
        // No tick/pruning between settlement and the user's next FindMatch.
        service.enqueue(
            entry.profile.clone(),
            entry.session.clone(),
            2,
            entry.preference.into(),
            None,
            home(),
            now,
        );
        assert_eq!(
            service.request_id(&entry.profile.profile_id, &entry.session),
            Some(2)
        );
        assert!(matches!(
            service.view(&entry.profile.profile_id, &entry.session, now),
            Some(MatchServiceView::Waiting { .. })
        ));
        if let MatchService::Lobby(lobby) = service {
            assert!(!lobby.assignments.contains_key(&entry.profile.profile_id));
            assert_eq!(lobby.waiting.len(), 1);
        }
    }

    fn lobby_with(pool: crate::match_pool::Pool, assignments: &[(&Waiting, &str)]) -> MatchService {
        MatchService::Lobby(Lobby {
            assignments: assignments
                .iter()
                .map(|(w, id)| (w.profile.profile_id.clone(), (*id).to_owned()))
                .collect(),
            order: 1,
            ..Lobby::new(pool, Limits::default())
        })
    }

    /// LR-13: Play after leaving a running match shows the in-match state
    /// with its latest end at once (no lobby tick needed) and queues nobody.
    /// Once the worker ends the match (LR-11 reaping), the seat frees without
    /// an error and the next FindMatch queues.
    #[test]
    fn a_player_who_left_a_running_match_sees_it_and_queues_once_it_ends() {
        let now = Instant::now();
        let entry = w(1, MatchPreference::BotPractice, now);
        let (profile, session) = (&entry.profile.profile_id, &entry.session);
        let mut pool = crate::match_pool::Pool::for_test();
        let directory = pool.test_directory().to_path_buf();
        let id = pool
            .allocate(manifest_for(std::slice::from_ref(&entry), entry.preference))
            .unwrap();
        let mut receipt = Status {
            allocation_id: id.clone(),
            server_epoch: 7,
            heartbeat_ms: unix_ms(),
            phase: Phase::Running,
            result_id: Some("r".repeat(32)),
            ends_by_ms: Some(unix_ms() + 200_000),
        };
        pool.write_status_for_test(&id, &receipt);
        // A live worker (the stand-in child exits at once and would be
        // relaunched in recovery).
        pool.adopt_for_test(&id);
        let mut service = lobby_with(pool, &[(&entry, &id)]);
        let find_match = |service: &mut MatchService, request_id| {
            service.enqueue(
                entry.profile.clone(),
                session.clone(),
                request_id,
                entry.preference.into(),
                None,
                home(),
                now,
            );
        };
        let in_match = |service: &MatchService| match service.view(profile, session, now) {
            Some(MatchServiceView::Assigned {
                allocation,
                in_match: Some(state),
            }) => {
                assert_eq!(allocation.allocation_id, id);
                assert_eq!(allocation.endpoint, "127.0.0.1:45000");
                state
            }
            other => panic!("expected the in-match state, got {other:?}"),
        };
        find_match(&mut service, 2);
        assert_eq!(service.request_id(profile, session), Some(2));
        let secs = in_match(&service).ends_within_secs.unwrap();
        assert!((195..=200).contains(&secs), "{secs}");
        // Retries and lobby ticks keep the seat and add no queue entry.
        service.tick(now + Duration::from_secs(1));
        find_match(&mut service, 2);
        in_match(&service);
        let MatchService::Lobby(lobby) = &service else {
            unreachable!()
        };
        assert!(lobby.waiting.is_empty());
        assert!(lobby.errors.is_empty());

        // The worker reaped the unattended match and retired.
        receipt.phase = Phase::Failed;
        receipt.ends_by_ms = None;
        receipt.heartbeat_ms = unix_ms();
        if let MatchService::Lobby(lobby) = &service {
            lobby.pool.write_status_for_test(&id, &receipt);
        }
        service.tick(now + Duration::from_secs(2));
        assert_eq!(
            service.view(profile, session, now),
            Some(MatchServiceView::Idle),
            "a played match that ended is not a failed allocation"
        );
        find_match(&mut service, 2);
        assert!(matches!(
            service.view(profile, session, now),
            Some(MatchServiceView::Waiting { .. })
        ));
        let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
    }

    /// The handoff itself is unchanged: a worker that has not started its
    /// match is handed out without the in-match state, and an allocation
    /// that fails before it ran still reports why.
    #[test]
    fn an_unstarted_worker_is_a_plain_handoff_and_its_failure_is_reported() {
        let now = Instant::now();
        let entry = w(1, MatchPreference::BotPractice, now);
        let (profile, session) = (&entry.profile.profile_id, &entry.session);
        let mut pool = crate::match_pool::Pool::for_test();
        let directory = pool.test_directory().to_path_buf();
        let id = pool
            .allocate(manifest_for(std::slice::from_ref(&entry), entry.preference))
            .unwrap();
        let mut receipt = Status {
            allocation_id: id.clone(),
            server_epoch: 7,
            heartbeat_ms: unix_ms(),
            phase: Phase::Forming,
            result_id: None,
            ends_by_ms: None,
        };
        pool.write_status_for_test(&id, &receipt);
        let mut service = lobby_with(pool, &[(&entry, &id)]);
        service.enqueue(
            entry.profile.clone(),
            session.clone(),
            2,
            entry.preference.into(),
            None,
            home(),
            now,
        );
        assert!(matches!(
            service.view(profile, session, now),
            Some(MatchServiceView::Assigned { in_match: None, .. })
        ));
        receipt.phase = Phase::Failed;
        if let MatchService::Lobby(lobby) = &service {
            lobby.pool.write_status_for_test(&id, &receipt);
        }
        service.tick(now + Duration::from_secs(1));
        assert_eq!(
            service.view(profile, session, now),
            Some(MatchServiceView::Failed {
                code: "allocation_cancelled_or_expired".into()
            })
        );
        let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
    }

    #[test]
    fn a_party_bot_practice_manifest_is_accepted_by_the_worker_contract() {
        let now = Instant::now();
        let q = vec![
            in_party(w(1, MatchPreference::BotPractice, now), 7, 2),
            in_party(w(2, MatchPreference::BotPractice, now), 7, 2),
        ];
        let chosen: Vec<_> = picked(&q, now, MatchPreference::BotPractice)
            .iter()
            .map(|&i| q[i].clone())
            .collect();
        let manifest = manifest_for(&chosen, MatchPreference::BotPractice);
        assert_eq!(manifest.validate(), Ok(()));
        // A solo practice still validates; a split bot practice does not.
        assert_eq!(
            manifest_for(&chosen[..1], MatchPreference::BotPractice).validate(),
            Ok(())
        );
        let mut split = manifest;
        split.humans[1].team = shared::map::Team::Blue;
        assert!(split.validate().is_err());
    }

    #[test]
    fn casual_matches_keep_every_party_on_one_team() {
        let now = Instant::now();
        let mut q: Vec<_> = (1..=10)
            .map(|i| w(i, MatchPreference::Casual, now))
            .collect();
        for i in [0, 1, 2] {
            q[i] = in_party(q[i].clone(), 1, 3);
        }
        for i in [5, 6] {
            q[i] = in_party(q[i].clone(), 2, 2);
        }
        let ten = picked(&q, now, MatchPreference::Casual);
        assert_eq!(ten.len(), 10);
        let chosen: Vec<_> = ten.iter().map(|&i| q[i].clone()).collect();
        let teams = split_teams(&chosen);
        let team_of = |order: u64| teams[chosen.iter().position(|c| c.order == order).unwrap()];
        assert!([2, 3].iter().all(|&o| team_of(o) == team_of(1)));
        assert_eq!(team_of(6), team_of(7));
        let green = teams
            .iter()
            .filter(|t| **t == shared::map::Team::Green)
            .count();
        assert_eq!(green, 5);
    }

    #[test]
    fn parties_that_cannot_be_seated_wait_for_a_better_mix() {
        assert!(splits(&[5, 5]));
        assert!(splits(&[3, 2, 3, 2]));
        assert!(!splits(&[4, 3, 3]));
        assert!(!splits(&[4, 2, 2, 2]));
        // A lone party in a bot-filled Casual match still plays together.
        let now = Instant::now();
        let q = vec![
            in_party(w(1, MatchPreference::Casual, now), 3, 2),
            in_party(w(2, MatchPreference::Casual, now), 3, 2),
        ];
        let pair = picked(&q, now + CASUAL_BOT_FILL, MatchPreference::Casual);
        let chosen: Vec<_> = pair.iter().map(|&i| q[i].clone()).collect();
        let teams = split_teams(&chosen);
        assert_eq!(teams[0], teams[1]);
    }

    #[test]
    fn ranked_rating_windows_and_queues_stay_separate() {
        let now = Instant::now();
        let mut q: Vec<_> = (1..=10)
            .map(|i| w(i, MatchPreference::Ranked, now))
            .collect();
        q[9].profile.rating = 1400;
        assert!(select(&q, now).is_none());
        q[9].profile.rating = 1000;
        q[9].preference = MatchPreference::Casual;
        q[9].casual_since = Some(now);
        assert!(select(&q, now).is_none());
    }

    /// A practice lobby with room for several arenas, and the receipt a
    /// worker writes: `phase` with a fresh heartbeat.
    fn practice_lobby() -> (MatchService, std::path::PathBuf) {
        let mut pool = crate::match_pool::Pool::for_test();
        pool.set_capacity_for_test(8);
        let directory = pool.test_directory().to_path_buf();
        (lobby_with(pool, &[]), directory)
    }
    fn report(service: &mut MatchService, profile: &str, phase: Phase) {
        let MatchService::Lobby(lobby) = service else {
            unreachable!()
        };
        let id = lobby.assignments[profile].clone();
        lobby.pool.adopt_for_test(&id);
        lobby.pool.write_status_for_test(
            &id,
            &Status {
                allocation_id: id.clone(),
                server_epoch: 7,
                heartbeat_ms: unix_ms(),
                phase,
                result_id: None,
                ends_by_ms: None,
            },
        );
    }
    fn practice(service: &mut MatchService, entry: &Waiting, source: SourceIp, at: Instant) {
        service.enqueue(
            entry.profile.clone(),
            entry.session.clone(),
            1,
            MatchPreference::BotPractice.into(),
            None,
            source,
            at,
        );
    }
    fn failed(service: &MatchService, entry: &Waiting, at: Instant) -> Option<String> {
        match service.view(&entry.profile.profile_id, &entry.session, at) {
            Some(MatchServiceView::Failed { code }) => Some(code),
            _ => None,
        }
    }
    fn neighbour() -> SourceIp {
        SourceIp::of(std::net::SocketAddr::from(([198, 51, 100, 2], 40000)))
    }

    /// LR-16: a second vs Bots allocation from an IP whose first one is
    /// Forming or Running is refused with a code the client shows in its own
    /// words; another IP is not affected, and the IP plays again once its
    /// match ended.
    #[test]
    fn a_second_bot_practice_from_one_ip_is_refused_while_the_first_holds_an_arena() {
        let now = Instant::now();
        let (mut service, directory) = practice_lobby();
        let (first, second, other) = (
            w(1, MatchPreference::BotPractice, now),
            w(2, MatchPreference::BotPractice, now),
            w(3, MatchPreference::BotPractice, now),
        );
        practice(&mut service, &first, home(), now);
        service.tick(now);
        for (phase, at) in [(Phase::Forming, 1), (Phase::Running, 2)] {
            let at = now + Duration::from_secs(at);
            report(&mut service, &first.profile.profile_id, phase);
            service.tick(at);
            practice(&mut service, &second, home(), at);
            assert_eq!(failed(&service, &second, at).as_deref(), Some(ARENA_LIMIT));
        }
        // Another IP gets its own arena.
        let at = now + Duration::from_secs(2);
        practice(&mut service, &other, neighbour(), at);
        service.tick(now + Duration::from_secs(3));
        assert!(matches!(
            service.view(&other.profile.profile_id, &other.session, at),
            Some(MatchServiceView::Allocating | MatchServiceView::Assigned { .. })
        ));
        // A party or a lone Casual player from that IP is held to the same
        // rule when bots would fill the match: it waits, then is refused.
        let casual = w(4, MatchPreference::Casual, now);
        let at = now + Duration::from_secs(4);
        service.enqueue(
            casual.profile.clone(),
            casual.session.clone(),
            1,
            MatchPreference::Casual.into(),
            None,
            home(),
            at,
        );
        let fill = at + CASUAL_BOT_FILL + Duration::from_secs(1);
        service.enqueue(
            casual.profile.clone(),
            casual.session.clone(),
            1,
            MatchPreference::Casual.into(),
            None,
            home(),
            fill,
        );
        service.tick(fill);
        assert_eq!(
            failed(&service, &casual, fill).as_deref(),
            Some(ARENA_LIMIT)
        );
        // The first match ended: the IP may practice again.
        report(&mut service, &first.profile.profile_id, Phase::Finished);
        let later = fill + Duration::from_secs(1);
        service.tick(later);
        practice(&mut service, &second, home(), later);
        assert_eq!(failed(&service, &second, later), None);
        let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
    }

    /// LR-16: an IP that left `practice_abandons` vs Bots allocations
    /// unplayed within ten minutes cools down; a played match never counts.
    #[test]
    fn repeated_abandoned_bot_practice_triggers_a_short_cooldown() {
        let limits = Limits::default();
        let start = Instant::now();
        let (mut service, directory) = practice_lobby();
        let mut at = start;
        for id in 1..=limits.practice_abandons as u64 {
            let entry = w(id, MatchPreference::BotPractice, start);
            practice(&mut service, &entry, home(), at);
            at += Duration::from_secs(1);
            service.tick(at);
            assert_eq!(failed(&service, &entry, at), None, "{id}");
            // Allocated, never joined: the worker gives up.
            report(&mut service, &entry.profile.profile_id, Phase::Failed);
            at += Duration::from_secs(1);
            service.tick(at);
        }
        let next = w(100, MatchPreference::BotPractice, start);
        practice(&mut service, &next, home(), at);
        assert_eq!(
            failed(&service, &next, at).as_deref(),
            Some(PRACTICE_COOLDOWN)
        );
        // Only that IP cools down, and only for a while.
        let other = w(101, MatchPreference::BotPractice, start);
        practice(&mut service, &other, neighbour(), at);
        assert_eq!(failed(&service, &other, at), None);
        let after = at + limits.practice_cooldown;
        service.tick(after);
        practice(&mut service, &next, home(), after);
        assert_eq!(failed(&service, &next, after), None);
        let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
    }
}
