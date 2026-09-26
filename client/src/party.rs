//! Client side of parties: the latest server [`PartyView`], the periodic
//! presence announcement (nickname and avatar other members see), and
//! following the leader's launch into hero select.
//!
//! The server owns every rule (`server/src/party.rs`); this module only sends
//! [`PartyCommand`]s and remembers what came back.
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
}

impl PartyClient {
    pub fn apply_view(&mut self, server_epoch: u64, sequence: u64, view: PartyView, now: Instant) {
        if server_epoch == 0 {
            return;
        }
        if server_epoch != self.server_epoch {
            // Another server (or a restart): sequences start over and nothing
            // it launched before is ours to follow.
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
}

pub(crate) struct PartyPlugin;

impl Plugin for PartyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PartyClient>()
            .add_systems(Update, (announce_presence, expire_view, follow_launch));
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
    if current != AppScreen::HeroSelect {
        next.set(AppScreen::HeroSelect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::match_service::MatchPreference;
    use shared::party::PartyInfo;

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
