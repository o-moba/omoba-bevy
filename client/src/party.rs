//! Client side of parties: the latest server [`PartyView`], the periodic
//! presence announcement (nickname and avatar other members see), and
//! following the leader's launch directly into the public queue.
//!
//! The server owns every rule (`server/src/party.rs`); this module only sends
//! [`PartyCommand`]s and remembers what came back.
//! No player-facing text: the lobby (`frontend::lobby`) renders the view.
// i18n-strict
use std::time::{Duration, Instant};

use bevy::prelude::*;
use shared::party::{PartyCommand, PartyLaunch, PartyView};

use crate::frontend::AppScreen;
use crate::net::{ClientConnectionState, ClientSession, NetworkCommand};

/// Presence is re-announced this often; the server forgets a silent player
/// after eight seconds.
const PRESENCE_EVERY: Duration = Duration::from_secs(2);
/// Without a fresh view (another server, a match worker) the party UI shows
/// nothing rather than a stale roster.
const VIEW_STALE_AFTER: Duration = Duration::from_secs(6);
/// A refusal or a notice stays on the lobby screen this long.
pub(crate) const NOTICE_FOR: Duration = Duration::from_secs(5);

/// Something the lobby tells the player for a few seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PartyNotice {
    /// The server refused a command: a stable code (`PartyView::error`),
    /// shown as `party.error.<code>`.
    Error(String),
    /// The lobby restarted (a new server epoch) while the player was in a
    /// party; the party is gone with it.
    LobbyRestarted,
}

#[derive(Resource, Default)]
pub(crate) struct PartyClient {
    pub view: PartyView,
    server_epoch: u64,
    sequence: u64,
    received_at: Option<Instant>,
    /// Party whose launches have been accounted for, and the newest launch
    /// already seen in it. Joining a party or reconnecting never replays an
    /// old launch.
    seen_party: Option<u64>,
    seen_launch: u64,
    pending_launch: Option<PartyLaunch>,
    notice: Option<(PartyNotice, Instant)>,
    /// Expiry hides stale UI but must not erase why a new epoch dissolved it.
    had_party: bool,
    pending_restart: Option<bool>,
}

impl PartyClient {
    pub fn apply_view(
        &mut self,
        server_epoch: u64,
        sequence: u64,
        mut view: PartyView,
        now: Instant,
    ) {
        if server_epoch == 0 {
            return;
        }
        if server_epoch != self.server_epoch {
            // Another server (or a restart): sequences start over and nothing
            // it launched before is ours to follow.
            if self.server_epoch != 0 {
                self.pending_restart = Some(self.had_party);
                if self.had_party {
                    self.notice = Some((PartyNotice::LobbyRestarted, now));
                }
            }
            self.server_epoch = server_epoch;
            self.sequence = 0;
            self.seen_party = None;
            self.seen_launch = 0;
            self.pending_launch = None;
        } else if sequence <= self.sequence {
            return;
        }
        self.sequence = sequence;
        self.received_at = Some(now);
        // The refusal is a notice, not part of the roster.
        if let Some(code) = view.error.take() {
            self.notice = Some((PartyNotice::Error(code), now));
        }
        let party_id = view.party.as_ref().map(|p| p.party_id);
        let launch = view.party.as_ref().and_then(|p| p.launch);
        if party_id != self.seen_party {
            self.seen_party = party_id;
            self.seen_launch = launch.map_or(0, |l| l.sequence);
        } else if let Some(launch) = launch
            && launch.sequence > self.seen_launch
        {
            self.seen_launch = launch.sequence;
            self.pending_launch = Some(launch);
        }
        self.had_party = view.party.is_some();
        self.view = view;
    }

    /// A view arrived recently from the server we are connected to.
    pub fn is_live(&self, now: Instant) -> bool {
        self.received_at
            .is_some_and(|at| now.saturating_duration_since(at) < VIEW_STALE_AFTER)
    }

