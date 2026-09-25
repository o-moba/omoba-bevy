//! Authoritative career UI. Match receipts outlive live entities and connections.
use crate::{
    input_context::InputContextSet,
    net::{GameState, GameStateSnapshot, NetworkCommand, SessionEvent, SessionReactions},
    platform::UiProfile,
    sprite::{PlayerVisualMode, SpriteVisualAssets},
    ui_theme as ui,
};
use bevy::{
    input::{
        keyboard::{Key, KeyboardInput},
        mouse::{MouseScrollUnit, MouseWheel},
    },
    prelude::*,
    window::PrimaryWindow,
};
use shared::career::{
    CareerRequest, CareerView, FriendAction, FriendPresence, FriendProfile, MatchOutcome,
    MatchResult, ParticipantResult, ProfileSummary, QueueView,
};
use std::time::{Duration, Instant};
#[path = "career_devices.rs"]
mod devices;
#[path = "career_web.rs"]
mod web;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum CareerModal {
    #[default]
    Closed,
    Profile,
    History,
    Result,
    Friends,
    FriendProfile,
    WebLink,
    Devices,
}

#[derive(Clone, PartialEq)]
enum PendingKind {
    History,
    Detail(String),
    Nickname(String),
    Friends,
    Friend(String, FriendAction),
    Profile(String),
    Lookup(String),
}

#[derive(Resource, Clone, PartialEq)]
pub(crate) struct CareerClient {
    pub view: CareerView,
    web: web::WebState,
    devices: devices::DeviceState,
    pub public_profile_id: Option<String>,
    pub nickname: String,
    pub local_player_id: Option<u64>,
    pub modal: CareerModal,
    draft: String,
    preedit: String,
    form_error: Option<String>,
    nickname_focused: bool,
    friend_code: String,
    friend_code_focused: bool,
    visited_profile_id: Option<String>,
    ime_owned: bool,
    announced_result: Option<String>,
    announced_player_id: Option<u64>,
    selected_result: Option<String>,
    expanded_player: Option<u64>,
    history_before: Option<u64>,
    history_back: Vec<Option<u64>>,
    request_sequence: u64,
    latest_query: Option<(PendingKind, u64)>,
    pending: Option<(PendingKind, u64, Instant, Instant)>,
    request_error: Option<String>,
}
impl Default for CareerClient {
    fn default() -> Self {
        Self {
            view: CareerView::default(),
            web: web::WebState::default(),
            devices: devices::DeviceState::default(),
            public_profile_id: None,
            nickname: "Player".into(),
            local_player_id: None,
            modal: CareerModal::Closed,
            draft: String::new(),
            preedit: String::new(),
            form_error: None,
            nickname_focused: false,
            friend_code: String::new(),
            friend_code_focused: false,
            visited_profile_id: None,
            ime_owned: false,
            announced_result: None,
            announced_player_id: None,
            selected_result: None,
            expanded_player: None,
            history_before: None,
            history_back: Vec::new(),
            request_sequence: 0,
            latest_query: None,
            pending: None,
            request_error: None,
        }
    }
}
impl CareerClient {
    fn begin_request(&mut self, kind: PendingKind) -> u64 {
        self.request_sequence = self
            .request_sequence
            .checked_add(1)
            .expect("career request sequence exhausted");
        let request_id = self.request_sequence;
        self.latest_query = Some((kind.clone(), request_id));
        let now = Instant::now();
        self.pending = Some((
            kind,
            request_id,
            now + Duration::from_secs(8),
            now + Duration::from_secs(2),
        ));
        self.request_error = None;
        self.view.error = None;
        self.view.loading = true;
        request_id
    }
    fn expire_request(&mut self, now: Instant) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, _, deadline, _)| now >= *deadline)
        {
            self.pending = None;
            self.view.loading = false;
            self.request_error =
                Some("Saved progress is unavailable. Check your connection and try again.".into());
        }
    }
    pub fn modal_open(&self) -> bool {
        self.modal != CareerModal::Closed
    }
    pub fn nickname_editor_open(&self) -> bool {
        self.modal == CareerModal::Profile && self.nickname_focused
    }
    pub(crate) fn request_failed(&mut self, error: String) {
        self.pending = None;
        self.view.loading = false;
        self.request_error = Some(error);
    }
    pub(crate) fn rename_request_id(&mut self) -> u64 {
        if let Some((PendingKind::Nickname(_), request_id, _, _)) = &self.pending {
            *request_id
        } else {
            self.begin_request(PendingKind::Nickname(self.nickname.clone()))
        }
    }
    fn retry_request(&mut self, now: Instant) -> Option<CareerRequest> {
        let (kind, request_id, deadline, next) = self.pending.as_mut()?;
        if now < *next || now >= *deadline {
            return None;
        }
        *next = now + Duration::from_secs(2);
        Some(match kind {
            PendingKind::History => CareerRequest::History {
                request_id: *request_id,
                before: self.history_before,
            },
            PendingKind::Detail(id) => CareerRequest::Detail {
                request_id: *request_id,
                result_id: id.clone(),
            },
            PendingKind::Nickname(name) => CareerRequest::Rename {
                request_id: *request_id,
                nickname: name.clone(),
            },
            PendingKind::Friends => CareerRequest::Friends {
                request_id: *request_id,
            },
            PendingKind::Friend(id, action) => CareerRequest::Friend {
                request_id: *request_id,
                profile_id: id.clone(),
                action: *action,
            },
            PendingKind::Lookup(handle) => CareerRequest::LookupPlayer {
                request_id: *request_id,
                handle: handle.clone(),
            },
            PendingKind::Profile(id) => CareerRequest::Profile {
                request_id: *request_id,
                profile_id: id.clone(),
            },
        })
    }
    /// Only the explicitly enabled native visual QA plugin calls this fixture
    /// boundary. It bypasses networking and is never an authoritative result.
    #[cfg(feature = "qa")]
    pub(crate) fn present_visual_fixture(&mut self, view: CareerView, modal: CareerModal) {
        *self = Self::default();
        self.public_profile_id = view
            .profile
            .as_ref()
            .map(|profile| profile.profile_id.clone());
        self.nickname = view
            .profile
            .as_ref()
            .map_or_else(|| "QA Player".into(), |profile| profile.nickname.clone());
        self.draft.clone_from(&self.nickname);
        self.selected_result = view
            .last_result
            .as_ref()
            .map(|result| result.result_id.clone());
        self.announced_result.clone_from(&self.selected_result);
        self.visited_profile_id = view
            .visited_profile
            .as_ref()
            .map(|profile| profile.profile_id.clone());
        self.view = view;
        self.modal = modal;
    }
    /// Call on an account OR server namespace change before applying a new view.
    /// Never mix another server's history with this server's authenticated profile.
    pub fn clear_account(&mut self) {
        // A late response from the old namespace must never share a new ID.
        let request_sequence = self.request_sequence;
        // Native enrollment belongs to the account service, not a match server.
        // Reconnecting while its HTTP worker runs must not discard its new key proof.
        let mut devices = std::mem::take(&mut self.devices);
        devices.focused = false;
        devices.recovery_code.clear();
        *self = Self::default();
        self.request_sequence = request_sequence;
        self.devices = devices;
    }
    /// A default/live snapshot does not erase the last immutable result receipt.
    pub fn apply_view(&mut self, mut next: CareerView) {
        if let Some(profile) = &next.profile {
            if self
                .public_profile_id
                .as_ref()
                .is_some_and(|id| id != &profile.profile_id)
            {
                self.clear_account();
            }
            self.public_profile_id = Some(profile.profile_id.clone());
            self.nickname.clone_from(&profile.nickname);
        }
        let query_kind = self
            .latest_query
            .as_ref()
            .and_then(|(kind, request_id)| (next.response_id == Some(*request_id)).then_some(kind));
        let history_response = matches!(query_kind, Some(PendingKind::History))
            && next.history_loaded
            && !next.loading
            && next.error.is_none();
        let detail_response = match query_kind {
            Some(PendingKind::Detail(id)) => {
                next.detail
                    .as_ref()
                    .is_some_and(|result| &result.result_id == id)
                    && !next.loading
                    && next.error.is_none()
            }
            _ => false,
        };
        let friends_response = matches!(
            query_kind,
            Some(PendingKind::Friends | PendingKind::Friend(_, _))
        ) && next.friends.is_some()
            && !next.loading
            && next.error.is_none();
        let lookup_response = matches!(query_kind, Some(PendingKind::Lookup(_)))
            && next.found_player.is_some()
            && !next.loading
            && next.error.is_none();
        if !lookup_response {
            next.found_player.clone_from(&self.view.found_player);
        }
        let profile_response = matches!(query_kind, Some(PendingKind::Profile(id)) if next.visited_profile.as_ref().is_some_and(|profile| &profile.profile_id == id))
            && !next.loading
            && next.error.is_none();
        let nickname_response = matches!(query_kind, Some(PendingKind::Nickname(name)) if next.profile.as_ref().is_some_and(|profile| &profile.nickname == name))
            && !next.loading
            && next.error.is_none();
        // Query payloads are correlated independently of live queue/profile/result
        // updates. A repeated prior page or a normal snapshot cannot replace them.
        if !history_response {
            next.history.clone_from(&self.view.history);
            next.history_loaded = self.view.history_loaded;
            next.history_next = self.view.history_next;
        }
        if !detail_response {
            next.detail.clone_from(&self.view.detail);
        }
        if !friends_response {
            next.friends.clone_from(&self.view.friends);
        }
        if !profile_response {
            next.visited_profile.clone_from(&self.view.visited_profile);
        }
        if next.response_id.is_some() && query_kind.is_none() {
            next.error = None;
            next.loading = false;
        }
        if history_response
            || detail_response
            || friends_response
            || profile_response
            || nickname_response
            || lookup_response
        {
            self.request_error = None;
        }
        if let Some((kind, request_id, _, _)) = &self.pending {
            let matched = next.response_id == Some(*request_id);
            let complete = !next.loading
                && match kind {
                    PendingKind::History => matched && history_response,
                    PendingKind::Detail(_) => matched && detail_response,
                    PendingKind::Nickname(_) => matched && nickname_response,
                    PendingKind::Friends | PendingKind::Friend(_, _) => matched && friends_response,
                    PendingKind::Profile(_) => matched && profile_response,
                    PendingKind::Lookup(_) => matched && lookup_response,
                };
            if complete || (matched && next.error.is_some()) {
                self.request_error.clone_from(&next.error);
                self.pending = None;
            } else {
                next.loading = true;
                next.error = None;
            }
        }
        if next.supporter.is_none() {
            next.supporter.clone_from(&self.view.supporter);
        }
        if next.last_result.is_none() {
            next.last_result.clone_from(&self.view.last_result);
        }
        if let Some(result) = &next.last_result
            && self.announced_result.as_deref() != Some(result.result_id.as_str())
        {
            self.announced_result = Some(result.result_id.clone());
            self.announced_player_id = self.local_player_id;
            self.selected_result = Some(result.result_id.clone());
            self.modal = CareerModal::Result;
            self.nickname_focused = false;
            self.friend_code_focused = false;
            self.expanded_player = None;
        }
        if self.view != next {
            self.view = next;
        }
    }
    fn result(&self) -> Option<&MatchResult> {
        let selected = self.selected_result.as_deref()?;
        self.view
            .detail
            .as_ref()
            .filter(|r| r.result_id == selected)
            .or_else(|| {
                self.view
                    .last_result
                    .as_ref()
                    .filter(|r| r.result_id == selected)
            })
    }
    fn open_profile(&mut self) {
        self.modal = CareerModal::Profile;
        self.draft.clone_from(&self.nickname);
        self.preedit.clear();
        self.form_error = None;
        self.nickname_focused = true;
        self.friend_code_focused = false;
    }
    /// Opens the profile modal from outside the career UI (front-end shell).
    pub(crate) fn open_profile_modal(&mut self) {
        self.open_profile();
    }

    fn close(&mut self) {
        self.web.dismiss();
        self.devices.focused = false;
        self.devices.recovery_code.clear();
        self.modal = CareerModal::Closed;
        self.nickname_focused = false;
        self.friend_code_focused = false;
        self.preedit.clear();
        self.pending = None;
        self.view.loading = false;
    }
}

