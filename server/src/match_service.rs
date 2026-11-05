//! One public lobby schedules independent, immutable game workers.
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use omoba_career_store::matchmaking::{CohortAllowance, fits_cohort_with, same_cohort_with};
use shared::career::ProfileSummary;
use shared::match_service::{
    CapacityLine, InMatch, MatchAllocation, MatchPreference, MatchServiceView,
};
use shared::wire::{GameState, ServerPacket};

use crate::admission::{Limits, PRACTICE_ABANDON_WINDOW, SourceIp};
use crate::match_allocation::{AllocatedHuman, Manifest, Phase, Status, Worker, unix_ms};
use crate::runtime::ServerRuntime;

/// Queued entries the lobby holds, `OMOBA_LOBBY_MAX_WAITING` (1..=4096).
/// Players beyond it are told `queue_full` and their client keeps retrying.
const DEFAULT_MAX_WAITING: usize = 1024;
const MAX_WAITING_CAP: usize = 4096;
/// Profiles whose recent `FindMatch` the lobby remembers (it answers their
/// views); one beyond it is told `lobby_busy` instead of being ignored.
const MAX_REQUESTS: usize = 4096;
/// How long a remembered request outlives its last packet when the profile is
/// neither waiting nor assigned.
const REQUEST_RETENTION: Duration = Duration::from_secs(30);
/// Initial persisted dodge cooldown; repeat dodges within 24 hours escalate.
/// Also applies to missed or declined match confirmations in every queue.
const DODGE_COOLDOWN: Duration = Duration::from_secs(60);
const READY_WINDOW: Duration = Duration::from_secs(15);
/// A roster whose worker failed to spawn is refused for this long, so a broken
/// deployment does not re-select the same ten players every pass.
const SPAWN_FAILURE_COOLDOWN: Duration = Duration::from_secs(10);
/// A human re-queued after their allocation ended keeps the place this long
/// before their client's next `FindMatch` must renew it: the client is still
/// returning from the dead worker to the lobby.
const REQUEUE_GRACE: Duration = Duration::from_secs(30);
/// The searching screen tells a party for this long that a member left the
/// search; the party keeps searching without that member.
const PARTY_LEFT_NOTICE: Duration = Duration::from_secs(8);
/// A member who left a party's search is left out of its head count for this
/// long, unless they queue again.
const PARTY_OPT_OUT_TTL: Duration = Duration::from_secs(600);
/// Casual PvP waits this long for more humans, then bots fill the empty seats
/// (owner default O-2).
const PARTY_GATHER_WINDOW: Duration = Duration::from_secs(60);
const CASUAL_BOT_FILL: Duration = Duration::from_secs(30);
/// A spawned worker that no allocated human has joined within this long
/// aborts: the handoff normally completes within seconds (LR-32).
const NO_SHOW_ABORT: Duration = Duration::from_secs(60);
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
/// The member who cancelled a found Ranked match (LR-32).
const DODGE_COOLDOWN_CODE: &str = "dodge_cooldown";
/// A party member left the search; the rest keep searching without them.
const PARTY_MEMBER_LEFT: &str = "party_member_left";
/// The lobby remembers too many recent requests to take another profile.
const LOBBY_BUSY: &str = "lobby_busy";
/// The worker binary could not be started (LR-17: an operator problem).
const WORKER_SPAWN_FAILED: &str = "worker_spawn_failed";

/// `OMOBA_LOBBY_MAX_WAITING`, or the default when unset or invalid.
fn max_waiting_from_env() -> usize {
    std::env::var("OMOBA_LOBBY_MAX_WAITING")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| (1..=MAX_WAITING_CAP).contains(n))
        .unwrap_or(DEFAULT_MAX_WAITING)
}

