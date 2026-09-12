//! Phone layouts and a shell-free address entry for controlled native playtests.
use bevy::{
    input::keyboard::{Key, KeyboardInput},
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
            ZIndex(30),
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
            PhoneAction::Server if !session.join_flow_committed && session.last_join.is_none() => {
                entry.open = true;
                entry.address = if session.server_addr_display == "127.0.0.1:4000" {
                    String::new()
                } else {
                    session.server_addr_display.clone()
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

fn address_keyboard(
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
) {
    if mobile.enabled && !entry.initialized && !session.server_addr_display.is_empty() {
        entry.initialized = true;
        if session.server_addr_display == "127.0.0.1:4000" {
            entry.open = true;
        }
    }
    for mut node in &mut bar {
        node.display = if mobile.enabled && mobile.landscape {
            Display::Flex
        } else {
            Display::None
        };
        node.right = Val::Px(mobile.safe.right);
        node.top = Val::Px(mobile.safe.top);
    }
    for (action, mut node) in &mut buttons {
        if matches!(action, PhoneAction::Server) {
            node.display = if !session.join_flow_committed && session.last_join.is_none() {
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
                "PauseMenuPanel" => "pause",
                "GameStateCard" => "result",
                "HelpPanel" => "help",
                "MatchHudColumn" => "hero",
                "MatchObjectiveRoot" => "objective",
                "EquipmentHud" => "equipment",
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
) {
    if !mobile.enabled || !mobile.landscape {
        return;
    }
    let left = mobile.safe.left;
    let top = mobile.safe.top;
    let bottom = mobile.safe.bottom;
    let width = mobile.viewport.x - left - mobile.safe.right;
    let height = mobile.viewport.y - top - bottom;
    let minimap_size = (height * 0.37).clamp(112.0, 142.0);
    // Keep status beside the menu, above the right thumb's combat fan. The
    // objective uses only the space between the minimap and this status card.
    let hero_left = mobile.viewport.x - mobile.safe.right - 300.0;
    let objective_left = left + minimap_size + 12.0;
    let class_width = (width * 0.26).clamp(150.0, 210.0);
    let grid_left = left + class_width + 20.0;
    let grid_width = width - class_width - 20.0;
    for (entity, name, mut node, transform) in &mut nodes {
        match name.as_str() {
            "MinimapRoot" => {
                let size = minimap_size;
                absolute(
                    &mut node,
                    left + (size - 252.0) * 0.5,
                    top + (size - 252.0) * 0.5,
                    252.0,
                    Some(252.0),
                );
                if let Some(mut transform) = transform {
                    transform.scale = Vec2::splat(size / 252.0);
                } else {
                    commands
                        .entity(entity)
                        .insert(UiTransform::from_scale(Vec2::splat(size / 252.0)));
                }
            }
            "MatchHudColumn" => {
                absolute(&mut node, hero_left, top, 172.0, Some(108.0));
                node.padding = UiRect::all(Val::Px(7.0));
                node.row_gap = Val::Px(3.0);
            }
            "HudHeroPortrait" => {
                node.width = Val::Px(30.0);
                node.height = Val::Px(30.0);
            }
            "MatchHudBar-HP" | "MatchHudBar-MP" => node.height = Val::Px(15.0),
            "MatchObjectiveRoot" => {
                absolute(
                    &mut node,
                    objective_left,
                    top,
                    (hero_left - objective_left - 12.0).max(1.0),
                    None,
                );
            }
            "MatchObjectivePanel" => {
                node.max_width = Val::Percent(100.0);
                node.padding = UiRect::all(Val::Px(6.0));
                node.row_gap = Val::Px(2.0);
            }
            "EquipmentHud" => {
                absolute(
                    &mut node,
                    left,
                    top + minimap_size + 8.0,
                    minimap_size,
                    Some(71.0),
                );
                node.padding = UiRect::all(Val::Px(4.0));
                node.row_gap = Val::Px(3.0);
            }
            "InventorySlots" => node.display = Display::None,
            "ShopOpenButton" => {
                node.height = Val::Px(44.0);
                node.min_height = Val::Px(44.0);
                node.padding = UiRect::all(Val::Px(4.0));
            }
            "TeamSelectOverlay" => {
                node.padding = UiRect::ZERO;
            }
            "ClassSelectTitle" => absolute(&mut node, left, top + 7.0, class_width, None),
            "ClassButtonsRow" => {
                absolute(&mut node, left, top + 36.0, class_width, None);
                node.flex_direction = FlexDirection::Column;
                node.row_gap = Val::Px(6.0);
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
            "TeamGreenButton" | "TeamBlueButton" => {
                node.width = Val::Px((grid_width - 10.0) * 0.5);
                node.height = Val::Px(44.0);
            }
            "TeamSelectHint" => absolute(
                &mut node,
                left,
                mobile.viewport.y - bottom - 17.0,
                width,
                None,
            ),
            "ServerEntryPanel" => {
                node.width = Val::Px(width.min(860.0));
                node.max_width = Val::Px(width);
                node.padding = UiRect::all(Val::Px(10.0));
                node.row_gap = Val::Px(6.0);
            }
            "HelpPanel" => {
                node.width = Val::Px(width.min(740.0));
                node.max_width = Val::Px(width);
                node.padding = UiRect::all(Val::Px(14.0));
                node.row_gap = Val::Px(10.0);
            }
            "ShopPanel" => {
                node.width = Val::Px(width);
                node.max_width = Val::Px(width);
                node.padding = UiRect::all(Val::Px(10.0));
                node.row_gap = Val::Px(6.0);
            }
            "ShopCards" => {
                node.column_gap = Val::Px(6.0);
                node.row_gap = Val::Px(6.0);
            }
            "ShopCloseButton" => {
                node.min_height = Val::Px(44.0);
                node.padding = UiRect::axes(Val::Px(10.0), Val::Px(5.0));
            }
            "PauseMenuPanel" => {
                node.width = Val::Px(width.min(650.0));
                node.height = Val::Px(height);
                node.padding = UiRect::all(Val::Px(10.0));
                node.row_gap = Val::Px(6.0);
                node.overflow = Overflow::scroll_y();
                commands
                    .entity(entity)
                    .insert_if_new(ScrollPosition::default());
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
            "hero" | "objective" | "equipment" => original.clamp(11.0, 13.0),
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
            "result" => 20.0,
            "help" => 15.0,
            "pause" => original.clamp(14.0, 22.0),
            _ => original,
        };
    }
    for (name, mut text) in &mut copy {
        match name.as_str() {
            "RendererStatus" => text.0 = "Swipe to choose your hero".into(),
            "TeamSelectHint" => text.0 = "Choose your class and hero, then join a team. Teams balance automatically.".into(),
            "EquipmentGold" => text.0 = text.0.replace("   /   Equipment", ""),
            "ShopOpenLabel" => text.0 = "SHOP".into(),
            "ShopCloseLabel" => text.0 = "CLOSE".into(),
            "ShopSummary" => text.0 = text.0.replace("click an item", "tap an item"),
            "HelpDismissLabel" => text.0 = "Got it — play".into(),
            "MatchStatusText" => text.0 = text.0
                .replace("clear all towers in one lane to expose the enemy base.", "Clear a lane's towers to unlock the base.")
                .replace("Select a foe  /  P shop  /  F1 help", "Tap ATTACK · Drag to lock")
                .replace("Target locked — basic attack or Q/W/E/R", "Target locked · ATTACK or Q/W/E/R"),
            "ShopFooter" => text.0 = "Buy at your base. Items survive respawn and reset next round.".into(),
            name if name.starts_with("ShopDescription-") => text.0 = text.0.replace("maximum HP", "max HP"),
            "HelpBody" => text.0 = "YOUR FIRST MATCH\n\nMOVE: Drag the left stick. Release to stop.\nATTACK: Tap the large right button; hold to repeat. No mana needed.\nTARGET: Drag ATTACK to extend the reticle. Release on a highlighted foe to lock. Drag to X to cancel.\nSKILLS: Q/W/E/R surround ATTACK. Tap to use the locked target, or drag to aim.\nGROW: Abilities unlock as you level. Tap + to spend skill points.\nWIN: Follow your minions, clear all towers in one lane, then destroy the enemy base.\nRECOVER: Return to your base to shop. If defeated, wait to respawn.\nLOOK: Tap the minimap to scout. Move the stick to follow your hero again.\n\nThe match continues while menus are open. Stay connected for the next round.".into(),
            "GameStateLabel" => text.0 = text.0.replace("Escape: settings or exit game.", "MENU: settings or exit game."),
            _ => {}
        }
    }
}

#[derive(Default)]
struct ScrollTouch {
    id: Option<u64>,
    entity: Option<Entity>,
    previous: Vec2,
}

fn scroll_phone_panels(
    mobile: Res<MobileControls>,
    touches: Res<Touches>,
    mut drag: Local<ScrollTouch>,
    mut panels: Query<(
        Entity,
        &ComputedNode,
        &UiGlobalTransform,
        &mut ScrollPosition,
        Option<&InheritedVisibility>,
    )>,
) {
    if !mobile.enabled {
        return;
    }
    if let Some(id) = drag.id {
        if let Some(touch) = touches.get_pressed(id) {
            if let Some(entity) = drag.entity
                && let Ok((_, node, _, mut scroll, visible)) = panels.get_mut(entity)
            {
                if visible.is_none_or(|visible| visible.get()) {
                    let max = ((node.content_size().y - node.size().y)
                        * node.inverse_scale_factor())
                    .max(0.0);
                    scroll.y = (scroll.y + drag.previous.y - touch.position().y).clamp(0.0, max);
                }
            }
            drag.previous = touch.position();
        } else {
            *drag = ScrollTouch::default();
        }
        return;
    }
    for touch in touches.iter_just_pressed() {
        for (entity, node, transform, _, visible) in &mut panels {
            if visible.is_some_and(|visible| !visible.get()) {
                continue;
            }
            let size = node.size() * node.inverse_scale_factor();
            let rect =
                Rect::from_center_size(transform.translation * node.inverse_scale_factor(), size);
            if rect.contains(touch.position()) {
                *drag = ScrollTouch {
                    id: Some(touch.id()),
                    entity: Some(entity),
                    previous: touch.position(),
                };
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