#[derive(Message, Clone, Debug)]
pub(crate) struct NicknameChanged(pub String);
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CareerUiSet;
pub(crate) struct CareerPlugin;
impl Plugin for CareerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CareerClient>()
            .init_resource::<web::Worker>()
            .init_resource::<devices::Worker>()
            .init_resource::<crate::career_identity::CareerIdentity>()
            .add_message::<NicknameChanged>()
            .add_message::<bevy::input::touch::TouchInput>()
            .add_systems(
                Update,
                (
                    web::poll,
                    devices::poll,
                    bump_gesture_epoch_on_navigation,
                    actions,
                    dismiss_with_escape,
                    nickname_input,
                )
                    .chain()
                    .after(crate::ui::UiSet::Gesture)
                    .after(crate::mobile_ui::address_keyboard)
                    .after(crate::net::ClientNetPipeline::ApplySnapshot)
                    .before(crate::pause_menu::toggle_pause_menu)
                    .in_set(InputContextSet::Modal)
                    .in_set(CareerUiSet),
            )
            .add_systems(Update, render.after(CareerUiSet))
            .add_systems(
                Update,
                clear_account_on_scope_reset.in_set(SessionReactions),
            )
            .add_systems(
                PostUpdate,
                scroll_desktop.before(bevy::ui::UiSystems::Layout),
            );
    }
}

/// Another server scope (offline practice, a new address, an allocated
/// arena, the lobby after a leave) makes the account view stale: clear
/// transport-local views until the destination server authenticates this
/// account against its configured backend. Runs in `SessionReactions`, in the
/// frame the session announced [`SessionEvent::ServerScopeReset`], so the
/// next frame's ingest applies the new server's view to a cleared client.
pub(crate) fn clear_account_on_scope_reset(
    mut session_events: MessageReader<SessionEvent>,
    mut career: ResMut<CareerClient>,
) {
    let mut scope_reset = false;
    for event in session_events.read() {
        scope_reset |= *event == SessionEvent::ServerScopeReset;
    }
    if scope_reset {
        career.clear_account();
    }
}
#[derive(Component)]
struct CareerRoot;
#[derive(Component)]
struct CareerScroll;
#[derive(Component, Clone)]
enum Action {
    Supporter,
    DevicesOpen,
    DevicesStart,
    DevicesPoll,
    DevicesRecoveryEdit,
    DevicesRecover,
    DevicesConfirm,
    WebOpen,
    WebEdit,
    WebLookup,
    WebApprove,
    WebDeny,
    Profile,
    History,
    Close,
    EditName,
    SaveName,
    Detail(String),
    Expand(u64),
    Next,
    Previous,
    Refresh,
    PlayAgain,
    CancelQueue,
    LastResult,
    Friends,
    EditFriendCode,
    AddFriend,
    LookupFriend,
    Friend(String, FriendAction),
    VisitProfile(String),
}

fn profile_id(raw: &str) -> Result<String, &'static str> {
    let value = raw.trim();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid player reference. Find the player again.");
    }
    Ok(value.to_ascii_lowercase())
}

fn lookup_player(career: &mut CareerClient, requests: &mut MessageWriter<NetworkCommand>) {
    let handle = match shared::career::normalize_player_handle(&career.friend_code) {
        Ok(handle) => handle,
        Err(e) => {
            career.form_error = Some(e.into());
            return;
        }
    };
    career.form_error = None;
    career.friend_code_focused = false;
    career.friend_code.clone_from(&handle);
    career.preedit.clear();
    career.view.found_player = None;
    let request_id = career.begin_request(PendingKind::Lookup(handle.clone()));
    requests.write(NetworkCommand::Career(CareerRequest::LookupPlayer {
        request_id,
        handle,
    }));
}

fn request_friends(career: &mut CareerClient, requests: &mut MessageWriter<NetworkCommand>) {
    career.modal = CareerModal::Friends;
    career.nickname_focused = false;
    career.friend_code_focused = false;
    career.preedit.clear();
    let request_id = career.begin_request(PendingKind::Friends);
    requests.write(NetworkCommand::Career(CareerRequest::Friends {
        request_id,
    }));
}

/// Opens the match-history modal from outside the career UI (front-end shell),
/// reusing the same request path the in-career button uses.
pub(crate) fn open_history_modal(
    career: &mut CareerClient,
    requests: &mut MessageWriter<NetworkCommand>,
) {
    career.history_back.clear();
    request_history(career, None, requests);
}

/// Opens the friends modal from outside the career UI (front-end shell).
pub(crate) fn open_friends_modal(
    career: &mut CareerClient,
    requests: &mut MessageWriter<NetworkCommand>,
) {
    request_friends(career, requests);
}

fn friend_action(
    career: &mut CareerClient,
    id: &str,
    action: FriendAction,
    requests: &mut MessageWriter<NetworkCommand>,
) {
    let id = match profile_id(id) {
        Ok(id) => id,
        Err(error) => {
            career.form_error = Some(error.into());
            return;
        }
    };
    if career.public_profile_id.as_deref() == Some(&id) {
        career.form_error = Some("That is your own player tag.".into());
        return;
    }
    career.form_error = None;
    career.friend_code_focused = false;
    career.preedit.clear();
    career.modal = CareerModal::Friends;
    let request_id = career.begin_request(PendingKind::Friend(id.clone(), action));
    requests.write(NetworkCommand::Career(CareerRequest::Friend {
        request_id,
        profile_id: id,
        action,
    }));
}

fn visit_profile(
    career: &mut CareerClient,
    id: &str,
    requests: &mut MessageWriter<NetworkCommand>,
) {
    let id = match profile_id(id) {
        Ok(id) => id,
        Err(error) => {
            career.form_error = Some(error.into());
            return;
        }
    };
    career.form_error = None;
    career.friend_code_focused = false;
    career.nickname_focused = false;
    career.preedit.clear();
    career.visited_profile_id = Some(id.clone());
    career.modal = CareerModal::FriendProfile;
    let request_id = career.begin_request(PendingKind::Profile(id.clone()));
    requests.write(NetworkCommand::Career(CareerRequest::Profile {
        request_id,
        profile_id: id,
    }));
}

fn request_history(
    career: &mut CareerClient,
    before: Option<u64>,
    requests: &mut MessageWriter<NetworkCommand>,
) {
    career.modal = CareerModal::History;
    career.nickname_focused = false;
    career.friend_code_focused = false;
    career.history_before = before;
    let request_id = career.begin_request(PendingKind::History);
    requests.write(NetworkCommand::Career(CareerRequest::History {
        request_id,
        before,
    }));
}
fn save_name(career: &mut CareerClient, changed: &mut MessageWriter<NicknameChanged>) {
    match shared::career::normalize_player_handle(&career.draft) {
        Ok(name) => {
            career.nickname = name.clone();
            career.draft = name.clone();
            career.form_error = None;
            career.nickname_focused = false;
            career.preedit.clear();
            career.begin_request(PendingKind::Nickname(name.clone()));
            changed.write(NicknameChanged(name));
        }
        Err(error) => career.form_error = Some(error.into()),
    }
}
/// A modal page change drops the tap held across it (see `ui::gesture`).
fn bump_gesture_epoch_on_navigation(
    career: Res<CareerClient>,
    mut epoch: ResMut<crate::ui::GestureEpoch>,
    mut last: Local<Option<CareerModal>>,
) {
    if *last != Some(career.modal) {
        if last.is_some() {
            epoch.bump();
        }
        *last = Some(career.modal);
    }
}