/// What `select` found: the roster, its queue and, for Ranked, the widened
/// cohort it was admitted with (LR-35).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Selection {
    members: Vec<usize>,
    queue: MatchPreference,
    cohort: Option<CohortAllowance>,
}

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
struct ReadyRoster {
    entries: Vec<Waiting>,
    queue: MatchPreference,
    cohort: Option<CohortAllowance>,
    accepted: HashSet<String>,
    deadline: Instant,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct DodgePenalty {
    strikes: u32,
    until_ms: u64,
    last_ms: u64,
}

pub(crate) struct Lobby {
    pool: crate::match_pool::Pool,
    waiting: Vec<Waiting>,
    ready: HashMap<String, ReadyRoster>,
    ready_profiles: HashMap<String, String>,
    penalties: HashMap<String, DodgePenalty>,
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
    /// While every arena is busy: each queued profile whose roster is ready,
    /// with its place in the line for the next free arena (LR-09).
    line: HashMap<String, u32>,
    turnover: crate::capacity::Turnover,
    capacity: crate::capacity::CapacityMetric,
    max_waiting: usize,
    max_requests: usize,
    /// The queue entries of each live allocation this lobby made, as they
    /// were seated. An allocation that ends before its match starts puts them
    /// back in the queue with their place (LR-32).
    parked: HashMap<String, Vec<Waiting>>,
    /// Profiles the lobby refuses to queue until a time, with the code it
    /// answers: a dodge cooldown, a worker spawn failure.
    refusals: HashMap<String, (&'static str, Instant)>,
    /// Rejected searches still carry their session and request id so the
    /// client can correlate the `lobby_busy` answer with its current search.
    busy: HashMap<String, (String, u64, Instant)>,
    /// Party members who cancelled their party's search, and when: the party
    /// searches without them, and the searching screen says so for a moment.
    party_opt_outs: HashMap<u64, HashMap<String, Instant>>,
    /// Members shown "a member left" until a time.
    notices: HashMap<String, Instant>,
    party_rosters: HashMap<u64, Vec<(String, String)>>,
}

impl Lobby {
    fn new(pool: crate::match_pool::Pool, limits: Limits) -> Self {
        let penalties: HashMap<String, DodgePenalty> =
            std::fs::read(pool.root().join("dodge-penalties.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<HashMap<String, DodgePenalty>>(&b).ok())
                .unwrap_or_default()
                .into_iter()
                .filter(|(_, p)| unix_ms().saturating_sub(p.last_ms) < 86_400_000)
                .collect();
        let now = Instant::now();
        let wall = unix_ms();
        let refusals = penalties
            .iter()
            .filter(|(_, p)| p.until_ms > wall)
            .map(|(profile, p)| {
                (
                    profile.clone(),
                    (
                        DODGE_COOLDOWN_CODE,
                        now + Duration::from_millis(p.until_ms - wall),
                    ),
                )
            })
            .collect();
        Self {
            pool,
            ready: HashMap::new(),
            ready_profiles: HashMap::new(),
            penalties,
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
            line: HashMap::new(),
            turnover: Default::default(),
            capacity: Default::default(),
            max_waiting: max_waiting_from_env(),
            max_requests: MAX_REQUESTS,
            parked: HashMap::new(),
            refusals,
            busy: HashMap::new(),
            party_opt_outs: HashMap::new(),
            notices: HashMap::new(),
            party_rosters: HashMap::new(),
        }
    }

    fn allocate_ready(
        &mut self,
        id: String,
        selected: Vec<Waiting>,
        queue: MatchPreference,
        cohort: Option<CohortAllowance>,
        now: Instant,
    ) {
        let teams = split_teams(&selected, now);
        let humans = selected
            .iter()
            .zip(teams)
            .map(|(w, team)| AllocatedHuman {
                profile_id: w.profile.profile_id.clone(),
                session_id: w.session.clone(),
                team,
            })
            .collect();
        let manifest = Manifest {
            version: 1,
            allocation_id: id,
            endpoint: String::new(),
            bind: String::new(),
            preference: queue,
            humans,
            join_deadline_ms: unix_ms() + 60_000,
            cohort: cohort.map(|c| (c.spread, c.merge_newcomers)),
        };
        match self.pool.allocate(manifest) {
            Ok(id) => {
                let mut ips: Vec<SourceIp> = selected
                    .iter()
                    .filter_map(|w| self.sources.get(&w.profile.profile_id).copied())
                    .collect();
                ips.sort();
                ips.dedup();
                self.arena_sources.insert(id.clone(), ips);
                for w in &selected {
                    self.assignments
                        .insert(w.profile.profile_id.clone(), id.clone());
                }
                let ids: HashSet<_> = selected.iter().map(|w| &w.profile.profile_id).collect();
                self.waiting
                    .retain(|w| !ids.contains(&w.profile.profile_id));
                self.parked.insert(id, selected);
            }
            Err(error) => {
                // The roster leaves the queue with the error; a spawn
                // failure also holds it back for a moment, so a broken
                // worker binary is not retried every pass (LR-17).
                let ids: HashSet<_> = selected.iter().map(|w| &w.profile.profile_id).collect();
                self.waiting
                    .retain(|w| !ids.contains(&w.profile.profile_id));
                for w in &selected {
                    self.errors
                        .insert(w.profile.profile_id.clone(), error.clone());
                    if error == WORKER_SPAWN_FAILED {
                        self.refusals.insert(
                            w.profile.profile_id.clone(),
                            (WORKER_SPAWN_FAILED, now + SPAWN_FAILURE_COOLDOWN),
                        );
                    }
                }
            }
        }
    }
    fn penalize_dodge(&mut self, profile: &str, now: Instant) {
        let wall = unix_ms();
        self.penalties
            .retain(|_, p| wall.saturating_sub(p.last_ms) < 86_400_000);
        let strikes = self
            .penalties
            .get(profile)
            .map_or(1, |p| p.strikes.saturating_add(1));
        let secs = match strikes {
            1 => DODGE_COOLDOWN.as_secs(),
            2 => 300,
            3 => 900,
            _ => 3600,
        };
        self.penalties.insert(
            profile.to_owned(),
            DodgePenalty {
                strikes,
                until_ms: wall + secs * 1000,
                last_ms: wall,
            },
        );
        self.refusals.insert(
            profile.to_owned(),
            (DODGE_COOLDOWN_CODE, now + Duration::from_secs(secs)),
        );
        if let Err(error) = crate::match_allocation::atomic_json(
            &self.pool.root().join("dodge-penalties.json"),
            &self.penalties,
        ) {
            eprintln!("Cannot persist matchmaking cooldown: {error}");
        }
    }
    /// A decline returns everybody else; a timeout returns only accepted humans.
    fn cancel_ready(&mut self, id: &str, declined: Option<&str>, now: Instant) {
        let Some(check) = self.ready.remove(id) else {
            return;
        };
        let excluded: HashSet<_> = check
            .entries
            .iter()
            .filter(|w| {
                declined.map_or(!check.accepted.contains(&w.profile.profile_id), |p| {
                    w.profile.profile_id == p
                })
            })
            .map(|w| w.profile.profile_id.clone())
            .collect();
        for entry in &check.entries {
            if excluded.contains(&entry.profile.profile_id) {
                if let Some((party, _)) = entry.party {
                    self.party_opt_outs
                        .entry(party)
                        .or_default()
                        .insert(entry.profile.profile_id.clone(), now);
                }
                self.penalize_dodge(&entry.profile.profile_id, now);
                self.errors
                    .insert(entry.profile.profile_id.clone(), DODGE_COOLDOWN_CODE.into());
            }
        }
        let mut removed_by_party: HashMap<u64, usize> = HashMap::new();
        for entry in &check.entries {
            if excluded.contains(&entry.profile.profile_id)
                && let Some((party, _)) = entry.party
            {
                *removed_by_party.entry(party).or_default() += 1;
            }
        }
        for mut entry in check.entries {
            self.ready_profiles.remove(&entry.profile.profile_id);
            if excluded.contains(&entry.profile.profile_id) {
                continue;
            }
            if let Some((party, size)) = &mut entry.party {
                let gone = removed_by_party.get(party).copied().unwrap_or(0);
                *size = size.saturating_sub(gone).max(1);
            }
            entry.touched = now + REQUEUE_GRACE;
            self.waiting.push(entry);
        }
        self.waiting.sort_by_key(|w| w.order);
    }
    fn tick_ready(&mut self, now: Instant) {
        let expired: Vec<_> = self
            .ready
            .iter()
            .filter(|(_, r)| now >= r.deadline && r.accepted.len() != r.entries.len())
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.cancel_ready(&id, None, now);
        }
        let mut approved: Vec<_> = self
            .ready
            .iter()
            .filter(|(_, r)| r.accepted.len() == r.entries.len())
            .map(|(id, r)| {
                (
                    r.entries.iter().map(|w| w.order).min().unwrap_or(0),
                    id.clone(),
                )
            })
            .collect();
        approved.sort();
        for (_, id) in approved {
            if self.pool.full() {
                break;
            }
            let check = self.ready.remove(&id).unwrap();
            for entry in &check.entries {
                self.ready_profiles.remove(&entry.profile.profile_id);
            }
            self.allocate_ready(id, check.entries, check.queue, check.cohort, now);
        }
    }

    /// Returns the humans of an ended, never-started allocation to the queue
    /// in their original order and with their original queue age, except
    /// `left` (profiles or sessions that chose to leave). Their assignment is
    /// dropped at once, so their next `FindMatch` renews the entry instead of
    /// binding to the dead allocation. `false` when nothing was parked (an
    /// allocation adopted after a lobby restart).
    fn requeue_parked(&mut self, id: &str, left: &[String], now: Instant) -> bool {
        let Some(entries) = self.parked.remove(id) else {
            return false;
        };
        for mut entry in entries {
            // Those who left keep the dead assignment: the normal path tells
            // them it was cancelled; the others are back in the queue.
            if left.contains(&entry.profile.profile_id)
                || left.contains(&entry.session)
                || self
                    .waiting
                    .iter()
                    .any(|w| w.profile.profile_id == entry.profile.profile_id)
            {
                continue;
            }
            if self
                .assignments
                .get(&entry.profile.profile_id)
                .is_some_and(|assigned| assigned == id)
            {
                self.assignments.remove(&entry.profile.profile_id);
            }
            entry.touched = now + REQUEUE_GRACE;
            self.errors.remove(&entry.profile.profile_id);
            self.waiting.push(entry);
        }
        self.waiting.sort_by_key(|w| w.order);
        true
    }

    fn party_gathering(
        &self,
        index: usize,
        now: Instant,
    ) -> Option<shared::match_service::PartyGathering> {
        let entry = &self.waiting[index];
        let (party, expected) = entry.party?;
        let members: Vec<_> = self
            .waiting
            .iter()
            .filter(|w| w.party.is_some_and(|(id, _)| id == party))
            .collect();
        let other_queue = members
            .iter()
            .filter(|w| w.preference != entry.preference)
            .count();
        let present: Vec<_> = members
            .iter()
            .filter(|w| w.preference == entry.preference)
            .collect();
        let needed = expected
            .saturating_sub(other_queue)
            .max(1)
            .saturating_sub(present.len());
        let oldest = present.iter().map(|w| w.queued).min()?;
        let deadline = oldest + PARTY_GATHER_WINDOW;
        if needed == 0 || now >= deadline {
            return None;
        }
        let missing = self
            .party_rosters
            .get(&party)
            .into_iter()
            .flatten()
            .filter(|(id, _)| !members.iter().any(|w| w.profile.profile_id == *id))
            .filter(|(id, _)| {
                !self
                    .party_opt_outs
                    .get(&party)
                    .is_some_and(|out| out.contains_key(id))
            })
            .map(|(_, name)| name.clone())
            .take(needed)
            .collect();
        Some(shared::match_service::PartyGathering {
            missing,
            needed: needed as u32,
            remaining_secs: deadline.saturating_duration_since(now).as_secs().max(1),
        })
    }

    /// Recomputes the line for a free arena (LR-09), empty unless every
    /// arena is busy: the rosters ready to start, in the order they get an
    /// arena (`select`'s: vs Bots, then Ranked, then Casual PvP), each of
    /// their profiles with its 1-based place. A roster the per-source limits
    /// would refuse takes no place.
    fn refresh_line(&mut self, now: Instant) {
        self.line.clear();
        if self.pool.occupied() + self.ready.len() < self.pool.capacity() {
            return;
        }
        let mut remaining: Vec<usize> = (0..self.waiting.len()).collect();
        let mut position = 0_u32;
        loop {
            let rest: Vec<Waiting> = remaining.iter().map(|&i| self.waiting[i].clone()).collect();
            let Some(Selection { members, queue, .. }) = select(&rest, now) else {
                break;
            };
            let members: Vec<usize> = members.iter().map(|&i| remaining[i]).collect();
            if self.refusal(&members, queue, now).is_none() {
                position += 1;
                for &i in &members {
                    self.line
                        .insert(self.waiting[i].profile.profile_id.clone(), position);
                }
            }
            remaining.retain(|i| !members.contains(i));
        }
    }

    /// The capacity-busy metric after a scheduling pass (LR-09): the journal
    /// line and `<match root>/capacity.json`.
    fn report_capacity(&mut self, now: Instant) {
        let update = self.capacity.update(
            crate::capacity::Sample {
                capacity: self.pool.capacity(),
                occupied: self.pool.occupied() + self.ready.len(),
                searching: self.waiting.len(),
                line: &self.line,
            },
            now,
            unix_ms(),
        );
        if let Some(line) = &update.log {
            println!("{line}");
        }
        if update.write {
            let _ = crate::match_allocation::atomic_json(
                &self.pool.root().join(crate::capacity::REPORT_FILE),
                &update.report,
            );
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
                    saw_human: false,
                    released: Vec::new(),
                }))
            }
            Some(_) => Err("Invalid OMOBA_SERVER_ROLE".into()),
        }
    }
    pub(crate) fn set_party_roster(&mut self, party: u64, members: Vec<(String, String)>) {
        if let Self::Lobby(lobby) = self {
            lobby.party_rosters.insert(
                party,
                members
                    .into_iter()
                    .take(shared::party::MAX_PARTY_SIZE)
                    .collect(),
            );
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
        if lobby.requests.len() >= lobby.max_requests
            && !lobby.requests.contains_key(&profile.profile_id)
        {
            // Say so instead of leaving the screen on "connecting" (bounded:
            // the busy set never grows past the request map it guards).
            if lobby.busy.len() < lobby.max_requests || lobby.busy.contains_key(&profile.profile_id)
            {
                lobby.busy.insert(
                    profile.profile_id,
                    (session, request_id, now + Duration::from_secs(10)),
                );
            }
            return;
        }
        lobby.requests.insert(
            profile.profile_id.clone(),
            (session.clone(), request_id, now),
        );
        lobby.busy.remove(&profile.profile_id);
        lobby.errors.remove(&profile.profile_id);
        lobby.sources.insert(profile.profile_id.clone(), source);
        // A member who left a party's search is back, or a new search of the
        // party begins: the party counts them again.
        let party = party.map(|(id, size)| {
            let opted_out = lobby.party_opt_outs.entry(id).or_default();
            opted_out.remove(&profile.profile_id);
            (id, size.saturating_sub(opted_out.len()).max(1))
        });
        if let Some(check) = lobby
            .ready_profiles
            .get(&profile.profile_id)
            .and_then(|id| lobby.ready.get(id))
        {
            if !check.entries.iter().any(|w| {
                w.profile.profile_id == profile.profile_id
                    && w.session == session
                    && w.preference == preference
            }) {
                lobby
                    .errors
                    .insert(profile.profile_id, "already_queued".into());
            }
            return;
        }
        if let Some((code, until)) = lobby.refusals.get(&profile.profile_id).copied() {
            if now < until {
                lobby
                    .waiting
                    .retain(|w| w.profile.profile_id != profile.profile_id);
                lobby.errors.insert(profile.profile_id, code.into());
                return;
            }
            lobby.refusals.remove(&profile.profile_id);
        }
        // A new request can arrive between worker settlement and the lobby's
        // periodic pruning. Never bind it to the completed allocation.
        if let Some(slot) = lobby
            .assignments
            .get(&profile.profile_id)
            .and_then(|id| lobby.pool.slots.get_mut(id))
        {
            slot.refresh_status();
            if slot
                .status
                .as_ref()
                .is_some_and(|s| s.released_sessions.contains(&session))
            {
                lobby.assignments.remove(&profile.profile_id);
            }
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
        if lobby.waiting.len() >= lobby.max_waiting {
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
        // A newcomer to a full lobby learns its place at once.
        if lobby.pool.full() {
            lobby.refresh_line(now);
        }
    }
    /// Answers a `FindMatch` with a refusal `code` instead of queueing it (a
    /// moderation sanction, LR-15): the profile leaves the queue, and its view
    /// is `Failed { code }` until a later `FindMatch` is accepted.
    pub fn refuse(
        &mut self,
        profile: &str,
        session: String,
        request_id: u64,
        code: &str,
        now: Instant,
    ) {
        let Self::Lobby(lobby) = self else {
            return;
        };
        if lobby.requests.len() >= lobby.max_requests && !lobby.requests.contains_key(profile) {
            return;
        }
        lobby
            .requests
            .insert(profile.to_owned(), (session, request_id, now));
        lobby.waiting.retain(|w| w.profile.profile_id != profile);
        lobby.errors.insert(profile.to_owned(), code.to_owned());
    }
    /// A signed `CancelQueue`: leaves the queue, and a found match that has not
    /// started. The other humans of that match go back to the queue with their
    /// place (LR-32); cancelling a found Ranked match costs a short cooldown. A
    /// party member's cancel leaves the party searching without them.
    pub fn respond_ready(
        &mut self,
        profile: &str,
        session: &str,
        request_id: u64,
        check_id: &str,
        accepted: bool,
        now: Instant,
    ) {
        let Self::Lobby(lobby) = self else {
            return;
        };
        if lobby
            .requests
            .get(profile)
            .is_none_or(|(s, id, _)| s != session || *id != request_id)
        {
            return;
        }
        let Some(check) = lobby.ready.get_mut(check_id) else {
            return;
        };
        if now >= check.deadline
            || !check
                .entries
                .iter()
                .any(|w| w.profile.profile_id == profile && w.session == session)
        {
            return;
        }
        if accepted {
            check.accepted.insert(profile.to_owned());
        } else {
            lobby.cancel_ready(check_id, Some(profile), now);
        }
    }
    pub fn cancel(&mut self, profile: &str, now: Instant) {
        let Self::Lobby(lobby) = self else {
            return;
        };
        if let Some(id) = lobby.ready_profiles.get(profile).cloned() {
            let expired = lobby.ready.get(&id).is_some_and(|r| now >= r.deadline);
            lobby.cancel_ready(&id, if expired { None } else { Some(profile) }, now);
            return;
        }
        if let Some(entry) = lobby
            .waiting
            .iter()
            .find(|w| w.profile.profile_id == profile)
            && let Some((party_id, _)) = entry.party
        {
            lobby
                .party_opt_outs
                .entry(party_id)
                .or_default()
                .insert(profile.to_owned(), now);
            for mate in lobby
                .waiting
                .iter()
                .filter(|w| w.party.is_some_and(|(id, _)| id == party_id))
                .filter(|w| w.profile.profile_id != profile)
            {
                lobby
                    .notices
                    .insert(mate.profile.profile_id.clone(), now + PARTY_LEFT_NOTICE);
            }
        }
        lobby.waiting.retain(|w| w.profile.profile_id != profile);
        lobby.errors.remove(profile);
        let Some(id) = lobby.assignments.get(profile).cloned() else {
            return;
        };
        if !lobby.pool.cancel(&id) {
            return; // Running: the seat stays resumable (LR-13).
        }
        let ranked = lobby
            .pool
            .slots
            .get(&id)
            .is_some_and(|slot| slot.manifest.preference == MatchPreference::Ranked);
        lobby.requeue_parked(&id, &[profile.to_owned()], now);
        lobby.assignments.remove(profile);
        if ranked {
            lobby.penalize_dodge(profile, now);
        }
    }
    /// Profiles this lobby has handed a live allocation (forming or running):
    /// the party lobby shows them as "in match" (LR-55).
    pub fn assigned_profiles(&self) -> impl Iterator<Item = &str> + '_ {
        let assignments = match self {
            Self::Lobby(lobby) => Some(&lobby.assignments),
            _ => None,
        };
        assignments
            .into_iter()
            .flat_map(|a| a.keys().map(String::as_str))
    }
    pub fn request_id(&self, profile: &str, session: &str) -> Option<u64> {
        if let Self::Lobby(l) = self {
            l.requests
                .get(profile)
                .filter(|(s, _, _)| s == session)
                .map(|(_, id, _)| *id)
                .or_else(|| {
                    l.busy
                        .get(profile)
                        .filter(|(s, _, _)| s == session)
                        .map(|(_, id, _)| *id)
                })
        } else {
            None
        }
    }
    pub fn view(&self, profile: &str, session: &str, now: Instant) -> Option<MatchServiceView> {
        let Self::Lobby(lobby) = self else {
            return None;
        };
        if let Some((id, check)) = lobby
            .ready_profiles
            .get(profile)
            .and_then(|id| lobby.ready.get(id).map(|c| (id, c)))
        {
            if check
                .entries
                .iter()
                .any(|w| w.profile.profile_id == profile && w.session == session)
            {
                return Some(MatchServiceView::ReadyCheck {
                    check_id: id.clone(),
                    preference: check.queue,
                    accepted: check.accepted.contains(profile),
                    accepted_count: check.accepted.len() as u32,
                    needed: check.entries.len() as u32,
                    remaining_secs: check
                        .deadline
                        .saturating_duration_since(now)
                        .as_millis()
                        .div_ceil(1000) as u64,
                });
            }
            return Some(MatchServiceView::Failed {
                code: "profile_has_another_session".into(),
            });
        }
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
        if lobby
            .busy
            .get(profile)
            .is_some_and(|(s, _, until)| s == session && now < *until)
        {
            return Some(MatchServiceView::Failed {
                code: LOBBY_BUSY.into(),
            });
        }
        if lobby.notices.get(profile).is_some_and(|until| now < *until) {
            return Some(MatchServiceView::Failed {
                code: PARTY_MEMBER_LEFT.into(),
            });
        }
        if let Some(index) = lobby
            .waiting
            .iter()
            .position(|w| w.profile.profile_id == profile)
        {
            let line = lobby.line.get(profile).map(|&position| CapacityLine {
                position,
                eta_secs: lobby.turnover.eta_secs(position, now),
            });
            let mut view = waiting_view(&lobby.waiting, index, line, now);
            if let MatchServiceView::Waiting {
                party_gathering, ..
            } = &mut view
            {
                *party_gathering = lobby.party_gathering(index, now);
            }
            return Some(view);
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
        lobby.turnover.observe(lobby.pool.occupied(), now);
        // A match that started keeps its seats (LR-13); its parked entries are
        // no longer needed. One that ended without ever starting puts its
        // humans back in the queue with their place, except those who left
        // (LR-32); only an allocation this lobby did not seat itself (adopted
        // after a restart) is reported as cancelled instead.
        let ended: Vec<String> = lobby
            .pool
            .slots
            .iter()
            .filter(|(_, slot)| slot.ran() || slot.terminal())
            .map(|(id, _)| id.clone())
            .collect();
        for id in ended {
            let (ran, failed) = lobby.pool.slots.get(&id).map_or((false, false), |slot| {
                (
                    slot.ran(),
                    slot.status
                        .as_ref()
                        .is_some_and(|s| s.phase == Phase::Failed),
                )
            });
            if ran {
                lobby.parked.remove(&id);
            } else if failed && lobby.parked.contains_key(&id) {
                // Who chose to leave, or never came: the worker names the
                // leavers in cancel.json, and its last receipt names who had
                // joined. A receipt the lobby wrote itself (epoch 0: the child
                // died or was killed before it ever reported) is a broken
                // worker, not a lost player: all queued humans keep their
                // places; there is no evidence to blame any of them for a no-show.
                let reported = lobby
                    .pool
                    .slots
                    .get(&id)
                    .and_then(|slot| slot.status.clone())
                    .filter(|s| s.server_epoch != 0);
                if let Some(status) = reported {
                    let mut left = lobby.pool.cancelled_sessions(&id);
                    if !left.is_empty()
                        && let Some(slot) = lobby.pool.slots.get(&id)
                        && slot.manifest.preference == MatchPreference::Ranked
                    {
                        // Draft cancellation reaches the worker, not the
                        // lobby. Apply the same cooldown as a lobby cancel;
                        // a timeout/no-show alone is not an explicit dodge.
                        let dodgers: Vec<_> = slot
                            .manifest
                            .humans
                            .iter()
                            .filter(|human| left.contains(&human.session_id))
                            .map(|human| human.profile_id.clone())
                            .collect();
                        for profile in dodgers {
                            lobby.penalize_dodge(&profile, now);
                        }
                    }
                    if left.is_empty()
                        && let Some(slot) = lobby.pool.slots.get(&id)
                    {
                        left = slot
                            .manifest
                            .humans
                            .iter()
                            .filter(|h| !status.joined_sessions.contains(&h.session_id))
                            .map(|h| h.session_id.clone())
                            .collect();
                    }
                    lobby.requeue_parked(&id, &left, now);
                } else {
                    lobby.requeue_parked(&id, &[], now);
                }
            }
        }
        lobby.assignments.retain(|profile, id| {
            if let Some(slot) = lobby.pool.slots.get(id) {
                if slot.status.as_ref().is_some_and(|status| {
                    slot.manifest.humans.iter().any(|h| {
                        h.profile_id == *profile && status.released_sessions.contains(&h.session_id)
                    })
                }) {
                    return false;
                }
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
        lobby.parked.retain(|id, _| {
            lobby
                .pool
                .slots
                .get(id)
                .is_some_and(|slot| !slot.terminal())
        });
        lobby.tick_ready(now);
        expire_waiting(&mut lobby.waiting, now);
        lobby.track_abandons(now);
        lobby.requests.retain(|profile, (_, _, touched)| {
            now.saturating_duration_since(*touched) < REQUEST_RETENTION
                || lobby.assignments.contains_key(profile)
                || lobby.ready_profiles.contains_key(profile)
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
        lobby.busy.retain(|_, (_, _, until)| now < *until);
        lobby.notices.retain(|_, until| now < *until);
        lobby.refusals.retain(|_, (_, until)| now < *until);
        lobby.party_opt_outs.retain(|_, members| {
            members.retain(|_, at| now.saturating_duration_since(*at) < PARTY_OPT_OUT_TTL);
            !members.is_empty()
        });
        let queued_parties: HashSet<_> = lobby
            .waiting
            .iter()
            .filter_map(|w| w.party.map(|(id, _)| id))
            .collect();
        lobby
            .party_rosters
            .retain(|id, _| queued_parties.contains(id));
        // At most four OS process launches per scheduling pass.
        for _ in 0..4 {
            if lobby.pool.occupied() + lobby.ready.len() >= lobby.pool.capacity() {
                break;
            }
            let Some(Selection {
                members: indices,
                queue,
                cohort,
            }) = select(&lobby.waiting, now)
            else {
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
            let mut bytes = [0_u8; 16];
            if getrandom::fill(&mut bytes).is_err() {
                break;
            }
            let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let ids: HashSet<_> = selected
                .iter()
                .map(|w| w.profile.profile_id.clone())
                .collect();
            lobby
                .waiting
                .retain(|w| !ids.contains(&w.profile.profile_id));
            for profile in ids {
                lobby.ready_profiles.insert(profile, id.clone());
            }
            lobby.ready.insert(
                id,
                ReadyRoster {
                    entries: selected,
                    queue,
                    cohort,
                    accepted: HashSet::new(),
                    deadline: now + READY_WINDOW,
                },
            );
        }
        lobby.turnover.settle(lobby.pool.occupied());
        lobby.refresh_line(now);
        lobby.report_capacity(now);
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

/// The searching screen's view of one queued entry. `line` is its place in
/// the line for a free arena, while every arena is busy (LR-09).
fn waiting_view(
    waiting: &[Waiting],
    index: usize,
    line: Option<CapacityLine>,
    now: Instant,
) -> MatchServiceView {
    let w = &waiting[index];
    let units = units(waiting, now);
    let own = units.iter().find(|unit| unit.contains(&index));
    let ranked = w.preference == MatchPreference::Ranked;
    // The lobby never seats a mixed party in Ranked: a party is one base
    // cohort, whatever window its wait opens (LR-08); say so instead of
    // waiting forever.
    if ranked
        && own.is_some_and(|unit| {
            !same_cohort_with(
                unit.iter().map(|&i| &waiting[i].profile),
                CohortAllowance::BASE,
            )
        })
    {
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
        // The largest rated cohort this player can be seated in, under the
        // window their own wait has opened (LR-35).
        MatchPreference::Ranked => {
            let allowance = CohortAllowance::after_wait(now.saturating_duration_since(w.queued));
            let peers = waiting
                .iter()
                .filter(|low| {
                    low.preference == w.preference
                        && fits_cohort_with(&low.profile, &w.profile, allowance)
                })
                .map(|low| {
                    waiting
                        .iter()
                        .filter(|other| {
                            other.preference == w.preference
                                && fits_cohort_with(&low.profile, &other.profile, allowance)
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
        capacity_wait: line.is_some(),
        searching: searching.map(|n| n as u32),
        casual_offer_after_secs: ranked.then_some(RANKED_CASUAL_OFFER.as_secs()),
        // For a party, only once every member took the offer.
        also_casual: ranked
            && own.is_some_and(|unit| searches(waiting, unit, MatchPreference::Casual)),
        capacity_line: line,
        party_gathering: None,
    }
}
/// Queue units: a party's queued members travel together, everyone else
/// alone. A party is a unit only once every member it waits for has queued
/// with the same preference. Units are in queue order.
fn units(waiting: &[Waiting], now: Instant) -> Vec<Vec<usize>> {
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
        // A member who was already searching another queue when the party
        // launched searches it alone; the party's head count leaves them out
        // instead of waiting for a member who never queues its preference.
        let preference = waiting[members[0]].preference;
        let (party, solo): (Vec<usize>, Vec<usize>) = members
            .iter()
            .partition(|&&i| waiting[i].preference == preference);
        let need = need.saturating_sub(solo.len()).max(1);
        // Missing members cannot hold a public search forever. Preserve the
        // queue age and seat the present party together once gathering ends.
        let timed_out = party
            .iter()
            .any(|&i| now.saturating_duration_since(waiting[i].queued) >= PARTY_GATHER_WINDOW);
        let need = if timed_out {
            need.min(party.len())
        } else {
            need
        };
        out.extend(solo.into_iter().map(|i| vec![i]));
        if party.len() >= need && party.len() <= 5 {
            out.push(party);
        }
    }
    out.sort_by_key(|unit| unit.iter().map(|&i| waiting[i].order).min());
    out
}

#[cfg(test)]
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

/// Exact bounded packing: 6 × 6 team capacities and a bot-fill-ready bit.
/// Each state retains the lexicographically oldest roster. Unlike greedy
/// packing this finds 3+2 vs 3+2 even when two older parties of four precede it.
fn pack_units(
    waiting: &[Waiting],
    eligible: &[&Vec<usize>],
    mandatory: Option<&Vec<usize>>,
    allow_fill: bool,
    now: Instant,
) -> Option<Vec<usize>> {
    let slot = |green: usize, blue: usize, due: bool| (green * 6 + blue) * 2 + usize::from(due);
    let due = |unit: &[usize]| {
        unit.iter().any(|&i| {
            waiting[i]
                .casual_since
                .is_some_and(|at| now.saturating_duration_since(at) >= CASUAL_BOT_FILL)
        })
    };
    let older = |a: &[usize], b: &[usize]| {
        a.iter()
            .map(|&i| waiting[i].order)
            .cmp(b.iter().map(|&i| waiting[i].order))
            .is_lt()
    };
    let mut states: Vec<Option<Vec<usize>>> = vec![None; 72];
    let seed = mandatory.cloned().unwrap_or_default();
    if seed.len() > 5 {
        return None;
    }
    let seed_slot = slot(seed.len(), 0, due(&seed));
    states[seed_slot] = Some(seed);
    let mut earliest: Vec<_> = eligible.iter().flat_map(|u| u.iter().copied()).collect();
    earliest.sort_by_key(|&i| waiting[i].order);
    earliest.truncate(10);
    for unit in eligible {
        if mandatory.is_some_and(|m| std::ptr::eq(m, *unit)) {
            continue;
        }
        let previous = states.clone();
        for green in 0..=5 {
            for blue in 0..=5 {
                for was_due in [false, true] {
                    let Some(roster) = &previous[slot(green, blue, was_due)] else {
                        continue;
                    };
                    for (g, b) in [(green + unit.len(), blue), (green, blue + unit.len())] {
                        if g > 5 || b > 5 {
                            continue;
                        }
                        let mut next = roster.clone();
                        next.extend(unit.iter().copied());
                        next.sort_by_key(|&i| waiting[i].order);
                        let target = &mut states[slot(g, b, was_due || due(unit))];
                        if target.as_ref().is_none_or(|old| older(&next, old)) {
                            *target = Some(next);
                        }
                    }
                }
            }
        }
        if earliest.len() == 10
            && [false, true]
                .into_iter()
                .any(|d| states[slot(5, 5, d)].as_ref() == Some(&earliest))
        {
            break;
        }
    }
    let mut best: Option<Vec<usize>> = None;
    for (index, roster) in states.into_iter().enumerate() {
        let Some(roster) = roster else {
            continue;
        };
        if roster.is_empty() || (roster.len() != 10 && !(allow_fill && index % 2 == 1)) {
            continue;
        }
        if best.as_ref().is_none_or(|old| {
            roster.len() > old.len() || (roster.len() == old.len() && older(&roster, old))
        }) {
            best = Some(roster);
        }
    }
    best
}

/// Team per selected entry: parties stay together, head counts as even as
/// possible, then rating sums as close as possible. A lone party (bot
/// practice) plays Green.
fn split_teams(selected: &[Waiting], now: Instant) -> Vec<shared::map::Team> {
    use shared::map::Team;
    let units = units(selected, now);
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
/// - vs Bots: the oldest practice unit.
/// - Ranked: ten humans of one rated cohort (the career store's policy, so
///   the store never rejects the roster), never filled with bots. The cohort
///   window widens with the anchor unit's wait (LR-35). A Ranked roster goes
///   before a Casual one, which keeps a Ranked player's place while it also
///   searches Casual.
/// - Casual PvP: every unit that searches Casual, whatever its rating or
///   newcomer cohort; once anyone has searched Casual for
///   [`CASUAL_BOT_FILL`], bots fill the empty seats.
///
/// Among ready rosters the one seating more humans takes the arena first
/// (a 5v5 before a solo vs Bots game when arenas are short), then the older;
/// within one queue the oldest wins.
fn select(waiting: &[Waiting], now: Instant) -> Option<Selection> {
    let units = units(waiting, now);
    let mut candidates: Vec<Selection> = Vec::new();
    if let Some(unit) = units
        .iter()
        .find(|unit| searches(waiting, unit, MatchPreference::BotPractice))
    {
        candidates.push(Selection {
            members: unit.clone(),
            queue: MatchPreference::BotPractice,
            cohort: None,
        });
    }
    let older = |a: &Vec<usize>, b: &Vec<usize>| {
        a.iter()
            .map(|&i| waiting[i].order)
            .cmp(b.iter().map(|&i| waiting[i].order))
            .is_lt()
    };
    for queue in [MatchPreference::Ranked, MatchPreference::Casual] {
        if queue == MatchPreference::Casual {
            let eligible: Vec<_> = units
                .iter()
                .filter(|u| searches(waiting, u, queue))
                .collect();
            if let Some(members) = pack_units(waiting, &eligible, None, true, now) {
                candidates.push(Selection {
                    members,
                    queue,
                    cohort: None,
                });
            }
            continue;
        }
        let mut best: Option<(Vec<usize>, Option<CohortAllowance>)> = None;
        let mut windows = HashSet::new();
        for anchor in units.iter().filter(|unit| searches(waiting, unit, queue)) {
            let low = anchor
                .iter()
                .map(|&i| &waiting[i])
                .min_by_key(|w| w.profile.rating)
                .expect("units are never empty");
            // The window the anchor's wait has opened (LR-35): the oldest
            // member of the unit decides.
            let waited = anchor
                .iter()
                .map(|&i| now.saturating_duration_since(waiting[i].queued))
                .max()
                .unwrap_or_default();
            let allowance = CohortAllowance::after_wait(waited);
            // Ranked admits a unit only when its every member fits the
            // cohort that starts at the anchor's lowest rating; a party is
            // itself one base cohort, whatever the window (LR-08).
            let fits = |unit: &Vec<usize>| {
                queue != MatchPreference::Ranked
                    || (unit
                        .iter()
                        .all(|&i| fits_cohort_with(&low.profile, &waiting[i].profile, allowance))
                        && (unit.len() == 1
                            || same_cohort_with(
                                unit.iter().map(|&i| &waiting[i].profile),
                                CohortAllowance::BASE,
                            )))
            };
            if !fits(anchor)
                || !windows.insert((
                    low.profile.rating,
                    low.profile.newcomer(),
                    allowance.spread,
                    allowance.merge_newcomers,
                    anchor.len(),
                ))
            {
                continue;
            }
            // Equal-size anchors in the same window are interchangeable;
            // forcing the oldest one dominates forcing any later one.
            let eligible: Vec<_> = units
                .iter()
                .filter(|u| searches(waiting, u, queue) && fits(u))
                .collect();
            let Some(members) = pack_units(waiting, &eligible, Some(anchor), false, now) else {
                continue;
            };
            let ready = true;
            if ready && best.as_ref().is_none_or(|(b, _)| older(&members, b)) {
                best = Some((
                    members,
                    (queue == MatchPreference::Ranked).then_some(allowance),
                ));
            }
        }
        if let Some((members, cohort)) = best {
            candidates.push(Selection {
                members,
                queue,
                cohort,
            });
        }
    }
    // An opted-in Ranked player belongs to a ready Ranked roster before any
    // Casual roster can take them, even when older Casual players are queued.
    if let Some(ranked) = candidates
        .iter()
        .find(|c| c.queue == MatchPreference::Ranked)
    {
        let ranked_members: HashSet<usize> = ranked.members.iter().copied().collect();
        candidates.retain(|c| {
            c.queue != MatchPreference::Casual
                || !c.members.iter().any(|i| ranked_members.contains(i))
        });
    }
    candidates.into_iter().min_by(|a, b| {
        b.members
            .len()
            .cmp(&a.members.len())
            .then_with(|| {
                a.members
                    .iter()
                    .map(|&i| waiting[i].order)
                    .min()
                    .cmp(&b.members.iter().map(|&i| waiting[i].order).min())
            })
            .then_with(|| queue_rank(a.queue).cmp(&queue_rank(b.queue)))
    })
}

/// Tie-break between equally large, equally old rosters: Ranked, Casual, vs Bots.
fn queue_rank(queue: MatchPreference) -> u8 {
    match queue {
        MatchPreference::Ranked => 0,
        MatchPreference::Casual => 1,
        MatchPreference::BotPractice => 2,
    }
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
        let mut joined_sessions: Vec<String> = self
            .world
            .players
            .iter()
            .filter(|(_, p)| p.joined && !p.hero.identity.is_bot)
            .filter_map(|(addr, _)| self.career.backend.authenticated_session(*addr))
            .collect();
        joined_sessions.sort();
        if let MatchService::Worker(worker) = &mut self.match_service {
            worker.saw_human |= !joined_sessions.is_empty();
            if !worker.manifest.preference.rated()
                && !joined_sessions.is_empty()
                && self.match_started_at.is_none()
                && unix_ms() > worker.manifest.join_deadline_ms
            {
                for human in &worker.manifest.humans {
                    if !joined_sessions.contains(&human.session_id)
                        && !worker.released.contains(&human.session_id)
                    {
                        worker.released.push(human.session_id.clone());
                    }
                }
            }
        }
        if self.match_started_at.is_none()
            && self
                .match_service
                .worker()
                .is_some_and(|w| !w.released.is_empty() && !w.manifest.preference.rated())
        {
            self.fill_practice_bots(now);
        }
        let Some(worker) = self.match_service.worker() else {
            return;
        };
        let recovery = worker.recovery;
        // A worker nobody ever reached (the handoff failed, the app closed)
        // frees its arena after NO_SHOW_ABORT instead of the full join deadline.
        let nobody_came = !worker.saw_human
            && !recovery
            && now.saturating_duration_since(worker.booted_at) >= NO_SHOW_ABORT;
        let abort = worker.aborted
            || recovery
            || (self.match_started_at.is_none()
                && (worker.cancelled()
                    || nobody_came
                    || (unix_ms() > worker.manifest.join_deadline_ms
                        && worker.manifest.preference.rated())
                    || unix_ms() > worker.manifest.join_deadline_ms.saturating_add(60_000)));
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
        // Deadline/file cancellation retires the simulation as well as its
        // disk receipt. The tick then repeats the draft return code to peers.
        if abort && !recovery {
            worker.aborted = true;
        }
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
                    joined_sessions: Vec::new(),
                    released_sessions: worker.released.clone(),
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
                joined_sessions,
                released_sessions: worker.released.clone(),
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
        let selection = select(waiting, now).expect("a roster forms");
        assert_eq!(selection.queue, queue);
        selection.members
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
                    waiting_view(&q, 0, None, now),
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
                let Selection {
                    members,
                    queue,
                    cohort,
                } = select(&q, later).expect("ten solos form Ranked");
                assert_eq!(queue, MatchPreference::Ranked);
                assert!(!members.contains(&0) && !members.contains(&1));
                assert!(same_cohort_with(
                    members.iter().map(|&i| &q[i].profile),
                    cohort.expect("a Ranked roster carries its window")
                ));

                // The same ten in Casual PvP: one unrated roster.
                q.truncate(10);
                for entry in &mut q {
                    entry.preference = MatchPreference::Casual;
                    entry.casual_since = Some(now);
                }
                let all = picked(&q, now, MatchPreference::Casual);
                assert_eq!(all.len(), 10);
                let chosen: Vec<_> = all.iter().map(|&i| q[i].clone()).collect();
                let teams = split_teams(&chosen, now);
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
        let casual_count = |lobby: &Lobby| match waiting_view(&lobby.waiting, 1, None, offered) {
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
        let also = |q: &[Waiting]| match waiting_view(q, 0, None, late) {
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
            tick_accepted(&mut service, now);
            let MatchService::Lobby(lobby) = &service else {
                unreachable!()
            };
            assert!(lobby.waiting.is_empty(), "{queue:?}");
            let (id, slot) = lobby.pool.slots.iter().next().unwrap();
            assert_eq!(slot.manifest.preference, queue);
            assert_eq!(slot.manifest.humans.len(), 10);
            let receipt = Status {
                released_sessions: Vec::new(),
                allocation_id: id.clone(),
                server_epoch: 7,
                heartbeat_ms: unix_ms(),
                phase: Phase::Ready,
                result_id: None,
                ends_by_ms: None,
                joined_sessions: Vec::new(),
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
        let teams = split_teams(&chosen, now);
        assert_eq!(teams, vec![shared::map::Team::Green; 2]);
    }

    fn manifest_for(chosen: &[Waiting], queue: MatchPreference) -> Manifest {
        let teams = split_teams(chosen, Instant::now() + PARTY_GATHER_WINDOW);
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
            cohort: None,
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
            saw_human: false,
            released: Vec::new(),
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
    fn sanctions_refuse_find_match_with_their_code_until_lifted() {
        use crate::runtime::ports::{ManualClock, MemoryTransport};
        use ed25519_dalek::Signer;
        use shared::career::{CareerAction, CareerRequest};
        use shared::moderation::{Sanction, SanctionKind, code};
        let now = Instant::now();
        let (lobby, directory) = practice_lobby();
        let mut rt = ServerRuntime::for_test(
            MemoryTransport::new("127.0.0.1:4000".parse().unwrap()),
            ManualClock::new(now),
            crate::career_backend::MemoryCareer::test_backend(7),
            crate::match_rules::MatchConfig::dev(),
        );
        rt.match_service = lobby;
        let player: std::net::SocketAddr = "127.0.0.1:60200".parse().unwrap();
        rt.career.backend.test_authenticated(
            player,
            ProfileSummary::new("c".repeat(64), "Rude#0001".into()),
            "rude",
        );
        let key = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
        let mut sequence = 0;
        let mut find = |rt: &mut ServerRuntime, preference| {
            // One account request per 100 ms per endpoint.
            std::thread::sleep(Duration::from_millis(110));
            sequence += 1;
            let action = CareerAction::FindMatch {
                request_id: sequence,
                preference,
                also_casual: false,
            };
            let nonce = "b".repeat(64);
            let signature = key
                .sign(&shared::career::authorized_signing_bytes(
                    7, &nonce, sequence, &action,
                ))
                .to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            rt.career.backend.handle(
                player,
                CareerRequest::Authorized {
                    session_nonce: nonce,
                    sequence,
                    action,
                    signature,
                },
            );
            rt.poll_career(now);
            rt.career_view(player, now).match_service.unwrap()
        };
        let failed = |code: &str| MatchServiceView::Failed { code: code.into() };
        let hour = crate::moderation::unix_now() + 3600;
        rt.career.backend.test_set_sanctions(
            player,
            vec![Sanction {
                kind: SanctionKind::RankedBan,
                until: Some(hour),
            }],
        );
        assert_eq!(
            find(&mut rt, MatchPreference::Ranked),
            failed(code::RANKED_BAN)
        );
        assert!(matches!(
            find(&mut rt, MatchPreference::Casual),
            MatchServiceView::Waiting { .. }
        ));
        // A matchmaking ban applied while searching takes the player out.
        rt.career.backend.test_set_sanctions(
            player,
            vec![Sanction {
                kind: SanctionKind::MatchmakingBan,
                until: None,
            }],
        );
        assert_eq!(
            find(&mut rt, MatchPreference::Casual),
            failed(code::MATCHMAKING_BAN)
        );
        assert_eq!(
            find(&mut rt, MatchPreference::BotPractice),
            failed(code::MATCHMAKING_BAN)
        );
        let MatchService::Lobby(lobby) = &rt.match_service else {
            unreachable!()
        };
        assert!(lobby.waiting.is_empty());
        // Lifted: the next search queues, with no restart.
        rt.career.backend.test_set_sanctions(player, Vec::new());
        assert!(matches!(
            find(&mut rt, MatchPreference::Ranked),
            MatchServiceView::Waiting { .. }
        ));
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
            released_sessions: Vec::new(),
            allocation_id: id.clone(),
            server_epoch: 7,
            heartbeat_ms: unix_ms(),
            phase: Phase::Running,
            result_id: Some("finished-round".into()),
            ends_by_ms: None,
            joined_sessions: Vec::new(),
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

    fn tick_accepted(service: &mut MatchService, now: Instant) {
        service.tick(now);
        let MatchService::Lobby(lobby) = service else {
            return;
        };
        for check in lobby.ready.values_mut() {
            check
                .accepted
                .extend(check.entries.iter().map(|w| w.profile.profile_id.clone()));
        }
        lobby.tick_ready(now);
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
            released_sessions: Vec::new(),
            allocation_id: id.clone(),
            server_epoch: 7,
            heartbeat_ms: unix_ms(),
            phase: Phase::Running,
            result_id: Some("r".repeat(32)),
            ends_by_ms: Some(unix_ms() + 200_000),
            joined_sessions: Vec::new(),
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
        tick_accepted(&mut service, now + Duration::from_secs(1));
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
        tick_accepted(&mut service, now + Duration::from_secs(2));
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
            released_sessions: Vec::new(),
            allocation_id: id.clone(),
            server_epoch: 7,
            heartbeat_ms: unix_ms(),
            phase: Phase::Forming,
            result_id: None,
            ends_by_ms: None,
            joined_sessions: Vec::new(),
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
        tick_accepted(&mut service, now + Duration::from_secs(1));
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
        let teams = split_teams(&chosen, now);
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
        let teams = split_teams(&chosen, now);
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
                released_sessions: Vec::new(),
                allocation_id: id.clone(),
                server_epoch: 7,
                heartbeat_ms: unix_ms(),
                phase,
                result_id: None,
                ends_by_ms: None,
                joined_sessions: Vec::new(),
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
        tick_accepted(&mut service, now);
        for (phase, at) in [(Phase::Forming, 1), (Phase::Running, 2)] {
            let at = now + Duration::from_secs(at);
            report(&mut service, &first.profile.profile_id, phase);
            tick_accepted(&mut service, at);
            practice(&mut service, &second, home(), at);
            assert_eq!(failed(&service, &second, at).as_deref(), Some(ARENA_LIMIT));
        }
        // Another IP gets its own arena.
        let at = now + Duration::from_secs(2);
        practice(&mut service, &other, neighbour(), at);
        tick_accepted(&mut service, now + Duration::from_secs(3));
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
        tick_accepted(&mut service, fill);
        assert_eq!(
            failed(&service, &casual, fill).as_deref(),
            Some(ARENA_LIMIT)
        );
        // The first match ended: the IP may practice again.
        report(&mut service, &first.profile.profile_id, Phase::Finished);
        let later = fill + Duration::from_secs(1);
        tick_accepted(&mut service, later);
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
            tick_accepted(&mut service, at);
            assert_eq!(failed(&service, &entry, at), None, "{id}");
            // Allocated, never joined: the worker gives up.
            report(&mut service, &entry.profile.profile_id, Phase::Failed);
            at += Duration::from_secs(1);
            tick_accepted(&mut service, at);
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
        tick_accepted(&mut service, after);
        practice(&mut service, &next, home(), after);
        assert_eq!(failed(&service, &next, after), None);
        let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
    }

    /// LR-09: with every arena busy, each ready roster sees its place in the
    /// line for the next free arena (vs Bots first, as `select` seats them),
    /// with an estimate once arenas have freed; a search that is not ready
    /// (Ranked 1/10) shows its own progress. The lobby reports the busy
    /// period in `capacity.json`.
    #[test]
    fn a_full_lobby_shows_each_ready_roster_its_place_in_the_line() {
        use crate::capacity::{CapacityReport, REPORT_FILE};
        let now = Instant::now();
        let pool = crate::match_pool::Pool::for_test(); // one arena
        let directory = pool.test_directory().to_path_buf();
        let mut service = lobby_with(pool, &[]);
        let ip = |n: u8| SourceIp::of(std::net::SocketAddr::from(([198, 51, 100, n], 40000)));
        let first = w(1, MatchPreference::BotPractice, now);
        practice(&mut service, &first, ip(1), now);
        tick_accepted(&mut service, now);
        report(&mut service, &first.profile.profile_id, Phase::Running);
        let (second, third) = (
            w(2, MatchPreference::BotPractice, now),
            w(3, MatchPreference::BotPractice, now),
        );
        let ranked = w(4, MatchPreference::Ranked, now);
        let at = now + Duration::from_secs(1);
        service.enqueue(
            ranked.profile.clone(),
            ranked.session.clone(),
            1,
            MatchPreference::Ranked.into(),
            None,
            ip(4),
            at,
        );
        practice(&mut service, &second, ip(2), at);
        practice(&mut service, &third, ip(3), at);
        let line = |service: &MatchService, entry: &Waiting, at| match service.view(
            &entry.profile.profile_id,
            &entry.session,
            at,
        ) {
            Some(MatchServiceView::Waiting {
                capacity_wait,
                capacity_line,
                ..
            }) => {
                assert_eq!(capacity_wait, capacity_line.is_some());
                capacity_line
            }
            other => panic!("expected a wait, got {other:?}"),
        };
        // Known at once, before the next pass.
        assert_eq!(
            line(&service, &second, at),
            Some(CapacityLine {
                position: 1,
                eta_secs: None
            })
        );
        tick_accepted(&mut service, at);
        assert_eq!(line(&service, &third, at).unwrap().position, 2);
        assert_eq!(
            line(&service, &ranked, at),
            None,
            "not ready: no capacity wait"
        );
        let read = || -> CapacityReport {
            serde_json::from_slice(&std::fs::read(directory.join(REPORT_FILE)).unwrap()).unwrap()
        };
        let busy = read();
        assert_eq!(
            (
                busy.capacity,
                busy.occupied,
                busy.searching,
                busy.waiting_for_arena
            ),
            (1, 1, 3, 2)
        );
        assert!(busy.busy_since_ms.is_some());
        assert_eq!(busy.busy_periods, 1);

        // The match ends and its port frees: the first in line gets it.
        let MatchService::Lobby(lobby) = &mut service else {
            unreachable!()
        };
        let id = lobby.assignments[&first.profile.profile_id].clone();
        lobby.pool.write_status_for_test(
            &id,
            &Status {
                released_sessions: Vec::new(),
                allocation_id: id.clone(),
                server_epoch: 7,
                heartbeat_ms: unix_ms() - 36_000,
                phase: Phase::Finished,
                result_id: None,
                ends_by_ms: None,
                joined_sessions: Vec::new(),
            },
        );
        let freed = now + Duration::from_secs(2);
        tick_accepted(&mut service, freed);
        let MatchService::Lobby(lobby) = &service else {
            unreachable!()
        };
        assert!(lobby.assignments.contains_key(&second.profile.profile_id));
        assert_eq!(line(&service, &third, freed).unwrap().position, 1);
        // This fixture represents a second live match. The stand-in child is
        // /usr/bin/true; without a receipt it is now correctly re-queued as a
        // failed worker rather than remaining an occupied, running arena.
        report(&mut service, &second.profile.profile_id, Phase::Running);
        // A minute on, one arena freed in 61 s: about a minute to wait.
        let later = now + Duration::from_secs(61);
        practice(&mut service, &third, ip(3), later);
        service.enqueue(
            ranked.profile.clone(),
            ranked.session.clone(),
            1,
            MatchPreference::Ranked.into(),
            None,
            ip(4),
            later,
        );
        tick_accepted(&mut service, later);
        assert_eq!(
            line(&service, &third, later),
            Some(CapacityLine {
                position: 1,
                eta_secs: Some(61)
            })
        );
        // Nobody ready waits any more: the busy period is over.
        service.cancel(&third.profile.profile_id, later);
        tick_accepted(&mut service, later + Duration::from_secs(1));
        let clear = read();
        assert_eq!(clear.busy_since_ms, None);
        assert_eq!(clear.waiting_for_arena, 0);
        assert_eq!(clear.searching, 1);
        let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
    }

    fn enqueue_all(service: &mut MatchService, entries: &[Waiting], request_id: u64, at: Instant) {
        for entry in entries {
            service.enqueue(
                entry.profile.clone(),
                entry.session.clone(),
                request_id,
                entry.preference.into(),
                entry.party,
                home(),
                at,
            );
        }
    }
    fn waiting_orders(service: &MatchService) -> Vec<u64> {
        let MatchService::Lobby(lobby) = service else {
            unreachable!()
        };
        lobby.waiting.iter().map(|w| w.order).collect()
    }

    /// LR-32: one member's cancel after "match found" sends the other nine back
    /// to the queue with their places; the dodger of a Ranked match waits a
    /// minute before queueing again, and nobody keeps the dead allocation.
    #[test]
    fn cancelling_a_found_ranked_match_requeues_the_others_and_cools_the_dodger() {
        let now = Instant::now();
        let pool = crate::match_pool::Pool::for_test();
        let directory = pool.test_directory().to_path_buf();
        let mut service = lobby_with(pool, &[]);
        let entries: Vec<_> = (1..=10)
            .map(|i| w(i, MatchPreference::Ranked, now))
            .collect();
        enqueue_all(&mut service, &entries, 1, now);
        tick_accepted(&mut service, now);
        assert!(waiting_orders(&service).is_empty());
        let dodger = &entries[2];
        let at = now + Duration::from_secs(1);
        service.cancel(&dodger.profile.profile_id, at);
        assert_eq!(waiting_orders(&service), vec![2, 3, 5, 6, 7, 8, 9, 10, 11]);
        let MatchService::Lobby(lobby) = &service else {
            unreachable!()
        };
        assert!(
            lobby.assignments.is_empty(),
            "nobody keeps the dead allocation"
        );
        assert!(lobby.parked.is_empty());
        // Their next FindMatch renews the place, with the original age.
        let later = at + Duration::from_secs(2);
        service.enqueue(
            entries[0].profile.clone(),
            entries[0].session.clone(),
            1,
            MatchPreference::Ranked.into(),
            None,
            home(),
            later,
        );
        match service.view(&entries[0].profile.profile_id, &entries[0].session, later) {
            Some(MatchServiceView::Waiting { elapsed_secs, .. }) => assert_eq!(elapsed_secs, 3),
            other => panic!("{other:?}"),
        }
        // The dodger is refused for a minute, then queues.
        service.enqueue(
            dodger.profile.clone(),
            dodger.session.clone(),
            2,
            MatchPreference::Ranked.into(),
            None,
            home(),
            later,
        );
        assert_eq!(
            failed(&service, dodger, later).as_deref(),
            Some(DODGE_COOLDOWN_CODE)
        );
        let cooled = at + DODGE_COOLDOWN;
        tick_accepted(&mut service, cooled);
        service.enqueue(
            dodger.profile.clone(),
            dodger.session.clone(),
            2,
            MatchPreference::Ranked.into(),
            None,
            home(),
            cooled,
        );
        assert_eq!(failed(&service, dodger, cooled), None);
        // The nine did not renew their place in this fixture, so it expired;
        // the dodger is queued again, at the back.
        assert_eq!(waiting_orders(&service).len(), 1);
        assert!(matches!(
            service.view(&dodger.profile.profile_id, &dodger.session, cooled),
            Some(MatchServiceView::Waiting { .. })
        ));
        let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
    }

    /// LR-32: when the worker gives up because one human never came, the nine
    /// who had joined return to the queue in order; the no-show does not.
    #[test]
    fn a_no_show_abort_requeues_the_humans_who_had_joined() {
        let now = Instant::now();
        let pool = crate::match_pool::Pool::for_test();
        let directory = pool.test_directory().to_path_buf();
        let mut service = lobby_with(pool, &[]);
        let entries: Vec<_> = (1..=10)
            .map(|i| w(i, MatchPreference::Casual, now))
            .collect();
        enqueue_all(&mut service, &entries, 1, now);
        tick_accepted(&mut service, now);
        let MatchService::Lobby(lobby) = &mut service else {
            unreachable!()
        };
        let id = lobby.assignments[&entries[0].profile.profile_id].clone();
        lobby.pool.adopt_for_test(&id);
        let no_show = &entries[4];
        lobby.pool.write_status_for_test(
            &id,
            &Status {
                released_sessions: Vec::new(),
                allocation_id: id.clone(),
                server_epoch: 7,
                heartbeat_ms: unix_ms() - 36_000,
                phase: Phase::Failed,
                result_id: None,
                ends_by_ms: None,
                // Old enough for the adopted slot's port to count as free.
                joined_sessions: entries
                    .iter()
                    .filter(|e| e.session != no_show.session)
                    .map(|e| e.session.clone())
                    .collect(),
            },
        );
        let at = now + Duration::from_secs(1);
        tick_accepted(&mut service, at);
        assert_eq!(waiting_orders(&service), vec![2, 3, 4, 5, 7, 8, 9, 10, 11]);
        assert_eq!(
            failed(&service, no_show, at).as_deref(),
            Some("allocation_cancelled_or_expired")
        );
        // Casual: the nine searched for a minute already, so bots fill at once
        // when the arena frees.
        let freed = at + Duration::from_secs(40);
        tick_accepted(&mut service, freed);
        let MatchService::Lobby(lobby) = &service else {
            unreachable!()
        };
        assert_eq!(lobby.assignments.len(), 9);
        assert!(!lobby.assignments.contains_key(&no_show.profile.profile_id));
        let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
    }

    /// A party member's cancel tells the others for a moment and the party
    /// searches on without that member; queueing again counts them back in.
    #[test]
    fn a_party_members_cancel_leaves_the_party_searching_without_them() {
        let now = Instant::now();
        let pool = crate::match_pool::Pool::for_test();
        let directory = pool.test_directory().to_path_buf();
        let mut service = lobby_with(pool, &[]);
        let party: Vec<_> = (1..=3)
            .map(|i| in_party(w(i, MatchPreference::Casual, now), 7, 3))
            .collect();
        enqueue_all(&mut service, &party, 1, now);
        let at = now + Duration::from_secs(5);
        service.cancel(&party[1].profile.profile_id, at);
        assert_eq!(
            failed(&service, &party[0], at).as_deref(),
            Some(PARTY_MEMBER_LEFT)
        );
        let after_notice = at + PARTY_LEFT_NOTICE;
        // Retries keep the party tag of three; the lobby counts two.
        enqueue_all(
            &mut service,
            &[party[0].clone(), party[2].clone()],
            1,
            after_notice,
        );
        assert_eq!(failed(&service, &party[0], after_notice), None);
        let fill = now + CASUAL_BOT_FILL;
        enqueue_all(&mut service, &[party[0].clone(), party[2].clone()], 1, fill);
        tick_accepted(&mut service, fill);
        let MatchService::Lobby(lobby) = &service else {
            unreachable!()
        };
        assert_eq!(lobby.assignments.len(), 2, "two humans, bots fill the rest");
        assert!(!lobby.assignments.contains_key(&party[1].profile.profile_id));
        let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
    }

    /// A member who was searching Casual alone when the party launched Ranked
    /// keeps that search; the party's head count leaves them out.
    #[test]
    fn a_party_member_searching_another_queue_is_seated_alone() {
        let now = Instant::now();
        let leader = in_party(w(1, MatchPreference::Ranked, now), 7, 2);
        let mut member = in_party(w(2, MatchPreference::Casual, now), 7, 2);
        member.casual_since = Some(now);
        let q = vec![leader, member];
        assert_eq!(units(&q, now), vec![vec![0], vec![1]]);
        assert_eq!(
            picked(&q, now + CASUAL_BOT_FILL, MatchPreference::Casual),
            vec![1]
        );
    }

    /// When arenas are short, a ready 5v5 takes the next one before a solo vs
    /// Bots game that queued earlier; with arenas free, vs Bots is immediate.
    #[test]
    fn a_full_pvp_roster_takes_an_arena_before_a_solo_bot_game() {
        let now = Instant::now();
        let mut q = vec![w(1, MatchPreference::BotPractice, now)];
        q.extend((2..=11).map(|i| w(i, MatchPreference::Casual, now)));
        let picked = select(&q, now).expect("a roster forms");
        assert_eq!(picked.queue, MatchPreference::Casual);
        assert_eq!(picked.members.len(), 10);
        let solo = select(&q[..1], now).expect("vs Bots alone forms at once");
        assert_eq!(solo.queue, MatchPreference::BotPractice);
    }

    /// LR-35: ten Ranked humans across the base cohort form once the oldest
    /// unit has waited long enough for the window to hold them, and the
    /// manifest records the window; a mixed party still never forms.
    #[test]
    fn the_ranked_window_widens_with_the_wait() {
        let now = Instant::now();
        let mut q: Vec<_> = (1..=10)
            .map(|i| w(i, MatchPreference::Ranked, now))
            .collect();
        for entry in q.iter_mut().skip(5) {
            entry.profile.rating = 1500;
        }
        assert!(select(&q, now + Duration::from_secs(179)).is_none());
        let formed = select(&q, now + Duration::from_secs(180)).expect("500 points fit at 3 min");
        assert_eq!(formed.queue, MatchPreference::Ranked);
        assert_eq!(formed.members.len(), 10);
        assert_eq!(
            formed.cohort,
            Some(CohortAllowance {
                spread: 500,
                merge_newcomers: false
            })
        );
        let chosen: Vec<_> = formed.members.iter().map(|&i| q[i].clone()).collect();
        let mut manifest = manifest_for(&chosen, MatchPreference::Ranked);
        manifest.cohort = formed.cohort.map(|c| (c.spread, c.merge_newcomers));
        assert_eq!(manifest.validate(), Ok(()));
        manifest.cohort = Some((601, false));
        assert!(manifest.validate().is_err(), "never wider than the policy");

        // Newcomers and veterans merge after five minutes.
        for entry in q.iter_mut() {
            entry.profile.rating = 1000;
        }
        for entry in q.iter_mut().skip(5) {
            entry.profile.rated_matches = 40;
        }
        assert!(select(&q, now + Duration::from_secs(299)).is_none());
        let merged = select(&q, now + Duration::from_secs(300)).expect("brackets merge at 5 min");
        assert!(merged.cohort.is_some_and(|c| c.merge_newcomers));
        // The searching screen counts the widened peers.
        match waiting_view(&q, 0, None, now + Duration::from_secs(300)) {
            MatchServiceView::Waiting { humans, .. } => assert_eq!(humans, 10),
            other => panic!("{other:?}"),
        }
        match waiting_view(&q, 0, None, now) {
            MatchServiceView::Waiting { humans, .. } => assert_eq!(humans, 5),
            other => panic!("{other:?}"),
        }
    }

    /// The lobby's caps answer with a code instead of silence.
    #[test]
    fn queue_and_request_caps_answer_with_a_code() {
        let now = Instant::now();
        let pool = crate::match_pool::Pool::for_test();
        let directory = pool.test_directory().to_path_buf();
        let mut service = MatchService::Lobby(Lobby {
            max_waiting: 2,
            max_requests: 3,
            ..Lobby::new(pool, Limits::default())
        });
        let entries: Vec<_> = (1..=4)
            .map(|i| w(i, MatchPreference::Ranked, now))
            .collect();
        enqueue_all(&mut service, &entries[..3], 1, now);
        assert_eq!(
            failed(&service, &entries[2], now).as_deref(),
            Some("queue_full")
        );
        enqueue_all(&mut service, &entries[3..], 1, now);
        assert_eq!(
            failed(&service, &entries[3], now).as_deref(),
            Some(LOBBY_BUSY)
        );
        assert_eq!(waiting_orders(&service), vec![1, 2]);
        assert_eq!(
            service.request_id(&entries[3].profile.profile_id, &entries[3].session),
            Some(1)
        );
        assert_eq!(
            service.request_id(&entries[3].profile.profile_id, "another-session"),
            None
        );
        let _ = std::fs::remove_dir_all(directory); // Only this test's unique fixture.
    }

    #[test]
    fn older_casual_players_cannot_take_an_opted_in_player_from_a_ready_ranked_roster() {
        let now = Instant::now();
        let later = now + RANKED_CASUAL_OFFER;
        let mut q: Vec<_> = (1..=9)
            .map(|i| w(i, MatchPreference::Casual, now))
            .collect();
        let mut ranked = w(10, MatchPreference::Ranked, now);
        ranked.also_casual(true, later);
        q.push(ranked);
        q.extend((11..=19).map(|i| w(i, MatchPreference::Ranked, later)));
        let selected = picked(&q, later + CASUAL_BOT_FILL, MatchPreference::Ranked);
        assert_eq!(selected, (9..19).collect::<Vec<_>>());
    }

    #[test]
    fn a_worker_that_failed_before_its_first_receipt_keeps_everyones_queue_place() {
        let now = Instant::now();
        let pool = crate::match_pool::Pool::for_test();
        let directory = pool.test_directory().to_path_buf();
        let mut service = lobby_with(pool, &[]);
        let entries: Vec<_> = (1..=10)
            .map(|i| w(i, MatchPreference::Ranked, now))
            .collect();
        enqueue_all(&mut service, &entries, 1, now);
        let before = waiting_orders(&service);
        tick_accepted(&mut service, now);
        let MatchService::Lobby(lobby) = &mut service else {
            unreachable!()
        };
        let id = lobby.assignments[&entries[0].profile.profile_id].clone();
        lobby.pool.adopt_for_test(&id);
        lobby.pool.write_status_for_test(
            &id,
            &Status {
                released_sessions: Vec::new(),
                allocation_id: id.clone(),
                server_epoch: 0,
                heartbeat_ms: unix_ms(),
                phase: Phase::Failed,
                result_id: None,
                ends_by_ms: None,
                joined_sessions: Vec::new(),
            },
        );
        tick_accepted(&mut service, now + Duration::from_secs(1));
        assert_eq!(waiting_orders(&service), before);
        for entry in &entries {
            assert!(matches!(
                service.view(&entry.profile.profile_id, &entry.session, now),
                Some(MatchServiceView::Waiting { .. })
            ));
        }
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_ranked_draft_cancel_at_the_worker_cools_only_the_explicit_dodger() {
        let now = Instant::now();
        let pool = crate::match_pool::Pool::for_test();
        let directory = pool.test_directory().to_path_buf();
        let mut service = lobby_with(pool, &[]);
        let entries: Vec<_> = (1..=10)
            .map(|i| w(i, MatchPreference::Ranked, now))
            .collect();
        enqueue_all(&mut service, &entries, 1, now);
        tick_accepted(&mut service, now);
        let MatchService::Lobby(lobby) = &mut service else {
            unreachable!()
        };
        let id = lobby.assignments[&entries[0].profile.profile_id].clone();
        lobby.pool.adopt_for_test(&id);
        crate::match_allocation::atomic_json(
            &directory.join(&id).join("cancel.json"),
            &vec![entries[0].session.clone()],
        )
        .unwrap();
        lobby.pool.write_status_for_test(
            &id,
            &Status {
                released_sessions: Vec::new(),
                allocation_id: id.clone(),
                server_epoch: 7,
                heartbeat_ms: unix_ms(),
                phase: Phase::Failed,
                result_id: None,
                ends_by_ms: None,
                joined_sessions: entries.iter().map(|e| e.session.clone()).collect(),
            },
        );
        let later = now + Duration::from_secs(1);
        tick_accepted(&mut service, later);
        assert_eq!(waiting_orders(&service).len(), 9);
        enqueue_all(&mut service, &entries[..1], 2, later);
        assert_eq!(
            failed(&service, &entries[0], later).as_deref(),
            Some(DODGE_COOLDOWN_CODE)
        );
        let MatchService::Lobby(lobby) = &service else {
            unreachable!()
        };
        assert_eq!(lobby.refusals.len(), 1);
        let until = lobby.refusals[&entries[0].profile.profile_id].1;
        tick_accepted(&mut service, later + Duration::from_secs(1));
        let MatchService::Lobby(lobby) = &service else {
            unreachable!()
        };
        assert_eq!(
            lobby.refusals[&entries[0].profile.profile_id].1, until,
            "polling the failed slot must not extend a dodge penalty"
        );
        let _ = std::fs::remove_dir_all(directory);
    }
    #[test]
    fn exact_party_packing_finds_ten_when_older_fours_cannot_fit() {
        let now = Instant::now();
        for queue in [MatchPreference::Ranked, MatchPreference::Casual] {
            let mut q = Vec::new();
            let mut id = 1;
            for (party, size) in [4, 4, 3, 3, 2, 2].into_iter().enumerate() {
                for _ in 0..size {
                    q.push(in_party(w(id, queue, now), party as u64 + 1, size));
                    id += 1;
                }
            }
            let chosen = picked(&q, now, queue);
            assert_eq!(chosen, (8..18).collect::<Vec<_>>());
            let roster: Vec<_> = chosen.iter().map(|&i| q[i].clone()).collect();
            let teams = split_teams(&roster, now);
            for i in 0..roster.len() {
                for j in i + 1..roster.len() {
                    if roster[i].party == roster[j].party {
                        assert_eq!(teams[i], teams[j]);
                    }
                }
            }
        }
    }

    #[test]
    fn missing_party_member_is_named_then_present_members_continue_at_one_minute() {
        let now = Instant::now();
        let pool = crate::match_pool::Pool::for_test();
        let directory = pool.test_directory().to_path_buf();
        let mut service = lobby_with(pool, &[]);
        let entry = in_party(w(1, MatchPreference::BotPractice, now), 7, 2);
        service.set_party_roster(
            7,
            vec![
                (entry.profile.profile_id.clone(), "Here".into()),
                ("missing".into(), "Late friend".into()),
            ],
        );
        enqueue_all(&mut service, std::slice::from_ref(&entry), 1, now);
        let Some(MatchServiceView::Waiting {
            party_gathering: Some(gathering),
            ..
        }) = service.view(&entry.profile.profile_id, &entry.session, now)
        else {
            panic!("must explain the gathering");
        };
        assert_eq!(gathering.missing, vec!["Late friend"]);
        assert_eq!(gathering.needed, 1);
        assert_eq!(gathering.remaining_secs, 60);
        let before = now + Duration::from_secs(59);
        enqueue_all(&mut service, std::slice::from_ref(&entry), 1, before);
        tick_accepted(&mut service, before);
        assert!(matches!(
            service.view(&entry.profile.profile_id, &entry.session, before),
            Some(MatchServiceView::Waiting { .. })
        ));
        let after = now + PARTY_GATHER_WINDOW;
        enqueue_all(&mut service, std::slice::from_ref(&entry), 1, after);
        tick_accepted(&mut service, after);
        let MatchService::Lobby(lobby) = &service else {
            unreachable!()
        };
        let id = &lobby.assignments[&entry.profile.profile_id];
        assert_eq!(lobby.pool.slots[id].manifest.humans.len(), 1);
        assert_eq!(
            lobby.parked[id][0].queued, now,
            "gathering preserves FIFO age"
        );
        let _ = std::fs::remove_dir_all(directory);
    }
    fn ready_fixture(now: Instant) -> (MatchService, Vec<Waiting>, PathBuf, String) {
        let pool = crate::match_pool::Pool::for_test();
        let directory = pool.test_directory().to_path_buf();
        let mut service = lobby_with(pool, &[]);
        let entries: Vec<_> = (1..=10)
            .map(|i| w(i, MatchPreference::Ranked, now))
            .collect();
        enqueue_all(&mut service, &entries, 7, now);
        service.tick(now);
        let Some(MatchServiceView::ReadyCheck {
            check_id,
            needed: 10,
            accepted_count: 0,
            remaining_secs: 15,
            ..
        }) = service.view(&entries[0].profile.profile_id, &entries[0].session, now)
        else {
            panic!("ready check is required");
        };
        (service, entries, directory, check_id)
    }
    #[test]
    fn ready_check_never_spawns_before_ten_explicit_answers_and_rejects_stale_answers() {
        let now = Instant::now();
        let (mut service, entries, directory, check_id) = ready_fixture(now);
        for entry in &entries[..9] {
            service.respond_ready(
                &entry.profile.profile_id,
                &entry.session,
                7,
                &check_id,
                true,
                now,
            );
            service.respond_ready(
                &entry.profile.profile_id,
                &entry.session,
                7,
                &check_id,
                true,
                now,
            );
        }
        let last = &entries[9];
        for (session, request, id) in [
            ("wrong-session", 7, check_id.as_str()),
            (last.session.as_str(), 6, check_id.as_str()),
            (last.session.as_str(), 7, "old-check"),
        ] {
            service.respond_ready(&last.profile.profile_id, session, request, id, true, now);
        }
        service.tick(now + Duration::from_secs(14));
        let MatchService::Lobby(lobby) = &service else {
            unreachable!()
        };
        assert!(
            lobby.pool.slots.is_empty(),
            "nine accepts must never spawn a worker"
        );
        assert_eq!(lobby.ready[&check_id].accepted.len(), 9);
        service.respond_ready(
            &last.profile.profile_id,
            &last.session,
            7,
            &check_id,
            true,
            now + Duration::from_millis(14_999),
        );
        service.tick(now + READY_WINDOW);
        let MatchService::Lobby(lobby) = service else {
            unreachable!()
        };
        assert_eq!(lobby.pool.slots.len(), 1);
        assert_eq!(lobby.assignments.len(), 10);
        assert!(lobby.ready.is_empty() && lobby.ready_profiles.is_empty());
        drop(lobby);
        let _ = std::fs::remove_dir_all(directory);
    }
    #[test]
    fn ready_timeout_at_exactly_fifteen_seconds_requeues_nine_with_original_fifo_age() {
        let now = Instant::now();
        let (mut service, entries, directory, check_id) = ready_fixture(now);
        for entry in &entries[..9] {
            service.respond_ready(
                &entry.profile.profile_id,
                &entry.session,
                7,
                &check_id,
                true,
                now,
            );
        }
        let last = &entries[9];
        service.respond_ready(
            &last.profile.profile_id,
            &last.session,
            7,
            &check_id,
            true,
            now + READY_WINDOW,
        );
        service.tick(now + READY_WINDOW);
        let MatchService::Lobby(lobby) = &service else {
            unreachable!()
        };
        assert!(lobby.pool.slots.is_empty());
        assert!(lobby.ready.is_empty());
        assert_eq!(
            lobby.waiting.iter().map(|w| w.order).collect::<Vec<_>>(),
            (2..=10).collect::<Vec<_>>()
        );
        assert!(lobby.waiting.iter().all(|w| w.queued == now));
        assert!(
            matches!(service.view(&last.profile.profile_id, &last.session, now + READY_WINDOW), Some(MatchServiceView::Failed {code}) if code == DODGE_COOLDOWN_CODE)
        );
        // A retry cannot reinsert the missing player during their cooldown.
        enqueue_all(
            &mut service,
            std::slice::from_ref(last),
            8,
            now + READY_WINDOW,
        );
        assert_eq!(waiting_orders(&service).len(), 9);
        drop(service);
        let _ = std::fs::remove_dir_all(directory);
    }
    #[test]
    fn ready_decline_returns_others_and_escalating_cooldown_survives_lobby_recreation() {
        let now = Instant::now();
        let (mut service, entries, directory, check_id) = ready_fixture(now);
        let first = &entries[0];
        service.respond_ready(
            &first.profile.profile_id,
            &first.session,
            7,
            &check_id,
            false,
            now,
        );
        service.respond_ready(
            &first.profile.profile_id,
            &first.session,
            7,
            &check_id,
            false,
            now,
        );
        let MatchService::Lobby(mut lobby) = service else {
            unreachable!()
        };
        assert_eq!(
            lobby.waiting.len(),
            9,
            "an immediate decline does not blame the other nine"
        );
        assert_eq!(
            lobby.penalties[&first.profile.profile_id].strikes, 1,
            "duplicate answer is idempotent"
        );
        lobby.penalize_dodge(&first.profile.profile_id, now);
        assert_eq!(
            lobby.refusals[&first.profile.profile_id]
                .1
                .duration_since(now),
            Duration::from_secs(300)
        );
        assert!(lobby.pool.slots.is_empty());
        let mut restored = lobby_with(lobby.pool, &[]);
        enqueue_all(&mut restored, std::slice::from_ref(first), 8, now);
        assert!(waiting_orders(&restored).is_empty());
        assert!(
            matches!(restored.view(&first.profile.profile_id, &first.session, now), Some(MatchServiceView::Failed {code}) if code == DODGE_COOLDOWN_CODE)
        );
        drop(restored);
        let _ = std::fs::remove_dir_all(directory);
    }
    #[test]
    fn ready_decline_does_not_subtract_an_older_party_opt_out_twice() {
        let now = Instant::now();
        let (mut service, entries, directory, check_id) = ready_fixture(now);
        let MatchService::Lobby(lobby) = &mut service else {
            unreachable!()
        };
        lobby
            .party_opt_outs
            .entry(9)
            .or_default()
            .insert("f".repeat(64), now);
        for entry in &mut lobby.ready.get_mut(&check_id).unwrap().entries[..4] {
            entry.party = Some((9, 4));
        }
        service.respond_ready(
            &entries[0].profile.profile_id,
            &entries[0].session,
            7,
            &check_id,
            false,
            now,
        );
        let MatchService::Lobby(lobby) = &service else {
            unreachable!()
        };
        let present: Vec<_> = lobby
            .waiting
            .iter()
            .filter_map(|w| w.party.filter(|(id, _)| *id == 9))
            .collect();
        assert_eq!(present, [(9, 3), (9, 3), (9, 3)]);
        drop(service);
        let _ = std::fs::remove_dir_all(directory);
    }
}
