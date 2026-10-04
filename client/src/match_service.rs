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
    resume_scope: Option<(u64, u64)>,
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
    /// A fresh search must not re-enter a worker whose seat we already left.
    /// Its terminal disk receipt can lag the client result or reconnect cutoff.
    last_left_allocation: Option<String>,
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
    pub(crate) fn restore_resume(&mut self, saved: &crate::net::recovery::SavedResume) {
        self.allocation = Some(saved.allocation.clone());
        self.lobby_addr = Some(saved.lobby.clone());
        self.active = true;
        self.resume_scope = Some((saved.server_epoch, saved.match_id));
        self.pending_join = Some(NetworkCommand::JoinPrematch {
            handheld: saved.join.handheld.clone(),
            character: saved.join.character,
            hero_class: saved.join.hero_class,
            avatar: saved.join.avatar.clone(),
            sprite_character: saved.join.sprite_character.clone(),
        });
        self.last_queue_join = self.pending_join.clone();
        self.last_request = None;
    }

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
        let allocation = self.allocation.take();
        if let Some(allocation) = &allocation {
            self.last_left_allocation = Some(allocation.allocation_id.clone());
        }
        let destination = allocation.and(self.lobby_addr.take());
        self.active = false;
        self.resume_scope = None;
        self.pending_join = None;
        self.last_request = None;
        self.lobby_addr = None;
        destination
    }
}

