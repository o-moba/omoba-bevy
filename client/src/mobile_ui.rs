//! Phone layouts and a shell-free address entry for controlled native playtests.
use bevy::{
    input::keyboard::{Key, KeyboardInput},
    input::touch::{TouchInput, TouchPhase},
    prelude::*,
    window::PrimaryWindow,
};

use crate::{
    mobile_controls::{MobileControls, MobileControlsSet},
    net::{ClientSession, SessionUiCommand},
    ui_theme as ui,
};

pub(crate) struct MobileUiPlugin;

impl Plugin for MobileUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MobileControls>();
        if !app.world().resource::<MobileControls>().enabled {
            // Desktop keeps its own HUD and keyboard/IME ownership. Do not
            // install phone-only overlays or address-entry systems there.
            return;
        }
        app.init_resource::<ServerEntry>()
            .add_systems(Startup, setup_phone_ui)
            .add_systems(
                Update,
                phone_menu_actions
                    .after(MobileControlsSet::Layout)
                    .before(crate::help_overlay::HelpOverlaySet::Input)
                    .in_set(crate::input_context::InputContextSet::Modal),
            )
            .add_systems(
                Update,
                (address_keyboard, sync_phone_ui)
                    .chain()
                    .after(phone_menu_actions),
            )
            .add_systems(
                PostUpdate,
                (adapt_phone_layout, scroll_phone_panels).before(bevy::ui::UiSystems::Layout),
            );
    }
}

#[derive(Resource, Default)]
pub(crate) struct ServerEntry {
    pub open: bool,
    address: String,
    error: String,
    initialized: bool,
    keyboard: bool,
}

#[derive(Component, Clone)]
enum PhoneAction {
    Menu,
    Help,
    Server,
    Connect,
    Close,
    Key(char),
    Backspace,
    Keyboard,
}
#[derive(Component)]
struct PhoneBar;
#[derive(Component)]
struct ServerEntryRoot;
#[derive(Component)]
struct ServerAddressLabel;
#[derive(Component)]
struct ServerErrorLabel;

fn phone_button(parent: &mut ChildSpawnerCommands, label: &str, name: &str, action: PhoneAction) {
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
            action,
            Name::new(name.to_owned()),
        ))
        .with_children(|button| {
            button.spawn((Text::new(label), ui::text(14.0), TextColor(ui::IVORY)));
        });
}

