//! Phone layouts, and a contour build's connection panel: the shell-free
//! profile selector (Dev, Local, Custom) that Home, the party lobby and the
//! legacy phone bar open on desktop and phone alike.
//!
//! A production build spawns no server or connection UI at all: no panel, no
//! SERVER button, no presets (`crate::connection_profile`). In a contour build
//! the panel saves a profile for the next start; nothing switches in the
//! running process, so an account link or a store cache never crosses
//! profiles.
// i18n-strict
use bevy::{
    input::keyboard::{Key, KeyboardInput},
    prelude::*,
    window::PrimaryWindow,
};

use crate::{
    connection_profile::{
        ActiveConnection, ConnectionProfile, CustomEndpoints, MAX_FIELD_CHARS, ProfileId,
        validate_lobby,
    },
    i18n::{Localized, UiLabel, tr},
    mobile_controls::{MobileControls, MobileControlsSet},
    net::ClientSession,
    ui::{
        Activated, ModalId, ModalRoot, ScrollArea, TestId, UiAction, UiActionAppExt,
        test_id::NodeKey,
        theme::{
            self as ui, ButtonKind, Form, TextStyle,
            metric::{self, PhoneText},
        },
        tokens::{TextRole, border, color, radius, space},
        widgets::{self as kit, ButtonStyle, controls, surfaces},
    },
};

pub(crate) struct MobileUiPlugin;

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MobileUiLayout;

impl Plugin for MobileUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MobileControls>();
        app.init_resource::<ServerEntry>()
            .add_ui_action::<PhoneAction>()
            .add_systems(Startup, setup_phone_ui)
            .add_systems(
                Update,
                phone_menu_actions
                    .after(MobileControlsSet::Layout)
                    .after(crate::ui::UiSet::Dispatch)
                    .before(crate::help_overlay::HelpOverlaySet::Input)
                    .in_set(crate::input_context::InputContextSet::Modal),
            )
            .add_systems(
                Update,
                (address_keyboard, sync_phone_ui, refresh_connection_panel)
                    .chain()
                    .after(phone_menu_actions),
            )
            .add_systems(
                Update,
                close_server_entry_on_back
                    .before(crate::pause_menu::PauseMenuSet::Close)
                    .in_set(crate::input_context::InputContextSet::Modal),
            )
            .add_systems(
                PostUpdate,
                // It rewrites phone copy, so the font pass must see it, and a
                // language change must not relabel over it.
                adapt_phone_layout
                    .in_set(MobileUiLayout)
                    .before(bevy::ui::UiSystems::Layout)
                    .after(crate::i18n::I18nSystems::Relabel)
                    .before(crate::i18n::I18nSystems::Font),
            );
    }
}

/// The custom profile field the keyboard types into.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum CustomField {
    #[default]
    Lobby,
    Registry,
}

/// The connection panel of a contour build (`ModalId::ServerEntry`): the
/// profile it shows, the custom fields, the confirmation step and the outcome
/// of the last choice. It never opens in a production build.
#[derive(Resource, Default)]
pub(crate) struct ServerEntry {
    pub open: bool,
    /// The profile the panel shows; the one that runs next when it opens.
    choice: Option<ProfileId>,
    lobby: String,
    registry: String,
    field: CustomField,
    /// The profile the confirmation step asks to switch to.
    confirming: Option<ConnectionProfile>,
    /// A `connection.error.*` key and the custom field it is about.
    error: Option<(&'static str, Option<CustomField>)>,
    /// A `connection.notice.*` key: the outcome of the last choice.
    notice: Option<&'static str>,
    keyboard: bool,
}

impl ServerEntry {
    /// Opens the panel on the profile that runs next, with the saved custom
    /// lobby and registry filled in. Nothing opens in a production build or
    /// while a join is committed.
    pub(crate) fn open_for(&mut self, session: &ClientSession, connection: &ActiveConnection) {
        if !connection.selectable() || session.join_in_flight() || session.has_committed_join() {
            return;
        }
        let custom = connection.custom.clone().unwrap_or_default();
        self.open = true;
        self.choice = Some(connection.next().id);
        self.lobby = custom.lobby;
        self.registry = custom.registry.unwrap_or_default();
        self.field = CustomField::Lobby;
        self.confirming = None;
        self.error = None;
        self.notice = None;
    }

    fn custom(&self) -> CustomEndpoints {
        CustomEndpoints {
            lobby: self.lobby.trim().to_owned(),
            registry: Some(self.registry.trim().to_owned()).filter(|registry| !registry.is_empty()),
        }
    }

    /// Whether typed text goes into a custom field.
    fn editing(&self) -> bool {
        self.open && self.choice == Some(ProfileId::Custom) && self.confirming.is_none()
    }

    fn field_text(&mut self) -> &mut String {
        match self.field {
            CustomField::Lobby => &mut self.lobby,
            CustomField::Registry => &mut self.registry,
        }
    }

    fn type_text(&mut self, typed: &str) {
        let field = self.field;
        edit_field(field, self.field_text(), typed);
        self.error = None;
        self.notice = None;
    }

    fn delete(&mut self) {
        self.field_text().pop();
        self.error = None;
        self.notice = None;
    }
}

/// Opens the connection panel (Home's and the lobby's connection buttons);
/// without the resource, with this process's connection.
pub(crate) fn open_connection_panel(
    entry: &mut ServerEntry,
    session: &ClientSession,
    connection: Option<&ActiveConnection>,
) {
    match connection {
        Some(connection) => entry.open_for(session, connection),
        None => entry.open_for(session, &ActiveConnection::current()),
    }
}

/// Whether this build shows connection UI, from the resource when it exists.
pub(crate) fn offers_connection(connection: Option<&ActiveConnection>) -> bool {
    connection.map_or_else(
        crate::connection_profile::selectable,
        ActiveConnection::selectable,
    )
}

/// Presses on the legacy phone bar and in the connection panel.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PhoneAction {
    Menu,
    Help,
    Server,
    /// Show a profile in the connection panel.
    Profile(ProfileId),
    /// Type into a custom field.
    Field(CustomField),
    /// Use the shown profile: asks to confirm a switch.
    Use,
    /// Save the switch for the next start.
    Confirm,
    /// Leave the confirmation step.
    Back,
    /// Desktop: start a fresh game now, so the saved profile runs.
    Restart,
    Close,
    Key(char),
    Backspace,
    Keyboard,
}
#[derive(Component)]
struct PhoneBar;
#[derive(Component)]
struct ServerEntryRoot;
/// The typed text of a custom field (`text_input`'s value), written in place.
#[derive(Component)]
struct ConnectionFieldValue(CustomField);

fn phone_button(
    parent: &mut ChildSpawnerCommands,
    label: impl UiLabel,
    name: &str,
    action: PhoneAction,
) {
    parent
        .spawn((
            Button,
            Node {
                min_width: Val::Px(48.0),
                height: Val::Px(44.0),
                flex_shrink: 0.0,
                padding: UiRect::horizontal(Val::Px(10.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(ui::TILE),
            UiAction(action),
            TestId::new(name.to_owned()),
        ))
        .with_children(|button| {
            button.spawn((label.into_text(), ui::text(14.0), TextColor(ui::IVORY)));
        });
}

fn setup_phone_ui(
    mut commands: Commands,
    mobile: Res<MobileControls>,
    connection: Option<Res<ActiveConnection>>,
) {
    spawn_phone_ui(
        &mut commands,
        mobile.enabled,
        offers_connection(connection.as_deref()),
    );
}

/// The legacy phone bar (phones only; its SERVER button only with a
/// connection selector) and, only in a contour build, the connection panel's
/// modal root. A production build spawns no server or connection UI.
fn spawn_phone_ui(commands: &mut Commands, phone: bool, selector: bool) {
    if phone {
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    display: Display::None,
                    column_gap: Val::Px(6.0),
                    ..default()
                },
                // Above the front-end screens: on a phone this bar is the only way
                // to settings and to the connection panel (there is no Escape key).
                ZIndex(crate::ui::theme::SCREEN_Z + 10),
                PhoneBar,
                Name::new("PhoneMenuBar"),
            ))
            .with_children(|bar| {
                phone_button(bar, "?", "LegacyPhoneHelpButton", PhoneAction::Help);
                phone_button(
                    bar,
                    Localized::new("phone.bar.menu"),
                    "LegacyPhoneMenuButton",
                    PhoneAction::Menu,
                );
                if selector {
                    phone_button(
                        bar,
                        Localized::new("phone.bar.server"),
                        "LegacyPhoneServerButton",
                        PhoneAction::Server,
                    );
                }
            });
    }
    if selector {
        // The panel itself is rebuilt by `refresh_connection_panel` while open.
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                display: Display::None,
                ..default()
            },
            ZIndex(150),
            ServerEntryRoot,
            ModalRoot(ModalId::ServerEntry),
            Name::new("ServerEntryRoot"),
        ));
    }
}

