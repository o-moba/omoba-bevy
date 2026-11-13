//! Opt-in compiled directory routing. No regional fetch starts before the
//! existing legal/region startup gate. A live party/queue/worker keeps its owner.
use super::{ClientSession, SessionUiCommand};
use crate::game_directory::{
    DirectoryError,
    admission::{Decision, Resolver, ResumeDecision, Route},
    config::BuildTrust,
};
use bevy::prelude::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectoryNotice {
    Fetching,
    Unavailable,
    Maintenance,
    Full,
    Incompatible,
    UpdateRequired,
}
impl DirectoryNotice {
    pub(crate) fn short_message(self) -> String {
        crate::i18n::tr(match self {
            Self::Fetching => "home.connection.connecting",
            Self::Unavailable | Self::Incompatible => "net.directory.unavailable_short",
            Self::Maintenance => "net.directory.maintenance_short",
            Self::Full => "net.directory.full_short",
            Self::UpdateRequired => "update.pill.required",
        })
        .to_owned()
    }
    pub(crate) fn message(self) -> String {
        crate::i18n::tr(match self {
            Self::Fetching => "net.directory.fetching",
            Self::Unavailable => "net.directory.unavailable",
            Self::Maintenance => "net.directory.maintenance",
            Self::Full => "net.directory.full",
            Self::Incompatible => "net.directory.incompatible",
            Self::UpdateRequired => "update.pill.required",
        })
        .to_owned()
    }
}
fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |now| now.as_millis() as u64)
}