fn setup_phone_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                column_gap: Val::Px(6.0),
                ..default()
            },
            // Above the front-end screens: on a phone this bar is the only way
            // to settings and to the server address (there is no Escape key).
            ZIndex(crate::frontend::widgets::SCREEN_Z + 10),
            PhoneBar,
            Name::new("PhoneMenuBar"),
        ))
        .with_children(|bar| {
            phone_button(bar, "?", "PhoneHelpButton", PhoneAction::Help);
            phone_button(bar, "MENU", "PhoneMenuButton", PhoneAction::Menu);
            phone_button(bar, "SERVER", "PhoneServerButton", PhoneAction::Server);
        });
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                display: Display::None,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.005, 0.02, 0.02, 0.98)),
            ZIndex(150),
            ServerEntryRoot,
            Name::new("ServerEntryRoot"),
        ))
        .with_children(|overlay| {
            overlay
                .spawn((
                    Node {
                        width: Val::Px(640.0),
                        max_width: Val::Percent(90.0),
                        padding: UiRect::all(Val::Px(14.0)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.0),
                        ..ui::panel_node()
                    },
                    BackgroundColor(ui::PANEL),
                    BorderColor::all(ui::EDGE),
                    Name::new("ServerEntryPanel"),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("Join a hosted game"),
                        ui::text(24.0),
                        TextColor(ui::IVORY),
                    ));
                    panel.spawn((
                        Text::new(
                            "Enter the address your host shared, for example 192.168.1.20:4000",
                        ),
                        ui::text(14.0),
                        TextColor(ui::MUTED),
                    ));
                    panel.spawn((
                        Node {
                            padding: UiRect::all(Val::Px(10.0)),
                            min_height: Val::Px(44.0),
                            ..default()
                        },
                        Text::new(""),
                        ui::text(20.0),
                        TextColor(ui::GOLD),
                        BackgroundColor(ui::TILE),
                        ServerAddressLabel,
                    ));
                    // A native numeric keypad also works when an Android IME does not
                    // deliver composed text to NativeActivity. Hostnames use normal IME.
                    panel
                        .spawn((
                            Node {
                                flex_wrap: FlexWrap::Wrap,
                                column_gap: Val::Px(6.0),
                                row_gap: Val::Px(6.0),
                                ..default()
                            },
                            Name::new("ServerKeypad"),
                        ))
                        .with_children(|keys| {
                            for key in "1234567890.:".chars() {
                                phone_button(
                                    keys,
                                    &key.to_string(),
                                    &format!("ServerKey-{key}"),
                                    PhoneAction::Key(key),
                                );
                            }
                            phone_button(keys, "DELETE", "ServerBackspace", PhoneAction::Backspace);
                        });
                    panel.spawn((
                        Text::new(""),
                        ui::text(13.0),
                        TextColor(ui::GOLD),
                        ServerErrorLabel,
                    ));
                    panel
                        .spawn((Node {
                            column_gap: Val::Px(8.0),
                            ..default()
                        },))
                        .with_children(|row| {
                            phone_button(
                                row,
                                "CONNECT",
                                "ServerConnectButton",
                                PhoneAction::Connect,
                            );
                            phone_button(row, "CLOSE", "ServerCloseButton", PhoneAction::Close);
                            phone_button(
                                row,
                                "KEYBOARD",
                                "ServerKeyboardButton",
                                PhoneAction::Keyboard,
                            );
                        });
                });
        });
}

fn edit_address(address: &mut String, text: &str) {
    for ch in text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-' | '[' | ']'))
    {
        if address.len() < 253 {
            address.push(ch);
        }
    }
}