fn actions(
    buttons: Query<
        (&Interaction, &Action, Option<&crate::ui::Pressable>),
        (
            With<Button>,
            Or<(Changed<Interaction>, Changed<crate::ui::Pressable>)>,
        ),
    >,
    mut career: ResMut<CareerClient>,
    mut requests: MessageWriter<NetworkCommand>,
    mut changed: MessageWriter<NicknameChanged>,
    mut server_entry: Option<ResMut<crate::mobile_ui::ServerEntry>>,
    social: Option<Res<crate::social::SocialClient>>,
    identity: Res<crate::career_identity::CareerIdentity>,
    mut web_worker: ResMut<web::Worker>,
    mut device_worker: ResMut<devices::Worker>,
    mut supporter: Option<ResMut<crate::supporter::SupporterUiState>>,
) {
    if social
        .as_ref()
        .is_some_and(|social| social.blocks_gameplay())
    {
        return;
    }
    career.expire_request(Instant::now());
    for (interaction, action, pressable) in &buttons {
        let effective =
            pressable.map_or(*interaction, |pressable| pressable.effective(*interaction));
        if effective != Interaction::Pressed {
            continue;
        }
        web::act(action, &mut career, &identity, &mut web_worker);
        devices::act(action, &mut career, &identity, &mut device_worker);
        match action {
            Action::Supporter => {
                career.close();
                if let Some(state) = supporter.as_deref_mut() {
                    state.open = true;
                }
                requests.write(NetworkCommand::Career(CareerRequest::SupporterStatus {
                    request_id: 0,
                }));
            }
            Action::DevicesOpen
            | Action::DevicesStart
            | Action::DevicesPoll
            | Action::DevicesRecoveryEdit
            | Action::DevicesRecover
            | Action::DevicesConfirm => {}
            Action::WebOpen
            | Action::WebEdit
            | Action::WebLookup
            | Action::WebApprove
            | Action::WebDeny => {}
            Action::Profile => career.open_profile(),
            Action::Friends => request_friends(&mut career, &mut requests),
            Action::EditFriendCode => {
                career.friend_code_focused = true;
                career.nickname_focused = false;
                career.form_error = None;
            }
            Action::AddFriend => {
                let found = career.view.found_player.clone().filter(|p| {
                    p.nickname.to_lowercase() == career.friend_code.trim().to_lowercase()
                });
                if let Some(found) = found {
                    friend_action(
                        &mut career,
                        &found.profile_id,
                        FriendAction::Request,
                        &mut requests,
                    );
                } else {
                    lookup_player(&mut career, &mut requests);
                }
            }
            Action::LookupFriend => lookup_player(&mut career, &mut requests),
            Action::Friend(id, action) => friend_action(&mut career, id, *action, &mut requests),
            Action::VisitProfile(id) => visit_profile(&mut career, id, &mut requests),
            Action::History | Action::Refresh => {
                career.history_back.clear();
                request_history(&mut career, None, &mut requests);
            }
            Action::Close => career.close(),
            Action::EditName => career.nickname_focused = true,
            Action::SaveName => save_name(&mut career, &mut changed),
            Action::Detail(id) => {
                career.selected_result = Some(id.clone());
                career.modal = CareerModal::Result;
                career.expanded_player = None;
                let request_id = career.begin_request(PendingKind::Detail(id.clone()));
                requests.write(NetworkCommand::Career(CareerRequest::Detail {
                    request_id,
                    result_id: id.clone(),
                }));
            }
            Action::Expand(id) => {
                career.expanded_player = if career.expanded_player == Some(*id) {
                    None
                } else {
                    Some(*id)
                }
            }
            Action::Next => {
                if let Some(next) = career.view.history_next {
                    let previous = career.history_before;
                    career.history_back.push(previous);
                    request_history(&mut career, Some(next), &mut requests);
                }
            }
            Action::Previous => {
                if let Some(previous) = career.history_back.pop() {
                    request_history(&mut career, previous, &mut requests);
                }
            }
            Action::PlayAgain => {
                career.close();
                requests.write(NetworkCommand::RequestRematch);
            }
            Action::CancelQueue => {
                career.pending = None;
                career.view.loading = false;
                requests.write(NetworkCommand::Career(CareerRequest::CancelQueue));
            }
            Action::LastResult => {
                if let Some(id) = career
                    .view
                    .last_result
                    .as_ref()
                    .map(|r| r.result_id.clone())
                {
                    career.selected_result = Some(id);
                    career.modal = CareerModal::Result;
                }
            }
        }
        if career.modal_open()
            && let Some(entry) = server_entry.as_deref_mut()
        {
            entry.open = false;
        }
    }
    if let Some(request) = career.retry_request(Instant::now()) {
        requests.write(NetworkCommand::Career(request));
    }
}
fn append_name(current: &mut String, text: &str) -> Result<(), &'static str> {
    let next = format!("{current}{text}");
    if next.chars().count() > shared::career::MAX_PLAYER_HANDLE_CHARS || next.len() > 85 {
        return Err("Use up to 20 name characters, # and four digits.");
    }
    if next.chars().any(char::is_control) {
        return Err("Names cannot contain control characters.");
    }
    *current = next;
    Ok(())
}
fn append_friend_code(current: &mut String, text: &str) -> Result<(), &'static str> {
    append_name(current, text)
}
fn field_text(value: &str, preedit: &str, focused: bool, placeholder: &str) -> String {
    format!(
        "{}{}{}",
        if value.is_empty() { placeholder } else { value },
        preedit,
        if focused { " |" } else { "" }
    )
}
/// Invoked only by an explicit native paste shortcut. Phone keyboard paste is
/// received as an IME commit; no clipboard polling or extra backend dependency.
pub(crate) fn clipboard_text() -> Result<String, &'static str> {
    #[cfg(target_os = "macos")]
    let output = std::process::Command::new("/usr/bin/pbpaste").output();
    #[cfg(target_os = "windows")]
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-Clipboard -Raw",
        ])
        .output();
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let output = output.map_err(|_| "Clipboard paste is unavailable.")?;
        if !output.status.success() || output.stdout.len() > 4096 {
            return Err("Clipboard text is unavailable or too long.");
        }
        String::from_utf8(output.stdout).map_err(|_| "Clipboard text is invalid.")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("Use your device keyboard's paste action, or type the code.")
}
fn dismiss_with_escape(
    mut career: ResMut<CareerClient>,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
) {
    if career.modal_open() && keyboard.just_pressed(KeyCode::Escape) {
        career.close();
        keyboard.clear_just_pressed(KeyCode::Escape);
    }
}
fn nickname_input(
    mut career: ResMut<CareerClient>,
    mut keys: MessageReader<KeyboardInput>,
    mut ime: MessageReader<Ime>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut changed: MessageWriter<NicknameChanged>,
    mut requests: MessageWriter<NetworkCommand>,
) {
    let friend_input = career.modal == CareerModal::Friends && career.friend_code_focused;
    let web_input = career.modal == CareerModal::WebLink && career.web.focused;
    let device_input = career.modal == CareerModal::Devices && career.devices.focused;
    let active = career.nickname_editor_open() || friend_input || web_input || device_input;
    if let Ok(mut window) = windows.single_mut() {
        if active {
            if !window.ime_enabled {
                window.ime_enabled = true;
            }
            career.ime_owned = true;
        } else if career.ime_owned {
            if window.ime_enabled {
                window.ime_enabled = false;
            }
            career.ime_owned = false;
        }
    }
    let mut committed = false;
    for event in ime.read() {
        if !active {
            continue;
        }
        match event {
            Ime::Preedit { value, .. } => career.preedit = value.clone(),
            Ime::Commit { value, .. } => {
                let result = if device_input {
                    devices::append(&mut career.devices.recovery_code, value)
                } else if web_input {
                    web::append(&mut career.web.code, value)
                } else if friend_input {
                    append_friend_code(&mut career.friend_code, value)
                } else {
                    append_name(&mut career.draft, value)
                };
                career.form_error = result.err().map(str::to_owned);
                career.preedit.clear();
                committed = true;
            }
            _ => {}
        }
    }
    for event in keys.read() {
        if !active || !event.state.is_pressed() || committed || !career.preedit.is_empty() {
            continue;
        }
        match &event.logical_key {
            Key::Character(value)
                if value.eq_ignore_ascii_case("v")
                    && keyboard.any_pressed([
                        KeyCode::ControlLeft,
                        KeyCode::ControlRight,
                        KeyCode::SuperLeft,
                        KeyCode::SuperRight,
                    ]) =>
            {
                let result = clipboard_text().and_then(|text| {
                    if device_input {
                        devices::append(&mut career.devices.recovery_code, &text)
                    } else if web_input {
                        web::append(&mut career.web.code, &text)
                    } else if friend_input {
                        append_friend_code(&mut career.friend_code, text.trim())
                    } else {
                        append_name(&mut career.draft, text.trim())
                    }
                });
                career.form_error = result.err().map(str::to_owned);
            }
            Key::Backspace => {
                if device_input {
                    career.devices.recovery_code.pop();
                } else if web_input {
                    career.web.code.pop();
                } else if friend_input {
                    career.friend_code.pop();
                } else {
                    career.draft.pop();
                }
                career.form_error = None;
            }
            Key::Escape => {
                career.nickname_focused = false;
                career.friend_code_focused = false;
                career.preedit.clear();
            }
            Key::Enter => {
                if device_input {
                    career.devices.focused = false;
                } else if web_input {
                    career.web.focused = false;
                } else if friend_input {
                    lookup_player(&mut career, &mut requests);
                } else {
                    save_name(&mut career, &mut changed);
                }
            }
            _ => {
                if keyboard.any_pressed([
                    KeyCode::ControlLeft,
                    KeyCode::ControlRight,
                    KeyCode::SuperLeft,
                    KeyCode::SuperRight,
                ]) {
                    continue;
                }
                if let Some(text) = event.text.as_deref().or_else(|| match &event.logical_key {
                    Key::Character(text) => Some(text.as_str()),
                    _ => None,
                }) {
                    let result = if device_input {
                        devices::append(&mut career.devices.recovery_code, text)
                    } else if web_input {
                        web::append(&mut career.web.code, text)
                    } else if friend_input {
                        append_friend_code(&mut career.friend_code, text)
                    } else {
                        append_name(&mut career.draft, text)
                    };
                    career.form_error = result.err().map(str::to_owned);
                }
            }
        }
    }
}

fn label(
    parent: &mut ChildSpawnerCommands,
    value: impl Into<String>,
    size: f32,
    color: Color,
    name: &str,
) {
    parent.spawn((
        Text::new(value),
        ui::text(size),
        TextColor(color),
        Name::new(name.to_owned()),
    ));
}
fn button(parent: &mut ChildSpawnerCommands, value: &str, action: Action, name: &str) {
    let input = matches!(
        action,
        Action::EditName | Action::EditFriendCode | Action::WebEdit | Action::DevicesRecoveryEdit
    );
    let emphasis = matches!(
        &action,
        Action::SaveName | Action::AddFriend | Action::LookupFriend | Action::PlayAgain
    );
    let menu = crate::frontend::widgets::MenuButton::new(if emphasis {
        crate::frontend::widgets::ButtonKind::Primary
    } else if matches!(&action, Action::WebDeny) {
        crate::frontend::widgets::ButtonKind::Danger
    } else {
        crate::frontend::widgets::ButtonKind::Secondary
    });
    parent
        .spawn((
            Button,
            crate::ui::Pressable::default(),
            menu,
            Node {
                min_height: Val::Px(if input { 52.0 } else { 44.0 }),
                height: if input { Val::Px(52.0) } else { Val::Auto },
                width: if input {
                    Val::Percent(100.0)
                } else {
                    Val::Auto
                },
                overflow: if input {
                    Overflow::clip()
                } else {
                    Overflow::default()
                },
                min_width: Val::Px(44.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(7.0)),
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(menu.idle_color()),
            BorderColor::all(ui::EDGE),
            action,
            Name::new(name.to_owned()),
        ))
        .with_children(|parent| label(parent, value, 15.0, ui::IVORY, &format!("{name}Label")));
}
fn row_node() -> Node {
    Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        flex_wrap: FlexWrap::Wrap,
        flex_shrink: 0.0,
        ..default()
    }
}
fn column_node() -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(8.0),
        flex_shrink: 0.0,
        ..default()
    }
}
fn duration(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
fn timestamp(ms: u64) -> String {
    let seconds = ms.min(253_402_300_799_999) / 1000;
    let z = (seconds / 86400) as i64 + 719468;
    let era = z / 146097;
    let day_of_era = z - era * 146097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = month_index + if month_index < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        (seconds % 86400) / 3600,
        (seconds % 3600) / 60
    )
}
fn number(value: f64) -> String {
    if value.is_finite() && value >= 0.0 {
        format!("{value:.0}")
    } else {
        "—".into()
    }
}
fn rating(change: Option<&shared::career::RatingChange>) -> String {
    change.map_or_else(
        || "Unrated".into(),
        |r| format!("{} ({:+})", r.after, r.delta),
    )
}
fn local_participant<'a>(
    result: &'a MatchResult,
    career: &CareerClient,
) -> Option<&'a ParticipantResult> {
    result
        .participants
        .iter()
        .find(|p| {
            career
                .public_profile_id
                .as_ref()
                .is_some_and(|id| p.profile_id.as_ref() == Some(id))
        })
        .or_else(|| {
            result.participants.iter().find(|p| {
                career.announced_result.as_deref() == Some(result.result_id.as_str())
                    && career.announced_player_id == Some(p.player_id)
            })
        })
}
fn result_title(result: &MatchResult, career: &CareerClient) -> &'static str {
    match result.outcome {
        MatchOutcome::Abandoned => "Match abandoned",
        MatchOutcome::Interrupted => "Match interrupted",
        MatchOutcome::Completed => match (result.winner, local_participant(result, career)) {
            (Some(winner), Some(player)) if winner == player.team => "Victory",
            (Some(_), Some(_)) => "Defeat",
            _ => "Match complete",
        },
    }
}
pub(crate) fn queue_text(queue: &QueueView) -> Option<String> {
    match queue {
        QueueView::Waiting {
            compatible,
            needed,
            elapsed_secs,
            newcomer,
        } => Some(format!(
            "Finding a fair match · {compatible}/{needed} compatible players · {}\n{}",
            duration(elapsed_secs.saturating_mul(1000)),
            if *newcomer {
                "New players are matched with other newcomers."
            } else {
                "Waiting for players with similar experience and rating."
            }
        )),
        QueueView::Full => Some("The waiting queue is full. Try again shortly.".into()),
        QueueView::Selected => Some("Match found · preparing teams…".into()),
        _ => None,
    }
}
/// Asset path of a career portrait's avatar thumbnail. `thumbnail` is the
/// passport resolver, so store avatars load from `ekza://` like everywhere
/// else instead of a bundled `avatars/` file that does not exist.
fn avatar_portrait_path(
    avatar: Option<&str>,
    thumbnail: impl Fn(&shared::AvatarDefinition) -> Option<String>,
) -> Option<String> {
    avatar.and_then(shared::avatar_definition).and_then(thumbnail)
}

