//! Authoritative team votes; counts and eligibility are supplied by the worker.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Remake,
    Surrender,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Start,
    Yes,
    No,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eligibility {
    pub available: bool,
    pub starts_in_secs: u64,
    /// Stable localization key, never a client authorization decision.
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Active {
    pub kind: Kind,
    pub generation: u64,
    pub initiator: u64,
    pub initiator_name: String,
    pub yes: u32,
    pub no: u32,
    pub required: u32,
    pub eligible: u32,
    pub remaining_secs: u64,
    pub my_vote: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub server_epoch: u64,
    pub match_id: u64,
    /// None if the generation counter has been exhausted; it never wraps.
    pub next_generation: Option<u64>,
    pub cooldown_secs: u64,
    pub remake: Eligibility,
    pub surrender: Eligibility,
    pub active: Option<Active>,
}