    fn expire(&mut self, now: Instant) {
        if self.received_at.is_some() && !self.is_live(now) {
            self.received_at = None;
            self.view = PartyView::default();
            self.seen_party = None;
            self.seen_launch = 0;
        }
    }

    pub fn take_launch(&mut self) -> Option<PartyLaunch> {
        self.pending_launch.take()
    }

    pub fn in_party(&self) -> bool {
        self.view.party.is_some()
    }

    /// The notice to show at `now`, if one is still fresh.
    pub fn notice(&self, now: Instant) -> Option<&PartyNotice> {
        self.notice
            .as_ref()
            .filter(|(_, at)| now.saturating_duration_since(*at) < NOTICE_FOR)
            .map(|(notice, _)| notice)
    }
}

pub(crate) struct PartyPlugin;

impl Plugin for PartyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PartyClient>()
            .add_message::<crate::ui::widgets::surfaces::ToastRequest>()
            .add_systems(Update, (announce_presence, expire_view))
            .add_systems(
                Update,
                announce_restart
                    .after(expire_view)
                    .after(crate::net::ClientNetPipeline::ApplySnapshot)
                    .before(crate::ui::UiSet::Paint),
            )
            .add_systems(
                Update,
                follow_launch.before(crate::net::ClientNetPipeline::SendCommands),
            );
    }
}

/// The avatar other members see: the profile card's showcase, else the hero
/// last picked.
pub(crate) fn presence_avatar(
    card: &crate::frontend::card::ProfileCard,
    selection: &crate::team::TeamSelection,
) -> Option<String> {
    card.showcase_avatar
        .clone()
        .or_else(|| selection.avatar.clone())
}

fn announce_presence(
    session: Res<ClientSession>,
    career: Res<crate::career::CareerClient>,
    card: Res<crate::frontend::card::ProfileCard>,
    selection: Res<crate::team::TeamSelection>,
    mut commands: MessageWriter<NetworkCommand>,
    mut last: Local<Option<Instant>>,
) {
    if session.state() != ClientConnectionState::Connected || session.is_offline() {
        *last = None;
        return;
    }
    let now = Instant::now();
    if last.is_some_and(|at| now.saturating_duration_since(at) < PRESENCE_EVERY) {
        return;
    }
    *last = Some(now);
    let nickname = career
        .view
        .profile
        .as_ref()
        .map_or_else(|| career.nickname.clone(), |p| p.nickname.clone());
    commands.write(NetworkCommand::Party(PartyCommand::Presence {
        nickname,
        avatar: presence_avatar(&card, &selection),
    }));
}

fn expire_view(mut party: ResMut<PartyClient>) {
    party.expire(Instant::now());
}

/// Connection loss matters on every screen, including Home and an active
/// search. A toast is independent of the party-lobby panel and fires once.
fn announce_restart(
    mut party: ResMut<PartyClient>,
    flow: Res<crate::match_service::MatchServiceClient>,
    mut toasts: MessageWriter<crate::ui::widgets::surfaces::ToastRequest>,
) {
    let Some(had_party) = party.pending_restart.take() else {
        return;
    };
    let key = if had_party {
        "party.notice.lobby_restarted"
    } else if flow.active {
        "party.notice.search_restarted"
    } else {
        return;
    };
    toasts.write(crate::ui::widgets::surfaces::ToastRequest {
        kind: crate::ui::widgets::surfaces::ToastKind::Warning,
        text: crate::i18n::tr(key).to_owned(),
    });
}

/// Screens a launch may pull a member away from: browsing, never a queue,
/// a draft or a match.
pub(crate) fn launch_can_leave(screen: AppScreen) -> bool {
    matches!(
        screen,
        AppScreen::Home
            | AppScreen::Lobby
            | AppScreen::Card
            | AppScreen::Collection
            | AppScreen::HeroSelect
            | AppScreen::PostMatch
    )
}

