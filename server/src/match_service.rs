//! One public lobby schedules independent, immutable game workers.
use crate::match_allocation::{AllocatedHuman, Manifest, Phase, Status, Worker, unix_ms};
use crate::*;
use shared::{
    career::ProfileSummary,
    match_service::{MatchAllocation, MatchPreference, MatchServiceView},
};
use std::path::PathBuf;

const MAX_WAITING: usize = 128;
const QUICK_WAIT: Duration = Duration::from_secs(30);
#[derive(Clone)]
struct Waiting {
    profile: ProfileSummary,
    session: String,
    preference: MatchPreference,
    queued: Instant,
    touched: Instant,
    order: u64,
}
pub(crate) struct Lobby {
    pool: crate::match_pool::Pool,
    waiting: Vec<Waiting>,
    assignments: HashMap<String, String>,
    errors: HashMap<String, String>,
    requests: HashMap<String, (String, u64, Instant)>,
    order: u64,
    last_poll: Option<Instant>,
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
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("OMOBA_SERVER_ROLE").ok().as_deref() {
            None | Some("") | Some("standalone") => Ok(Self::Standalone),
            Some("lobby") => {
                let pool = crate::match_pool::Pool::from_env()?;
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
                    pool,
                    waiting: Vec::new(),
                    assignments,
                    errors: HashMap::new(),
                    requests: HashMap::new(),
                    order: 0,
                    last_poll: None,
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
    pub fn enqueue(
        &mut self,
        profile: ProfileSummary,
        session: String,
        request_id: u64,
        preference: MatchPreference,
        now: Instant,
    ) {
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
        lobby.order = lobby.order.saturating_add(1);
        lobby.waiting.push(Waiting {
            profile,
            session,
            preference,
            queued: now,
            touched: now,
            order: lobby.order,
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
                    return Some(if slot.ready() {
                        MatchServiceView::Assigned {
                            allocation: MatchAllocation {
                                allocation_id: id.clone(),
                                endpoint: slot.manifest.endpoint.clone(),
                                preference: slot.manifest.preference,
                                team: h.team,
                                human_count: slot.manifest.humans.len() as u32,
                                bot_count: 10 - slot.manifest.humans.len() as u32,
                                rated: slot.manifest.humans.len() == 10,
                                join_deadline_ms: slot.manifest.join_deadline_ms,
                            },
                        }
                    } else {
                        MatchServiceView::Allocating
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
        if let Some(w) = lobby
            .waiting
            .iter()
            .find(|w| w.profile.profile_id == profile)
        {
            let humans = lobby
                .waiting
                .iter()
                .filter(|other| compatible(w, other))
                .count()
                .min(10) as u32;
            return Some(MatchServiceView::Waiting {
                preference: w.preference,
                humans,
                needed: if w.preference == MatchPreference::BotPractice {
                    1
                } else {
                    10
                },
                elapsed_secs: now.saturating_duration_since(w.queued).as_secs(),
                bot_fill_after_secs: (w.preference == MatchPreference::Quick)
                    .then_some(QUICK_WAIT.as_secs()),
                capacity_wait: lobby.pool.full(),
            });
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
                if slot
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
        // At most four OS process launches per scheduling pass.
        for _ in 0..4 {
            if lobby.pool.full() {
                break;
            }
            let Some(indices) = select(&lobby.waiting, now) else {
                break;
            };
            let mut selected: Vec<_> = indices.iter().map(|&i| lobby.waiting[i].clone()).collect();
            selected.sort_by_key(|w| (std::cmp::Reverse(w.profile.rating), w.order));
            let mut sums = [0_i64; 2];
            let mut counts = [0_usize; 2];
            let humans = selected
                .iter()
                .map(|w| {
                    let team =
                        if counts[0] > counts[1] || (counts[0] == counts[1] && sums[0] > sums[1]) {
                            1
                        } else {
                            0
                        };
                    counts[team] += 1;
                    sums[team] += i64::from(w.profile.rating);
                    AllocatedHuman {
                        profile_id: w.profile.profile_id.clone(),
                        session_id: w.session.clone(),
                        team: if team == 0 {
                            shared::map::Team::Green
                        } else {
                            shared::map::Team::Blue
                        },
                    }
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
                preference: selected[0].preference,
                humans,
                join_deadline_ms: unix_ms() + 180_000,
            };
            match lobby.pool.allocate(manifest) {
                Ok(id) => {
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

fn compatible(a: &Waiting, b: &Waiting) -> bool {
    a.preference == b.preference
        && a.profile.newcomer() == b.profile.newcomer()
        && (i64::from(a.profile.rating) - i64::from(b.profile.rating)).abs() <= 300
}
fn select(waiting: &[Waiting], now: Instant) -> Option<Vec<usize>> {
    let mut best: Option<Vec<usize>> = None;
    for (i, low) in waiting.iter().enumerate() {
        if low.preference == MatchPreference::BotPractice {
            return Some(vec![i]);
        }
        let mut cohort: Vec<_> = waiting
            .iter()
            .enumerate()
            .filter(|(_, w)| compatible(low, w) && w.profile.rating >= low.profile.rating)
            .map(|(i, _)| i)
            .collect();
        cohort.sort_by_key(|&i| waiting[i].order);
        cohort.truncate(10);
        if cohort.len() < 10
            && (low.preference == MatchPreference::HumansOnly
                || cohort
                    .iter()
                    .all(|&i| now.saturating_duration_since(waiting[i].queued) < QUICK_WAIT))
        {
            continue;
        }
        if best.as_ref().is_none_or(|b| {
            cohort
                .iter()
                .map(|&i| waiting[i].order)
                .cmp(b.iter().map(|&i| waiting[i].order))
                .is_lt()
        }) {
            best = Some(cohort);
        }
    }
    best
}
impl ServerRuntime {
    pub(crate) fn send_lobby_snapshots(&mut self, now: Instant) {
        if now.saturating_duration_since(self.last_bootstrap_at) < Duration::from_secs(1) {
            return;
        }
        self.last_bootstrap_at = now;
        self.snapshot_tick = self.snapshot_tick.saturating_add(1);
        for (addr, player) in &self.players {
            if !self.public_transport.validated(*addr, now)
                || (!self.match_service.is_lobby()
                    && self.career.backend.gameplay_principal(*addr).is_some())
            {
                continue;
            }
            let packet = ServerPacket::Snapshot {
                vision: None,
                sandbox: None,
                match_mode: if self.match_service.is_lobby() {
                    "lobby"
                } else {
                    self.match_config.mode_id()
                }
                .into(),
                geometry_id: self.map_config.geometry_id.clone(),
                map_profile: self.map_config.map_profile.clone(),
                meta: shared::protocol::SnapshotMeta::new(
                    self.server_epoch,
                    self.match_id,
                    self.snapshot_tick,
                ),
                join_error: None,
                your_id: player.state.id,
                players: Vec::new(),
                scoreboard: None,
                prematch: None,
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
                let _ = self.socket.send_to(&bytes, addr);
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
        } else if matches!(self.game_state, GameState::Running) {
            Phase::Running
        } else if matches!(self.game_state, GameState::Victory { .. }) {
            if terminal {
                Phase::Finished
            } else {
                Phase::Settling
            }
        } else if self.players.values().any(|p| p.joined) {
            Phase::Forming
        } else {
            Phase::Ready
        };
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
    use super::*;
    fn w(id: u64, p: MatchPreference, now: Instant) -> Waiting {
        Waiting {
            profile: ProfileSummary::new(format!("{id:064x}"), "Player".into()),
            session: format!("s{id}"),
            preference: p,
            queued: now,
            touched: now,
            order: id,
        }
    }
    #[test]
    fn abandoned_queue_expires_without_explicit_find_match_retry() {
        let now = Instant::now();
        let mut waiting = vec![w(1, MatchPreference::HumansOnly, now)];
        expire_waiting(&mut waiting, now + Duration::from_secs(14));
        assert_eq!(waiting.len(), 1);
        // Runtime heartbeats do not touch queue intent. Only enqueue/retry does.
        expire_waiting(&mut waiting, now + Duration::from_secs(15));
        assert!(waiting.is_empty());
        waiting.push(w(2, MatchPreference::HumansOnly, now));
        waiting[0].touched = now + Duration::from_secs(14);
        expire_waiting(&mut waiting, now + Duration::from_secs(15));
        assert_eq!(waiting.len(), 1);
    }

    #[test]
    fn quick_waits_thirty_seconds_and_humans_never_fill() {
        let now = Instant::now();
        let mut q = vec![w(1, MatchPreference::Quick, now)];
        assert!(select(&q, now + Duration::from_secs(29)).is_none());
        assert_eq!(select(&q, now + QUICK_WAIT), Some(vec![0]));
        q[0].preference = MatchPreference::HumansOnly;
        assert!(select(&q, now + Duration::from_secs(999)).is_none());
    }
    #[test]
    fn full_humans_and_practice_allocate_immediately() {
        let now = Instant::now();
        let q: Vec<_> = (1..=10)
            .map(|i| w(i, MatchPreference::HumansOnly, now))
            .collect();
        assert_eq!(select(&q, now).unwrap().len(), 10);
        assert_eq!(
            select(&[w(1, MatchPreference::BotPractice, now)], now),
            Some(vec![0])
        );
    }
    #[test]
    fn rating_windows_and_preferences_stay_separate() {
        let now = Instant::now();
        let mut q: Vec<_> = (1..=10)
            .map(|i| w(i, MatchPreference::HumansOnly, now))
            .collect();
        q[9].profile.rating = 1400;
        assert!(select(&q, now).is_none());
        q[9].profile.rating = 1000;
        q[9].preference = MatchPreference::Quick;
        assert!(select(&q, now).is_none());
    }
}