fn phone_menu_actions(
    actions: Query<(&Interaction, &PhoneAction), (With<Button>, Changed<Interaction>)>,
    mobile: Res<MobileControls>,
    session: Res<ClientSession>,
    mut entry: ResMut<ServerEntry>,
    mut pause: ResMut<crate::pause_menu::PauseMenuState>,
    mut help: ResMut<crate::help_overlay::HelpOverlayVisible>,
    mut requests: MessageWriter<SessionUiCommand>,
) {
    if !mobile.enabled {
        return;
    }
    for (interaction, action) in &actions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            PhoneAction::Menu => pause.open = !pause.open,
            PhoneAction::Help => help.0 = !help.0,
            PhoneAction::Server if !session.join_in_flight() && !session.has_committed_join() => {
                entry.open = true;
                entry.address = if session.server_addr() == "127.0.0.1:4000" {
                    String::new()
                } else {
                    session.server_addr().to_owned()
                };
                entry.error.clear();
            }
            PhoneAction::Close => {
                entry.open = false;
                entry.keyboard = false;
            }
            PhoneAction::Keyboard => entry.keyboard = !entry.keyboard,
            PhoneAction::Key(ch) if entry.open => edit_address(&mut entry.address, &ch.to_string()),
            PhoneAction::Backspace if entry.open => {
                entry.address.pop();
            }
            PhoneAction::Connect if entry.open => {
                if let Some(address) = crate::persistence::validate_game_server_addr(&entry.address)
                {
                    requests.write(SessionUiCommand::ConnectTo(address));
                    entry.open = false;
                    entry.error.clear();
                } else {
                    entry.error =
                        "Use a host address and port, for example 192.168.1.20:4000".into();
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn address_keyboard(
    mut entry: ResMut<ServerEntry>,
    mut input: MessageReader<KeyboardInput>,
    mut ime: MessageReader<Ime>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if let Ok(mut window) = windows.single_mut() {
        window.ime_enabled = entry.open && entry.keyboard;
    }
    for event in ime.read() {
        if entry.open
            && let Ime::Commit { value, .. } = event
        {
            edit_address(&mut entry.address, value);
        }
    }
    for event in input.read() {
        if !entry.open || !event.state.is_pressed() {
            continue;
        }
        match &event.logical_key {
            Key::Backspace => {
                entry.address.pop();
            }
            Key::Escape => entry.open = false,
            _ => {
                let text = event.text.as_deref().or_else(|| match &event.logical_key {
                    Key::Character(text) => Some(text.as_str()),
                    _ => None,
                });
                if let Some(text) = text {
                    edit_address(&mut entry.address, text);
                }
            }
        }
    }
}

fn sync_phone_ui(
    mobile: Res<MobileControls>,
    session: Res<ClientSession>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    shop: Option<Res<crate::shop::ShopState>>,
    mut entry: ResMut<ServerEntry>,
    mut bar: Query<
        &mut Node,
        (
            With<PhoneBar>,
            Without<ServerEntryRoot>,
            Without<PhoneAction>,
        ),
    >,
    mut overlay: Query<
        &mut Node,
        (
            With<ServerEntryRoot>,
            Without<PhoneBar>,
            Without<PhoneAction>,
        ),
    >,
    mut buttons: Query<(&PhoneAction, &mut Node), (Without<PhoneBar>, Without<ServerEntryRoot>)>,
    mut address: Query<&mut Text, (With<ServerAddressLabel>, Without<ServerErrorLabel>)>,
    mut errors: Query<&mut Text, (With<ServerErrorLabel>, Without<ServerAddressLabel>)>,
    ui_scale: Option<Res<UiScale>>,
) {
    let scale = ui_scale.as_ref().map_or(1.0, |scale| scale.0.max(0.1));
    if mobile.enabled && !entry.initialized && !session.server_addr().is_empty() {
        entry.initialized = true;
        if session.server_addr() == "127.0.0.1:4000" {
            entry.open = true;
        }
    }
    // The bar is how a phone reaches settings and the server address: it
    // belongs on the home screen and the picker. The match has its compact
    // score/menu strip. The other menus
    // have their own Back and would collide with it in the corner.
    let bar_wanted = screen.as_ref().is_some_and(|screen| {
        use crate::frontend::AppScreen;
        matches!(screen.get(), AppScreen::Home | AppScreen::HeroSelect)
    });
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
    for (action, mut node) in &mut buttons {
        let width = match action {
            PhoneAction::Help => Some(48.0),
            PhoneAction::Menu => Some(64.0),
            PhoneAction::Server => Some(88.0),
            _ => None,
        };
        if let Some(width) = width {
            node.width = Val::Px(width / scale);
            node.min_width = Val::Px(48.0 / scale);
            node.height = Val::Px(44.0 / scale);
            node.min_height = Val::Px(44.0 / scale);
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
        node.display = if mobile.enabled && entry.open && mobile.landscape {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut text in &mut address {
        text.0 = format!("{}|", entry.address);
    }
    for mut text in &mut errors {
        text.0.clone_from(&entry.error);
    }
}

#[derive(Component)]
struct PhoneFontSize(f32);

fn phone_family(
    entity: Entity,
    hierarchy: &Query<(Option<&ChildOf>, Option<&Name>)>,
) -> Option<&'static str> {
    let mut next = Some(entity);
    for _ in 0..12 {
        let (parent, name) = hierarchy.get(next?).ok()?;
        if let Some(name) = name {
            if name.as_str().starts_with("ShopBuy-") {
                return Some("shop-card");
            }
            let family = match name.as_str() {
                "TeamSelectOverlay" => "entry",
                "ShopPanel" => "shop",
                "ShopSummary" => "shop-summary",
                "PauseMenuPanel" => "pause",
                "GameStateCard" => "result",
                "HelpPanel" => "help",
                "PhoneMenuBar" => "phone-menu",
                _ => "",
            };
            if !family.is_empty() {
                return Some(family);
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
    mut nodes: Query<(Entity, &Name, &mut Node, Option<&mut UiTransform>)>,
    mut fonts: Query<(Entity, &mut TextFont, Option<&PhoneFontSize>)>,
    mut copy: Query<(&Name, &mut Text)>,
    hierarchy: Query<(Option<&ChildOf>, Option<&Name>)>,
    pause: Option<Res<crate::pause_menu::PauseMenuState>>,
    ui_scale: Option<Res<UiScale>>,
) {
    if !mobile.enabled || !mobile.landscape {
        return;
    }
    let scale = ui_scale.as_ref().map_or(1.0, |scale| scale.0.max(0.1));
    let left = mobile.safe.left;
    let top = mobile.safe.top;
    let bottom = mobile.safe.bottom;
    let width = mobile.viewport.x - left - mobile.safe.right;
    let height = mobile.viewport.y - top - bottom;
    let class_width = (width * 0.26).clamp(150.0, 210.0);
    let grid_left = left + class_width + 20.0;
    let grid_width = width - class_width - 20.0;
    for (entity, name, mut node, _transform) in &mut nodes {
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
                node.height = Val::Px(44.0);
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
                    Some(44.0),
                );
                node.column_gap = Val::Px(0.0);
            }
            "HeroSelectBack" => {
                node.width = Val::Percent(100.0);
                node.height = Val::Px(44.0);
            }
            // Desktop-only picker parts: no room beside the phone grid, and the
            // wallet pairing flow is not supported on a phone yet.
            "HeroSelectTitle" | "HeroSelectStatus" | "HeroSelectPanel" | "EkzaConnectRow" => {
                node.display = Display::None;
            }
            "ServerEntryPanel" => {
                node.width = Val::Px(width.min(860.0));
                node.max_width = Val::Px(width);
                node.padding = UiRect::all(Val::Px(10.0));
                node.row_gap = Val::Px(6.0);
            }
            "HelpPanel" => {
                node.max_height = Val::Px(height);
                node.width = Val::Px(width.min(740.0));
                node.max_width = Val::Px(width);
                node.padding = UiRect::all(Val::Px(14.0));
                node.row_gap = Val::Px(10.0);
            }
            "HelpBody" => {
                // New combat actions remain discoverable without pushing the
                // fixed 44px dismiss action below a landscape phone viewport.
                node.max_height = Val::Px((height - 82.0).max(120.0));
                node.min_height = Val::Px(0.0);
                node.flex_shrink = 1.0;
                node.overflow = Overflow::scroll_y();
                commands
                    .entity(entity)
                    .insert_if_new((ScrollPosition::default(), TouchScrollPanel));
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
                    .insert_if_new((ScrollPosition::default(), TouchScrollPanel));
            }
            "ShopFooter" => node.display = Display::None,
            "ShopCloseButton" => {
                node.min_height = Val::Px(44.0);
                node.padding = UiRect::axes(Val::Px(10.0), Val::Px(5.0));
            }
            "PauseMenuPanel" => {
                node.top = Val::Px((top - bottom) * 0.5);
                node.width = Val::Px(width.min(650.0));
                // Bound both bodies so short windows scroll between the fixed
                // header/close control and footer.
                node.height = if pause.as_ref().is_some_and(|pause| pause.in_settings) {
                    Val::Px(height)
                } else {
                    Val::Px(height.min(360.0))
                };
                node.max_height = Val::Px(height);
                node.padding = UiRect::all(Val::Px(10.0));
                node.row_gap = Val::Px(6.0);
                node.overflow = Overflow::clip();
            }
            "PauseMenuMainSection" => node.row_gap = Val::Px(10.0),
            "GameStateCard" => {
                node.width = Val::Px(width.min(640.0));
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
            "ConnectionRetryButton" => node.height = Val::Px(44.0),
            name if name.starts_with("ClassButton-") => {
                node.width = Val::Px(class_width);
                node.height = Val::Px(48.0);
            }
            name if name.starts_with("AvatarButton-") || name.starts_with("SpriteButton-") => {
                node.width = Val::Px(72.0);
                node.height = Val::Px(82.0);
            }
            name if name.starts_with("ShopBuy-") => {
                node.width = Val::Px((width - 36.0) / 3.0);
                node.height = Val::Px(103.0);
                node.padding = UiRect::all(Val::Px(if width < 650.0 { 6.0 } else { 7.0 }));
                node.row_gap = Val::Px(if width < 650.0 { 2.0 } else { 3.0 });
            }
            _ => {}
        }
    }
    for (entity, mut font, base) in &mut fonts {
        let Some(family) = phone_family(entity, &hierarchy) else {
            continue;
        };
        let original = base.map_or(font.font_size, |base| base.0);
        if base.is_none() {
            commands.entity(entity).insert(PhoneFontSize(original));
        }
        font.font_size = match family {
            "phone-menu" => original / scale,
            "entry" => original.clamp(11.0, 16.0),
            "shop-card" if width < 650.0 => {
                if original >= 18.0 {
                    15.0
                } else {
                    12.0
                }
            }
            "shop-card" => original.clamp(12.0, 18.0),
            "shop" => original.clamp(12.0, 18.0),
            "shop-summary" => 14.0,
            "result" => 20.0,
            "help" => 15.0,
            "pause" => original.clamp(14.0, 22.0),
            _ => original,
        };
    }
    for (name, mut text) in &mut copy {
        match name.as_str() {
            "RendererStatus" => text.0 = "Swipe to choose your hero".into(),
            "ClassSelectTitle" => text.0 = "01  CLASS".into(),
            "AvatarSelectTitle" => text.0 = "02  HERO".into(),
            "TeamSelectTitle" => text.0 = "03  MATCH".into(),
            "TeamSelectHint" => text.0 = "Your team and side are assigned automatically.".into(),

            "ShopCloseLabel" => text.0 = "CLOSE".into(),
            "ShopSummary" => text.0 = text.0.replace("click an item", "tap an item"),
            "HelpDismissLabel" => text.0 = "Got it — play".into(),
            "MatchStatusText" => text.0 = text.0
                .replace("clear all towers in one lane to expose the enemy base.", "Clear a lane's towers to unlock the base.")
                .replace("Select a foe · P shop · F1 help", "Tap ATTACK · Drag to lock")
                .replace("Target locked · Attack / Q W E R", "Target locked · ATTACK / skills"),
            "ShopFooter" => text.0 = "Buy at your base. Items survive respawn and reset next round.".into(),
            name if name.starts_with("ShopDescription-") => text.0 = text.0.replace("maximum HP", "max HP"),
            "HelpBody" => text.0 = "YOUR FIRST MATCH\n\nMOVE: Drag the left stick. Release to stop.\nATTACK: Tap the large right button; hold to repeat. No mana needed.\nTARGET: Drag ATTACK to extend the reticle. Release on a highlighted foe to lock. Drag to X to cancel.\nFARM: The small minion and tower buttons target only that category.\nUTILITY: Dash moves in your stick direction; drag it to aim. Haste boosts movement briefly.\nSKILLS: Q/W/E/R surround ATTACK. Tap to use the locked target, or drag to aim.\nGROW: Abilities unlock as you level. Tap RANK, then a glowing skill to spend a point.\nWIN: Follow your minions, clear all towers in one lane, then destroy the enemy base.\nRECOVER: Return to your base to heal and shop. If defeated, wait to respawn.\nLOOK: Tap the minimap to scout. Move the stick to follow your hero again.\n\nThe match continues while menus are open. Stay connected for the next round.".into(),
            "GameStateLabel" => text.0 = text.0.replace("Escape: settings or exit game.", "MENU: settings or exit game."),
            _ => {}
        }
    }
}

/// Explicit ownership prevents a hidden or underlying panel from stealing drags.
#[derive(Component)]
pub(crate) struct TouchScrollPanel;

#[derive(Default)]
struct ScrollTouch {
    held: Option<(u64, Entity, Vec2, Vec2, bool)>,
}

pub(crate) use crate::ui::gesture::logical_ui_rect;

fn scroll_phone_panels(
    mobile: Res<MobileControls>,
    window: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut events: MessageReader<TouchInput>,
    mut drag: Local<ScrollTouch>,
    career: Option<Res<crate::career::CareerClient>>,
    pause: Option<Res<crate::pause_menu::PauseMenuState>>,
    mut panels: Query<
        (
            Entity,
            &Name,
            &ComputedNode,
            &UiGlobalTransform,
            &mut ScrollPosition,
            Option<&InheritedVisibility>,
            Option<&bevy::ui::CalculatedClip>,
        ),
        With<TouchScrollPanel>,
    >,
) {
    let Ok((window_id, window)) = window.single() else {
        events.clear();
        drag.held = None;
        return;
    };
    if !mobile.enabled || !mobile.focused || !mobile.landscape || !window.focused {
        events.clear();
        drag.held = None;
        return;
    }
    let allowed = |name: &str| {
        if career.as_ref().is_some_and(|c| c.modal_open()) {
            name == "CareerBody"
        } else if pause.as_ref().is_some_and(|p| p.open) {
            if pause.as_ref().is_some_and(|p| p.in_settings) {
                name == "PauseMenuSettingsSection"
            } else {
                name == "PauseMenuMainSection"
            }
        } else {
            name == "HelpBody" || name == "ShopCards"
        }
    };
    for event in events.read().filter(|e| e.window == window_id) {
        if event.phase == TouchPhase::Started && drag.held.is_none() {
            let candidate = panels
                .iter()
                .filter(|(_, name, node, _, _, visible, _)| {
                    allowed(name.as_str())
                        && visible.is_none_or(|v| v.get())
                        && node.size().min_element() > 0.0
                        && node.content_size().y > node.size().y
                })
                .filter(|(_, _, node, transform, _, _, clip)| {
                    logical_ui_rect(node, transform, *clip, window.scale_factor())
                        .contains(event.position)
                })
                .min_by(|a, b| {
                    (a.2.size().x * a.2.size().y).total_cmp(&(b.2.size().x * b.2.size().y))
                })
                .map(|p| p.0);
            if let Some(entity) = candidate {
                drag.held = Some((event.id, entity, event.position, event.position, false));
            }
            continue;
        }
        let Some((id, entity, start, previous, moved)) =
            drag.held.as_mut().filter(|(id, ..)| *id == event.id)
        else {
            continue;
        };
        let _ = id;
        if let Ok((_, name, node, _, mut scroll, visible, _)) = panels.get_mut(*entity) {
            if !allowed(name.as_str()) || visible.is_some_and(|v| !v.get()) {
                drag.held = None;
                continue;
            }
            let was_moved = *moved;
            *moved |= start.distance(event.position) > 10.0;
            if *moved && matches!(event.phase, TouchPhase::Moved | TouchPhase::Ended) {
                let delta = if was_moved { previous.y } else { start.y } - event.position.y;
                let max = ((node.content_size().y - node.size().y) * node.inverse_scale_factor())
                    .max(0.0);
                // ScrollPosition is in UI units; TouchInput is in logical window pixels.
                scroll.y = (scroll.y + delta * window.scale_factor() * node.inverse_scale_factor())
                    .clamp(0.0, max);
            }
            *previous = event.position;
        } else {
            drag.held = None;
            continue;
        }
        if matches!(event.phase, TouchPhase::Ended | TouchPhase::Canceled) {
            drag.held = None;
        }
    }
}

#[cfg(test)]
pub(crate) fn add_pause_layout_test_systems(app: &mut App) {
    app.add_systems(
        PostUpdate,
        (adapt_phone_layout, scroll_phone_panels).before(bevy::ui::UiSystems::Layout),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_scroll_uses_window_pixels_and_owns_drag_through_release() {
        for (dpi, ui_scale) in [(1.0, 1.0), (2.0, 1.0), (2.0, 0.75)] {
            let mut app = App::new();
            let mut mobile = MobileControls::default();
            mobile.enabled = true;
            mobile.focused = true;
            mobile.landscape = true;
            app.insert_resource(mobile)
                .insert_resource(crate::pause_menu::PauseMenuState {
                    open: true,
                    in_settings: true,
                })
                .add_message::<TouchInput>()
                .add_systems(Update, scroll_phone_panels);
            let mut window = Window::default();
            window.resolution.set_scale_factor_override(Some(dpi));
            let window = app.world_mut().spawn((window, PrimaryWindow)).id();
            let combined = dpi * ui_scale;
            let panel = app
                .world_mut()
                .spawn((
                    TouchScrollPanel,
                    Name::new("PauseMenuSettingsSection"),
                    ComputedNode {
                        size: Vec2::new(400.0, 200.0) * combined,
                        content_size: Vec2::new(400.0, 800.0) * combined,
                        inverse_scale_factor: 1.0 / combined,
                        ..default()
                    },
                    UiGlobalTransform::from(bevy::math::Affine2::from_translation(
                        Vec2::new(500.0, 400.0) * dpi,
                    )),
                    ScrollPosition::default(),
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
            // Short taps don't scroll. Closed menus cannot retain ownership.
            app.world_mut()
                .write_message(event(3, TouchPhase::Started, Vec2::new(500.0, 400.0)));
            app.world_mut()
                .write_message(event(3, TouchPhase::Ended, Vec2::new(500.0, 397.0)));
            app.update();
            assert!(
                (app.world().get::<ScrollPosition>(panel).unwrap().y - 100.0 / ui_scale).abs()
                    < 0.01
            );
        }
    }

    #[test]
    fn desktop_does_not_install_phone_forms_or_change_keyboard_ime() {
        let mut app = App::new();
        let mut controls = MobileControls::default();
        controls.enabled = false;
        app.insert_resource(controls).add_plugins(MobileUiPlugin);
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
        assert!(!app.world().contains_resource::<ServerEntry>());
        assert_eq!(
            app.world_mut()
                .query::<&PhoneBar>()
                .iter(app.world())
                .count(),
            0
        );
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
            let mut buttons = app.world_mut().query::<(&PhoneAction, &Node)>();
            for (action, node) in buttons.iter(app.world()) {
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
            assert_eq!(
                node.display,
                if shop_open {
                    Display::None
                } else {
                    Display::Flex
                }
            );
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
                    assert!((font.font_size * scale - 14.0).abs() < 0.001);
                }
            }
        }
    }

    #[test]
    fn phone_menu_bar_belongs_only_to_home_and_hero_picker() {
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
            assert_eq!(
                bars.single(app.world()).unwrap().display,
                if matches!(
                    screen,
                    crate::frontend::AppScreen::Home | crate::frontend::AppScreen::HeroSelect
                ) {
                    Display::Flex
                } else {
                    Display::None
                }
            );
        }
    }

    #[test]
    fn address_keypad_filters_controls_and_supports_ipv4_hostnames() {
        let mut address = String::new();
        edit_address(&mut address, "192.168.1.20:4000\n\t/;💥");
        assert_eq!(address, "192.168.1.20:4000");
        assert!(crate::persistence::validate_game_server_addr(&address).is_some());
        address.clear();
        edit_address(&mut address, "beta-server.example:4000");
        assert!(crate::persistence::validate_game_server_addr(&address).is_some());
        address.clear();
        edit_address(&mut address, "[2001:db8::1]:4000");
        assert!(crate::persistence::validate_game_server_addr(&address).is_some());
        edit_address(&mut address, &"a".repeat(300));
        assert_eq!(address.len(), 253);
    }
}