#[derive(Resource)]
pub(crate) struct DirectoryRouting {
    config: Result<Option<BuildTrust>, DirectoryError>,
    resolver: Option<Resolver>,
    pub(super) active: bool,
    boot_pending: bool,
    return_owner: bool,
    selected: Option<Route>,
    hint: Option<super::recovery::SavedResume>,
    pending_worker: Option<String>,
    last_refresh: Option<Instant>,
    last_policy: Option<Instant>,
    entropy: u64,
    clock: fn() -> u64,
}
impl Default for DirectoryRouting {
    fn default() -> Self {
        Self::with_config(BuildTrust::compiled())
    }
}
impl DirectoryRouting {
    fn with_config(config: Result<Option<BuildTrust>, DirectoryError>) -> Self {
        Self {
            config,
            resolver: None,
            active: false,
            boot_pending: false,
            return_owner: false,
            selected: None,
            hint: None,
            pending_worker: None,
            last_refresh: None,
            last_policy: None,
            entropy: 0,
            clock: wall_ms,
        }
    }
    pub(super) fn configured(&self) -> bool {
        !matches!(self.config, Ok(None))
    }
    pub(super) fn begin(
        &mut self,
        session: &mut ClientSession,
        identity: &str,
        return_owner: bool,
    ) {
        self.active = true;
        self.boot_pending = true;
        self.return_owner = return_owner && self.selected.is_some();
        self.pending_worker = None;
        self.hint = if self.return_owner {
            None
        } else {
            super::recovery::directory_resume_hint(identity)
        };
        session.directory_notice = Some(DirectoryNotice::Fetching);
        session.directory_update_url = None;
        session.state = super::ClientConnectionState::Connecting;
        session.waiting_since = None;
        session.ephemeral_endpoint = false;
        session.server_addr_display.clear();
        if self.resolver.is_none() {
            let region = shared::region::active().map(|region| region.id);
            let settings = crate::connection_profile::profile_file("directory-cache");
            if let (Some(region), Some(settings)) = (region, settings) {
                self.initialize(region, settings);
            }
        }
        self.refresh(true);
        if self.resolver.is_none() {
            session.directory_notice = Some(DirectoryNotice::Unavailable);
        }
    }
    fn initialize(&mut self, region: shared::region::RegionId, settings: PathBuf) {
        let Ok(Some(config)) = &self.config else {
            return;
        };
        let Ok(region) = config.region(region) else {
            return;
        };
        self.resolver = region
            .admission(&settings)
            .ok()
            .and_then(|a| Resolver::new(a).ok());
        let mut bytes = [0; 8];
        if getrandom::fill(&mut bytes).is_ok() {
            self.entropy = u64::from_le_bytes(bytes);
        }
    }
    pub(super) fn refresh(&mut self, manual: bool) {
        let interval = Duration::from_secs(if manual { 5 } else { 30 });
        if self.last_refresh.is_some_and(|at| at.elapsed() < interval) {
            return;
        }
        if let Some(resolver) = &mut self.resolver {
            if resolver.begin_refresh() {
                self.last_refresh = Some(Instant::now());
            }
        }
    }
    fn decision(&self) -> Decision {
        let Some(resolver) = &self.resolver else {
            return Decision::Unavailable;
        };
        let owner = self.selected.clone().map(Route::retain);
        resolver.admission().decision_at(
            (self.clock)(),
            &shared::compatibility::ReleaseContract::current(),
            self.entropy,
            owner.as_ref(),
        )
    }
    pub(super) fn blocks_new_admission(&self) -> bool {
        self.active && !matches!(self.decision(), Decision::Route(_))
    }
    pub(super) fn pending_handoff(&self) -> bool {
        self.pending_worker.is_some()
    }
    pub(super) fn worker(&self, raw: &str) -> Option<String> {
        let (Some(resolver), Some(owner)) = (&self.resolver, &self.selected) else {
            return None;
        };
        match resolver
            .admission()
            .resume_at((self.clock)(), &owner.endpoint().to_string(), raw)
        {
            ResumeDecision::Ready(route) => Some(route.worker().to_string()),
            _ => None,
        }
    }
    pub(super) fn defer_worker(&mut self, raw: &str) {
        self.pending_worker = Some(raw.to_owned());
        self.refresh(true);
    }
    /// Association for persisted hints only; every fresh transport separately
    /// revalidates current ownership, including expiry and removals.
    pub(super) fn owned_lobby(&self) -> Option<String> {
        self.selected
            .as_ref()
            .map(|route| route.endpoint().to_string())
    }
    pub(super) fn reconnect_endpoint(&self, session: &ClientSession) -> Option<String> {
        if session.ephemeral_endpoint {
            return self.worker(session.server_addr());
        }
        let resolver = self.resolver.as_ref()?;
        match resolver
            .admission()
            .owner_at((self.clock)(), session.server_addr())
        {
            Decision::Route(route) => Some(route.endpoint().to_string()),
            _ => None,
        }
    }
    pub(super) fn cancel_handoff(&mut self) {
        self.pending_worker = None;
        self.boot_pending = false;
    }
    pub(super) fn complete_handoff(&mut self) {
        self.pending_worker = None;
    }
    pub(super) fn rejects(
        &self,
        command: &super::NetworkCommand,
        session: &ClientSession,
        flow: Option<&crate::match_service::MatchServiceClient>,
    ) -> bool {
        use super::NetworkCommand;
        if !self.active || session.is_offline() {
            return false;
        }
        if (self.boot_pending || self.pending_handoff())
            && matches!(
                command,
                NetworkCommand::Join { .. } | NetworkCommand::JoinPrematch { .. }
            )
        {
            return true;
        }
        if session.ephemeral_endpoint
            || session.has_committed_join()
            || !self.blocks_new_admission()
        {
            return false;
        }
        match command {
            NetworkCommand::Join { .. } => true,
            NetworkCommand::JoinPrematch { .. } => !flow.is_some_and(|flow| flow.active),
            NetworkCommand::Party(shared::party::PartyCommand::Launch { .. }) => true,
            NetworkCommand::Career(shared::career::CareerRequest::FindMatch {
                request_id, ..
            }) => !flow.is_some_and(|flow| flow.queue_request_is_active(*request_id)),
            _ => false,
        }
    }
    fn apply_policy(&self, session: &mut ClientSession) {
        let (notice, url) = match self.decision() {
            Decision::Pending => (Some(DirectoryNotice::Fetching), None),
            Decision::Route(_) => (None, None),
            Decision::UpdateRequired { url } => (Some(DirectoryNotice::UpdateRequired), url),
            Decision::Maintenance { .. } => (Some(DirectoryNotice::Maintenance), None),
            Decision::Full => (Some(DirectoryNotice::Full), None),
            Decision::Incompatible => (Some(DirectoryNotice::Incompatible), None),
            Decision::Unavailable => (Some(DirectoryNotice::Unavailable), None),
        };
        if session.directory_notice != notice || session.directory_update_url != url {
            session.directory_notice = notice;
            session.directory_update_url = url;
        }
    }
}