fn portrait(
    parent: &mut ChildSpawnerCommands,
    avatar: Option<&str>,
    sprite: Option<&str>,
    mode: PlayerVisualMode,
    assets: Option<&AssetServer>,
    sprites: Option<&SpriteVisualAssets>,
    size: f32,
) {
    let image = if mode == PlayerVisualMode::Sprite2d {
        sprite
            .and_then(|id| {
                shared::sprite_character_roster()
                    .iter()
                    .position(|entry| entry.id == id)
            })
            .zip(sprites)
            .map(|(index, sprites)| {
                let (image, layout, index) = sprites.portrait(index);
                ImageNode::from_atlas_image(image, TextureAtlas { layout, index })
            })
    } else {
        None
    }
    .or_else(|| {
        avatar_portrait_path(avatar, crate::passport::thumbnail_asset_path)
            .zip(assets)
            .map(|(path, assets)| ImageNode::new(assets.load(path)))
    });
    let mut entity = parent.spawn((
        Node {
            width: Val::Px(size),
            height: Val::Px(size),
            flex_shrink: 0.0,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(ui::TILE),
        Name::new("CareerPortrait"),
    ));
    if let Some(image) = image {
        entity.insert(image);
    } else {
        entity.with_children(|p| label(p, "H", 22.0, ui::GOLD, "CareerPortraitFallback"));
    }
}
fn hero_name(player: &ParticipantResult) -> String {
    let cosmetic = player
        .avatar
        .as_deref()
        .and_then(shared::avatar_definition)
        .map(|a| a.display_name.as_str())
        .or(player.sprite_character.as_deref())
        .unwrap_or(&player.character);
    format!("{} · {cosmetic}", player.hero_class.display_name())
}
fn player_detail(parent: &mut ChildSpawnerCommands, player: &ParticipantResult) {
    let s = &player.stats;
    label(
        parent,
        format!(
            "Damage to heroes: {}   ·   Creeps: {}\nStructures: {}   ·   Damage taken: {}\nMinion last hits: {}   ·   Jungle last hits: {}\nStructures destroyed: {}   ·   Hero level: {}",
            number(s.damage_to_heroes),
            number(s.damage_to_creeps),
            number(s.damage_to_structures),
            number(s.damage_taken),
            s.minion_last_hits,
            s.jungle_last_hits,
            s.structures_destroyed,
            s.final_level
        ),
        14.0,
        ui::IVORY,
        "CareerPlayerStats",
    );
}
fn desktop_team(
    parent: &mut ChildSpawnerCommands,
    result: &MatchResult,
    team: shared::map::Team,
    assets: Option<&AssetServer>,
    sprites: Option<&SpriteVisualAssets>,
    mode: PlayerVisualMode,
) {
    team_label(parent, result, team);
    parent
        .spawn((row_node(), Name::new("CareerDesktopTableHeader")))
        .with_children(|p| {
            for (title, width) in [
                ("PLAYER / HERO", 280.0),
                ("K / D / A", 90.0),
                ("HERO DMG", 100.0),
                ("CREEP DMG", 100.0),
                ("STRUCTURE", 100.0),
                ("TAKEN", 90.0),
            ] {
                cell(p, title.into(), width, ui::MUTED);
            }
        });
    for player in result.participants.iter().filter(|p| p.team == team) {
        parent
            .spawn((
                Node {
                    padding: UiRect::all(Val::Px(8.0)),
                    ..row_node()
                },
                BackgroundColor(ui::TILE),
                Name::new(format!("CareerDesktopPlayer-{}", player.player_id)),
            ))
            .with_children(|p| {
                p.spawn(Node {
                    width: Val::Px(280.0),
                    flex_shrink: 0.0,
                    ..row_node()
                })
                .with_children(|p| {
                    portrait(
                        p,
                        player.avatar.as_deref(),
                        player.sprite_character.as_deref(),
                        mode,
                        assets,
                        sprites,
                        38.0,
                    );
                    p.spawn(Node {
                        width: Val::Px(220.0),
                        ..column_node()
                    })
                    .with_children(|p| {
                        label(
                            p,
                            if player.is_bot {
                                format!("{} · BOT", player.nickname)
                            } else {
                                player.nickname.clone()
                            },
                            16.0,
                            ui::IVORY,
                            "CareerPlayerNickname",
                        );
                        label(p, hero_name(player), 12.0, ui::MUTED, "CareerPlayerHero");
                    });
                });
                cell(
                    p,
                    format!(
                        "{} / {} / {}",
                        player.stats.kills, player.stats.deaths, player.stats.assists
                    ),
                    90.0,
                    ui::IVORY,
                );
                for (v, w) in [
                    (player.stats.damage_to_heroes, 100.0),
                    (player.stats.damage_to_creeps, 100.0),
                    (player.stats.damage_to_structures, 100.0),
                    (player.stats.damage_taken, 90.0),
                ] {
                    cell(p, number(v), w, ui::IVORY);
                }
                label(
                    p,
                    format!(
                        "{} · +{} career XP{}",
                        rating(player.rating.as_ref()),
                        player.progression_xp_gained,
                        if player.disconnected {
                            " · Disconnected"
                        } else {
                            ""
                        }
                    ),
                    12.0,
                    ui::MUTED,
                    "CareerPlayerProgress",
                );
            });
    }
}
fn cell(parent: &mut ChildSpawnerCommands, text: String, width: f32, color: Color) {
    parent
        .spawn(Node {
            width: Val::Px(width),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|p| label(p, text, 14.0, color, "CareerCell"));
}
fn team_label(parent: &mut ChildSpawnerCommands, result: &MatchResult, team: shared::map::Team) {
    let name = if team == shared::map::Team::Green {
        "Green team"
    } else {
        "Blue team"
    };
    let outcome = match result.winner {
        Some(winner) if winner == team => " · Winners",
        Some(_) => " · Defeated",
        None => "",
    };
    label(
        parent,
        format!("{name}{outcome}"),
        18.0,
        ui::GOLD,
        "CareerTeamLabel",
    );
}
fn mobile_team(
    parent: &mut ChildSpawnerCommands,
    result: &MatchResult,
    team: shared::map::Team,
    career: &CareerClient,
    assets: Option<&AssetServer>,
    sprites: Option<&SpriteVisualAssets>,
    mode: PlayerVisualMode,
) {
    team_label(parent, result, team);
    for player in result.participants.iter().filter(|p| p.team == team) {
        parent
            .spawn((
                Node {
                    padding: UiRect::all(Val::Px(8.0)),
                    ..column_node()
                },
                BackgroundColor(ui::TILE),
                Name::new(format!("CareerMobilePlayer-{}", player.player_id)),
            ))
            .with_children(|p| {
                p.spawn(row_node()).with_children(|p| {
                    portrait(
                        p,
                        player.avatar.as_deref(),
                        player.sprite_character.as_deref(),
                        mode,
                        assets,
                        sprites,
                        34.0,
                    );
                    p.spawn(Node {
                        flex_grow: 1.0,
                        flex_basis: Val::Px(160.0),
                        ..column_node()
                    })
                    .with_children(|p| {
                        label(
                            p,
                            if player.is_bot {
                                format!("{} · BOT", player.nickname)
                            } else {
                                player.nickname.clone()
                            },
                            16.0,
                            ui::IVORY,
                            "CareerPlayerNickname",
                        );
                        label(p, hero_name(player), 12.0, ui::MUTED, "CareerPlayerHero");
                    });
                    button(
                        p,
                        if career.expanded_player == Some(player.player_id) {
                            "Less"
                        } else {
                            "Stats"
                        },
                        Action::Expand(player.player_id),
                        "CareerPlayerDetails",
                    );
                });
                label(
                    p,
                    format!(
                        "{} / {} / {} KDA · {} hero damage\n{} · +{} career XP{}",
                        player.stats.kills,
                        player.stats.deaths,
                        player.stats.assists,
                        number(player.stats.damage_to_heroes),
                        rating(player.rating.as_ref()),
                        player.progression_xp_gained,
                        if player.disconnected {
                            " · Disconnected"
                        } else {
                            ""
                        }
                    ),
                    14.0,
                    ui::IVORY,
                    "CareerPlayerSummary",
                );
                if career.expanded_player == Some(player.player_id) {
                    player_detail(p, player);
                }
            });
    }
}
fn result_body(
    parent: &mut ChildSpawnerCommands,
    career: &CareerClient,
    profile: UiProfile,
    assets: Option<&AssetServer>,
    sprites: Option<&SpriteVisualAssets>,
    mode: PlayerVisualMode,
) {
    let Some(result) = career.result() else {
        label(
            parent,
            if career.view.error.is_some() || career.request_error.is_some() {
                "The result could not be loaded."
            } else {
                "Loading match result…"
            },
            18.0,
            ui::MUTED,
            "CareerResultLoading",
        );
        return;
    };
    label(
        parent,
        result_title(result, career),
        if profile == UiProfile::Mobile {
            26.0
        } else {
            34.0
        },
        ui::GOLD,
        "CareerResultTitle",
    );
    label(
        parent,
        format!(
            "{} played · {}\n{}",
            duration(result.duration_ms),
            if result.rated {
                "Rated match"
            } else {
                "Unrated match"
            },
            if result.saved {
                "Result saved"
            } else if result.ruleset != "practice-bots-v1" && career.view.storage_enabled {
                "Saving result…"
            } else {
                "Local practice result · not saved"
            }
        ),
        15.0,
        ui::IVORY,
        "CareerResultMeta",
    );
    if let Some(reason) = result.unrated_reason.as_ref() {
        label(parent, reason, 14.0, ui::MUTED, "CareerUnratedReason");
    }
    if let Some(me) = local_participant(result, career) {
        label(
            parent,
            format!(
                "Your rating: {}  ·  +{} career XP",
                rating(me.rating.as_ref()),
                me.progression_xp_gained
            ),
            17.0,
            ui::JADE,
            "CareerMyProgress",
        );
    }
    for team in [shared::map::Team::Green, shared::map::Team::Blue] {
        if profile == UiProfile::Mobile {
            mobile_team(parent, result, team, career, assets, sprites, mode)
        } else {
            desktop_team(parent, result, team, assets, sprites, mode)
        }
    }
}
fn profile_body(parent: &mut ChildSpawnerCommands, career: &CareerClient) {
    label(parent, "Your profile", 26.0, ui::GOLD, "CareerProfileTitle");
    if let Some(profile) = &career.view.profile {
        friend_code_label(parent, &profile.nickname);
        label(
            parent,
            format!(
                "Career level {} · {} XP\nRating {} · {} rated matches\n{} wins · {} losses · {} matches played",
                profile.level(),
                profile.progression_xp,
                profile.rating,
                profile.rated_matches,
                profile.wins,
                profile.losses,
                profile.matches_played
            ),
            17.0,
            ui::IVORY,
            "CareerProfileProgress",
        );
        if profile.newcomer() {
            label(
                parent,
                "Newcomer matchmaking",
                14.0,
                ui::JADE,
                "CareerNewcomer",
            );
        }
    } else {
        label(
            parent,
            if career.view.loading {
                "Connecting your profile…"
            } else {
                "Saved progress is not available yet. Connect to a server that supports profiles."
            },
            16.0,
            ui::MUTED,
            "CareerProfilePending",
        );
    }
    label(
        parent,
        "Player tag · name (1–20 characters) # 4 digits",
        15.0,
        ui::IVORY,
        "CareerNicknamePrompt",
    );
    button(
        parent,
        &format!(
            "{}{}{}",
            career.draft,
            career.preedit,
            if career.nickname_focused { " |" } else { "" }
        ),
        Action::EditName,
        "CareerNicknameField",
    );
    label(
        parent,
        career.form_error.as_deref().unwrap_or(""),
        14.0,
        ui::GOLD,
        "CareerNicknameError",
    );
    button(
        parent,
        "Save player tag",
        Action::SaveName,
        "CareerSaveNickname",
    );
    label(
        parent,
        "Rating measures match results. Career XP records your progress.",
        14.0,
        ui::MUTED,
        "CareerRatingExplanation",
    );
    parent
        .spawn(Node {
            margin: UiRect::top(Val::Px(12.0)),
            column_gap: Val::Px(8.0),
            row_gap: Val::Px(8.0),
            flex_wrap: FlexWrap::Wrap,
            ..default()
        })
        .with_children(|row| {
            button(
                row,
                "Link or recover account",
                Action::DevicesOpen,
                "CareerDevices",
            );
            button(row, "Player website", Action::WebOpen, "CareerWebsite");
            button(row, "Support OMOBA", Action::Supporter, "CareerSupporter");
        });
}

fn friend_code_label(parent: &mut ChildSpawnerCommands, handle: &str) {
    label(
        parent,
        "Player tag · share this with friends",
        13.0,
        ui::MUTED,
        "CareerFriendCodePrompt",
    );
    label(parent, handle, 20.0, ui::JADE, "CareerFriendCode");
}

fn profile_summary(parent: &mut ChildSpawnerCommands, profile: &ProfileSummary) {
    label(
        parent,
        &profile.nickname,
        24.0,
        ui::IVORY,
        "CareerVisitedNickname",
    );
    label(
        parent,
        format!(
            "Career level {} · {} XP\nRating {} · {} rated matches\n{} wins · {} losses · {} matches played",
            profile.level(),
            profile.progression_xp,
            profile.rating,
            profile.rated_matches,
            profile.wins,
            profile.losses,
            profile.matches_played
        ),
        17.0,
        ui::IVORY,
        "CareerVisitedStats",
    );
    if profile.newcomer() {
        label(
            parent,
            "Newcomer matchmaking",
            14.0,
            ui::JADE,
            "CareerVisitedNewcomer",
        );
    }
    friend_code_label(parent, &profile.nickname);
}

fn friend_row(
    parent: &mut ChildSpawnerCommands,
    friend: &FriendProfile,
    actions: &[(&str, FriendAction)],
) {
    parent
        .spawn((
            Node {
                padding: UiRect::all(Val::Px(8.0)),
                ..column_node()
            },
            BackgroundColor(ui::TILE),
            Name::new(format!("CareerFriend-{}", friend.profile.profile_id)),
        ))
        .with_children(|row| {
            let presence = match friend.presence {
                FriendPresence::Offline => "Offline",
                FriendPresence::Online => "Online",
                FriendPresence::Playing => "In a match",
            };
            label(
                row,
                &friend.profile.nickname,
                18.0,
                ui::IVORY,
                "CareerFriendNickname",
            );
            label(
                row,
                format!(
                    "{presence} · Rating {} · Career level {}",
                    friend.profile.rating,
                    friend.profile.level()
                ),
                14.0,
                if friend.presence == FriendPresence::Offline {
                    ui::MUTED
                } else {
                    ui::JADE
                },
                "CareerFriendPresence",
            );
            row.spawn(row_node()).with_children(|buttons| {
                button(
                    buttons,
                    "Profile",
                    Action::VisitProfile(friend.profile.profile_id.clone()),
                    "CareerFriendProfile",
                );
                for (title, action) in actions {
                    button(
                        buttons,
                        title,
                        Action::Friend(friend.profile.profile_id.clone(), *action),
                        &format!("CareerFriend{action:?}"),
                    );
                }
            });
        });
}

fn friends_body(parent: &mut ChildSpawnerCommands, career: &CareerClient) {
    label(parent, "Friends", 26.0, ui::GOLD, "CareerFriendsTitle");
    label(
        parent,
        "Friends and their status on this server. Refresh to update.",
        14.0,
        ui::MUTED,
        "CareerFriendsScope",
    );
    if let Some(profile) = &career.view.profile {
        friend_code_label(parent, &profile.nickname);
    }
    label(
        parent,
        "Find a friend by nickname#1234",
        15.0,
        ui::IVORY,
        "CareerFriendInputPrompt",
    );
    let draft = &career.friend_code;
    button(
        parent,
        &field_text(
            draft,
            &career.preedit,
            career.friend_code_focused,
            "Nickname#1234",
        ),
        Action::EditFriendCode,
        "CareerFriendCodeField",
    );
    label(
        parent,
        career.form_error.as_deref().unwrap_or(""),
        14.0,
        ui::GOLD,
        "CareerFriendCodeError",
    );
    if let Some(found) = &career.view.found_player {
        label(
            parent,
            format!("Found: {}", found.nickname),
            17.0,
            ui::JADE,
            "CareerFoundPlayer",
        );
        if career.public_profile_id.as_ref() != Some(&found.profile_id) {
            button(
                parent,
                "Send friend request",
                Action::AddFriend,
                "CareerSendFriendRequest",
            );
        }
    }
    parent.spawn(row_node()).with_children(|buttons| {
        button(
            buttons,
            "Find player",
            Action::LookupFriend,
            "CareerLookupProfile",
        );
        button(buttons, "Refresh", Action::Friends, "CareerFriendsRefresh");
    });
    if career.view.loading {
        label(
            parent,
            "Updating friends…",
            16.0,
            ui::MUTED,
            "CareerFriendsLoading",
        );
    }
    if career.request_error.is_some() || career.view.error.is_some() {
        label(
            parent,
            "The list may be out of date. Refresh to try again.",
            15.0,
            ui::MUTED,
            "CareerFriendsUnavailable",
        );
    }
    let Some(friends) = &career.view.friends else {
        if !career.view.loading {
            label(
                parent,
                "Connect your profile, then refresh your friends list.",
                16.0,
                ui::MUTED,
                "CareerFriendsUnloaded",
            );
        }
        return;
    };
    label(
        parent,
        format!("Incoming requests · {}", friends.incoming.len()),
        18.0,
        ui::GOLD,
        "CareerIncomingTitle",
    );
    for friend in &friends.incoming {
        friend_row(
            parent,
            friend,
            &[
                ("Accept", FriendAction::Accept),
                ("Decline", FriendAction::Reject),
            ],
        );
    }
    label(
        parent,
        format!("Friends · {}", friends.friends.len()),
        18.0,
        ui::GOLD,
        "CareerFriendsListTitle",
    );
    if friends.friends.is_empty() {
        label(
            parent,
            "No friends yet. Send a request using a friend code.",
            15.0,
            ui::MUTED,
            "CareerFriendsEmpty",
        );
    }
    for friend in &friends.friends {
        friend_row(parent, friend, &[("Remove", FriendAction::Remove)]);
    }
    label(
        parent,
        format!("Sent requests · {}", friends.outgoing.len()),
        18.0,
        ui::GOLD,
        "CareerOutgoingTitle",
    );
    for friend in &friends.outgoing {
        friend_row(parent, friend, &[("Cancel request", FriendAction::Cancel)]);
    }
}

fn visited_profile_body(parent: &mut ChildSpawnerCommands, career: &CareerClient) {
    label(
        parent,
        "Player profile",
        26.0,
        ui::GOLD,
        "CareerVisitedTitle",
    );
    if career.view.loading {
        label(
            parent,
            "Loading profile…",
            17.0,
            ui::MUTED,
            "CareerVisitedLoading",
        );
        return;
    }
    let profile = career.view.visited_profile.as_ref().filter(|profile| {
        career.visited_profile_id.as_deref() == Some(profile.profile_id.as_str())
    });
    if let Some(profile) = profile {
        profile_summary(parent, profile);
        if career.public_profile_id.as_deref() != Some(profile.profile_id.as_str()) {
            let known = career.view.friends.as_ref().is_some_and(|friends| {
                friends
                    .friends
                    .iter()
                    .chain(&friends.incoming)
                    .chain(&friends.outgoing)
                    .any(|friend| friend.profile.profile_id == profile.profile_id)
            });
            if !known {
                button(
                    parent,
                    "Send friend request",
                    Action::Friend(profile.profile_id.clone(), FriendAction::Request),
                    "CareerVisitedAddFriend",
                );
            }
        }
    } else {
        label(
            parent,
            "This profile is unavailable. Check the friend code and try again.",
            17.0,
            ui::MUTED,
            "CareerVisitedUnavailable",
        );
    }
    button(
        parent,
        "Back to friends",
        Action::Friends,
        "CareerVisitedBack",
    );
}
fn history_body(
    parent: &mut ChildSpawnerCommands,
    career: &CareerClient,
    assets: Option<&AssetServer>,
    sprites: Option<&SpriteVisualAssets>,
    mode: PlayerVisualMode,
) {
    label(
        parent,
        "Match history",
        26.0,
        ui::GOLD,
        "CareerHistoryTitle",
    );
    if career.view.loading {
        label(
            parent,
            "Loading history…",
            17.0,
            ui::MUTED,
            "CareerHistoryLoading",
        );
        return;
    }
    if career.view.error.is_some() || career.request_error.is_some() {
        label(
            parent,
            "History is unavailable. Try refreshing.",
            17.0,
            ui::MUTED,
            "CareerHistoryUnavailable",
        );
        return;
    }
    if !career.view.history_loaded {
        label(
            parent,
            "Load your history to see saved matches.",
            17.0,
            ui::MUTED,
            "CareerHistoryUnloaded",
        );
        return;
    }
    if career.view.history.is_empty() {
        label(
            parent,
            "No saved matches yet.",
            17.0,
            ui::MUTED,
            "CareerHistoryEmpty",
        );
    }
    for (index, item) in career.view.history.iter().enumerate() {
        parent
            .spawn((
                Node {
                    padding: UiRect::all(Val::Px(8.0)),
                    ..row_node()
                },
                BackgroundColor(ui::TILE),
                Name::new(format!("CareerHistoryRow-{index}")),
            ))
            .with_children(|p| {
                portrait(
                    p,
                    item.avatar.as_deref(),
                    item.sprite_character.as_deref(),
                    mode,
                    assets,
                    sprites,
                    36.0,
                );
                p.spawn(Node {
                    flex_grow: 1.0,
                    flex_basis: Val::Px(180.0),
                    ..column_node()
                })
                .with_children(|p| {
                    let outcome = match item.outcome {
                        MatchOutcome::Abandoned => "Abandoned",
                        MatchOutcome::Interrupted => "Interrupted",
                        MatchOutcome::Completed => match item.won {
                            Some(true) => "Victory",
                            Some(false) => "Defeat",
                            None => "Complete",
                        },
                    };
                    label(
                        p,
                        format!(
                            "{outcome} · {} · {}",
                            item.hero_class.display_name(),
                            duration(item.duration_ms)
                        ),
                        16.0,
                        ui::IVORY,
                        "CareerHistoryOutcome",
                    );
                    label(
                        p,
                        timestamp(item.ended_at_ms),
                        12.0,
                        ui::MUTED,
                        "CareerHistoryDate",
                    );
                    label(
                        p,
                        format!(
                            "{} / {} / {} · {} hero damage · {}",
                            item.kills,
                            item.deaths,
                            item.assists,
                            number(item.damage_to_heroes),
                            rating(item.rating.as_ref())
                        ),
                        14.0,
                        ui::MUTED,
                        "CareerHistoryStats",
                    );
                });
                button(
                    p,
                    "Details",
                    Action::Detail(item.result_id.clone()),
                    "CareerHistoryDetail",
                );
            });
    }
    parent.spawn(row_node()).with_children(|p| {
        if !career.history_back.is_empty() {
            button(p, "Previous", Action::Previous, "CareerHistoryPrevious");
        }
        if career.view.history_next.is_some() {
            button(p, "Older", Action::Next, "CareerHistoryNext");
        }
        button(p, "Refresh", Action::Refresh, "CareerHistoryRefresh");
    });
}

#[derive(Clone, PartialEq)]
struct RenderKey {
    career: CareerClient,
    profile: UiProfile,
    viewport: Vec2,
    insets: [f32; 4],
    show_entry: bool,
    selection_recovery: bool,
    mode: PlayerVisualMode,
}
fn render(
    mut commands: Commands,
    career: Res<CareerClient>,
    profile: Res<crate::ui::UiPlatform>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    pause: Option<Res<crate::pause_menu::PauseMenuState>>,
    game: Option<Res<GameStateSnapshot>>,
    session: Option<Res<crate::net::ClientSession>>,
    mode: Option<Res<PlayerVisualMode>>,
    assets: Option<Res<AssetServer>>,
    sprites: Option<Res<SpriteVisualAssets>>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    roots: Query<Entity, With<CareerRoot>>,
    scrolls: Query<&ScrollPosition, With<CareerScroll>>,
    mut previous: Local<Option<RenderKey>>,
    mut texts: Query<(&Name, &mut Text)>,
) {
    // Keep focused widgets alive. Updating glyphs must never replace the modal,
    // reset pointer capture/scroll, or dismiss the phone's IME.
    for (name, mut text) in &mut texts {
        let value = match name.as_str() {
            "CareerFriendCodeFieldLabel" => Some(field_text(
                &career.friend_code,
                &career.preedit,
                career.friend_code_focused,
                "Nickname#1234",
            )),
            "CareerNicknameFieldLabel" => Some(field_text(
                &career.draft,
                &career.preedit,
                career.nickname_focused,
                "Nickname#1234",
            )),
            "DeviceRecoveryFieldLabel" => Some(devices::recovery_field(&career.devices)),
            "DeviceInputError" => Some(career.form_error.clone().unwrap_or_default()),
            "CareerNicknameError" | "CareerFriendCodeError" => {
                Some(career.form_error.clone().unwrap_or_default())
            }
            _ => None,
        };
        if let Some(value) = value {
            if text.0 != value {
                text.0 = value;
            }
        }
    }
    let viewport = windows.single().map_or(Vec2::new(1280.0, 720.0), |w| {
        Vec2::new(w.width(), w.height())
    });
    let insets = mobile
        .as_ref()
        .filter(|_| profile.0 == UiProfile::Mobile)
        .map_or([0.0; 4], |m| {
            [m.safe.left, m.safe.top, m.safe.right, m.safe.bottom]
        });
    let selection_recovery = matches!(career.view.queue, QueueView::Idle)
        && career.view.error.is_some()
        && session
            .as_ref()
            .is_some_and(|session| session.join_in_flight() && !session.join_confirmed());
    // The front end has its own navigation; the in-match career bar would
    // otherwise float over the menus.
    let front_end_menu = screen.as_ref().is_some_and(|screen| screen.get().is_menu());
    let show_entry = !front_end_menu
        && (selection_recovery
            || queue_text(&career.view.queue).is_some()
            || !game
                .as_ref()
                .is_some_and(|g| matches!(g.state, GameState::Running))
            || pause.as_ref().is_some_and(|p| p.open));
    let mode = mode
        .as_deref()
        .copied()
        .unwrap_or(PlayerVisualMode::Models3d);
    let editing = career.nickname_editor_open()
        || (career.modal == CareerModal::Friends && career.friend_code_focused)
        || (career.modal == CareerModal::Devices && career.devices.focused);
    if editing
        && previous.as_ref().is_some_and(|p| {
            p.career.modal == career.modal
                && p.viewport == viewport
                && p.insets == insets
                && p.profile == profile.0
        })
    {
        return;
    }
    let mut render_state = career.clone();
    render_state.draft.clear();
    render_state.friend_code.clear();
    render_state.devices.recovery_code.clear();
    render_state.devices.focused = false;
    render_state.preedit.clear();
    render_state.form_error = None;
    render_state.nickname_focused = false;
    render_state.friend_code_focused = false;
    render_state.ime_owned = false;
    let key = RenderKey {
        career: render_state,
        profile: profile.0,
        viewport,
        insets,
        show_entry,
        selection_recovery,
        mode,
    };
    if previous.as_ref() == Some(&key) {
        return;
    }
    let preserve_scroll = previous.as_ref().is_some_and(|previous| {
        previous.career.modal == career.modal
            && previous.career.history_before == career.history_before
            && previous.career.selected_result == career.selected_result
            && previous.career.visited_profile_id == career.visited_profile_id
    });
    let scroll_position = if preserve_scroll {
        scrolls.single().cloned().unwrap_or_default()
    } else {
        ScrollPosition::default()
    };
    *previous = Some(key);
    for entity in &roots {
        commands.entity(entity).despawn();
    }
    if !career.modal_open() {
        if show_entry {
            commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(insets[0] + 6.0),
                        bottom: Val::Px(insets[3] + 8.0),
                        ..row_node()
                    },
                    ZIndex(110),
                    CareerRoot,
                    Name::new("CareerEntryActions"),
                ))
                .with_children(|p| {
                    button(p, "Profile", Action::Profile, "CareerProfileButton");
                    button(p, "History", Action::History, "CareerHistoryButton");
                    button(p, "Friends", Action::Friends, "CareerFriendsButton");
                    if selection_recovery {
                        button(
                            p,
                            "Back to selection",
                            Action::CancelQueue,
                            "CareerBackToSelection",
                        );
                    }
                    if matches!(career.view.queue, QueueView::Waiting { .. }) {
                        button(
                            p,
                            "Leave queue",
                            Action::CancelQueue,
                            "CareerQueueLeaveButton",
                        );
                    }
                });
        }
        return;
    }
    let phone = profile.0 == UiProfile::Mobile;
    let compact = matches!(
        career.modal,
        CareerModal::Profile
            | CareerModal::WebLink
            | CareerModal::Devices
            | CareerModal::FriendProfile
    );
    let width = (viewport.x - insets[0] - insets[2] - if phone { 12.0 } else { 48.0 })
        .max(180.0)
        .min(if phone {
            f32::MAX
        } else if compact {
            760.0
        } else {
            1120.0
        });
    let height = (viewport.y - insets[1] - insets[3] - if phone { 12.0 } else { 40.0 })
        .max(160.0)
        .min(if phone {
            f32::MAX
        } else {
            match career.modal {
                CareerModal::WebLink => 420.0,
                CareerModal::Devices | CareerModal::FriendProfile => 560.0,
                CareerModal::Profile => 640.0,
                _ => f32::MAX,
            }
        });
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                padding: UiRect {
                    left: Val::Px(insets[0]),
                    right: Val::Px(insets[2]),
                    top: Val::Px(insets[1]),
                    bottom: Val::Px(insets[3]),
                },
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.025, 0.03, 0.94)),
            ZIndex(120),
            CareerRoot,
            Name::new(if phone {
                "CareerMobileRoot"
            } else {
                "CareerDesktopRoot"
            }),
        ))
        .with_children(|outer| {
            outer
                .spawn((
                    Node {
                        width: Val::Px(width),
                        height: Val::Px(height),
                        padding: UiRect::all(Val::Px(if phone { 10.0 } else { 24.0 })),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..column_node()
                    },
                    BackgroundColor(ui::PANEL.with_alpha(1.0)),
                    BorderColor::all(ui::EDGE),
                    Name::new("CareerPanel"),
                ))
                .with_children(|panel| {
                    panel.spawn(row_node()).with_children(|p| {
                        for (title, action, name, selected) in [
                            (
                                "Profile",
                                Action::Profile,
                                "CareerProfileTab",
                                career.modal == CareerModal::Profile,
                            ),
                            (
                                "History",
                                Action::History,
                                "CareerHistoryTab",
                                career.modal == CareerModal::History,
                            ),
                            (
                                "Friends",
                                Action::Friends,
                                "CareerFriendsTab",
                                matches!(
                                    career.modal,
                                    CareerModal::Friends | CareerModal::FriendProfile
                                ),
                            ),
                        ] {
                            let menu = crate::frontend::widgets::MenuButton::tile(selected);
                            p.spawn((
                                Button,
                                menu,
                                action,
                                Name::new(name),
                                Node {
                                    min_height: Val::Px(44.0),
                                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                                    align_items: AlignItems::Center,
                                    border: UiRect::bottom(Val::Px(2.0)),
                                    border_radius: BorderRadius::top(Val::Px(6.0)),
                                    ..default()
                                },
                                BackgroundColor(menu.idle_color()),
                                BorderColor::all(if selected { ui::GOLD } else { ui::EDGE }),
                            ))
                            .with_children(|p| {
                                label(p, title, 15.0, ui::IVORY, &format!("{name}Label"))
                            });
                        }
                        if career.view.last_result.is_some() {
                            button(
                                p,
                                "Last match",
                                Action::LastResult,
                                "CareerLastResultButton",
                            );
                        }
                        if career.modal == CareerModal::History {
                            button(p, "Refresh", Action::Refresh, "CareerHistoryRetry");
                        }
                        button(p, "Close", Action::Close, "CareerClose");
                    });
                    panel
                        .spawn((
                            Node {
                                flex_grow: 1.0,
                                flex_shrink: 1.0,
                                flex_basis: Val::Px(0.0),
                                min_height: Val::Px(0.0),
                                overflow: Overflow::scroll_y(),
                                ..column_node()
                            },
                            scroll_position,
                            CareerScroll,
                            crate::mobile_ui::TouchScrollPanel,
                            Name::new("CareerBody"),
                        ))
                        .with_children(|body| {
                            if let Some(error) =
                                career.request_error.as_ref().or(career.view.error.as_ref())
                            {
                                label(body, error, 14.0, ui::GOLD, "CareerServerError");
                            }
                            if selection_recovery {
                                button(
                                    body,
                                    "Back to selection",
                                    Action::CancelQueue,
                                    "CareerBackToSelection",
                                );
                            }
                            if let Some(queue) = queue_text(&career.view.queue) {
                                label(body, queue, 16.0, ui::IVORY, "CareerQueueStatus");
                                button(
                                    body,
                                    "Leave queue",
                                    Action::CancelQueue,
                                    "CareerCancelQueue",
                                );
                            }
                            match career.modal {
                                CareerModal::Profile => profile_body(body, &career),
                                CareerModal::WebLink => web::body(body, &career),
                                CareerModal::Devices => devices::body(body, &career),
                                CareerModal::Friends => friends_body(body, &career),
                                CareerModal::FriendProfile => visited_profile_body(body, &career),
                                CareerModal::History => history_body(
                                    body,
                                    &career,
                                    assets.as_deref(),
                                    sprites.as_deref(),
                                    mode,
                                ),
                                CareerModal::Result => result_body(
                                    body,
                                    &career,
                                    profile.0,
                                    assets.as_deref(),
                                    sprites.as_deref(),
                                    mode,
                                ),
                                CareerModal::Closed => {}
                            }
                        });
                    if career.modal == CareerModal::Result {
                        panel.spawn(row_node()).with_children(|p| {
                            button(p, "Play again", Action::PlayAgain, "CareerPlayAgain");
                            button(p, "Match history", Action::History, "CareerResultHistory");
                        });
                    }
                });
        });
}
fn scroll_desktop(
    profile: Res<crate::ui::UiPlatform>,
    mut wheel: MessageReader<MouseWheel>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<CareerScroll>>,
) {
    let delta: f32 = wheel
        .read()
        .map(|e| {
            e.y * if e.unit == MouseScrollUnit::Line {
                28.0
            } else {
                1.0
            }
        })
        .sum();
    if profile.0 != UiProfile::Desktop {
        return;
    }
    for (node, mut scroll) in &mut panels {
        let max = ((node.content_size().y - node.size().y) * node.inverse_scale_factor()).max(0.0);
        scroll.y = (scroll.y - delta).clamp(0.0, max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O11: a store avatar's portrait resolves through the passport path
    /// (`ekza://avatars/...`), not a bundled `avatars/` file.
    #[test]
    fn career_portrait_loads_store_avatars_from_the_ekza_source() {
        let avatar = shared::avatar_roster()
            .iter()
            .find(|a| a.thumbnail.is_some())
            .expect("a roster avatar with a thumbnail");
        let file = avatar.thumbnail.as_deref().unwrap();
        let store = avatar_portrait_path(Some(&avatar.slug), |a| {
            crate::passport::thumbnail_asset_path_in(a, true)
        });
        assert_eq!(store, Some(format!("ekza://avatars/{file}")));
        let bundled = avatar_portrait_path(Some(&avatar.slug), |a| {
            crate::passport::thumbnail_asset_path_in(a, false)
        });
        assert_eq!(bundled, Some(format!("avatars/{file}")));
        assert_eq!(
            avatar_portrait_path(Some(&avatar.slug), crate::passport::thumbnail_asset_path),
            bundled,
            "without a store runtime the roster avatar stays bundled"
        );
        assert_eq!(avatar_portrait_path(None, crate::passport::thumbnail_asset_path), None);
    }
    #[test]
    fn career_touch_actions_wait_for_release_and_cancel_after_scroll() {
        use bevy::input::touch::{TouchInput, TouchPhase};
        let mut app = App::new();
        app.init_resource::<CareerClient>()
            .insert_resource(crate::ui::UiPlatform(UiProfile::Mobile))
            .add_message::<TouchInput>()
            .add_systems(Update, crate::ui::gesture::recognize_presses);
        let mut window = Window::default();
        window.resolution.set_scale_factor_override(Some(2.0));
        let window = app.world_mut().spawn((window, PrimaryWindow)).id();
        let button = app
            .world_mut()
            .spawn((
                crate::ui::Pressable::default(),
                ComputedNode {
                    size: Vec2::new(160.0, 88.0),
                    inverse_scale_factor: 0.5,
                    ..default()
                },
                UiGlobalTransform::from(bevy::math::Affine2::from_translation(Vec2::splat(400.0))),
            ))
            .id();
        let e = |id, phase, position| TouchInput {
            id,
            phase,
            position,
            window,
            force: None,
        };
        app.world_mut()
            .write_message(e(1, TouchPhase::Started, Vec2::splat(200.0)));
        app.update();
        assert!(
            !app.world()
                .get::<crate::ui::Pressable>(button)
                .unwrap()
                .activated
        );
        app.world_mut()
            .write_message(e(1, TouchPhase::Moved, Vec2::new(200.0, 140.0)));
        app.world_mut()
            .write_message(e(1, TouchPhase::Ended, Vec2::splat(200.0)));
        app.update();
        assert!(
            !app.world()
                .get::<crate::ui::Pressable>(button)
                .unwrap()
                .activated
        );
        app.world_mut()
            .write_message(e(2, TouchPhase::Started, Vec2::splat(200.0)));
        app.world_mut()
            .write_message(e(2, TouchPhase::Ended, Vec2::splat(201.0)));
        app.update();
        assert!(
            app.world()
                .get::<crate::ui::Pressable>(button)
                .unwrap()
                .activated
        );
        app.update();
        assert!(
            !app.world()
                .get::<crate::ui::Pressable>(button)
                .unwrap()
                .activated
        );
    }

    fn result() -> MatchResult {
        MatchResult {
            result_id: "r1".into(),
            server_epoch: 1,
            match_id: 1,
            started_at_ms: 0,
            ended_at_ms: 90000,
            duration_ms: 90000,
            map_profile: "verdant_default".into(),
            ruleset: "default".into(),
            outcome: MatchOutcome::Completed,
            winner: Some(shared::map::Team::Green),
            rated: false,
            unrated_reason: Some("Practice match".into()),
            participants: vec![ParticipantResult {
                is_bot: false,
                player_id: 7,
                profile_id: Some("a".repeat(64)),
                nickname: "Дмитрий".into(),
                team: shared::map::Team::Green,
                hero_class: shared::HeroClass::Mage,
                character: "ipfs".into(),
                avatar: Some("agnes".into()),
                sprite_character: None,
                stats: shared::career::MatchStats {
                    kills: 2,
                    damage_to_heroes: 123.0,
                    ..default()
                },
                disconnected: false,
                rating: None,
                progression_xp_gained: 50,
            }],
            saved: true,
        }
    }
    #[test]
    fn results_survive_live_reset_and_close_without_reopening() {
        let mut career = CareerClient {
            public_profile_id: Some("a".repeat(64)),
            ..default()
        };
        career.apply_view(CareerView {
            last_result: Some(result()),
            ..default()
        });
        assert_eq!(career.modal, CareerModal::Result);
        assert_eq!(result_title(career.result().unwrap(), &career), "Victory");
        career.close();
        career.apply_view(CareerView::default());
        assert!(!career.modal_open());
        assert!(career.view.last_result.is_some());
        career.clear_account();
        assert!(career.view.last_result.is_none());
        assert!(career.public_profile_id.is_none());
    }
    #[test]
    fn absent_live_player_is_not_a_false_defeat_and_detail_never_reuses_wrong_match() {
        let mut career = CareerClient::default();
        assert_eq!(result_title(&result(), &career), "Match complete");
        career.apply_view(CareerView {
            last_result: Some(result()),
            ..default()
        });
        career.selected_result = Some("other".into());
        assert!(career.result().is_none());
    }
    #[test]
    fn unicode_editor_is_bounded_and_does_not_split_utf8() {
        let mut value = "Дми".to_string();
        append_name(&mut value, "трий").unwrap();
        assert_eq!(
            shared::career::normalize_nickname(&value).unwrap(),
            "Дмитрий"
        );
        value.pop();
        assert_eq!(value, "Дмитри");
        let before = value.clone();
        assert!(append_name(&mut value, &"界".repeat(20)).is_err());
        assert_eq!(value, before);
    }
    fn render_app(profile: UiProfile, modal: CareerModal) -> App {
        let mut app = App::new();
        app.insert_resource(crate::ui::UiPlatform(profile))
            .insert_resource(CareerClient { modal, ..default() })
            .add_systems(Update, render);
        app
    }
    fn texts(app: &mut App) -> Vec<String> {
        app.world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|t| t.0.clone())
            .collect()
    }
    #[test]
    fn result_save_status_distinguishes_local_practice_pending_storage_and_saved() {
        for profile in [UiProfile::Desktop, UiProfile::Mobile] {
            let mut app = render_app(profile, CareerModal::Result);
            let mut local_result = result();
            local_result.saved = false;
            app.world_mut()
                .resource_mut::<CareerClient>()
                .apply_view(CareerView {
                    last_result: Some(local_result),
                    ..default()
                });
            app.update();
            assert!(
                texts(&mut app)
                    .iter()
                    .any(|text| { text.contains("Local practice result · not saved") })
            );
            {
                let mut career = app.world_mut().resource_mut::<CareerClient>();
                career.view.storage_enabled = true;
                career.view.error = Some("The database is temporarily unavailable.".into());
            }
            app.update();
            assert!(
                texts(&mut app)
                    .iter()
                    .any(|text| text.contains("Saving result…"))
            );
            app.world_mut()
                .resource_mut::<CareerClient>()
                .view
                .last_result
                .as_mut()
                .unwrap()
                .saved = true;
            app.update();
            assert!(
                texts(&mut app)
                    .iter()
                    .any(|text| text.contains("Result saved"))
            );
        }
    }
    #[test]
    fn history_unloaded_and_loaded_empty_are_distinct() {
        let mut app = render_app(UiProfile::Desktop, CareerModal::History);
        app.update();
        assert!(
            texts(&mut app)
                .iter()
                .any(|t| t.contains("Load your history"))
        );
        app.world_mut()
            .resource_mut::<CareerClient>()
            .view
            .history_loaded = true;
        app.update();
        assert!(texts(&mut app).iter().any(|t| t == "No saved matches yet."));
        app.world_mut().resource_mut::<CareerClient>().view.loading = true;
        app.update();
        assert!(texts(&mut app).iter().any(|t| t == "Loading history…"));
    }
    #[test]
    fn same_viewport_keeps_target_specific_results_and_scroll_ownership() {
        for (profile, expected, other) in [
            (
                UiProfile::Desktop,
                "CareerDesktopPlayer-7",
                "CareerMobilePlayer-7",
            ),
            (
                UiProfile::Mobile,
                "CareerMobilePlayer-7",
                "CareerDesktopPlayer-7",
            ),
        ] {
            let mut app = render_app(profile, CareerModal::Closed);
            app.world_mut()
                .resource_mut::<CareerClient>()
                .apply_view(CareerView {
                    last_result: Some(result()),
                    ..default()
                });
            app.update();
            let names: Vec<_> = app
                .world_mut()
                .query::<&Name>()
                .iter(app.world())
                .map(|n| n.as_str().to_owned())
                .collect();
            assert!(names.iter().any(|n| n == expected));
            assert!(!names.iter().any(|n| n == other));
            assert_eq!(
                app.world_mut()
                    .query_filtered::<Entity, (With<CareerScroll>, With<ScrollPosition>)>()
                    .iter(app.world())
                    .count(),
                1
            );
        }
    }
    #[test]
    fn guest_result_identity_is_frozen_before_live_player_changes() {
        let mut career = CareerClient {
            local_player_id: Some(7),
            ..default()
        };
        career.apply_view(CareerView {
            last_result: Some(result()),
            ..default()
        });
        career.local_player_id = Some(999);
        assert_eq!(result_title(career.result().unwrap(), &career), "Victory");
    }
    #[test]
    fn native_ime_commit_is_not_duplicated_by_keyboard_text() {
        let mut app = App::new();
        app.init_resource::<CareerClient>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<KeyboardInput>()
            .add_message::<Ime>()
            .add_message::<NicknameChanged>()
            .add_message::<NetworkCommand>()
            .add_systems(Update, nickname_input);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        {
            let mut career = app.world_mut().resource_mut::<CareerClient>();
            career.nickname.clear();
            career.open_profile();
        }
        app.world_mut().write_message(Ime::Commit {
            window,
            value: "小明".into(),
        });
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code: KeyCode::KeyA,
            logical_key: Key::Character("小明".into()),
            text: Some("小明".into()),
            state: bevy::input::ButtonState::Pressed,
            repeat: false,
        });
        app.update();
        assert_eq!(app.world().resource::<CareerClient>().draft, "小明");
        assert!(app.world().get::<Window>(window).unwrap().ime_enabled);
        app.world_mut().resource_mut::<CareerClient>().close();
        app.update();
        assert!(!app.world().get::<Window>(window).unwrap().ime_enabled);
    }
    #[test]
    fn history_dates_use_authoritative_utc_timestamps() {
        assert_eq!(timestamp(0), "1970-01-01 00:00 UTC");
        assert_eq!(timestamp(951_782_400_000), "2000-02-29 00:00 UTC");
    }
    #[test]
    fn legacy_or_silent_backend_cannot_leave_history_loading_forever() {
        let mut career = CareerClient::default();
        career.begin_request(PendingKind::History);
        career.apply_view(CareerView::default());
        assert!(career.view.loading);
        career.expire_request(Instant::now() + Duration::from_secs(9));
        assert!(!career.view.loading);
        assert!(career.request_error.is_some());
        career.apply_view(CareerView::default());
        assert!(career.request_error.is_some());
        let request_id = career.begin_request(PendingKind::History);
        career.apply_view(CareerView {
            response_id: Some(request_id),
            history_loaded: true,
            ..default()
        });
        assert!(!career.view.loading);
        assert!(career.pending.is_none());
        assert!(career.request_error.is_none());
    }
    fn history_page(request_id: u64, name: &str, next: u64) -> CareerView {
        CareerView {
            response_id: Some(request_id),
            history_loaded: true,
            history_next: Some(next),
            history: vec![shared::career::MatchSummary {
                result_id: name.into(),
                ended_at_ms: next,
                duration_ms: 1000,
                outcome: MatchOutcome::Completed,
                won: Some(true),
                hero_class: shared::HeroClass::Mage,
                avatar: None,
                sprite_character: None,
                kills: 1,
                deaths: 0,
                assists: 0,
                damage_to_heroes: 12.0,
                rating: None,
            }],
            ..default()
        }
    }
    #[test]
    fn late_history_page_cannot_complete_or_replace_a_newer_page() {
        let mut career = CareerClient::default();
        let a = career.begin_request(PendingKind::History);
        let page_a = history_page(a, "page-a", 100);
        career.apply_view(page_a.clone());
        assert!(career.pending.is_none());
        let b = career.begin_request(PendingKind::History);
        assert!(b > a);
        career.apply_view(page_a.clone());
        assert!(career.pending.is_some());
        assert!(career.view.loading);
        career.apply_view(history_page(b, "page-b", 50));
        assert!(career.pending.is_none());
        assert!(!career.view.loading);

        let mut late_a = page_a;
        late_a.error = Some("an old request failed".into());
        late_a.queue = QueueView::Waiting {
            compatible: 3,
            needed: 10,
            elapsed_secs: 12,
            newcomer: true,
        };
        late_a.profile = Some(shared::career::ProfileSummary::new(
            "a".repeat(64),
            "Updated".into(),
        ));
        late_a.last_result = Some(result());
        career.apply_view(late_a);
        assert_eq!(career.view.history[0].result_id, "page-b");
        assert_eq!(career.view.history_next, Some(50));
        assert!(career.view.error.is_none());
        assert!(career.request_error.is_none());
        assert!(matches!(
            career.view.queue,
            QueueView::Waiting { compatible: 3, .. }
        ));
        assert_eq!(career.view.profile.as_ref().unwrap().nickname, "Updated");
        assert!(career.view.last_result.is_some());
        career.apply_view(CareerView::default());
        assert_eq!(career.view.history[0].result_id, "page-b");
    }
    #[test]
    fn detail_requires_matching_request_and_result_and_survives_live_snapshots() {
        let mut career = CareerClient::default();
        let a = career.begin_request(PendingKind::Detail("a".into()));
        let b = career.begin_request(PendingKind::Detail("b".into()));
        let mut result_a = result();
        result_a.result_id = "a".into();
        let mut result_b = result();
        result_b.result_id = "b".into();
        for response_id in [a, b] {
            career.apply_view(CareerView {
                response_id: Some(response_id),
                detail: Some(result_a.clone()),
                ..default()
            });
            assert!(career.pending.is_some());
            assert!(career.view.detail.is_none());
        }
        career.apply_view(CareerView {
            response_id: Some(b),
            detail: Some(result_b.clone()),
            ..default()
        });
        assert!(career.pending.is_none());
        career.apply_view(CareerView {
            response_id: Some(a),
            detail: Some(result_a),
            ..default()
        });
        assert_eq!(career.view.detail.as_ref(), Some(&result_b));
        career.apply_view(CareerView::default());
        assert_eq!(career.view.detail.as_ref(), Some(&result_b));
    }
    #[test]
    fn namespace_reset_does_not_reuse_query_ids_or_accept_old_history() {
        let mut career = CareerClient::default();
        let old = career.begin_request(PendingKind::History);
        career.clear_account();
        let current = career.begin_request(PendingKind::History);
        assert!(current > old);
        career.apply_view(history_page(old, "old-server", 100));
        assert!(career.view.history.is_empty());
        assert!(career.pending.is_some());
        career.apply_view(history_page(current, "new-server", 50));
        assert_eq!(career.view.history[0].result_id, "new-server");
        assert!(career.pending.is_none());
    }
    #[test]
    fn career_entry_and_dashboard_are_above_the_pause_overlay() {
        let mut app = render_app(UiProfile::Desktop, CareerModal::Closed);
        app.update();
        let entry_layer = app
            .world_mut()
            .query_filtered::<&ZIndex, With<CareerRoot>>()
            .single(app.world())
            .unwrap()
            .0;
        assert!(entry_layer > 100, "pause overlay uses layer 100");
        app.world_mut().resource_mut::<CareerClient>().modal = CareerModal::History;
        app.update();
        let modal_layer = app
            .world_mut()
            .query_filtered::<&ZIndex, With<CareerRoot>>()
            .single(app.world())
            .unwrap()
            .0;
        assert!(modal_layer > entry_layer);
    }
    #[test]
    fn escape_closes_career_without_toggling_the_underlying_pause_menu() {
        let mut app = App::new();
        app.insert_resource(CareerClient {
            modal: CareerModal::History,
            ..default()
        })
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<crate::pause_menu::PauseMenuState>()
        .add_systems(
            Update,
            (dismiss_with_escape, crate::pause_menu::toggle_pause_menu).chain(),
        );
        app.world_mut()
            .resource_mut::<crate::pause_menu::PauseMenuState>()
            .open = true;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        assert!(!app.world().resource::<CareerClient>().modal_open());
        assert!(
            app.world()
                .resource::<crate::pause_menu::PauseMenuState>()
                .open
        );
        assert!(
            !app.world()
                .resource::<ButtonInput<KeyCode>>()
                .just_pressed(KeyCode::Escape)
        );
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.reset(KeyCode::Escape);
            keys.press(KeyCode::Escape);
        }
        app.update();
        assert!(
            !app.world()
                .resource::<crate::pause_menu::PauseMenuState>()
                .open
        );
    }
    #[test]
    fn player_handle_input_is_unicode_bounded_and_atomic() {
        let mut field = String::new();
        append_friend_code(&mut field, "Лесная Лиса#0042").unwrap();
        assert_eq!(field, "Лесная Лиса#0042");
        assert!(shared::career::normalize_player_handle(&field).is_ok());
        let before = field.clone();
        assert!(append_friend_code(&mut field, "\ninvalid").is_err());
        assert_eq!(field, before);
        assert!(append_friend_code(&mut field, &"a".repeat(30)).is_err());
        assert_eq!(field, before);
    }
    #[test]
    fn focused_friend_field_keeps_its_entity_and_scroll_on_every_keystroke() {
        for profile in [UiProfile::Desktop, UiProfile::Mobile] {
            let mut app = render_app(profile, CareerModal::Friends);
            app.update();
            let root = app
                .world_mut()
                .query_filtered::<Entity, With<CareerRoot>>()
                .single(app.world())
                .unwrap();
            let field = app
                .world_mut()
                .query::<(Entity, &Name)>()
                .iter(app.world())
                .find(|(_, n)| n.as_str() == "CareerFriendCodeField")
                .unwrap()
                .0;
            let scroll = app
                .world_mut()
                .query_filtered::<Entity, With<CareerScroll>>()
                .single(app.world())
                .unwrap();
            app.world_mut()
                .entity_mut(scroll)
                .insert(ScrollPosition(Vec2::new(0.0, 37.0)));
            app.world_mut()
                .resource_mut::<CareerClient>()
                .friend_code_focused = true;
            for ch in "MossFox#0427".chars() {
                {
                    let mut c = app.world_mut().resource_mut::<CareerClient>();
                    c.friend_code.push(ch);
                    c.ime_owned = true;
                }
                app.update();
                assert!(app.world().get_entity(root).is_ok());
                assert!(app.world().get_entity(field).is_ok());
                assert_eq!(app.world().get::<ScrollPosition>(scroll).unwrap().y, 37.0);
                let draft = app.world().resource::<CareerClient>().friend_code.clone();
                assert!(texts(&mut app).contains(&format!("{draft} |")));
            }
            assert_eq!(
                app.world().get::<Node>(field).unwrap().height,
                Val::Px(52.0)
            );
        }
    }
    #[test]
    fn handle_lookup_ignores_stale_reply_and_preserves_stable_account_id() {
        let mut c = CareerClient::default();
        let old = c.begin_request(PendingKind::Lookup("Old#0001".into()));
        let current = c.begin_request(PendingKind::Lookup("New#0002".into()));
        let found = shared::career::PlayerReference {
            profile_id: "a".repeat(64),
            nickname: "New#0002".into(),
        };
        c.apply_view(CareerView {
            response_id: Some(old),
            found_player: Some(found.clone()),
            ..default()
        });
        assert!(c.view.found_player.is_none());
        c.apply_view(CareerView {
            response_id: Some(current),
            found_player: Some(found.clone()),
            ..default()
        });
        assert_eq!(c.view.found_player, Some(found));
        assert!(c.pending.is_none());
    }
    #[test]
    fn friends_and_profile_payloads_are_correlated_and_survive_live_snapshots() {
        let mut career = CareerClient::default();
        let friend = FriendProfile {
            profile: ProfileSummary::new("b".repeat(64), "小明".into()),
            presence: FriendPresence::Playing,
        };
        let stale = career.begin_request(PendingKind::Friends);
        let latest = career.begin_request(PendingKind::Friends);
        let friends = shared::career::FriendsView {
            friends: vec![friend.clone()],
            ..default()
        };
        career.apply_view(CareerView {
            response_id: Some(stale),
            friends: Some(friends.clone()),
            ..default()
        });
        assert!(career.view.friends.is_none());
        assert!(career.pending.is_some());
        career.apply_view(CareerView {
            response_id: Some(latest),
            friends: Some(friends.clone()),
            ..default()
        });
        assert!(career.pending.is_none());
        let request = career.begin_request(PendingKind::Profile(friend.profile.profile_id.clone()));
        let mut other = friend.profile.clone();
        other.profile_id = "c".repeat(64);
        career.apply_view(CareerView {
            response_id: Some(request),
            visited_profile: Some(other),
            ..default()
        });
        assert!(career.pending.is_some());
        assert!(career.view.visited_profile.is_none());
        career.apply_view(CareerView {
            response_id: Some(request),
            visited_profile: Some(friend.profile.clone()),
            ..default()
        });
        assert!(career.pending.is_none());
        career.apply_view(CareerView::default());
        assert_eq!(career.view.friends, Some(friends));
        assert_eq!(career.view.visited_profile, Some(friend.profile));
        career.clear_account();
        assert!(career.view.friends.is_none());
        assert!(career.view.visited_profile.is_none());
    }
    #[test]
    fn friends_render_real_names_presence_and_all_relationship_actions_in_both_modes() {
        for mode in [UiProfile::Desktop, UiProfile::Mobile] {
            let mut app = render_app(mode, CareerModal::Friends);
            let make = |id: &str, name: &str, presence| FriendProfile {
                profile: ProfileSummary::new(id.repeat(64), name.into()),
                presence,
            };
            app.world_mut().resource_mut::<CareerClient>().view = CareerView {
                profile: Some(ProfileSummary::new("a".repeat(64), "Дмитрий".into())),
                friends: Some(shared::career::FriendsView {
                    friends: vec![make("b", "小明", FriendPresence::Playing)],
                    incoming: vec![make("c", "Мария", FriendPresence::Online)],
                    outgoing: vec![make("d", "Alex", FriendPresence::Offline)],
                }),
                ..default()
            };
            app.update();
            let text = texts(&mut app).join("\n");
            for expected in [
                "小明",
                "Мария",
                "Alex",
                "In a match",
                "Online",
                "Offline",
                "Accept",
                "Decline",
                "Remove",
                "Cancel request",
                "Find player",
            ] {
                assert!(text.contains(expected), "{expected}");
            }
            assert!(!text.contains("Copy code"));
            assert_eq!(
                app.world_mut()
                    .query_filtered::<Entity, (With<CareerScroll>, With<ScrollPosition>)>()
                    .iter(app.world())
                    .count(),
                1
            );
        }
    }
    #[test]
    fn retries_keep_request_and_payload_and_stop_on_reply_close_or_deadline() {
        let mut career = CareerClient {
            history_before: Some(55),
            ..default()
        };
        let id = career.begin_request(PendingKind::History);
        let (_, _, deadline, next) = career.pending.as_ref().unwrap();
        let next = *next;
        let deadline = *deadline;
        assert!(
            career
                .retry_request(next - Duration::from_millis(1))
                .is_none()
        );
        assert!(
            matches!(career.retry_request(next), Some(CareerRequest::History { request_id, before: Some(55) }) if request_id == id)
        );
        assert!(career.retry_request(deadline).is_none());
        career.apply_view(history_page(id, "reply", 40));
        assert!(
            career
                .retry_request(next + Duration::from_secs(2))
                .is_none()
        );
        career.begin_request(PendingKind::Friend("b".repeat(64), FriendAction::Accept));
        let next = career.pending.as_ref().unwrap().3;
        assert!(
            matches!(career.retry_request(next), Some(CareerRequest::Friend { profile_id, action: FriendAction::Accept, .. }) if profile_id == "b".repeat(64))
        );
        career.close();
        assert!(
            career
                .retry_request(next + Duration::from_secs(2))
                .is_none()
        );
    }
    #[test]
    fn rejected_unadmitted_selection_exposes_recovery_but_admitted_query_errors_do_not() {
        for admitted in [false, true] {
            let mut app = render_app(UiProfile::Desktop, CareerModal::Profile);
            let mut session = if admitted {
                crate::net::ClientSession::admitted_for_test()
            } else {
                crate::net::ClientSession::default()
            };
            session.set_join_in_flight_for_test(true);
            app.insert_resource(session);
            app.world_mut().resource_mut::<CareerClient>().view.error = Some("start failed".into());
            app.update();
            let names: Vec<_> = app
                .world_mut()
                .query::<&Name>()
                .iter(app.world())
                .map(|name| name.as_str().to_owned())
                .collect();
            assert_eq!(
                names.iter().any(|name| name == "CareerBackToSelection"),
                !admitted
            );
        }
    }
    #[test]
    fn recovery_input_stays_mounted_and_never_renders_the_secret() {
        let mut app = render_app(UiProfile::Mobile, CareerModal::Devices);
        {
            let mut career = app.world_mut().resource_mut::<CareerClient>();
            career.devices.status = Some(shared::device_account::DeviceEnrollmentStatus {
                state: "pending".into(),
                code: Some("ABCDEFGH".into()),
                profile_id: None,
                nickname: None,
            });
            career.devices.focused = true;
        }
        app.update();
        let field = |app: &mut App| {
            app.world_mut()
                .query::<(Entity, &Name)>()
                .iter(app.world())
                .find(|(_, n)| n.as_str() == "DeviceRecoveryField")
                .map(|(e, _)| e)
                .unwrap()
        };
        let original = field(&mut app);
        let secret = "a1234567".repeat(8);
        for character in secret.chars() {
            app.world_mut()
                .resource_mut::<CareerClient>()
                .devices
                .recovery_code
                .push(character);
            app.update();
            assert_eq!(field(&mut app), original);
        }
        let displayed = texts(&mut app).join(" ");
        assert!(displayed.contains("64 / 64 characters"));
        assert!(!displayed.contains(&secret));
        app.world_mut().resource_mut::<CareerClient>().close();
        assert!(
            app.world()
                .resource::<CareerClient>()
                .devices
                .recovery_code
                .is_empty()
        );
    }

    #[test]
    fn queue_updates_preserve_scroll_but_screen_and_history_page_navigation_reset_it() {
        let mut app = render_app(UiProfile::Mobile, CareerModal::Friends);
        app.update();
        {
            let world = app.world_mut();
            world
                .query_filtered::<&mut ScrollPosition, With<CareerScroll>>()
                .single_mut(world)
                .unwrap()
                .y = 175.0;
        }
        app.world_mut().resource_mut::<CareerClient>().view.queue = QueueView::Waiting {
            compatible: 3,
            needed: 10,
            elapsed_secs: 2,
            newcomer: true,
        };
        app.update();
        let scroll = |app: &mut App| {
            let world = app.world_mut();
            world
                .query_filtered::<&ScrollPosition, With<CareerScroll>>()
                .single(world)
                .unwrap()
                .y
        };
        assert_eq!(scroll(&mut app), 175.0);
        app.world_mut().resource_mut::<CareerClient>().modal = CareerModal::History;
        app.update();
        assert_eq!(scroll(&mut app), 0.0);
        {
            let world = app.world_mut();
            world
                .query_filtered::<&mut ScrollPosition, With<CareerScroll>>()
                .single_mut(world)
                .unwrap()
                .y = 90.0;
        }
        app.world_mut()
            .resource_mut::<CareerClient>()
            .history_before = Some(42);
        app.update();
        assert_eq!(scroll(&mut app), 0.0);
    }
}