fn follow_launch(
    mut party: ResMut<PartyClient>,
    screen: Res<State<AppScreen>>,
    mut next: ResMut<NextState<AppScreen>>,
    mut matchmaking: ResMut<crate::match_service::MatchServiceClient>,
    card: Res<crate::frontend::card::ProfileCard>,
    mut selection: ResMut<crate::team::TeamSelection>,
    mut requests: MessageWriter<NetworkCommand>,
    session: Res<ClientSession>,
    mut session_ui: MessageWriter<crate::net::SessionUiCommand>,
) {
    let Some(launch) = party.take_launch() else {
        return;
    };
    let current = *screen.get();
    if !launch_can_leave(current) || (current == AppScreen::PostMatch && session.join_confirmed()) {
        return;
    }
    if session.is_offline() {
        session_ui.write(crate::net::SessionUiCommand::LeaveMatch);
    }
    matchmaking.preference = launch.preference;
    requests.write(selection.queue_from_card(&card));
    next.set(AppScreen::Searching);
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::match_service::MatchPreference;
    use shared::party::PartyInfo;

    #[test]
    fn a_party_launch_joins_the_public_queue_once_without_a_first_picker() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .init_state::<AppScreen>()
            .init_resource::<PartyClient>()
            .init_resource::<ClientSession>()
            .init_resource::<crate::frontend::card::ProfileCard>()
            .init_resource::<crate::team::TeamSelection>()
            .init_resource::<crate::match_service::MatchServiceClient>()
            .add_message::<NetworkCommand>()
            .add_message::<crate::net::SessionUiCommand>()
            .add_systems(Update, follow_launch);
        app.world_mut()
            .resource_mut::<crate::frontend::card::ProfileCard>()
            .main_class = shared::HeroClass::Ranger;
        app.world_mut().resource_mut::<PartyClient>().pending_launch = Some(PartyLaunch {
            sequence: 1,
            preference: MatchPreference::Ranked,
        });
        app.update();
        assert!(matches!(
            *app.world().resource::<NextState<AppScreen>>(),
            NextState::Pending(AppScreen::Searching)
        ));
        assert_eq!(
            app.world()
                .resource::<crate::match_service::MatchServiceClient>()
                .preference,
            MatchPreference::Ranked
        );
        let sent: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect();
        assert!(matches!(
            &sent[..],
            [NetworkCommand::JoinPrematch {
                hero_class: shared::HeroClass::Ranger,
                ..
            }]
        ));
        app.update();
        assert!(
            app.world()
                .resource::<Messages<NetworkCommand>>()
                .is_empty()
        );
    }

    fn view(party_id: Option<u64>, launch: Option<u64>) -> PartyView {
        PartyView {
            you: 1,
            party: party_id.map(|party_id| PartyInfo {
                party_id,
                leader: 1,
                members: Vec::new(),
                launch: launch.map(|sequence| PartyLaunch {
                    sequence,
                    preference: MatchPreference::BotPractice,
                }),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn only_a_new_launch_in_the_current_party_is_followed() {
        let now = Instant::now();
        let mut client = PartyClient::default();
        // Joining a party that launched before never replays that launch.
        client.apply_view(9, 1, view(Some(4), Some(3)), now);
        assert!(client.take_launch().is_none());
        client.apply_view(9, 2, view(Some(4), Some(3)), now);
        assert!(client.take_launch().is_none());
        client.apply_view(9, 3, view(Some(4), Some(5)), now);
        assert_eq!(client.take_launch().map(|l| l.sequence), Some(5));
        assert!(client.take_launch().is_none(), "acted on once");
        // Out-of-order and duplicate views are ignored.
        client.apply_view(9, 2, view(Some(4), Some(7)), now);
        assert!(client.take_launch().is_none());
        // Another server starts over without replaying anything.
        client.apply_view(11, 1, view(Some(4), Some(8)), now);
        assert!(client.take_launch().is_none());
    }

    #[test]
    fn a_silent_server_clears_the_party() {
        let now = Instant::now();
        let mut client = PartyClient::default();
        client.apply_view(9, 1, view(Some(4), None), now);
        assert!(client.in_party() && client.is_live(now));
        client.expire(now + VIEW_STALE_AFTER);
        assert!(!client.in_party());
    }

    #[test]
    fn restart_toast_survives_party_expiry_and_reaches_any_screen_once() {
        use crate::ui::widgets::surfaces::ToastRequest;
        for had_party in [false, true] {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .init_resource::<PartyClient>()
                .init_resource::<crate::match_service::MatchServiceClient>()
                .add_message::<ToastRequest>()
                .add_systems(Update, announce_restart);
            app.world_mut()
                .resource_mut::<crate::match_service::MatchServiceClient>()
                .active = true;
            let now = Instant::now();
            app.world_mut().resource_mut::<PartyClient>().apply_view(
                9,
                1,
                view(had_party.then_some(4), None),
                now,
            );
            app.update();
            assert!(
                app.world().resource::<Messages<ToastRequest>>().is_empty(),
                "first admission is not a restart"
            );
            let later = now + VIEW_STALE_AFTER;
            let mut party = app.world_mut().resource_mut::<PartyClient>();
            party.expire(later);
            assert!(!party.in_party());
            party.apply_view(11, 1, view(None, None), later);
            app.update();
            let sent: Vec<_> = app
                .world_mut()
                .resource_mut::<Messages<ToastRequest>>()
                .drain()
                .collect();
            assert_eq!(sent.len(), 1);
            assert_eq!(
                sent[0].text,
                crate::i18n::tr(if had_party {
                    "party.notice.lobby_restarted"
                } else {
                    "party.notice.search_restarted"
                })
            );
            app.world_mut().resource_mut::<PartyClient>().apply_view(
                11,
                2,
                view(None, None),
                later,
            );
            app.update();
            assert!(
                app.world().resource::<Messages<ToastRequest>>().is_empty(),
                "no repeated toast"
            );
        }
    }

    #[test]
    fn a_refusal_is_shown_briefly_and_kept_out_of_the_roster() {
        let now = Instant::now();
        let mut client = PartyClient::default();
        client.apply_view(9, 1, view(None, None), now);
        assert_eq!(client.notice(now), None);
        let mut refused = view(None, None);
        refused.error = Some("party_full".into());
        client.apply_view(9, 2, refused, now);
        assert_eq!(
            client.notice(now),
            Some(&PartyNotice::Error("party_full".into()))
        );
        assert_eq!(client.view.error, None);
        // The periodic view that follows does not clear it early.
        client.apply_view(9, 3, view(None, None), now + Duration::from_secs(1));
        assert!(client.notice(now + Duration::from_secs(1)).is_some());
        assert_eq!(client.notice(now + NOTICE_FOR), None);
    }

    #[test]
    fn a_lobby_restart_that_dissolves_the_party_is_announced() {
        let now = Instant::now();
        let mut client = PartyClient::default();
        // The first server seen is not a restart.
        client.apply_view(9, 1, view(Some(4), None), now);
        assert_eq!(client.notice(now), None);
        // A new epoch while in a party: the party is gone, say so.
        client.apply_view(11, 1, view(None, None), now);
        assert_eq!(client.notice(now), Some(&PartyNotice::LobbyRestarted));
        assert!(!client.in_party());
        // A new epoch while not in a party says nothing.
        let later = now + NOTICE_FOR;
        client.apply_view(12, 1, view(None, None), later);
        assert_eq!(client.notice(later), None);
    }

    #[test]
    fn a_launch_never_pulls_a_player_out_of_a_queue_or_match() {
        for screen in [
            AppScreen::Searching,
            AppScreen::Draft,
            AppScreen::Loading,
            AppScreen::InMatch,
        ] {
            assert!(!launch_can_leave(screen), "{screen:?}");
        }
        assert!(launch_can_leave(AppScreen::Lobby));
        assert!(launch_can_leave(AppScreen::Home));
    }
}