pub(super) fn poll_directory(
    mut commands: Commands,
    mut routing: ResMut<DirectoryRouting>,
    mut session: ResMut<ClientSession>,
    mut resume: Option<ResMut<super::recovery::ResumeMatchState>>,
    flow: Option<Res<crate::match_service::MatchServiceClient>>,
    mut session_ui: MessageWriter<SessionUiCommand>,
) {
    if !routing.active {
        return;
    }
    if let Some(resolver) = &mut routing.resolver {
        resolver.poll();
    }
    routing.refresh(false);
    if session.is_offline() {
        return;
    }
    if let Some(raw) = routing.pending_worker.clone() {
        if let Some(endpoint) = routing.worker(&raw) {
            // Keep the guard until lifecycle actually swaps transport. Command
            // sending runs earlier and must not send the arena Join to a lobby.
            session_ui.write(SessionUiCommand::ConnectAllocated(endpoint));
        }
    }
    if session.ephemeral_endpoint || session.has_committed_join() {
        return;
    }
    if routing.boot_pending {
        let Some(resolver) = &routing.resolver else {
            routing.apply_policy(&mut session);
            return;
        };
        let now = (routing.clock)();
        let route = if let Some(hint) = &routing.hint {
            match resolver
                .admission()
                .resume_at(now, &hint.lobby, &hint.allocation.endpoint)
            {
                ResumeDecision::Pending => {
                    session.directory_notice = Some(DirectoryNotice::Fetching);
                    return;
                }
                ResumeDecision::Unavailable => {
                    session.directory_notice = Some(DirectoryNotice::Unavailable);
                    return;
                }
                ResumeDecision::Ready(route) => {
                    if let Some(resume) = resume.as_mut() {
                        let mut saved = hint.clone();
                        saved.lobby = route.lobby().endpoint().to_string();
                        saved.allocation.endpoint = route.worker().to_string();
                        resume.saved = Some(saved);
                    }
                    route.lobby().clone()
                }
            }
        } else if routing.return_owner {
            let Some(owner) = &routing.selected else {
                return;
            };
            match resolver
                .admission()
                .owner_at(now, &owner.endpoint().to_string())
            {
                Decision::Route(route) => route,
                _ => {
                    session.directory_notice = Some(DirectoryNotice::Unavailable);
                    return;
                }
            }
        } else {
            match routing.decision() {
                Decision::Route(route) => route,
                _ => {
                    routing.apply_policy(&mut session);
                    return;
                }
            }
        };
        let endpoint = route.endpoint().to_string();
        routing.selected = Some(route);
        routing.boot_pending = false;
        super::transport::spawn_network_transport(&mut commands, &mut session, endpoint);
        routing.last_policy = None;
    }
    if flow.as_ref().is_some_and(|flow| flow.active) {
        return;
    }
    if routing
        .last_policy
        .is_none_or(|at| at.elapsed() >= Duration::from_millis(250))
    {
        routing.apply_policy(&mut session);
        routing.last_policy = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests;