/// Keeps the characters a lobby address (`host:port`, `[v6]:port`) or, in the
/// registry field, an origin URL can contain, up to the field limit.
fn edit_field(field: CustomField, text: &mut String, typed: &str) {
    for ch in typed.chars() {
        let allowed = ch.is_ascii_alphanumeric()
            || matches!(ch, '.' | ':' | '-' | '[' | ']')
            || (field == CustomField::Registry && ch == '/');
        if allowed && text.chars().count() < MAX_FIELD_CHARS {
            text.push(ch);
        }
    }
}

fn phone_menu_actions(
    mut activated: MessageReader<Activated<PhoneAction>>,
    session: Res<ClientSession>,
    mut entry: ResMut<ServerEntry>,
    mut connection: Option<ResMut<ActiveConnection>>,
    mut pause: ResMut<crate::pause_menu::PauseMenuState>,
    mut help: ResMut<crate::help_overlay::HelpOverlayVisible>,
    mut exit: MessageWriter<AppExit>,
) {
    let presses: Vec<PhoneAction> = activated.read().map(|a| a.action).collect();
    for action in presses {
        match action {
            PhoneAction::Menu => pause.open = !pause.open,
            PhoneAction::Help => help.0 = !help.0,
            PhoneAction::Server => {
                open_connection_panel(&mut entry, &session, connection.as_deref());
            }
            PhoneAction::Close => {
                entry.open = false;
                entry.keyboard = false;
            }
            PhoneAction::Keyboard => entry.keyboard = !entry.keyboard,
            PhoneAction::Key(ch) if entry.editing() => entry.type_text(&ch.to_string()),
            PhoneAction::Backspace if entry.editing() => entry.delete(),
            action if entry.open => {
                if let Some(connection) = connection.as_deref_mut() {
                    if let Some(exit_now) = panel_action(&mut entry, connection, action) {
                        exit.write(exit_now);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The panel's steps: show a profile, use it (the running one cancels a saved
/// switch at once; another asks to confirm), confirm (saved for the next
/// start), restart (desktop). Returns the exit a successful restart needs.
fn panel_action(
    entry: &mut ServerEntry,
    connection: &mut ActiveConnection,
    action: PhoneAction,
) -> Option<AppExit> {
    match action {
        PhoneAction::Profile(id) => {
            entry.choice = Some(id);
            entry.confirming = None;
            entry.error = None;
            entry.notice = None;
        }
        PhoneAction::Field(field) => entry.field = field,
        PhoneAction::Use => {
            entry.error = None;
            entry.notice = None;
            let id = entry.choice.unwrap_or(connection.next().id);
            match connection.candidate(id, &entry.custom()) {
                Err(error) => {
                    let field = (id == ProfileId::Custom).then(|| {
                        if validate_lobby(connection.contour.as_deref(), &entry.lobby).is_err() {
                            CustomField::Lobby
                        } else {
                            CustomField::Registry
                        }
                    });
                    entry.error = Some((error.key(), field));
                }
                Ok(profile) if profile == *connection.next() => {
                    entry.notice = Some(if connection.pending.is_some() {
                        "connection.notice.saved"
                    } else {
                        "connection.notice.current"
                    });
                }
                Ok(profile) if profile == connection.profile => save(entry, connection, profile),
                Ok(profile) => entry.confirming = Some(profile),
            }
        }
        PhoneAction::Confirm => {
            if let Some(profile) = entry.confirming.take() {
                save(entry, connection, profile);
            }
        }
        PhoneAction::Back => entry.confirming = None,
        PhoneAction::Restart if connection.pending.is_some() && crate::platform::CAN_RELAUNCH => {
            match crate::platform::relaunch() {
                Ok(()) => return Some(AppExit::Success),
                Err(_) => entry.error = Some(("connection.error.restart", None)),
            }
        }
        _ => {}
    }
    None
}

/// Saves `profile` for the next start; the running profile cancels a saved switch.
fn save(entry: &mut ServerEntry, connection: &mut ActiveConnection, profile: ConnectionProfile) {
    let cancels = profile == connection.profile;
    match connection.choose(profile) {
        Ok(()) => {
            entry.notice = Some(if cancels {
                "connection.notice.kept"
            } else {
                "connection.notice.saved"
            });
        }
        Err(_) => entry.error = Some(("connection.error.save", None)),
    }
}

/// The frontmost editor consumes keyboard/gamepad back before the pause menu:
/// it leaves the confirmation step first, then closes.
pub(crate) fn close_server_entry_on_back(
    mut back: crate::ui::BackInput,
    mut entry: ResMut<ServerEntry>,
) {
    if entry.open && back.just_pressed() {
        if entry.confirming.is_some() {
            entry.confirming = None;
        } else {
            entry.open = false;
        }
        back.consume();
    }
}

pub(crate) fn address_keyboard(
    mut entry: ResMut<ServerEntry>,
    mut input: MessageReader<KeyboardInput>,
    mut ime: MessageReader<Ime>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut owned_ime: Local<bool>,
) {
    // A closed connection panel must not disable another desktop field's IME.
    if (entry.open || *owned_ime)
        && let Ok(mut window) = windows.single_mut()
    {
        let enabled = entry.open && entry.keyboard;
        if window.ime_enabled != enabled {
            window.ime_enabled = enabled;
        }
    }
    *owned_ime = entry.open;
    for event in ime.read() {
        if entry.editing()
            && let Ime::Commit { value, .. } = event
        {
            entry.type_text(value);
        }
    }
    for event in input.read() {
        if !entry.editing() || !event.state.is_pressed() {
            continue;
        }
        match &event.logical_key {
            Key::Backspace => entry.delete(),
            Key::Tab => {
                entry.field = match entry.field {
                    CustomField::Lobby => CustomField::Registry,
                    CustomField::Registry => CustomField::Lobby,
                };
            }
            Key::Escape | Key::Enter => {}
            _ => {
                let text = event.text.as_deref().or_else(|| match &event.logical_key {
                    Key::Character(text) => Some(text.as_str()),
                    _ => None,
                });
                if let Some(text) = text {
                    entry.type_text(text);
                }
            }
        }
    }
}

fn sync_phone_ui(
    mobile: Res<MobileControls>,
    session: Res<ClientSession>,
    shop: Option<Res<crate::shop::ShopState>>,
    entry: Res<ServerEntry>,
    mut bar: Query<
        &mut Node,
        (
            With<PhoneBar>,
            Without<ServerEntryRoot>,
            Without<UiAction<PhoneAction>>,
        ),
    >,
    mut overlay: Query<
        &mut Node,
        (
            With<ServerEntryRoot>,
            Without<PhoneBar>,
            Without<UiAction<PhoneAction>>,
        ),
    >,
    mut buttons: Query<
        (&UiAction<PhoneAction>, &mut Node),
        (Without<PhoneBar>, Without<ServerEntryRoot>),
    >,
    mut values: Query<(&ConnectionFieldValue, &mut Text)>,
    ui_scale: Option<Res<UiScale>>,
) {
    let scale = metric::ui_scale(ui_scale.as_ref().map_or(1.0, |scale| scale.0));
    // Home now owns its Help/Settings/connection controls inside the header;
    // the picker owns Back. Keep the panel, hide the legacy floating bar.
    let bar_wanted = false;
    for mut node in &mut bar {
        node.display = if mobile.enabled
            && mobile.landscape
            && bar_wanted
            && !shop.as_ref().is_some_and(|shop| shop.open)
        {
            Display::Flex
        } else {
            Display::None
        };
        // Shell pages use a global scale; these utility controls retain their
        // real safe-area anchors and 44px touch targets through that transition.
        node.right = Val::Px(mobile.safe.right / scale);
        node.top = Val::Px(mobile.safe.top / scale);
        node.column_gap = Val::Px(6.0 / scale);
    }
    for (UiAction(action), mut node) in &mut buttons {
        let width = match action {
            PhoneAction::Help => Some(metric::PHONE_BAR_HELP_W),
            PhoneAction::Menu => Some(metric::PHONE_BAR_MENU_W),
            PhoneAction::Server => Some(metric::PHONE_BAR_SERVER_W),
            _ => None,
        };
        if let Some(width) = width {
            node.width = Val::Px(width / scale);
            node.min_width = Val::Px(metric::PHONE_BAR_MIN_W / scale);
            node.height = Val::Px(metric::TOUCH_MIN / scale);
            node.min_height = Val::Px(metric::TOUCH_MIN / scale);
            node.padding = UiRect::horizontal(Val::Px(10.0 / scale));
            node.border_radius = BorderRadius::all(Val::Px(8.0 / scale));
        }
        if matches!(action, PhoneAction::Server) {
            node.display = if !session.join_in_flight() && !session.has_committed_join() {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    for mut node in &mut overlay {
        node.display = if entry.open && (!mobile.enabled || mobile.landscape) {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (ConnectionFieldValue(field), mut text) in &mut values {
        let value = match field {
            CustomField::Lobby => &entry.lobby,
            CustomField::Registry => &entry.registry,
        };
        if text.0 != *value {
            text.0.clone_from(value);
        }
    }
}

/// What the connection panel shows; it is rebuilt when this changes (typed
/// text is written in place by [`sync_phone_ui`]).
#[derive(Clone, Debug, PartialEq)]
struct PanelView {
    choice: ProfileId,
    choices: &'static [ProfileId],
    field: CustomField,
    confirming: Option<ConnectionProfile>,
    error: Option<(&'static str, Option<CustomField>)>,
    notice: Option<&'static str>,
    running: ConnectionProfile,
    next: ConnectionProfile,
    /// The built-in profile the panel describes; none for custom.
    shown: Option<ConnectionProfile>,
    /// Desktop with a saved switch: offer to restart now.
    restart: bool,
    /// Phone on the custom profile: the numeric keypad.
    keypad: bool,
    form: Form,
    locale: u32,
}

fn panel_view(
    entry: &ServerEntry,
    connection: &ActiveConnection,
    phone: bool,
    locale: u32,
) -> PanelView {
    let choice = entry.choice.unwrap_or(connection.next().id);
    PanelView {
        choice,
        choices: connection.choices(),
        field: entry.field,
        confirming: entry.confirming.clone(),
        error: entry.error,
        notice: entry.notice,
        running: connection.profile.clone(),
        next: connection.next().clone(),
        shown: if choice == ProfileId::Custom {
            None
        } else {
            connection
                .candidate(choice, &CustomEndpoints::default())
                .ok()
        },
        restart: connection.pending.is_some() && crate::platform::CAN_RELAUNCH,
        keypad: phone && choice == ProfileId::Custom,
        form: Form::of(phone),
        locale,
    }
}

/// Spawns the panel when it opens and rebuilds it when what it shows changes.
fn refresh_connection_panel(
    mut commands: Commands,
    entry: Res<ServerEntry>,
    connection: Option<Res<ActiveConnection>>,
    mobile: Res<MobileControls>,
    locale: Option<Res<crate::i18n::Locale>>,
    roots: Query<Entity, With<ServerEntryRoot>>,
    mut shown: Local<Option<PanelView>>,
) {
    let Ok(root) = roots.single() else {
        return;
    };
    let generation = locale.as_deref().map_or(0, crate::i18n::Locale::generation);
    let view = connection
        .as_deref()
        .filter(|_| entry.open)
        .map(|connection| panel_view(&entry, connection, mobile.enabled, generation));
    if *shown == view {
        return;
    }
    // Only a fresh open plays the kit's open animation.
    let animate = shown.is_none();
    commands.entity(root).despawn_related::<Children>();
    if let Some(view) = &view {
        commands
            .entity(root)
            .with_children(|root| spawn_connection_panel(root, view, animate));
    }
    *shown = view;
}

/// A caption line in `ink`.
fn caption<L: UiLabel>(text: L, ink: Color) -> impl Bundle {
    (
        text.into_text(),
        ui::styled_text(TextStyle::keep_case(TextRole::Caption)),
        TextColor(ink),
    )
}

fn profile_name(profile: &ConnectionProfile) -> &'static str {
    tr(profile.id.label_key())
}

/// The connection panel, a kit modal over its scrim: the profiles this build
/// offers, what the shown one points at (or the custom fields), and the
/// use → confirm → saved steps.
fn spawn_connection_panel(root: &mut ChildSpawnerCommands, view: &PanelView, animate: bool) {
    let panel = surfaces::modal(
        root,
        Localized::new("connection.title"),
        PhoneAction::Close,
        view.form,
        "ServerEntry",
    );
    let mut commands = root.commands();
    if !animate {
        commands.entity(panel).insert((
            surfaces::PanelOpen { elapsed: f32::MAX },
            UiTransform::IDENTITY,
        ));
    }
    commands.entity(panel).with_children(|panel| {
        panel
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(space::S12),
                    max_height: Val::Vh(68.0),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                ScrollArea::menu(48.0),
                Name::new("ConnectionBody"),
            ))
            .with_children(|body| {
                body.spawn(caption(
                    Localized::new("connection.hint"),
                    color::TEXT_SECONDARY,
                ));
                body.spawn((
                    caption(
                        Localized::with_args(
                            "connection.running",
                            [
                                ("profile", &profile_name(&view.running)),
                                ("lobby", &view.running.lobby),
                            ],
                        ),
                        color::TEXT_PRIMARY,
                    ),
                    Name::new("ConnectionRunning"),
                ));
                if view.next != view.running {
                    body.spawn((
                        caption(
                            Localized::with_args(
                                "connection.pending",
                                [("profile", &profile_name(&view.next))],
                            ),
                            color::TEXT_GOLD,
                        ),
                        Name::new("ConnectionPending"),
                    ));
                }
                body.spawn((
                    Node {
                        column_gap: Val::Px(space::S8),
                        row_gap: Val::Px(space::S8),
                        flex_wrap: FlexWrap::Wrap,
                        ..default()
                    },
                    Name::new("ConnectionProfiles"),
                ))
                .with_children(|row| {
                    for id in view.choices {
                        kit::screen_tile(
                            row,
                            Localized::new(id.label_key()),
                            *id == view.choice,
                            PhoneAction::Profile(*id),
                            id.test_id(),
                        );
                    }
                });
                match &view.shown {
                    Some(profile) => spawn_details(body, profile),
                    None => spawn_custom_fields(body, view),
                }
                if let Some((key, _)) = view.error {
                    body.spawn((
                        caption(Localized::new(key), color::TEXT_DANGER),
                        Name::new("ConnectionError"),
                    ));
                }
                if let Some(key) = view.notice {
                    body.spawn((
                        caption(
                            Localized::with_args(key, [("profile", &profile_name(&view.next))]),
                            color::TEXT_GOLD,
                        ),
                        Name::new("ConnectionNotice"),
                    ));
                    if view.next != view.running && !view.restart {
                        body.spawn(caption(
                            Localized::new("connection.notice.reopen"),
                            color::TEXT_SECONDARY,
                        ));
                    }
                }
                spawn_actions(body, view);
            });
    });
}

/// Where a built-in profile points: lobby, registry, market, Studio, passport.
fn spawn_details(body: &mut ChildSpawnerCommands, profile: &ConnectionProfile) {
    let passport = if profile.passport_closed() {
        tr("connection.passport.closed").to_owned()
    } else {
        profile.passport.clone()
    };
    for (key, value) in [
        ("connection.detail.lobby", profile.lobby.as_str()),
        ("connection.detail.registry", profile.registry.as_str()),
        ("connection.detail.market", profile.market.as_str()),
        ("connection.detail.studio", profile.studio.as_str()),
        ("connection.detail.passport", passport.as_str()),
    ] {
        body.spawn(caption(
            Localized::with_args(key, [("value", &value)]),
            color::TEXT_SECONDARY,
        ));
    }
}

/// The custom lobby and registry fields (kit text inputs; the selected one is
/// typed into), and on a phone the numeric keypad.
fn spawn_custom_fields(body: &mut ChildSpawnerCommands, view: &PanelView) {
    for (field, label, placeholder, id) in [
        (
            CustomField::Lobby,
            "connection.custom.lobby",
            "connection.custom.lobby_hint",
            "ConnectionLobbyField",
        ),
        (
            CustomField::Registry,
            "connection.custom.registry",
            "connection.custom.registry_hint",
            "ConnectionRegistryField",
        ),
    ] {
        body.spawn(caption(Localized::new(label), color::TEXT_SECONDARY));
        let input = controls::text_input(
            body,
            Localized::new(placeholder),
            None,
            ConnectionFieldValue(field),
            PhoneAction::Field(field),
            id,
        );
        let error = view.error.is_some_and(|(_, at)| at == Some(field));
        body.commands().entity(input).insert((
            ButtonStyle {
                kind: ButtonKind::Tile,
                selected: view.field == field,
            },
            controls::InputError(error),
        ));
    }
    body.spawn(caption(
        Localized::new("connection.custom.rest"),
        color::TEXT_SECONDARY,
    ));
    if !view.keypad {
        return;
    }
    // A numeric keypad also works when an Android IME does not deliver
    // composed text to NativeActivity. Host names use the normal keyboard.
    body.spawn((
        Node {
            flex_wrap: FlexWrap::Wrap,
            column_gap: Val::Px(space::S4),
            row_gap: Val::Px(space::S4),
            ..default()
        },
        Name::new("ServerKeypad"),
    ))
    .with_children(|keys| {
        for key in "1234567890.:".chars() {
            key_button(
                keys,
                key.to_string(),
                PhoneAction::Key(key),
                format!("ServerKey-{key}"),
            );
        }
        key_button(
            keys,
            Localized::new("connection.key.delete"),
            PhoneAction::Backspace,
            "ServerBackspace".to_owned(),
        );
        key_button(
            keys,
            Localized::new("connection.key.keyboard"),
            PhoneAction::Keyboard,
            "ServerKeyboardButton".to_owned(),
        );
    });
}

/// One keypad key: a kit tile at the touch minimum.
fn key_button(
    parent: &mut ChildSpawnerCommands,
    label: impl UiLabel,
    action: PhoneAction,
    id: String,
) {
    kit::spawn_button(
        parent,
        Node {
            min_width: Val::Px(metric::TOUCH_MIN),
            height: Val::Px(metric::TOUCH_MIN),
            padding: UiRect::horizontal(Val::Px(space::S8)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            ..default()
        },
        label,
        TextStyle::keep_case(TextRole::Label),
        ButtonKind::Tile,
        None,
        action,
        TestId::new(id),
        (),
    );
}

/// Use (or, in the confirmation step, switch / back) and, on desktop with a
/// saved switch, restart now.
fn spawn_actions(body: &mut ChildSpawnerCommands, view: &PanelView) {
    if let Some(target) = &view.confirming {
        body.spawn((
            caption(
                Localized::with_args(
                    "connection.confirm",
                    [("profile", &profile_name(target)), ("lobby", &target.lobby)],
                ),
                color::TEXT_PRIMARY,
            ),
            Name::new("ConnectionConfirmText"),
        ));
    }
    body.spawn(Node {
        column_gap: Val::Px(space::S8),
        row_gap: Val::Px(space::S8),
        flex_wrap: FlexWrap::Wrap,
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(|row| {
        if view.confirming.is_some() {
            kit::screen_button(
                row,
                Localized::new("connection.switch"),
                ButtonKind::Primary,
                PhoneAction::Confirm,
                "ConnectionConfirm",
            );
            kit::screen_button(
                row,
                Localized::new("connection.back"),
                ButtonKind::Secondary,
                PhoneAction::Back,
                "ConnectionBack",
            );
            return;
        }
        kit::screen_button(
            row,
            Localized::with_args(
                "connection.use",
                [("profile", &tr(view.choice.label_key()))],
            ),
            ButtonKind::Primary,
            PhoneAction::Use,
            "ConnectionUse",
        );
        if view.restart {
            kit::screen_button(
                row,
                Localized::new("connection.restart"),
                ButtonKind::Secondary,
                PhoneAction::Restart,
                "ConnectionRestart",
            );
        }
    });
}

#[derive(Component)]
struct PhoneFontSize(f32);

fn phone_family(
    entity: Entity,
    hierarchy: &Query<(Option<&ChildOf>, NodeKey)>,
) -> Option<PhoneText> {
    let mut next = Some(entity);
    for _ in 0..12 {
        let (parent, key) = hierarchy.get(next?).ok()?;
        let name = key.as_str();
        if !name.is_empty() {
            if name.starts_with("ShopBuy-") {
                return Some(PhoneText::ShopCard);
            }
            let family = match name {
                "TeamSelectOverlay" => Some(PhoneText::Entry),
                "ShopPanel" => Some(PhoneText::Shop),
                "ShopSummary" => Some(PhoneText::ShopSummary),
                "PauseMenuPanel" => Some(PhoneText::Pause),
                "GameStateCard" => Some(PhoneText::Result),
                "PhoneMenuBar" => Some(PhoneText::Bar),
                _ => None,
            };
            if family.is_some() {
                return family;
            }
        }
        next = parent.map(ChildOf::parent);
    }
    None
}

fn absolute(node: &mut Node, left: f32, top: f32, width: f32, height: Option<f32>) {
    node.position_type = PositionType::Absolute;
    node.left = Val::Px(left);
    node.top = Val::Px(top);
    node.right = Val::Auto;
    node.bottom = Val::Auto;
    node.width = Val::Px(width);
    node.max_width = Val::Px(width);
    if let Some(height) = height {
        node.height = Val::Px(height);
    }
}

fn adapt_phone_layout(
    mut commands: Commands,
    mobile: Res<MobileControls>,
    session: Res<ClientSession>,
    mut nodes: Query<(Entity, NodeKey, &mut Node, Option<&mut UiTransform>)>,
    mut fonts: Query<(
        Entity,
        &mut TextFont,
        Option<&PhoneFontSize>,
        Option<&crate::ui::theme::TextStyle>,
    )>,
    mut copy: Query<(NodeKey, &mut Text), Without<crate::i18n::Localized>>,
    hierarchy: Query<(Option<&ChildOf>, NodeKey)>,
    pause: Option<Res<crate::pause_menu::PauseMenuState>>,
    ui_scale: Option<Res<UiScale>>,
) {
    if !mobile.enabled || !mobile.landscape {
        return;
    }
    let scale = ui_scale.as_ref().map_or(1.0, |scale| scale.0);
    let left = mobile.safe.left;
    let top = mobile.safe.top;
    let bottom = mobile.safe.bottom;
    let width = mobile.viewport.x - left - mobile.safe.right;
    let height = mobile.viewport.y - top - bottom;
    let class_width = metric::phone_class_column(width);
    let grid_left = left + class_width + 20.0;
    let grid_width = width - class_width - 20.0;
    for (entity, name, mut node, _transform) in &mut nodes {
        // Hero Select owns its responsive layout. Never overwrite it with
        // the legacy full-width grid/no-preview phone composition.
        if phone_family(entity, &hierarchy) == Some(PhoneText::Entry) {
            continue;
        }
        match name.as_str() {
            "TeamSelectOverlay" => {
                node.padding = UiRect::ZERO;
            }
            "ClassSelectTitle" => absolute(&mut node, left, top + 7.0, class_width, None),
            "ClassButtonsRow" => {
                absolute(
                    &mut node,
                    left,
                    top + if mobile.viewport.y <= 340.0 {
                        30.0
                    } else {
                        36.0
                    },
                    class_width,
                    None,
                );
                node.flex_direction = FlexDirection::Column;
                node.row_gap = Val::Px(if mobile.viewport.y <= 340.0 { 4.0 } else { 6.0 });
            }
            "AvatarSelectTitle" => {
                absolute(&mut node, grid_left, top + 7.0, grid_width - 220.0, None)
            }
            "RendererStatus" => absolute(&mut node, grid_left, top + 31.0, grid_width, None),
            "AvatarGrid" | "SpriteCharacterGrid" => {
                absolute(
                    &mut node,
                    grid_left,
                    top + 53.0,
                    grid_width,
                    Some((height - 152.0).max(120.0)),
                );
                node.max_height = Val::Px((height - 152.0).max(120.0));
                node.column_gap = Val::Px(6.0);
                node.row_gap = Val::Px(6.0);
            }
            "TeamSelectTitle" => absolute(
                &mut node,
                grid_left,
                mobile.viewport.y - bottom - 89.0,
                grid_width,
                None,
            ),
            "TeamButtonsRow" => {
                absolute(
                    &mut node,
                    grid_left,
                    mobile.viewport.y - bottom - 67.0,
                    grid_width,
                    None,
                );
                node.column_gap = Val::Px(10.0);
            }
            "FindMatchButton" => {
                node.width = Val::Px(grid_width);
                node.height = Val::Px(metric::TOUCH_MIN);
            }
            // The hint starts beside the Back button, not under it.
            "TeamSelectHint" => absolute(
                &mut node,
                grid_left,
                mobile.viewport.y - bottom - 20.0,
                grid_width,
                None,
            ),
            // Back sits under the class column; the top-right corner belongs
            // to the phone bar (?, MENU, SERVER).
            "HeroSelectHeader" => {
                absolute(
                    &mut node,
                    left,
                    mobile.viewport.y - bottom - 48.0,
                    class_width,
                    Some(metric::TOUCH_MIN),
                );
                node.column_gap = Val::Px(0.0);
            }
            "HeroSelectBack" => {
                node.width = Val::Percent(100.0);
                node.height = Val::Px(metric::TOUCH_MIN);
            }
            // Desktop-only picker parts: no room beside the phone grid, and the
            // wallet pairing flow is not supported on a phone yet.
            "HeroSelectTitle" | "HeroSelectStatus" | "HeroSelectPanel" | "EkzaConnectRow" => {
                node.display = Display::None;
            }
            "ServerEntryPanel" => {
                node.width = Val::Px(width.min(metric::PHONE_SERVER_W));
                node.max_width = Val::Px(width);
                node.padding = UiRect::all(Val::Px(10.0));
                node.row_gap = Val::Px(6.0);
            }
            // The help panel places itself in the safe area (`help_overlay`,
            // `hud-help.md`); its card list is a touch scroll area.
            "HelpBody" => {
                // The fixed 44 px dismiss row stays above the scrolling cards.
                node.max_height = Val::Px((height - 82.0).max(120.0));
                node.min_height = Val::Px(0.0);
                node.flex_shrink = 1.0;
                node.overflow = Overflow::scroll_y();
                commands
                    .entity(entity)
                    .insert_if_new(ScrollArea::phone_panel());
            }
            "ShopPanel" => {
                // Center inside the asymmetric safe area while the backdrop
                // continues to cover the entire viewport.
                node.top = Val::Px((top - bottom) * 0.5);
                node.width = Val::Px(width);
                node.max_width = Val::Px(width);
                node.max_height = Val::Px(height);
                node.padding = UiRect::all(Val::Px(8.0));
                node.row_gap = Val::Px(3.0);
            }
            "ShopCards" => {
                // Keep every item card at 103px and the header/close outside
                // the scroll area. At 844×390 both rows fit; narrower phones
                // retain scrolling room when the summary wraps to two lines.
                node.max_height =
                    Val::Px((height - if width < 700.0 { 162.0 } else { 144.0 }).max(103.0));
                node.min_height = Val::Px(0.0);
                node.flex_shrink = 1.0;
                node.column_gap = Val::Px(6.0);
                node.row_gap = Val::Px(4.0);
                node.overflow = Overflow::scroll_y();
                commands
                    .entity(entity)
                    .insert_if_new(ScrollArea::phone_panel());
            }
            "ShopFooter" => node.display = Display::None,
            "ShopCloseButton" => {
                node.min_height = Val::Px(metric::TOUCH_MIN);
                node.padding = UiRect::axes(Val::Px(10.0), Val::Px(5.0));
            }
            "PauseMenuPanel" => {
                let settings = pause.as_ref().is_some_and(|pause| pause.in_settings);
                node.top = Val::Px((top - bottom) * 0.5);
                node.width = Val::Px(if settings {
                    width - 24.0
                } else {
                    width.min(metric::PHONE_PAUSE_W)
                });
                // Bound both bodies so short windows scroll between the fixed
                // header/close control and footer.
                node.height = Val::Px(if settings {
                    height - 16.0
                } else {
                    metric::pause_panel_height(
                        metric::Form::Phone,
                        pause.as_ref().is_some_and(|pause| pause.in_settings),
                        height,
                    )
                });
                node.max_height = Val::Px(height);
                node.padding = UiRect::axes(Val::Px(16.0), Val::Px(12.0));
                node.row_gap = Val::Px(8.0);
                node.overflow = Overflow::clip();
            }
            "PauseMenuMainSection" => node.row_gap = Val::Px(10.0),
            "GameStateCard" => {
                node.width = Val::Px(width.min(metric::PHONE_RESULT_W));
                node.max_width = Val::Px(width);
                node.padding = UiRect::all(Val::Px(16.0));
            }
            "ConnectionStatusPanel" => {
                // The entry hint already explains hero/team selection. Keep
                // connection failures and pending admission visible, but avoid
                // placing the healthy welcome sentence over phone portraits.
                if session.is_choosing_loadout() {
                    node.display = Display::None;
                }
                absolute(
                    &mut node,
                    left + class_width + 20.0,
                    top + 53.0,
                    grid_width.min(400.0),
                    None,
                );
                node.padding = UiRect::all(Val::Px(8.0));
            }
            "ConnectionRetryButton" => node.height = Val::Px(metric::TOUCH_MIN),
            name if name.starts_with("ClassButton-") => {
                node.width = Val::Px(class_width);
                node.height = Val::Px(48.0);
            }
            name if name.starts_with("AvatarButton-") || name.starts_with("SpriteButton-") => {
                node.width = Val::Px(72.0);
                node.height = Val::Px(82.0);
            }
            name if name.starts_with("ShopBuy-") => {
                let (card_width, card_height) = metric::phone_shop_card(width);
                node.width = Val::Px(card_width);
                node.height = Val::Px(card_height);
                node.padding = UiRect::all(Val::Px(if width < 650.0 { 6.0 } else { 7.0 }));
                node.row_gap = Val::Px(if width < 650.0 { 2.0 } else { 3.0 });
            }
            _ => {}
        }
    }
    for (entity, mut font, base, style) in &mut fonts {
        let Some(family) = phone_family(entity, &hierarchy) else {
            continue;
        };
        if family == PhoneText::Entry || style.is_some() {
            continue;
        }
        let bevy::text::FontSize::Px(current_size) = font.font_size else {
            continue;
        };
        let original = base.map_or(current_size, |base| base.0);
        if base.is_none() {
            commands
                .entity(entity)
                .insert((PhoneFontSize(original), crate::ui::theme::PhoneSized));
        }
        font.font_size = metric::phone_font(family, original, width, scale).into();
    }
    // Phone copy of a help dismiss label spawned without its key (the guide
    // spawns `help.phone.dismiss` itself on a phone and relabels it). Other
    // modules write their own phone wording (chosen by
    // `MobileControls::enabled`); this pass only lays out.
    for (name, mut text) in &mut copy {
        let key = match name.as_str() {
            "HelpDismissLabel" => "help.phone.dismiss",
            _ => continue,
        };
        let copy = tr(key);
        if text.0 != copy {
            text.0 = copy.to_owned();
        }
    }
}

#[cfg(test)]
pub(crate) fn add_pause_layout_test_systems(app: &mut App) {
    app.add_systems(
        PostUpdate,
        adapt_phone_layout.before(bevy::ui::UiSystems::Layout),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::touch::{TouchInput, TouchPhase};

    fn scratch_selection(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let directory = std::env::temp_dir().join(format!(
            "omoba-connection-panel-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory.join(crate::connection_profile::SELECTION_FILE)
    }

    fn press(app: &mut App, action: PhoneAction) {
        app.world_mut().write_message(Activated {
            action,
            source: Entity::PLACEHOLDER,
        });
        app.update();
    }

    fn actions_app(connection: ActiveConnection) -> App {
        let mut app = App::new();
        let mut mobile = MobileControls::default();
        mobile.enabled = false;
        app.insert_resource(mobile)
            .insert_resource(connection)
            .init_resource::<ClientSession>()
            .init_resource::<ServerEntry>()
            .init_resource::<crate::pause_menu::PauseMenuState>()
            .init_resource::<crate::help_overlay::HelpOverlayVisible>()
            .add_message::<Activated<PhoneAction>>()
            .add_message::<crate::net::SessionUiCommand>()
            .add_message::<AppExit>()
            .add_systems(Update, phone_menu_actions);
        app
    }

    /// The panel saves a profile for the next start: it never reconnects this
    /// process, refuses production, and choosing the running profile cancels.
    #[test]
    fn the_connection_panel_saves_a_profile_for_the_next_start_and_never_connects() {
        use crate::connection_profile::load_selection;
        let path = scratch_selection("actions");
        let mut app = actions_app(ActiveConnection::for_contour(
            Some("dev"),
            Some(path.clone()),
        ));
        fn entry(app: &App) -> &ServerEntry {
            app.world().resource::<ServerEntry>()
        }
        fn connection(app: &App) -> ActiveConnection {
            app.world().resource::<ActiveConnection>().clone()
        }
        press(&mut app, PhoneAction::Server);
        assert!(entry(&app).open);
        assert_eq!(entry(&app).choice, Some(ProfileId::Dev));
        press(&mut app, PhoneAction::Profile(ProfileId::Local));
        press(&mut app, PhoneAction::Use);
        assert_eq!(
            entry(&app).confirming.as_ref().map(|profile| profile.id),
            Some(ProfileId::Local),
            "a switch asks first"
        );
        assert_eq!(
            load_selection(&path),
            None,
            "nothing saved before confirming"
        );
        press(&mut app, PhoneAction::Confirm);
        assert_eq!(entry(&app).notice, Some("connection.notice.saved"));
        assert_eq!(load_selection(&path).unwrap().profile, ProfileId::Local);
        assert_eq!(
            connection(&app).profile.id,
            ProfileId::Dev,
            "runs Dev until a restart"
        );
        assert_eq!(
            connection(&app).pending.map(|profile| profile.id),
            Some(ProfileId::Local)
        );
        // A custom production lobby is refused and marks the lobby field.
        press(&mut app, PhoneAction::Profile(ProfileId::Custom));
        app.world_mut().resource_mut::<ServerEntry>().lobby = "77.246.105.57:4000".into();
        press(&mut app, PhoneAction::Use);
        assert_eq!(
            entry(&app).error,
            Some(("connection.error.production", Some(CustomField::Lobby)))
        );
        assert!(entry(&app).confirming.is_none());
        app.world_mut().resource_mut::<ServerEntry>().lobby = "192.168.1.20:4000".into();
        app.world_mut().resource_mut::<ServerEntry>().registry = "https://registry.ekza.io".into();
        press(&mut app, PhoneAction::Use);
        assert_eq!(
            entry(&app).error,
            Some(("connection.error.production", Some(CustomField::Registry)))
        );
        app.world_mut()
            .resource_mut::<ServerEntry>()
            .registry
            .clear();
        press(&mut app, PhoneAction::Use);
        assert_eq!(
            entry(&app)
                .confirming
                .as_ref()
                .map(|profile| profile.lobby.as_str()),
            Some("192.168.1.20:4000")
        );
        press(&mut app, PhoneAction::Back);
        assert!(entry(&app).confirming.is_none());
        assert_eq!(load_selection(&path).unwrap().profile, ProfileId::Local);
        // Choosing the running profile cancels the saved switch at once.
        press(&mut app, PhoneAction::Profile(ProfileId::Dev));
        press(&mut app, PhoneAction::Use);
        assert_eq!(entry(&app).notice, Some("connection.notice.kept"));
        assert_eq!(connection(&app).pending, None);
        assert_eq!(load_selection(&path).unwrap().profile, ProfileId::Dev);
        press(&mut app, PhoneAction::Use);
        assert_eq!(entry(&app).notice, Some("connection.notice.current"));
        assert!(
            app.world()
                .resource::<Messages<crate::net::SessionUiCommand>>()
                .is_empty(),
            "the panel never connects this process anywhere"
        );
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn production_builds_never_open_the_panel() {
        let mut app = actions_app(ActiveConnection::for_contour(None, None));
        press(&mut app, PhoneAction::Server);
        assert!(!app.world().resource::<ServerEntry>().open);
        let mut entry = ServerEntry::default();
        entry.open_for(
            &ClientSession::default(),
            &ActiveConnection::for_contour(None, None),
        );
        assert!(!entry.open);
    }

    /// Every server or connection control a build might show, by name.
    fn connection_ui(app: &mut App) -> Vec<String> {
        let mut names: Vec<String> = app
            .world_mut()
            .query::<NodeKey>()
            .iter(app.world())
            .map(|key| key.as_str().to_owned())
            .filter(|name| {
                name.starts_with("Server")
                    || name.starts_with("Connection")
                    || name.contains("ServerButton")
                    || name.contains("Preset")
            })
            .collect();
        names.sort();
        names
    }

    fn phone_ui_app(phone: bool, connection: ActiveConnection) -> App {
        let mut app = App::new();
        let mut mobile = MobileControls::default();
        mobile.enabled = phone;
        mobile.landscape = true;
        mobile.viewport = Vec2::new(844.0, 390.0);
        app.insert_resource(mobile)
            .insert_resource(connection)
            .init_resource::<ClientSession>()
            .init_resource::<ServerEntry>()
            .add_systems(Startup, setup_phone_ui)
            .add_systems(Update, (sync_phone_ui, refresh_connection_panel).chain());
        app
    }

    #[test]
    fn production_builds_spawn_no_server_or_connection_ui_on_desktop_or_phone() {
        for phone in [false, true] {
            let mut app = phone_ui_app(phone, ActiveConnection::for_contour(None, None));
            app.update();
            app.world_mut().resource_mut::<ServerEntry>().open = true;
            app.update();
            assert_eq!(
                connection_ui(&mut app),
                Vec::<String>::new(),
                "phone {phone}"
            );
            assert!(
                !app.world_mut()
                    .query::<&ModalRoot>()
                    .iter(app.world())
                    .any(|root| root.0 == ModalId::ServerEntry)
            );
            assert!(
                !app.world_mut()
                    .query::<&UiAction<PhoneAction>>()
                    .iter(app.world())
                    .any(|UiAction(action)| matches!(action, PhoneAction::Server))
            );
        }
    }

    #[test]
    fn a_dev_build_offers_the_kit_connection_panel_and_no_presets() {
        for phone in [false, true] {
            let connection = ActiveConnection::for_contour(Some("dev"), None);
            let mut app = phone_ui_app(phone, connection.clone());
            app.update();
            assert_eq!(
                connection_ui(&mut app),
                if phone {
                    vec!["LegacyPhoneServerButton", "ServerEntryRoot"]
                } else {
                    vec!["ServerEntryRoot"]
                },
                "closed: only the root (and the legacy bar's button)"
            );
            app.world_mut()
                .resource_mut::<ServerEntry>()
                .open_for(&ClientSession::default(), &connection);
            app.update();
            let shown = connection_ui(&mut app);
            for id in [
                "ServerEntryPanel",
                "ServerEntryClose",
                "ConnectionProfileDev",
                "ConnectionProfileLocal",
                "ConnectionProfileCustom",
                "ConnectionRunning",
                "ConnectionUse",
            ] {
                assert!(shown.iter().any(|name| name == id), "{id} in {shown:?}");
            }
            for absent in [
                "ConnectionProfileProduction",
                "ConnectionLobbyField",
                "ServerKeypad",
                "ConnectionRestart",
            ] {
                assert!(!shown.iter().any(|name| name == absent), "{absent}");
            }
            assert!(!shown.iter().any(|name| name.contains("Preset")));
            // The custom profile shows its fields, and the keypad on a phone.
            app.world_mut().resource_mut::<ServerEntry>().choice = Some(ProfileId::Custom);
            app.update();
            let custom = connection_ui(&mut app);
            for id in ["ConnectionLobbyField", "ConnectionRegistryField"] {
                assert!(custom.iter().any(|name| name == id), "{id}");
            }
            assert_eq!(custom.iter().any(|name| name == "ServerKeypad"), phone);
            // The typed text is written into the field in place.
            app.world_mut().resource_mut::<ServerEntry>().lobby = "192.168.1.20:4000".into();
            app.update();
            let mut values = app.world_mut().query::<(&ConnectionFieldValue, &Text)>();
            assert!(values.iter(app.world()).any(|(value, text)| {
                value.0 == CustomField::Lobby && text.0 == "192.168.1.20:4000"
            }));
            // Exactly one panel after the rebuilds.
            assert_eq!(
                connection_ui(&mut app)
                    .iter()
                    .filter(|name| *name == "ServerEntryPanel")
                    .count(),
                1
            );
            app.world_mut().resource_mut::<ServerEntry>().open = false;
            app.update();
            assert!(
                !connection_ui(&mut app)
                    .iter()
                    .any(|name| name == "ServerEntryPanel")
            );
        }
    }

    #[test]
    fn a_saved_switch_shows_as_pending_and_offers_a_desktop_restart() {
        let path = scratch_selection("pending");
        let mut connection = ActiveConnection::for_contour(Some("dev"), Some(path.clone()));
        let local = connection
            .candidate(ProfileId::Local, &CustomEndpoints::default())
            .unwrap();
        connection.choose(local).unwrap();
        let mut app = phone_ui_app(false, connection.clone());
        app.update();
        app.world_mut()
            .resource_mut::<ServerEntry>()
            .open_for(&ClientSession::default(), &connection);
        app.update();
        let shown = connection_ui(&mut app);
        assert_eq!(
            app.world().resource::<ServerEntry>().choice,
            Some(ProfileId::Local),
            "opens on the profile that runs next"
        );
        assert!(shown.iter().any(|name| name == "ConnectionPending"));
        assert_eq!(
            shown.iter().any(|name| name == "ConnectionRestart"),
            crate::platform::CAN_RELAUNCH
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A gamepad back over the server entry closes the entry only; the pause
    /// menu under it stays open until the next back.
    #[test]
    fn escape_closes_only_the_frontmost_server_editor() {
        for pause_open in [false, true] {
            let mut app = App::new();
            app.init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<ServerEntry>()
                .init_resource::<crate::pause_menu::PauseMenuState>()
                .add_systems(
                    Update,
                    (
                        close_server_entry_on_back,
                        crate::pause_menu::toggle_pause_menu,
                    )
                        .chain(),
                );
            app.world_mut()
                .resource_mut::<crate::pause_menu::PauseMenuState>()
                .open = pause_open;
            app.world_mut().resource_mut::<ServerEntry>().open = true;
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Escape);
            app.update();
            assert!(!app.world().resource::<ServerEntry>().open);
            assert_eq!(
                app.world()
                    .resource::<crate::pause_menu::PauseMenuState>()
                    .open,
                pause_open
            );
            assert!(
                !app.world()
                    .resource::<ButtonInput<KeyCode>>()
                    .just_pressed(KeyCode::Escape)
            );
        }
    }

    #[test]
    fn pad_back_closes_the_server_entry_before_the_pause_menu_under_it() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<crate::ui::BackPress>()
            .init_resource::<ServerEntry>()
            .init_resource::<crate::pause_menu::PauseMenuState>()
            .add_systems(
                Update,
                (
                    close_server_entry_on_back,
                    crate::pause_menu::toggle_pause_menu,
                )
                    .chain(),
            );
        app.world_mut()
            .resource_mut::<crate::pause_menu::PauseMenuState>()
            .open = true;
        app.world_mut().resource_mut::<ServerEntry>().open = true;
        let pad_back = |app: &mut App| {
            app.world_mut()
                .resource_mut::<crate::ui::BackPress>()
                .press();
            app.update();
        };
        pad_back(&mut app);
        assert!(!app.world().resource::<ServerEntry>().open);
        assert!(
            app.world()
                .resource::<crate::pause_menu::PauseMenuState>()
                .open
        );
        pad_back(&mut app);
        assert!(
            !app.world()
                .resource::<crate::pause_menu::PauseMenuState>()
                .open
        );
    }

    /// The phone panels (help, shop, pause sections, career body) scroll
    /// through `ui::scroll` with the `mobile_ui` feel: window pixels, the
    /// first finger owns the drag, a short tap does not scroll.
    #[test]
    fn touch_scroll_uses_window_pixels_and_owns_drag_through_release() {
        for (dpi, ui_scale) in [(1.0, 1.0), (2.0, 1.0), (2.0, 0.75)] {
            let mut app = App::new();
            let mut mobile = MobileControls::default();
            mobile.enabled = true;
            mobile.focused = true;
            mobile.landscape = true;
            app.insert_resource(mobile)
                .add_message::<TouchInput>()
                .add_systems(Update, crate::ui::scroll::scroll_areas);
            let mut window = Window::default();
            window.resolution.set_scale_factor_override(Some(dpi));
            let window = app.world_mut().spawn((window, PrimaryWindow)).id();
            let combined = dpi * ui_scale;
            let panel = app
                .world_mut()
                .spawn((
                    ScrollArea::phone_panel(),
                    Name::new("HelpBody"),
                    ComputedNode {
                        size: Vec2::new(400.0, 200.0) * combined,
                        content_size: Vec2::new(400.0, 800.0) * combined,
                        inverse_scale_factor: 1.0 / combined,
                        ..default()
                    },
                    UiGlobalTransform::from(bevy::math::Affine2::from_translation(
                        Vec2::new(500.0, 400.0) * dpi,
                    )),
                ))
                .id();
            let event = |id, phase, position| TouchInput {
                id,
                phase,
                position,
                window,
                force: None,
            };
            // Batched events are normal on a busy mobile frame. An unrelated
            // second finger cannot steal this panel's captured pointer.
            for e in [
                event(1, TouchPhase::Started, Vec2::new(500.0, 440.0)),
                event(2, TouchPhase::Moved, Vec2::new(500.0, 200.0)),
                event(1, TouchPhase::Moved, Vec2::new(500.0, 340.0)),
                event(1, TouchPhase::Ended, Vec2::new(500.0, 340.0)),
            ] {
                app.world_mut().write_message(e);
            }
            app.update();
            assert!(
                (app.world().get::<ScrollPosition>(panel).unwrap().y - 100.0 / ui_scale).abs()
                    < 0.01
            );
            // Short taps don't scroll.
            app.world_mut()
                .write_message(event(3, TouchPhase::Started, Vec2::new(500.0, 400.0)));
            app.world_mut()
                .write_message(event(3, TouchPhase::Ended, Vec2::new(500.0, 397.0)));
            app.update();
            assert!(
                (app.world().get::<ScrollPosition>(panel).unwrap().y - 100.0 / ui_scale).abs()
                    < 0.01
            );
            // A hidden panel cannot take or keep a drag.
            app.world_mut()
                .entity_mut(panel)
                .insert(InheritedVisibility::HIDDEN);
            for e in [
                event(4, TouchPhase::Started, Vec2::new(500.0, 440.0)),
                event(4, TouchPhase::Moved, Vec2::new(500.0, 340.0)),
            ] {
                app.world_mut().write_message(e);
            }
            app.update();
            assert!(
                (app.world().get::<ScrollPosition>(panel).unwrap().y - 100.0 / ui_scale).abs()
                    < 0.01
            );
        }
    }

    #[test]
    fn help_and_shop_panels_become_phone_scroll_areas_and_the_connection_panel_is_a_modal_root() {
        let mut app = App::new();
        let mut mobile = MobileControls::default();
        mobile.enabled = true;
        mobile.landscape = true;
        mobile.viewport = Vec2::new(844.0, 390.0);
        app.insert_resource(mobile)
            .insert_resource(ActiveConnection::for_contour(Some("dev"), None))
            .init_resource::<ClientSession>()
            .add_systems(Startup, setup_phone_ui)
            .add_systems(PostUpdate, adapt_phone_layout);
        for name in ["HelpBody", "ShopCards"] {
            app.world_mut().spawn((Node::default(), Name::new(name)));
        }
        app.update();
        app.update();
        let mut areas = app.world_mut().query::<(&Name, &ScrollArea)>();
        let mut found: Vec<_> = areas
            .iter(app.world())
            .map(|(name, area)| {
                assert_eq!(*area, ScrollArea::phone_panel());
                name.as_str().to_owned()
            })
            .collect();
        found.sort();
        assert_eq!(found, ["HelpBody", "ShopCards"]);
        let mut roots = app.world_mut().query::<(&Name, &ModalRoot)>();
        assert!(roots.iter(app.world()).any(
            |(name, root)| name.as_str() == "ServerEntryRoot" && root.0 == ModalId::ServerEntry
        ));
    }

    #[test]
    fn closed_server_editor_does_not_change_another_fields_ime() {
        let mut app = App::new();
        app.init_resource::<ServerEntry>()
            .add_message::<KeyboardInput>()
            .add_message::<Ime>()
            .add_systems(Update, address_keyboard);
        let window = app
            .world_mut()
            .spawn((
                Window {
                    ime_enabled: true,
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        app.update();
        assert!(app.world().get::<Window>(window).unwrap().ime_enabled);
    }

    #[test]
    fn phone_utilities_keep_touch_targets_type_and_safe_insets_when_shell_scale_changes() {
        let mut app = App::new();
        let mut mobile = MobileControls::default();
        mobile.enabled = true;
        app.insert_resource(mobile)
            .init_resource::<ClientSession>()
            .init_resource::<ServerEntry>()
            .init_resource::<crate::shop::ShopState>()
            .init_resource::<UiScale>()
            .insert_resource(State::new(crate::frontend::AppScreen::Home))
            .add_systems(Startup, setup_phone_ui)
            .add_systems(Update, sync_phone_ui)
            .add_systems(PostUpdate, adapt_phone_layout);
        for (scale, shop_open) in [
            (1.0, false),
            (0.61, false),
            (0.61, true),
            (0.61, false),
            (0.8, false),
            (1.0, false),
        ] {
            app.world_mut().resource_mut::<UiScale>().0 = scale;
            app.world_mut()
                .resource_mut::<crate::shop::ShopState>()
                .open = shop_open;
            app.update();
            let mut buttons = app.world_mut().query::<(&UiAction<PhoneAction>, &Node)>();
            for (UiAction(action), node) in buttons.iter(app.world()) {
                if matches!(
                    action,
                    PhoneAction::Help | PhoneAction::Menu | PhoneAction::Server
                ) {
                    let Val::Px(height) = node.height else {
                        panic!("explicit target height")
                    };
                    let Val::Px(width) = node.width else {
                        panic!("explicit target width")
                    };
                    assert!((height * scale - 44.0).abs() < 0.001);
                    assert!(width * scale >= 47.999);
                }
            }
            let mut bars = app.world_mut().query_filtered::<&Node, With<PhoneBar>>();
            let node = bars.single(app.world()).unwrap();
            assert_eq!(node.display, Display::None);
            let Val::Px(top) = node.top else {
                panic!("safe top")
            };
            let Val::Px(right) = node.right else {
                panic!("safe right")
            };
            assert!((top * scale - 12.0).abs() < 0.001);
            assert!((right * scale - 32.0).abs() < 0.001);
            let mut fonts = app.world_mut().query::<(&Text, &TextFont)>();
            for (text, font) in fonts.iter(app.world()) {
                if matches!(text.0.as_str(), "?" | "MENU" | "SERVER") {
                    let bevy::text::FontSize::Px(px) = font.font_size else {
                        panic!("phone font must remain pixel-sized");
                    };
                    assert!((px * scale - 14.0).abs() < 0.001);
                }
            }
        }
    }

    #[test]
    fn screen_owned_headers_hide_the_legacy_floating_phone_bar() {
        let mut app = App::new();
        let mut mobile = MobileControls::default();
        mobile.enabled = true;
        app.insert_resource(mobile)
            .init_resource::<ClientSession>()
            .init_resource::<ServerEntry>()
            .add_systems(Startup, setup_phone_ui)
            .add_systems(Update, sync_phone_ui);
        for screen in [
            crate::frontend::AppScreen::Home,
            crate::frontend::AppScreen::HeroSelect,
            crate::frontend::AppScreen::InMatch,
            crate::frontend::AppScreen::Card,
        ] {
            app.insert_resource(State::new(screen));
            app.update();
            let mut bars = app.world_mut().query_filtered::<&Node, With<PhoneBar>>();
            assert_eq!(bars.single(app.world()).unwrap().display, Display::None);
        }
    }

    /// Phone copy is dictionary text: a language switch relabels the phone
    /// bar and the help overlay's phone copy, while text
    /// owned by other modules (they write their own phone wording) is left
    /// untouched.
    #[test]
    fn phone_copy_follows_the_language_and_leaves_other_modules_text_alone() {
        if crate::i18n::testing::isolated(
            "mobile_ui::tests::phone_copy_follows_the_language_and_leaves_other_modules_text_alone",
        ) {
            return;
        }
        use crate::i18n::{I18nPlugin, Locale, LocaleId, Localized};
        let mut app = App::new();
        let mut mobile = MobileControls::default();
        mobile.enabled = true;
        mobile.landscape = true;
        mobile.viewport = Vec2::new(844.0, 390.0);
        app.add_plugins(I18nPlugin::default())
            .insert_resource(mobile)
            .init_resource::<ClientSession>()
            .init_resource::<ServerEntry>()
            .add_systems(Startup, setup_phone_ui)
            .add_systems(Update, sync_phone_ui)
            .add_systems(PostUpdate, adapt_phone_layout);
        let world = app.world_mut();
        let dismiss = world
            .spawn((Text::new("Enter the arena"), Name::new("HelpDismissLabel")))
            .id();
        let others: Vec<(Entity, &str)> = [
            ("ClassSelectTitle", "01  Class"),
            ("ShopSummary", "Gold 40 · click an item to buy"),
            ("MatchStatusText", "Select a foe · P shop · F1 help"),
            ("GameStateLabel", "Escape: settings or exit game."),
            ("ShopDescription-vitality_gem", "+120 maximum HP"),
        ]
        .into_iter()
        .map(|(name, text)| (world.spawn((Text::new(text), Name::new(name))).id(), text))
        .collect();
        let read =
            |app: &mut App, entity: Entity| app.world().get::<Text>(entity).unwrap().0.clone();
        let label = |app: &mut App, key: &str| {
            let mut labels = app.world_mut().query::<(&Localized, &Text)>();
            labels
                .iter(app.world())
                .find(|(label, _)| label.key == key)
                .map(|(_, text)| text.0.clone())
                .unwrap()
        };
        app.update();
        assert_eq!(
            read(&mut app, dismiss),
            crate::i18n::tr("help.phone.dismiss")
        );
        assert_eq!(label(&mut app, "phone.bar.menu"), "MENU");
        let zh = LocaleId::parse("zh-Hans").unwrap();
        app.world_mut().resource_mut::<Locale>().set(zh);
        app.update();
        assert_eq!(read(&mut app, dismiss), "知道了，开始游戏");
        assert_eq!(label(&mut app, "phone.bar.menu"), "菜单");
        for (entity, text) in others {
            assert_eq!(read(&mut app, entity), text);
        }
    }

    #[test]
    fn custom_fields_keep_address_characters_within_the_limit() {
        let mut lobby = String::new();
        edit_field(CustomField::Lobby, &mut lobby, "192.168.1.20:4000\n\t/;💥");
        assert_eq!(lobby, "192.168.1.20:4000");
        assert!(crate::persistence::validate_game_server_addr(&lobby).is_some());
        lobby.clear();
        edit_field(CustomField::Lobby, &mut lobby, "beta-server.example:4000");
        assert!(crate::persistence::validate_game_server_addr(&lobby).is_some());
        lobby.clear();
        edit_field(CustomField::Lobby, &mut lobby, "[2001:db8::1]:4000");
        assert!(crate::persistence::validate_game_server_addr(&lobby).is_some());
        edit_field(CustomField::Lobby, &mut lobby, &"a".repeat(300));
        assert_eq!(lobby.chars().count(), MAX_FIELD_CHARS);
        let mut registry = String::new();
        edit_field(
            CustomField::Registry,
            &mut registry,
            "https://registry.lan.example:8443/ ?#",
        );
        assert_eq!(registry, "https://registry.lan.example:8443/");
    }
}
