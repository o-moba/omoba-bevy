//! Renderer-free coordinator: channel acceptance never counts as SQL consent.
use omoba_career_store::career_store::ready::{Action, Answer, Phase, Seat, Snapshot};
use shared::match_service::MatchPreference;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Reserve {
        preference: MatchPreference,
        seats: Vec<Seat>,
    },
    Update(Action),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Work {
    pub check_id: String,
    pub sequence: u64,
    pub operation: Operation,
}
impl Work {
    pub fn valid(&self) -> bool {
        self.check_id.len() == 32
            && self
                .check_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && self.sequence != 0
            && match &self.operation {
                Operation::Reserve { seats, .. } => {
                    !seats.is_empty()
                        && seats.len() <= 10
                        && seats.iter().all(|s| {
                            shared::career::valid_profile_id(&s.profile_id)
                                && !s.session_id.is_empty()
                                && s.session_id.len() <= 64
                        })
                }
                Operation::Update(Action::Answers(answers)) => {
                    !answers.is_empty()
                        && answers.len() <= 10
                        && answers.iter().all(|a| {
                            shared::career::valid_profile_id(&a.seat.profile_id)
                                && !a.seat.session_id.is_empty()
                                && a.seat.session_id.len() <= 64
                        })
                }
                Operation::Update(Action::Answer {
                    profile_id,
                    session_id,
                    ..
                }) => {
                    shared::career::valid_profile_id(profile_id)
                        && !session_id.is_empty()
                        && session_id.len() <= 64
                }
                _ => true,
            }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Completion {
    pub work: Work,
    pub result: Result<Snapshot, String>,
}
pub(crate) struct Consent {
    id: String,
    preference: MatchPreference,
    seats: Vec<Seat>,
    current: Option<Work>,
    sequence: u64,
    pub snapshot: Option<Snapshot>,
    votes: VecDeque<(Seat, bool)>,
    desired: HashMap<String, bool>,
    cancelling: bool,
    pub failure: Option<String>,
    retry_at: Instant,
    last_poll: Instant,
}
impl Consent {
    pub fn new(id: String, preference: MatchPreference, seats: Vec<Seat>, now: Instant) -> Self {
        Self {
            current: Some(Work {
                check_id: id.clone(),
                sequence: 1,
                operation: Operation::Reserve {
                    preference,
                    seats: seats.clone(),
                },
            }),
            id,
            preference,
            seats,
            sequence: 1,
            snapshot: None,
            votes: VecDeque::new(),
            desired: HashMap::new(),
            cancelling: false,
            failure: None,
            retry_at: now,
            last_poll: now,
        }
    }
    pub fn answer(&mut self, profile: &str, session: &str, accepted: bool) {
        if self.cancelling
            || !self.published()
            || self.desired.get(profile) == Some(&accepted)
            || self.desired.get(profile) == Some(&false)
        {
            return;
        }
        let Some(seat) = self
            .seats
            .iter()
            .find(|s| s.profile_id == profile && s.session_id == session)
            .cloned()
        else {
            return;
        };
        self.desired.insert(profile.into(), accepted);
        // At most one queued answer per seat. An in-flight yes may be followed
        // by no; allocation stays gated until that later response commits.
        self.votes.retain(|(s, _)| s.profile_id != profile);
        if accepted {
            self.votes.push_back((seat, true));
        } else {
            let index = self
                .votes
                .iter()
                .position(|(_, accepted)| *accepted)
                .unwrap_or(self.votes.len());
            self.votes.insert(index, (seat, false));
        }
    }
    pub fn cancel(&mut self) {
        self.cancelling = true;
        self.votes.clear();
    }
    pub fn published(&self) -> bool {
        !self.cancelling
            && self
                .snapshot
                .as_ref()
                .is_some_and(|s| matches!(s.phase, Phase::Open | Phase::Approved))
    }
    pub fn approved(&self) -> bool {
        !self.cancelling
            && self.failure.is_none()
            && self.votes.is_empty()
            && !self.current.as_ref().is_some_and(|w| {
                matches!(
                    w.operation,
                    Operation::Update(Action::Answer { .. } | Action::Answers(_) | Action::Cancel)
                )
            })
            && self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.phase == Phase::Approved)
    }
    pub fn terminal(&self) -> bool {
        self.failure.is_some()
            || self.snapshot.as_ref().is_some_and(|s| {
                matches!(s.phase, Phase::Cancelled | Phase::Expired | Phase::Consumed)
            })
    }
    pub fn deadline_ms(&self) -> Option<i64> {
        let s = self.snapshot.as_ref()?;
        s.opened_at_ms.map(|opened| opened.saturating_add(15_000))
    }
    pub fn next(&mut self, now: Instant) -> Option<Work> {
        if now < self.retry_at || self.terminal() {
            return None;
        }
        if let Some(w) = &self.current {
            return Some(w.clone());
        }
        let phase = self.snapshot.as_ref().map_or(Phase::Forming, |s| s.phase);
        let operation = if self.cancelling {
            Operation::Update(Action::Cancel)
        } else if phase == Phase::Forming {
            Operation::Update(Action::Publish)
        } else if now.saturating_duration_since(self.last_poll) >= Duration::from_millis(500) {
            self.last_poll = now;
            Operation::Update(Action::Poll)
        } else if !self.votes.is_empty() {
            Operation::Update(Action::Answers(
                self.votes
                    .drain(..)
                    .map(|(seat, accepted)| Answer { seat, accepted })
                    .collect(),
            ))
        } else {
            return None;
        };
        self.sequence += 1;
        let work = Work {
            check_id: self.id.clone(),
            sequence: self.sequence,
            operation,
        };
        self.current = Some(work.clone());
        Some(work)
    }
    pub fn complete(&mut self, completion: Completion, now: Instant) {
        if self.current.as_ref() != Some(&completion.work) {
            return;
        }
        match completion.result {
            Ok(snapshot) => {
                let same_roster = snapshot.members.len() == self.seats.len()
                    && snapshot
                        .members
                        .iter()
                        .all(|m| self.seats.contains(&m.seat));
                let closed = matches!(
                    snapshot.phase,
                    Phase::Cancelled | Phase::Expired | Phase::Consumed
                );
                if snapshot.check_id != self.id
                    || snapshot.preference != self.preference
                    || (!same_roster && !closed)
                {
                    self.current = None;
                    self.cancel();
                    return;
                }
                self.snapshot = Some(snapshot);
                if matches!(
                    completion.work.operation,
                    Operation::Update(
                        Action::Publish | Action::Poll | Action::Answer { .. } | Action::Answers(_)
                    )
                ) {
                    self.last_poll = now;
                }
                self.current = None;
                self.retry_at = now;
            }
            Err(error) => {
                use omoba_career_store::career_store::ready::{CLOSED, OWNED_ELSEWHERE};
                if matches!(
                    error.as_str(),
                    CLOSED
                        | OWNED_ELSEWHERE
                        | shared::career::error_code::DODGE_COOLDOWN
                        | shared::career::error_code::PLAY_COOLDOWN
                        | shared::career::error_code::RANKED_LOCKED
                        | shared::career::error_code::RANKED_SEASON_ENDED
                ) || error.starts_with("Invalid ready")
                    || error.starts_with("Duplicate ready")
                    || error.starts_with("Ready identity conflicts")
                    || error.starts_with("Ready response does not belong")
                {
                    self.failure = Some(error);
                    self.current = None;
                } else {
                    self.retry_at = now + Duration::from_secs(1);
                }
            }
        }
    }
}

pub(crate) async fn execute(
    store: omoba_career_store::career_store::CareerStore,
    work: Work,
) -> Completion {
    let result = match &work.operation {
        Operation::Reserve { preference, seats } => {
            store
                .ready_reserve(&work.check_id, *preference, seats.clone())
                .await
        }
        Operation::Update(action) => store.ready_update(&work.check_id, action.clone()).await,
    };
    Completion { work, result }
}

#[cfg(test)]
#[path = "ready_consent_tests.rs"]
mod tests;