pub(crate) struct MatchServicePlugin;
impl Plugin for MatchServicePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::net::recovery_ui::RecoveryUiPlugin)
            .init_resource::<crate::net::recovery::ResumeMatchState>()
            .add_systems(
                Startup,
                crate::net::recovery::load_resume
                    .after(crate::persistence::load_persistent_client_settings),
            )
            .add_systems(
                Update,
                (
                    crate::net::recovery::remember_resume,
                    crate::net::recovery::resume_actions,
                )
                    .chain()
                    .after(crate::net::ClientNetPipeline::ApplySnapshot)
                    .before(crate::net::ClientNetPipeline::SessionLifecycle),
            )
            .init_resource::<MatchServiceClient>()
            .add_systems(
                Update,
                (
                    update_match_service
                        .after(crate::net::ClientNetPipeline::ApplySnapshot)
                        .before(crate::net::ClientNetPipeline::SessionLifecycle)
                        .before(crate::frontend::FrontendSet),
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
    mut resume: Option<ResMut<crate::net::recovery::ResumeMatchState>>,
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
        if session.server_addr() == allocation.endpoint
            && session.is_connected()
            && flow
                .resume_scope
                .is_some_and(|scope| scope != (snapshot.meta.server_epoch, snapshot.meta.match_id))
        {
            if let Some(resume) = resume.as_mut() {
                resume.clear();
            }
            flow.pending_join = None;
            session_ui.write(SessionUiCommand::LeaveMatch);
            return;
        }
        if flow.resume_scope == Some((snapshot.meta.server_epoch, snapshot.meta.match_id))
            && authenticated
        {
            session.defer_active_seat_rejection();
        }
        if flow.resume_scope.is_some() && session.join_blocked() {
            // Return through the normal cleanup once. The saved record remains
            // available for an explicit later attempt; no automatic loop from Home.
            session.abandon_join();
            flow.pending_join = None;
            flow.active = false;
            session_ui.write(SessionUiCommand::LeaveMatch);
            return;
        }
        // A terminal receipt ends gameplay even when its durable save is pending.
        // Keep the transport for receipt updates, but never rejoin this round.
        if career.view.last_result.as_ref().is_some_and(|result| {
            result.server_epoch == snapshot.meta.server_epoch
                && result.match_id == snapshot.meta.match_id
        }) {
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
        && flow.last_left_allocation.as_deref() != Some(allocation.allocation_id.as_str())
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
    fn allocation() -> MatchAllocation {
        MatchAllocation {
            allocation_id: "allocated".into(),
            endpoint: "127.0.0.1:4001".into(),
            preference: MatchPreference::Quick,
            team: shared::map::Team::Green,
            human_count: 1,
            bot_count: 9,
            rated: false,
            join_deadline_ms: 0,
        }
    }

    fn lock_in() -> NetworkCommand {
        NetworkCommand::JoinPrematch {
            handheld: Default::default(),
            character: crate::team::CharacterChoice::default(),
            hero_class: shared::HeroClass::Mage,
            avatar: Some("agnes".into()),
            sprite_character: None,
        }
    }

    /// DECISIONS R7.4: Play again after an allocated match remembers the
    /// lobby and the same lock-in, survives the return to the lobby, and is
    /// sent once that lobby offers matchmaking again on Home.
    #[test]
    fn play_again_after_an_allocated_match_queues_the_same_lock_in_on_the_lobby() {
        use crate::frontend::AppScreen;
        let view = CareerView {
            match_service: Some(MatchServiceView::Idle),
            ..default()
        };
        let mut flow = MatchServiceClient::default();
        assert!(!flow.request_requeue(), "nothing to repeat without a queue");
        assert!(flow.intercept_join(&view, "127.0.0.1:4000", &lock_in()));
        flow.allocation = Some(allocation());
        flow.pending_join = None;
        assert!(flow.request_requeue());
        // LeaveMatch returns to the lobby and ends the old search …
        assert_eq!(
            flow.take_return_to_lobby().as_deref(),
            Some("127.0.0.1:4000")
        );
        // … but not the Play again.
        let requeue = flow.requeue.clone().expect("kept across the return");
        assert!(matches!(requeue.join, NetworkCommand::JoinPrematch { .. }));
        let mut session = ClientSession::default();
        session.set_state_for_test(crate::net::ClientConnectionState::Connected);
        let ready =
            |flow: &MatchServiceClient, view: &CareerView, session: &ClientSession, screen| {
                requeue_ready(flow.requeue.as_ref().unwrap(), flow, view, session, screen)
            };
        // Still on the result screen, or on the worker, or before the lobby's
        // career view: wait.
        assert!(!ready(&flow, &view, &session, AppScreen::PostMatch));
        assert!(
            !ready(&flow, &view, &session, AppScreen::Home),
            "other address"
        );
        session.set_server_addr_for_test("127.0.0.1:4000");
        assert!(!ready(
            &flow,
            &CareerView::default(),
            &session,
            AppScreen::Home
        ));
        assert!(ready(&flow, &view, &session, AppScreen::Home));
    }

    #[test]
    fn play_again_follows_the_lobby_into_the_search_and_gives_up_elsewhere() {
        use crate::frontend::AppScreen;
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<AppScreen>()
            .init_resource::<CareerClient>()
            .add_message::<NetworkCommand>()
            .add_systems(Update, resume_requeue);
        let mut session = ClientSession::default();
        session.set_state_for_test(crate::net::ClientConnectionState::Connected);
        session.set_server_addr_for_test("127.0.0.1:4000");
        app.insert_resource(session);
        app.world_mut()
            .resource_mut::<CareerClient>()
            .view
            .match_service = Some(MatchServiceView::Idle);
        let mut flow = MatchServiceClient {
            requeue: Some(Requeue {
                lobby: "127.0.0.1:4000".into(),
                join: lock_in(),
                since: Instant::now(),
            }),
            ..default()
        };
        app.insert_resource(flow);
        app.update();
        app.update();
        assert_eq!(
            *app.world().resource::<State<AppScreen>>().get(),
            AppScreen::Searching
        );
        let sent = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .filter(|command| matches!(command, NetworkCommand::JoinPrematch { .. }))
            .count();
        assert_eq!(sent, 1, "the same lock-in, once");
        assert!(
            app.world()
                .resource::<MatchServiceClient>()
                .requeue
                .is_none()
        );

        // Leaving Home for another screen drops it.
        flow = MatchServiceClient {
            requeue: Some(Requeue {
                lobby: "127.0.0.1:4000".into(),
                join: lock_in(),
                since: Instant::now(),
            }),
            ..default()
        };
        app.insert_resource(flow);
        app.world_mut()
            .resource_mut::<NextState<AppScreen>>()
            .set(AppScreen::Collection);
        app.update();
        app.update();
        assert!(
            app.world()
                .resource::<MatchServiceClient>()
                .requeue
                .is_none()
        );
    }

    #[test]
    fn saved_resume_rejects_reused_endpoint_before_sending_hero_join() {
        let mut flow = MatchServiceClient {
            active: true,
            allocation: Some(allocation()),
            pending_join: Some(lock_in()),
            resume_scope: Some((7, 2)),
            ..default()
        };
        let endpoint = flow.allocation.as_ref().unwrap().endpoint.clone();
        flow.lobby_addr = Some("127.0.0.1:4000".into());
        let mut session = ClientSession::queued_for_test();
        session.set_server_addr_for_test(endpoint);
        let mut app = App::new();
        app.init_resource::<CareerClient>()
            .init_resource::<CareerIdentity>()
            .init_resource::<ClientSessionId>()
            .add_message::<NetworkCommand>()
            .add_message::<SessionUiCommand>()
            .insert_resource(flow)
            .insert_resource(session)
            .insert_resource(GameStateSnapshot {
                meta: shared::protocol::SnapshotMeta::new(8, 1, 1),
                ..default()
            })
            .add_systems(Update, update_match_service);
        app.update();
        assert!(
            app.world()
                .resource::<Messages<NetworkCommand>>()
                .is_empty()
        );
        assert!(
            app.world()
                .resource::<MatchServiceClient>()
                .pending_join
                .is_none()
        );
        assert!(
            app.world_mut()
                .resource_mut::<Messages<SessionUiCommand>>()
                .drain()
                .any(|event| matches!(event, SessionUiCommand::LeaveMatch))
        );
    }

    #[test]
    fn terminal_abandoned_allocation_stops_rejoin_without_a_victory_snapshot() {
        use shared::career::{MatchOutcome, MatchResult};
        let mut app = App::new();
        app.init_resource::<CareerClient>()
            .init_resource::<CareerIdentity>()
            .init_resource::<ClientSessionId>()
            .add_message::<NetworkCommand>()
            .add_message::<SessionUiCommand>()
            .insert_resource(ClientSession::reconnecting_for_test())
            .insert_resource(MatchServiceClient {
                active: true,
                allocation: Some(allocation()),
                pending_join: Some(lock_in()),
                ..default()
            })
            .insert_resource(GameStateSnapshot {
                meta: shared::protocol::SnapshotMeta::new(7, 2, 5),
                state: crate::net::GameState::Running,
                ..default()
            })
            .add_systems(Update, update_match_service);
        let result = MatchResult {
            result_id: "abandoned".into(),
            server_epoch: 7,
            match_id: 1,
            started_at_ms: 0,
            ended_at_ms: 1000,
            duration_ms: 1000,
            map_profile: "verdant_default".into(),
            ruleset: "public-casual-v1".into(),
            outcome: MatchOutcome::Abandoned,
            winner: None,
            rated: false,
            unrated_reason: None,
            participants: vec![],
            saved: false,
        };
        app.world_mut()
            .resource_mut::<CareerClient>()
            .view
            .last_result = Some(result);
        app.update();
        assert!(
            app.world().resource::<ClientSession>().has_committed_join(),
            "previous receipt must not stop a new match"
        );
        app.world_mut()
            .resource_mut::<CareerClient>()
            .view
            .last_result
            .as_mut()
            .unwrap()
            .match_id = 2;
        app.update();
        assert!(!app.world().resource::<ClientSession>().has_committed_join());
        assert!(
            app.world()
                .resource::<MatchServiceClient>()
                .allocation
                .is_some(),
            "Home/Play again still need the return address"
        );
        // The result can arrive before either the worker's disk receipt or the
        // lobby cache leaves Running. Leaving also covers the client's 180s
        // cutoff while the server still retains its seat until about 185s.
        let lobby = "127.0.0.1:4000";
        let lobby_view = CareerView {
            match_service: Some(MatchServiceView::Idle),
            ..default()
        };
        let request_id = {
            let mut flow = app.world_mut().resource_mut::<MatchServiceClient>();
            flow.lobby_addr = Some(lobby.into());
            assert_eq!(flow.take_return_to_lobby().as_deref(), Some(lobby));
            assert!(flow.intercept_join(&lobby_view, lobby, &lock_in()));
            flow.request_id
        };
        app.insert_resource(CareerIdentity::authenticated_for_test(
            lobby,
            9,
            "mobile-retired",
        ));
        app.insert_resource(ClientSessionId("mobile-retired".into()));
        {
            let mut session = app.world_mut().resource_mut::<ClientSession>();
            session.set_server_addr_for_test(lobby);
            session.set_state_for_test(crate::net::ClientConnectionState::Connected);
        }
        app.world_mut().resource_mut::<GameStateSnapshot>().meta =
            shared::protocol::SnapshotMeta::new(9, 1, 1);
        {
            let mut career = app.world_mut().resource_mut::<CareerClient>();
            career.view = CareerView {
                match_service_request_id: Some(request_id),
                match_service: Some(MatchServiceView::Assigned {
                    allocation: allocation(),
                }),
                ..default()
            };
        }
        for _ in 0..2 {
            app.world_mut()
                .resource_mut::<MatchServiceClient>()
                .last_request = Some(Instant::now() - Duration::from_secs(3));
            app.update();
            assert!(
                app.world()
                    .resource::<MatchServiceClient>()
                    .allocation
                    .is_none()
            );
            assert!(app.world().resource::<MatchServiceClient>().is_searching());
            assert!(
                app.world()
                    .resource::<Messages<SessionUiCommand>>()
                    .is_empty(),
                "old allocation must not get a handoff even with a matching request ID"
            );
            assert_eq!(app.world_mut().resource_mut::<Messages<NetworkCommand>>().drain()
                .filter(|command| matches!(command, NetworkCommand::Career(shared::career::CareerRequest::FindMatch { request_id: id, .. }) if *id == request_id)).count(), 1,
                "rejecting the old worker must keep retrying matchmaking");
        }
        let mut next_allocation = allocation();
        next_allocation.allocation_id = "next-allocation".into();
        // Worker ports are reused; allocation identity, not endpoint, is the boundary.
        app.world_mut()
            .resource_mut::<CareerClient>()
            .view
            .match_service = Some(MatchServiceView::Assigned {
            allocation: next_allocation.clone(),
        });
        app.update();
        assert_eq!(
            app.world()
                .resource::<MatchServiceClient>()
                .allocation
                .as_ref()
                .unwrap()
                .allocation_id,
            next_allocation.allocation_id
        );
        assert!(app.world_mut().resource_mut::<Messages<SessionUiCommand>>().drain()
            .any(|command| matches!(command, SessionUiCommand::ConnectAllocated(endpoint) if endpoint == next_allocation.endpoint)));
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
