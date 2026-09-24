//! Public lobby matchmaking and the handoff to one allocated arena.
use std::time::{Duration, Instant};

use bevy::prelude::*;
use shared::{
    career::CareerView,
    match_service::{MatchAllocation, MatchPreference, MatchServiceView},
};

use crate::{
    career::CareerClient,
    career_identity::CareerIdentity,
    net::{ClientSession, GameStateSnapshot, NetworkCommand, SessionUiCommand},
    persistence::ClientSessionId,
};

#[derive(Resource, Default)]
pub(crate) struct MatchServiceClient {
    pub preference: MatchPreference,
    pub pending_join: Option<NetworkCommand>,
    pub lobby_addr: Option<String>,
    pub allocation: Option<MatchAllocation>,
    pub active: bool,
    request_id: u64,
    last_request: Option<Instant>,
}

impl MatchServiceClient {
    pub fn intercept_join(
        &mut self,
        view: &CareerView,
        address: &str,
        command: &NetworkCommand,
    ) -> bool {
        if view.match_service.is_none()
            || self.allocation.is_some()
            || !matches!(command, NetworkCommand::JoinPrematch { .. })
        {
            return false;
        }
        if !self.active {
            self.request_id = self.request_id.saturating_add(1);
            self.last_request = None;
        }
        self.active = true;
        self.pending_join = Some(command.clone());
        self.lobby_addr = Some(address.to_owned());
        true
    }

    pub fn is_searching(&self) -> bool {
        self.active && (self.pending_join.is_some() || self.allocation.is_none())
    }

    pub fn take_return_to_lobby(&mut self) -> Option<String> {
        let destination = self.allocation.take().and(self.lobby_addr.take());
        self.active = false;
        self.pending_join = None;
        self.last_request = None;
        self.lobby_addr = None;
        destination
    }
}

pub(crate) struct MatchServicePlugin;
impl Plugin for MatchServicePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MatchServiceClient>().add_systems(
            Update,
            update_match_service.after(crate::net::ClientNetPipeline::ApplySnapshot),
        );
    }
}

fn update_match_service(
    mut flow: ResMut<MatchServiceClient>,
    career: Res<CareerClient>,
    mut session: ResMut<ClientSession>,
    identity: Res<CareerIdentity>,
    snapshot: Res<GameStateSnapshot>,
    session_id: Res<ClientSessionId>,
    mut requests: MessageWriter<NetworkCommand>,
    mut session_ui: MessageWriter<SessionUiCommand>,
) {
    if !flow.active {
        return;
    }
    let authenticated = identity.authenticated_for_scope(
        session.server_addr(),
        snapshot.meta.server_epoch,
        &session_id.0,
    );
    if let Some(allocation) = &flow.allocation {
        // A durably completed worker will retire. It no longer needs reconnect
        // retries; the result screen retains its validated receipt until leaving.
        if matches!(snapshot.state, crate::net::GameState::Victory { .. })
            && career.view.last_result.as_ref().is_some_and(|result| {
                result.saved
                    && result.server_epoch == snapshot.meta.server_epoch
                    && result.match_id == snapshot.meta.match_id
            })
        {
            session.abandon_join();
            return;
        }
        if session.server_addr() == allocation.endpoint
            && authenticated
            && let Some(join) = flow.pending_join.take()
        {
            requests.write(join);
        }
        return;
    }
    if career.view.match_service_request_id == Some(flow.request_id)
        && let Some(MatchServiceView::Assigned { allocation }) = &career.view.match_service
    {
        if crate::persistence::validate_game_server_addr(&allocation.endpoint).is_some() {
            flow.allocation = Some(allocation.clone());
            session_ui.write(SessionUiCommand::ConnectAllocated(
                allocation.endpoint.clone(),
            ));
        }
        return;
    }
    if !authenticated {
        return;
    }
    let now = Instant::now();
    if flow
        .last_request
        .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(2))
    {
        flow.last_request = Some(now);
        requests.write(NetworkCommand::Career(
            shared::career::CareerRequest::FindMatch {
                request_id: flow.request_id,
                preference: flow.preference,
            },
        ));
    }
}

pub(crate) fn status_text(view: &MatchServiceView) -> String {
    match view {
        MatchServiceView::Idle => "Connecting your profile to matchmaking…".into(),
        MatchServiceView::Waiting {
            preference,
            humans,
            needed,
            elapsed_secs,
            bot_fill_after_secs,
            capacity_wait,
        } => {
            if *capacity_wait {
                return "All arenas are busy. Your place in the queue is reserved.".into();
            }
            let policy = match preference {
                MatchPreference::Quick => {
                    bot_fill_after_secs.map_or("Preparing your match…".into(), |seconds| {
                        format!(
                            "Bots fill empty seats in {}s",
                            seconds.saturating_sub(*elapsed_secs)
                        )
                    })
                }
                MatchPreference::HumansOnly => "Waiting for a full human roster · no bots".into(),
                MatchPreference::BotPractice => "Preparing your bot match…".into(),
            };
            format!("Players found · {humans}/{needed}\n{policy} · waiting {elapsed_secs}s")
        }
        MatchServiceView::Allocating => "Match found · preparing the arena…".into(),
        MatchServiceView::Assigned { .. } => "Match found · connecting to your team…".into(),
        MatchServiceView::Failed { code } => {
            format!("Could not start the match: {code}\nCancel and try again.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_discards_pending_handoff() {
        let mut flow = MatchServiceClient {
            active: true,
            lobby_addr: Some("127.0.0.1:4000".into()),
            ..default()
        };
        assert!(flow.is_searching());
        assert_eq!(flow.take_return_to_lobby(), None);
        assert!(!flow.is_searching());
        assert!(flow.lobby_addr.is_none());
    }
    #[test]
    fn human_only_copy_never_promises_bot_fallback() {
        let text = status_text(&MatchServiceView::Waiting {
            preference: MatchPreference::HumansOnly,
            humans: 3,
            needed: 10,
            elapsed_secs: 50,
            bot_fill_after_secs: None,
            capacity_wait: false,
        });
        assert!(text.contains("3/10"));
        assert!(text.contains("no bots"));
        assert!(!text.contains("fill empty"));
    }
}
