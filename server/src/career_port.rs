//! The career port: what the simulation asks of the account and result store.
//!
//! `CareerRuntime` and the dispatcher talk to the store only through this
//! trait. The production implementation is `career_backend::CareerBackend`
//! (the signed-account state machine over a bounded PostgreSQL worker
//! thread); tests use `career_backend::MemoryCareer`, the same state machine
//! over an in-memory job link that never blocks and acknowledges
//! deterministically. Every method is non-blocking and runs on the tick
//! thread; the store's own I/O never enters the simulation.
//!
//! The worker-vs-standalone split (`match_service.worker()`) is not part of
//! this contract: it decides *whether* a round is durable or public casual,
//! which the runtime derives at the call site, while the port only records
//! and acknowledges what it is handed.
use shared::career::{CareerRequest, CareerView, MatchResult, ProfileSummary};
use std::{net::SocketAddr, time::Instant};

pub(crate) trait CareerPort {
    /// Whether durable storage is configured; disabled stores accept nothing
    /// and report guest play in the view.
    fn enabled(&self) -> bool;
    fn new_result_id(&self) -> String;

    // Account state per endpoint.
    fn profile(&self, addr: SocketAddr) -> Option<ProfileSummary>;
    fn supporter_aura(&self, addr: SocketAddr) -> Option<shared::supporter::AuraStyle>;
    fn authenticated_session(&self, addr: SocketAddr) -> Option<String>;
    fn gameplay_principal(
        &self,
        addr: SocketAddr,
    ) -> Option<shared::public_transport::GameplayPrincipal>;
    fn view(&self, addr: SocketAddr) -> CareerView;
    fn forget(&mut self, addr: SocketAddr);
    fn touch(&mut self, addr: SocketAddr);
    fn set_playing(&mut self, addr: SocketAddr, playing: bool);
    fn recovery_confirmed_since(&self, since: Instant) -> bool;

    // Signed requests in, side effects out (drained by the runtime each tick).
    fn handle(&mut self, addr: SocketAddr, request: CareerRequest);
    fn poll(&mut self);
    fn take_match_requests(
        &mut self,
    ) -> Vec<(SocketAddr, u64, shared::match_service::MatchPreference)>;
    fn take_cancelled(&mut self) -> Vec<SocketAddr>;
    fn take_social(&mut self) -> Vec<(SocketAddr, shared::social::SocialRequest)>;
    fn take_settled(&mut self) -> Vec<MatchResult>;

    // Durable match records: enqueue returns whether the record was accepted;
    // acknowledgements arrive through `poll`.
    fn start(&mut self, result: MatchResult) -> bool;
    fn checkpoint(&mut self, result: MatchResult) -> bool;
    fn settle(&mut self, result: MatchResult) -> bool;
    fn started(&self, id: &str) -> bool;
    fn start_rejected(&self, id: &str) -> Option<String>;
    fn start_error(&self, id: &str) -> Option<String>;
    fn forget_start(&mut self, id: &str);

    // Test hooks: fixtures drive the store through the port, so the hooks
    // that seed an authenticated endpoint or acknowledge a record by hand are
    // part of the trait under `cfg(test)`; both implementations share them.
    #[cfg(test)]
    fn test_authenticated(&mut self, addr: SocketAddr, profile: ProfileSummary, session_id: &str);
    #[cfg(test)]
    fn test_ack_start(&mut self, id: &str);
    #[cfg(test)]
    fn test_ack_settle(&mut self, result: MatchResult);
    #[cfg(test)]
    fn test_reject_start(&mut self, id: &str, error: &str);
    #[cfg(test)]
    fn test_is_playing(&self, addr: SocketAddr) -> bool;
}
