//! Public lobby matchmaking and the handoff to one allocated arena.
//!
//! The status line shown while searching comes from the `searching`
//! dictionary (`searching.match.*`).
// i18n-strict
use std::time::{Duration, Instant};

use bevy::prelude::*;
use shared::{
    career::CareerView,
    match_service::{MatchAllocation, MatchPreference, MatchServiceView},
};

use crate::{
    career::CareerClient,
    career_identity::CareerIdentity,
    i18n::{tr, trf},
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
    /// The lock-in that started the current search: Play again queues it
    /// again after an allocated match (DECISIONS R7.4).
    last_queue_join: Option<NetworkCommand>,
    /// Play again was pressed after an allocated match: queue again once the
    /// client is back on this lobby.
    requeue: Option<Requeue>,
}

/// A queued "Play again": the lobby to return to and the lock-in to send.
#[derive(Clone, Debug)]
pub(crate) struct Requeue {
    lobby: String,
    join: NetworkCommand,
    since: Instant,
}

/// How long a Play again waits for the lobby (reconnect + career view)
/// before it gives up and leaves the player on Home.
pub(crate) const REQUEUE_TIMEOUT: Duration = Duration::from_secs(20);

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
        self.last_queue_join = Some(command.clone());
        self.lobby_addr = Some(address.to_owned());
        true
    }

    /// Play again after an allocated match: remember to queue the same
    /// lock-in (same hero, same preference) once the client is back on the
    /// lobby. `false` when this is not an allocated match started from the
    /// queue (nothing to repeat).
    pub fn request_requeue(&mut self) -> bool {
        let (Some(_), Some(lobby), Some(join)) = (
            self.allocation.as_ref(),
            self.lobby_addr.clone(),
            self.last_queue_join.clone(),
        ) else {
            return false;
        };
        self.requeue = Some(Requeue {
            lobby,
            join,
            since: Instant::now(),
        });
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
            (
                update_match_service.after(crate::net::ClientNetPipeline::ApplySnapshot),
                resume_requeue
                    .after(crate::net::ClientNetPipeline::ApplySnapshot)
                    .before(crate::frontend::FrontendSet),
            ),
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

/// Sends the remembered lock-in once the client is back on the lobby after a
/// Play again: connected to that address with a career view that offers
/// matchmaking. The join is intercepted like a fresh lock-in (a new search),
/// and the shell shows the search screen. Leaving Home for another screen,
/// or waiting longer than [`REQUEUE_TIMEOUT`], drops it.
fn resume_requeue(
    mut flow: ResMut<MatchServiceClient>,
    career: Res<CareerClient>,
    session: Res<ClientSession>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    mut next: Option<ResMut<NextState<crate::frontend::AppScreen>>>,
    mut requests: MessageWriter<NetworkCommand>,
) {
    use crate::frontend::AppScreen;
    let Some(requeue) = flow.requeue.as_ref() else {
        return;
    };
    let screen = screen.map_or(AppScreen::Home, |screen| *screen.get());
    if requeue.since.elapsed() > REQUEUE_TIMEOUT
        || !matches!(screen, AppScreen::PostMatch | AppScreen::Home)
    {
        flow.requeue = None;
        return;
    }
    if !requeue_ready(requeue, &flow, &career.view, &session, screen) {
        return;
    }
    let Some(requeue) = flow.requeue.take() else {
        return;
    };
    requests.write(requeue.join);
    if let Some(next) = next.as_mut() {
        next.set(AppScreen::Searching);
    }
}

/// The lobby is back: Home, the session connected to the remembered lobby,
/// no search or allocation left over, and matchmaking offered.
fn requeue_ready(
    requeue: &Requeue,
    flow: &MatchServiceClient,
    view: &CareerView,
    session: &ClientSession,
    screen: crate::frontend::AppScreen,
) -> bool {
    screen == crate::frontend::AppScreen::Home
        && !flow.active
        && flow.allocation.is_none()
        && session.state() == crate::net::ClientConnectionState::Connected
        && session.server_addr() == requeue.lobby
        && !session.has_committed_join()
        && view.match_service.is_some()
}

/// The matchmaking status line for the searching screen, in the active language.
pub(crate) fn status_text(view: &MatchServiceView) -> String {
    match view {
        MatchServiceView::Idle => tr("searching.match.connecting").into(),
        MatchServiceView::Waiting {
            preference,
            humans,
            needed,
            elapsed_secs,
            bot_fill_after_secs,
            capacity_wait,
        } => {
            if *capacity_wait {
                return tr("searching.match.capacity").into();
            }
            let policy = match preference {
                MatchPreference::Quick => {
                    bot_fill_after_secs.map_or(tr("searching.match.preparing").into(), |seconds| {
                        trf(
                            "searching.match.bot_fill",
                            &[("seconds", &seconds.saturating_sub(*elapsed_secs))],
                        )
                    })
                }
                MatchPreference::HumansOnly => tr("searching.match.humans_only").into(),
                MatchPreference::BotPractice => tr("searching.match.bot_practice").into(),
            };
            trf(
                "searching.match.waiting",
                &[
                    ("humans", humans),
                    ("needed", needed),
                    ("policy", &policy),
                    ("elapsed", elapsed_secs),
                ],
            )
        }
        MatchServiceView::Allocating => tr("searching.match.allocating").into(),
        MatchServiceView::Assigned { .. } => tr("searching.match.assigned").into(),
        MatchServiceView::Failed { code } => trf("searching.match.failed", &[("code", code)]),
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
    /// The searching status follows the active language.
    #[test]
    fn status_text_follows_the_language() {
        if crate::i18n::testing::isolated("match_service::tests::status_text_follows_the_language")
        {
            return;
        }
        use crate::i18n::{I18nPlugin, Locale, LocaleId};
        let waiting = MatchServiceView::Waiting {
            preference: MatchPreference::Quick,
            humans: 4,
            needed: 10,
            elapsed_secs: 12,
            bot_fill_after_secs: Some(30),
            capacity_wait: false,
        };
        let failed = MatchServiceView::Failed {
            code: "no_capacity".into(),
        };
        let mut app = App::new();
        app.add_plugins(I18nPlugin::default());
        assert_eq!(
            status_text(&waiting),
            "Players found · 4/10\nBots fill empty seats in 18s · waiting 12s"
        );
        assert_eq!(
            status_text(&failed),
            "Could not start the match: no_capacity\nCancel and try again."
        );
        app.world_mut()
            .resource_mut::<Locale>()
            .set(LocaleId::parse("zh-Hans").unwrap());
        assert_eq!(
            status_text(&waiting),
            "已找到玩家 · 4/10\n18 秒后由电脑玩家补满空位 · 已等待 12 秒"
        );
        assert_eq!(
            status_text(&failed),
            "无法开始对局：no_capacity\n请取消后重试。"
        );
        assert_eq!(
            status_text(&MatchServiceView::Allocating),
            "已找到对局 · 正在准备竞技场…"
        );
    }
}
