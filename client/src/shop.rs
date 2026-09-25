//! Server-authoritative base shop. Browsing is always safe; a receipt confirms payment.
use crate::{
    combat::{ActionFeedback, CombatStats},
    help_overlay::HelpOverlayVisible,
    input_context::InputContextSet,
    net::{
        ClientSession, GameState, GameStateSnapshot, NetworkCommand, NetworkHeroClass,
        PlayerEquipment,
    },
    pause_menu::PauseMenuState,
    player::{MovementTarget, Player},
    ui::{
        Activated, ModalId, ModalRoot, TestId, UiAction, UiActionAppExt,
        theme::{self as ui, ButtonKind},
        widgets::ButtonStyle,
    },
};
use bevy::prelude::*;
use shared::shop::{self, ItemId, PurchaseError};

pub(crate) struct ShopPlugin;
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ShopModalSet;
#[derive(Resource, Default)]
pub(crate) struct ShopState {
    pub open: bool,
    pending: Option<PendingPurchase>,
    next_request: u64,
    round: u64,
    epoch: u64,
    pub feedback: String,
}
impl ShopState {
    #[cfg(any(test, feature = "qa"))]
    pub(crate) fn purchase_pending(&self) -> bool {
        self.pending.is_some()
    }
}

#[derive(Clone)]
struct PendingPurchase {
    item: ItemId,
    request: u64,
    round: u64,
    epoch: u64,
    retry: f32,
}
#[derive(Component)]
struct ShopRoot;
/// Presses on the shop and its HUD shortcuts. Only the item cards are painted
/// by the kit (`ButtonKind::ShopItem`); the HUD buttons and the close button
/// keep their fixed colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShopAction {
    /// The gold HUD button and the equipment panel's OPEN SHOP.
    Toggle,
    Close,
    Buy(ItemId),
    QuickBuy(usize),
}
#[derive(Component)]
struct ShopBuy(ItemId);
#[derive(Component)]
struct ShopCardLabel(ItemId);
#[derive(Component)]
struct ShopSummary;
#[derive(Component)]
struct ShopFeedback;
#[derive(Component)]
struct EquipmentGold;
#[derive(Component)]
struct QuickBuySlot(usize);
#[derive(Component)]
struct QuickBuyPrice(usize);
#[derive(Component)]
struct QuickBuyIcon {
    slot: usize,
    shown: Option<ItemId>,
}
#[derive(Component)]
struct QuickGold;
#[derive(Component)]
enum EquipmentLayoutPart {
    Panel,
    Slot,
}
#[derive(Component)]
struct InventoryLabel(usize);
#[derive(Component)]
struct InventoryIcon {
    index: usize,
    shown: Option<ItemId>,
}

impl Plugin for ShopPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShopState>()
            .add_ui_action::<ShopAction>()
            .add_systems(Startup, (setup_shop, setup_quick_buy))
            .add_systems(Update, adapt_desktop_equipment_width)
            .add_systems(
                Update,
                (toggle_shop, sync_shop_visibility)
                    .chain()
                    .after(crate::help_overlay::HelpOverlaySet::Input)
                    .after(crate::ui::UiSet::Dispatch)
                    .in_set(ShopModalSet)
                    .in_set(InputContextSet::Modal),
            )
            .add_systems(
                Update,
                (
                    reconcile_purchase,
                    purchase_buttons,
                    update_shop,
                    update_inventory_icons,
                    update_quick_buy,
                )
                    .chain()
                    .in_set(InputContextSet::Actions),
            );
    }
}

pub(crate) fn item_code(id: ItemId) -> &'static str {
    match id {
        ItemId::EmberBlade => "EB",
        ItemId::SwiftGrip => "SG",
        ItemId::TrailBoots => "TB",
        ItemId::VitalityGem => "VG",
        ItemId::FocusCharm => "FC",
        ItemId::GuardianCrest => "GC",
    }
}

pub(crate) fn quick_offers(class: shared::HeroClass, inventory: &[ItemId]) -> [Option<ItemId>; 2] {
    let mut offers = shop::recommended_items(class)
        .iter()
        .copied()
        .filter(|id| !inventory.contains(id));
    [offers.next(), offers.next()]
}
fn compact_gold(gold: u32) -> String {
    if gold < 10_000 {
        gold.to_string()
    } else if gold < 1_000_000 {
        format!("{:.0}k", gold as f32 / 1000.0)
    } else {
        format!("{:.1}m", gold as f32 / 1_000_000.0)
    }
}
fn setup_quick_buy(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(16.0),
                top: Val::Px(168.0),
                column_gap: Val::Px(6.0),
                ..default()
            },
            ZIndex(12),
            Name::new("QuickBuyHud"),
        ))
        .with_children(|row| {
            row.spawn((
                Button,
                Node {
                    width: Val::Px(44.0),
                    height: Val::Px(44.0),
                    flex_shrink: 0.0,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(5.0)),
                    ..default()
                },
                BackgroundColor(ui::PANEL),
                BorderColor::all(ui::GOLD),
                UiAction(ShopAction::Toggle),
                TestId::new("GoldShopButton"),
            ))
            .with_children(|button| {
                button.spawn((
                    Node {
                        width: Val::Px(17.0),
                        height: Val::Px(9.0),
                        border: UiRect::all(Val::Px(2.0)),
                        border_radius: BorderRadius::all(Val::Percent(50.0)),
                        ..default()
                    },
                    BorderColor::all(ui::GOLD),
                    BackgroundColor(ui::TILE),
                ));
                button.spawn((
                    Text::new("80"),
                    ui::text(12.0),
                    TextColor(ui::GOLD),
                    QuickGold,
                    Name::new("QuickGoldText"),
                ));
            });
            for slot in 0..2 {
                row.spawn((
                    Button,
                    Node {
                        width: Val::Px(44.0),
                        height: Val::Px(44.0),
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        ..default()
                    },
                    BackgroundColor(ui::PANEL),
                    BorderColor::all(ui::EDGE),
                    QuickBuySlot(slot),
                    UiAction(ShopAction::QuickBuy(slot)),
                    TestId::new(format!("QuickBuy-{slot}")),
                ))
                .with_children(|button| {
                    button.spawn((
                        Node {
                            width: Val::Px(25.0),
                            height: Val::Px(25.0),
                            ..default()
                        },
                        QuickBuyIcon { slot, shown: None },
                    ));
                    button.spawn((
                        Text::new("—"),
                        ui::text(12.0),
                        TextColor(ui::MUTED),
                        QuickBuyPrice(slot),
                        Name::new(format!("QuickBuyPrice-{slot}")),
                    ));
                });
            }
        });
}
fn update_quick_buy(
    mut commands: Commands,
    state: Res<ShopState>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    player: Query<(&PlayerEquipment, &CombatStats, &NetworkHeroClass), With<Player>>,
    mut roots: Query<(&Name, &mut Node)>,
    mut gold: Query<&mut Text, (With<QuickGold>, Without<QuickBuyPrice>)>,
    mut prices: Query<(&QuickBuyPrice, &mut Text, &mut TextColor), Without<QuickGold>>,
    mut icons: Query<(Entity, &mut QuickBuyIcon)>,
    mut buttons: Query<(&QuickBuySlot, &mut BorderColor, &mut BackgroundColor)>,
) {
    for (name, mut node) in &mut roots {
        if name.as_str() == "QuickBuyHud" {
            let (left, top, gap) =
                mobile
                    .as_ref()
                    .filter(|m| m.enabled)
                    .map_or((16.0, 168.0, 6.0), |m| {
                        (
                            m.safe.left,
                            m.safe.top
                                + if m.viewport.y <= 340.0 {
                                    102.0
                                } else {
                                    122.0 * m.scale()
                                },
                            4.0,
                        )
                    });
            node.left = Val::Px(left);
            node.top = Val::Px(top);
            node.column_gap = Val::Px(gap);
        }
    }
    let Ok((equipment, stats, class)) = player.single() else {
        return;
    };
    let offers = quick_offers(class.0, &equipment.inventory);
    for mut text in &mut gold {
        text.0 = compact_gold(equipment.gold);
    }
    for (slot, mut text, mut color) in &mut prices {
        text.0 = offers[slot.0].map_or_else(
            || "—".into(),
            |id| {
                if state.pending.as_ref().is_some_and(|p| p.item == id) {
                    "...".into()
                } else {
                    shop::item(id).cost.to_string()
                }
            },
        );
        *color = TextColor(
            if offers[slot.0].is_some_and(|id| unavailable_reason(equipment, stats, id).is_none()) {
                ui::GOLD
            } else {
                ui::MUTED
            },
        );
    }
    for (entity, mut icon) in &mut icons {
        let next = offers[icon.slot];
        if icon.shown != next {
            icon.shown = next;
            commands.entity(entity).despawn_related::<Children>();
            if let Some(id) = next {
                commands
                    .entity(entity)
                    .with_children(|p| spawn_item_icon(p, id, 25.0));
            }
        }
    }
    for (slot, mut border, mut background) in &mut buttons {
        let available =
            offers[slot.0].is_some_and(|id| unavailable_reason(equipment, stats, id).is_none());
        *border = BorderColor::all(if available { ui::GOLD } else { ui::EDGE });
        *background = BackgroundColor(if available { ui::TILE } else { ui::PANEL });
    }
}

fn setup_shop(mut commands: Commands) {
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(16.0),
                bottom: Val::Px(16.0),
                width: Val::Px(224.0),
                height: Val::Px(136.0),
                padding: UiRect::all(Val::Px(10.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(5.0),
                ..ui::panel_node()
            },
            BackgroundColor(ui::PANEL),
            BorderColor::all(ui::EDGE),
            ZIndex(12),
            EquipmentLayoutPart::Panel,
            Name::new("EquipmentHud"),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("80 gold   /   Equipment"),
                ui::text(13.0),
                TextColor(ui::GOLD),
                EquipmentGold,
                Name::new("EquipmentGold"),
            ));
            panel
                .spawn((
                    Node {
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(5.0),
                        row_gap: Val::Px(5.0),
                        ..default()
                    },
                    Name::new("InventorySlots"),
                ))
                .with_children(|slots| {
                    for index in 0..shop::INVENTORY_CAPACITY {
                        slots
                            .spawn((
                                Node {
                                    width: Val::Px(64.0),
                                    height: Val::Px(27.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    column_gap: Val::Px(3.0),
                                    border: UiRect::all(Val::Px(1.0)),
                                    border_radius: BorderRadius::all(Val::Px(4.0)),
                                    ..default()
                                },
                                BackgroundColor(ui::TILE),
                                BorderColor::all(ui::EDGE),
                                EquipmentLayoutPart::Slot,
                                Name::new(format!("InventorySlot-{index}")),
                            ))
                            .with_children(|slot| {
                                slot.spawn((
                                    Node {
                                        width: Val::Px(22.0),
                                        height: Val::Px(22.0),
                                        flex_shrink: 0.0,
                                        ..default()
                                    },
                                    InventoryIcon { index, shown: None },
                                ));
                                slot.spawn((
                                    Text::new("-"),
                                    ui::text(11.0),
                                    TextColor(ui::MUTED),
                                    InventoryLabel(index),
                                ));
                            });
                    }
                });
            panel
                .spawn((
                    Button,
                    Node {
                        height: Val::Px(29.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        ..default()
                    },
                    BackgroundColor(ui::HOVER),
                    UiAction(ShopAction::Toggle),
                    TestId::new("ShopOpenButton"),
                ))
                .with_children(|button| {
                    button.spawn((
                        Text::new("P   OPEN SHOP"),
                        Name::new("ShopOpenLabel"),
                        ui::text(14.0),
                        TextColor(ui::GOLD),
                    ));
                });
        });
    commands.spawn((Node { position_type: PositionType::Absolute, left: Val::Px(0.0), right: Val::Px(0.0),
        top: Val::Px(0.0), bottom: Val::Px(0.0), display: Display::None,
        align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        BackgroundColor(Color::srgba(0.005, 0.025, 0.025, 0.68)), Visibility::Hidden,
        ZIndex(45), ShopRoot, ModalRoot(ModalId::Shop), Name::new("ShopRoot")))
        .with_children(|overlay| {
            overlay.spawn((Node { width: Val::Px(864.0), max_width: Val::Percent(94.0), flex_direction: FlexDirection::Column,
                row_gap: Val::Px(12.0), padding: UiRect::all(Val::Px(22.0)), ..ui::panel_node() },
                BackgroundColor(ui::PANEL.with_alpha(1.0)), BorderColor::all(ui::EDGE), Name::new("ShopPanel")))
                .with_children(|panel| {
                    panel.spawn((Node { align_items: AlignItems::Center, justify_content: JustifyContent::SpaceBetween, ..default() },))
                        .with_children(|row| {
                            row.spawn((Text::new("Sanctuary shop"), ui::text(26.0), TextColor(ui::IVORY)));
                            row.spawn((Button, Node { padding: UiRect::axes(Val::Px(12.0), Val::Px(7.0)), ..default() },
                                BackgroundColor(ui::TILE), UiAction(ShopAction::Close), TestId::new("ShopCloseButton")))
                                .with_children(|button| { button.spawn((Text::new("ESC  CLOSE"), Name::new("ShopCloseLabel"), ui::text(13.0), TextColor(ui::MUTED))); });
                        });
                    panel.spawn((Text::new(""), ui::text(16.0), TextColor(ui::GOLD), ShopSummary, Name::new("ShopSummary")));
                    panel.spawn((Node { flex_wrap: FlexWrap::Wrap, column_gap: Val::Px(10.0), row_gap: Val::Px(10.0), ..default() }, Name::new("ShopCards")))
                        .with_children(|cards| { for definition in shop::items() {
                            cards.spawn((Button, Node { width: Val::Px(264.0), height: Val::Px(150.0),
                                padding: UiRect::all(Val::Px(13.0)), flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(7.0), border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(7.0)), ..default() },
                                BackgroundColor(ui::TILE), BorderColor::all(ui::EDGE), ShopBuy(definition.id),
                                ButtonStyle::new(ButtonKind::ShopItem), UiAction(ShopAction::Buy(definition.id)),
                                TestId::new(format!("ShopBuy-{}", item_code(definition.id)))))
                                .with_children(|card| {
                                    card.spawn((Node { align_items: AlignItems::Center, column_gap: Val::Px(10.0), ..default() },)).with_children(|row| {
                                        spawn_item_icon(row, definition.id, 36.0);
                                        row.spawn((Text::new(definition.name), ui::text(18.0), TextColor(ui::IVORY)));
                                    });
                                    card.spawn((Text::new(definition.description), ui::text(14.0), TextColor(ui::MUTED),
                                        Name::new(format!("ShopDescription-{}", item_code(definition.id))),
                                        Node { flex_grow: 1.0, ..default() }));
                                    card.spawn((Text::new(""), ui::text(14.0), TextColor(ui::GOLD), ShopCardLabel(definition.id),
                                        Name::new(format!("ShopDetails-{}", item_code(definition.id)))));
                                });
                        }});
                    panel.spawn((Node { column_gap: Val::Px(6.0), flex_wrap: FlexWrap::Wrap, ..default() }, Name::new("ShopInventory")))
                        .with_children(|row| {
                            for index in 0..shop::INVENTORY_CAPACITY {
                                row.spawn((Node { width: Val::Px(74.0), height: Val::Px(28.0), align_items: AlignItems::Center,
                                    column_gap: Val::Px(3.0), ..default() }, BackgroundColor(ui::TILE)))
                                    .with_children(|slot| {
                                        slot.spawn((Node { width: Val::Px(22.0), height: Val::Px(22.0), ..default() }, InventoryIcon { index, shown: None }));
                                        slot.spawn((Text::new("-"), ui::text(12.0), TextColor(ui::MUTED), InventoryLabel(index)));
                                    });
                            }
                        });
                    panel.spawn((Text::new(""), ui::text(15.0), TextColor(ui::JADE), ShopFeedback,
                        Node { min_height: Val::Px(21.0), ..default() }, Name::new("ShopFeedback")));
                    panel.spawn((Text::new("Unique permanent items. Buy at your base; keep them through respawn.\nEarn 1 gold / second during the match, plus combat rewards. Equipment resets each new round."),
                        ui::text(13.0), TextColor(ui::MUTED), Name::new("ShopFooter")));
                });
        });
}

fn adapt_desktop_equipment_width(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut nodes: Query<(&EquipmentLayoutPart, &mut Node)>,
) {
    if mobile.is_some_and(|mobile| mobile.enabled) {
        return;
    }
    let Ok(window) = windows.single() else { return };
    let width = (window.width() - 32.0).clamp(0.0, 224.0);
    // Keep all six slots in two rows as the panel approaches the right edge.
    // The panel has 10px padding + 1px border, and two 5px column gaps.
    let slot_width = ((width - 22.0 - 10.0) / 3.0).clamp(0.0, 64.0);
    for (part, mut node) in &mut nodes {
        node.width = Val::Px(match part {
            EquipmentLayoutPart::Panel => width,
            EquipmentLayoutPart::Slot => slot_width,
        });
    }
}

/// Small original silhouettes built from UI geometry, shared by shop and
/// inventory. No downloaded item artwork or external icon dependency.
fn spawn_item_icon(parent: &mut ChildSpawnerCommands, id: ItemId, size: f32) {
    parent
        .spawn((
            Node {
                width: Val::Px(size),
                height: Val::Px(size),
                flex_shrink: 0.0,
                ..default()
            },
            Name::new(format!("ItemIcon-{}", id.id())),
        ))
        .with_children(|icon| {
            let scale = size / 36.0;
            let mut part =
                |x: f32, y: f32, w: f32, h: f32, angle: f32, color: Color, radius: f32| {
                    icon.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(x * scale),
                            top: Val::Px(y * scale),
                            width: Val::Px(w * scale),
                            height: Val::Px(h * scale),
                            border_radius: BorderRadius::all(Val::Px(radius * scale)),
                            ..default()
                        },
                        BackgroundColor(color),
                        UiTransform::from_rotation(Rot2::degrees(angle)),
                    ));
                };
            match id {
                ItemId::EmberBlade => {
                    part(17.0, 3.0, 6.0, 25.0, 35.0, ui::IVORY, 1.0);
                    part(8.0, 23.0, 16.0, 4.0, 35.0, ui::GOLD, 1.0);
                    part(9.0, 25.0, 5.0, 9.0, 35.0, ui::GOLD, 1.0);
                }
                ItemId::SwiftGrip => {
                    for finger in 0..4 {
                        part(
                            7.0 + finger as f32 * 5.0,
                            5.0 + (finger as f32 - 1.5).abs() * 2.0,
                            4.0,
                            15.0,
                            -8.0,
                            ui::GOLD,
                            2.0,
                        );
                    }
                    part(8.0, 18.0, 20.0, 11.0, -8.0, ui::IVORY, 3.0);
                    part(8.0, 28.0, 17.0, 4.0, -8.0, ui::GOLD, 1.0);
                }
                ItemId::TrailBoots => {
                    part(6.0, 7.0, 9.0, 18.0, 0.0, ui::JADE, 2.0);
                    part(6.0, 22.0, 16.0, 7.0, 0.0, ui::JADE, 3.0);
                    part(20.0, 4.0, 9.0, 18.0, 0.0, ui::IVORY, 2.0);
                    part(20.0, 19.0, 14.0, 7.0, 0.0, ui::IVORY, 3.0);
                    part(6.0, 28.0, 16.0, 3.0, 0.0, ui::GOLD, 1.0);
                }
                ItemId::VitalityGem => {
                    part(7.0, 7.0, 22.0, 22.0, 45.0, ui::JADE, 3.0);
                    part(13.0, 10.0, 8.0, 14.0, 45.0, ui::IVORY, 1.0);
                }
                ItemId::FocusCharm => {
                    part(6.0, 4.0, 24.0, 24.0, 0.0, ui::GOLD, 12.0);
                    part(9.0, 7.0, 18.0, 18.0, 0.0, ui::TILE, 9.0);
                    part(
                        13.0,
                        13.0,
                        10.0,
                        10.0,
                        45.0,
                        Color::srgb(0.38, 0.69, 1.0),
                        1.0,
                    );
                    part(16.0, 26.0, 4.0, 7.0, 0.0, ui::GOLD, 2.0);
                }
                ItemId::GuardianCrest => {
                    part(6.0, 5.0, 24.0, 23.0, 0.0, ui::GOLD, 5.0);
                    part(10.0, 9.0, 16.0, 17.0, 0.0, ui::TILE, 4.0);
                    part(12.0, 21.0, 12.0, 12.0, 45.0, ui::GOLD, 2.0);
                    part(16.0, 10.0, 4.0, 14.0, 0.0, ui::IVORY, 1.0);
                }
            }
        });
}

fn update_inventory_icons(
    mut commands: Commands,
    player: Query<&PlayerEquipment, With<Player>>,
    mut icons: Query<(Entity, &mut InventoryIcon)>,
) {
    let Ok(equipment) = player.single() else {
        return;
    };
    for (entity, mut icon) in &mut icons {
        let next = equipment.inventory.get(icon.index).copied();
        if icon.shown == next {
            continue;
        }
        icon.shown = next;
        commands.entity(entity).despawn_related::<Children>();
        if let Some(id) = next {
            commands
                .entity(entity)
                .with_children(|slot| spawn_item_icon(slot, id, 22.0));
        }
    }
}

fn toggle_shop(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    game: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    help: Res<HelpOverlayVisible>,
    pause: Res<PauseMenuState>,
    mut shop: ResMut<ShopState>,
    mut activated: MessageReader<Activated<ShopAction>>,
    moving: Query<Entity, (With<Player>, With<MovementTarget>)>,
    mut commands: Commands,
    career: Option<Res<crate::career::CareerClient>>,
    social: Option<Res<crate::social::SocialClient>>,
    scoreboard: Option<Res<crate::edge_hud::ScoreboardState>>,
) {
    // Every press is read, so one made while the shop is gated cannot fire later.
    let (mut toggle, mut close) = (false, false);
    for Activated { action, .. } in activated.read() {
        match action {
            ShopAction::Toggle => toggle = true,
            ShopAction::Close => close = true,
            ShopAction::Buy(_) | ShopAction::QuickBuy(_) => {}
        }
    }
    let allowed = session.join_confirmed() && !matches!(game.state, GameState::Victory { .. });
    let visible_help = matches!(game.state, GameState::Running) && help.0;
    if !allowed
        || visible_help
        || pause.open
        || scoreboard.is_some_and(|s| s.open)
        || social
            .as_ref()
            .is_some_and(|social| social.blocks_gameplay())
        || career.as_ref().is_some_and(|career| career.modal_open())
    {
        shop.open = false;
        return;
    }
    if shop.open && keys.just_pressed(KeyCode::Escape) {
        shop.open = false;
        keys.clear_just_pressed(KeyCode::Escape);
    } else if keys.just_pressed(KeyCode::KeyP) || toggle {
        shop.open = !shop.open;
    } else if close {
        shop.open = false;
    }
    if shop.open {
        // Opening the modal also stops a previously issued path; no unnoticed
        // walk out of the sanctuary while selecting equipment.
        for entity in &moving {
            commands.entity(entity).remove::<MovementTarget>();
        }
    }
}

fn sync_shop_visibility(
    shop: Res<ShopState>,
    mut roots: Query<(&mut Node, &mut Visibility), With<ShopRoot>>,
) {
    for (mut node, mut visibility) in &mut roots {
        node.display = if shop.open {
            Display::Flex
        } else {
            Display::None
        };
        *visibility = if shop.open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

fn unavailable_reason(
    equipment: &PlayerEquipment,
    stats: &CombatStats,
    id: ItemId,
) -> Option<String> {
    if !stats.is_alive() {
        Some("Wait for respawn".into())
    } else if equipment.inventory.contains(&id) {
        Some("Owned".into())
    } else if !equipment.shop_available {
        Some("Return to your base".into())
    } else if equipment.inventory.len() >= shop::INVENTORY_CAPACITY {
        Some("Inventory full".into())
    } else if equipment.gold < shop::item(id).cost {
        Some(format!(
            "Need {} more gold",
            shop::item(id).cost - equipment.gold
        ))
    } else {
        None
    }
}

fn purchase_buttons(
    mut state: ResMut<ShopState>,
    game: Res<GameStateSnapshot>,
    player: Query<(&PlayerEquipment, &CombatStats, Option<&NetworkHeroClass>), With<Player>>,
    mut activated: MessageReader<Activated<ShopAction>>,
    context: Option<Res<crate::input_context::GameplayInputContext>>,
    mut feedback: Option<ResMut<ActionFeedback>>,
    mut outgoing: MessageWriter<NetworkCommand>,
) {
    // Every press is read, so one made while a purchase is pending cannot
    // fire when it resolves.
    let presses: Vec<ShopAction> = activated.read().map(|activated| activated.action).collect();
    if state.pending.is_some() {
        return;
    }
    let Ok((equipment, stats, class)) = player.single() else {
        return;
    };
    for action in presses {
        let item = match action {
            ShopAction::Buy(item) if state.open => Some(item),
            ShopAction::QuickBuy(slot)
                if !state.open && context.as_ref().is_some_and(|c| c.gameplay_allowed()) =>
            {
                class.and_then(|class| quick_offers(class.0, &equipment.inventory)[slot])
            }
            _ => None,
        };
        let Some(item) = item else { continue };
        if let Some(reason) = unavailable_reason(equipment, stats, item) {
            if let Some(feedback) = feedback.as_mut() {
                feedback.push_line(reason.clone());
            }
            state.feedback = reason;
            return;
        }
        state.next_request = state
            .next_request
            .max(
                equipment
                    .last_purchase
                    .as_ref()
                    .map_or(0, |receipt| receipt.request_id),
            )
            .saturating_add(1);
        let pending = PendingPurchase {
            item,
            request: state.next_request,
            round: game.meta.match_id,
            epoch: game.meta.server_epoch,
            retry: 0.0,
        };
        send_purchase(&pending, &mut outgoing);
        state.feedback = format!("Purchasing {}...", shop::item(item).name);
        state.pending = Some(pending);
        return;
    }
}

fn send_purchase(pending: &PendingPurchase, outgoing: &mut MessageWriter<NetworkCommand>) {
    outgoing.write(NetworkCommand::BuyItem {
        server_epoch: pending.epoch,
        item_id: pending.item.id().to_owned(),
        request_id: pending.request,
        match_id: pending.round,
    });
}

fn reconcile_purchase(
    time: Res<Time>,
    game: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    mut state: ResMut<ShopState>,
    player: Query<&PlayerEquipment, With<Player>>,
    mut outgoing: MessageWriter<NetworkCommand>,
    mut feedback: ResMut<ActionFeedback>,
) {
    if (state.epoch, state.round) != (game.meta.server_epoch, game.meta.match_id) {
        state.epoch = game.meta.server_epoch;
        state.round = game.meta.match_id;
        state.pending = None;
        state.next_request = 0;
        state.feedback.clear();
    }
    let Some(pending) = state.pending.clone() else {
        return;
    };
    if let Ok(equipment) = player.single() {
        if let Some(receipt) = &equipment.last_purchase {
            if receipt.request_id == pending.request && receipt.match_id == pending.round {
                let message = receipt.error.map_or_else(
                    || {
                        format!(
                            "Purchased {}. Equipment updated.",
                            shop::item(pending.item).name
                        )
                    },
                    purchase_error_text,
                );
                info!(
                    "SHOP_RECEIPT request={} round={} item={:?} error={:?} gold={} inventory={:?}",
                    receipt.request_id,
                    receipt.match_id,
                    pending.item,
                    receipt.error,
                    equipment.gold,
                    equipment.inventory
                );
                state.feedback = message.clone();
                feedback.push_line(message);
                state.pending = None;
                return;
            }
        }
    }
    if let Some(pending) = state.pending.as_mut() {
        if session.join_confirmed() && !matches!(game.state, GameState::Victory { .. }) {
            pending.retry += time.delta_secs();
            if pending.retry >= 0.75 {
                pending.retry = 0.0;
                send_purchase(pending, &mut outgoing);
            }
        }
    }
}

fn purchase_error_text(error: PurchaseError) -> String {
    // Names come from the shared wire enum; no client-side success prediction.
    match error {
        PurchaseError::Dead => "Wait for respawn before purchasing.",
        PurchaseError::OutsideBase => "Return to your own base to purchase.",
        PurchaseError::InsufficientGold => {
            "Not enough gold. Earn more through combat or passive income."
        }
        PurchaseError::AlreadyOwned => "You already own this item.",
        PurchaseError::InventoryFull => "Your inventory is full.",
        PurchaseError::UnknownItem => "This item is unavailable in the server catalog.",
        _ => "Purchasing is unavailable right now. Try again in your base.",
    }
    .into()
}

#[derive(bevy::ecs::system::SystemParam)]
struct ShopLabels<'w, 's> {
    summary: Query<
        'w,
        's,
        &'static mut Text,
        (
            With<ShopSummary>,
            Without<ShopFeedback>,
            Without<ShopCardLabel>,
            Without<InventoryLabel>,
            Without<EquipmentGold>,
        ),
    >,
    feedback: Query<
        'w,
        's,
        &'static mut Text,
        (
            With<ShopFeedback>,
            Without<ShopSummary>,
            Without<ShopCardLabel>,
            Without<InventoryLabel>,
            Without<EquipmentGold>,
        ),
    >,
    cards: Query<
        'w,
        's,
        (&'static ShopCardLabel, &'static mut Text),
        (
            Without<ShopSummary>,
            Without<ShopFeedback>,
            Without<InventoryLabel>,
            Without<EquipmentGold>,
        ),
    >,
    inventory: Query<
        'w,
        's,
        (
            &'static InventoryLabel,
            &'static mut Text,
            &'static mut TextColor,
        ),
        (
            Without<ShopSummary>,
            Without<ShopFeedback>,
            Without<ShopCardLabel>,
            Without<EquipmentGold>,
        ),
    >,
    gold: Query<
        'w,
        's,
        &'static mut Text,
        (
            With<EquipmentGold>,
            Without<ShopSummary>,
            Without<ShopFeedback>,
            Without<ShopCardLabel>,
            Without<InventoryLabel>,
        ),
    >,
}
fn update_shop(
    state: Res<ShopState>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    player: Query<(&PlayerEquipment, &CombatStats, &NetworkHeroClass), With<Player>>,
    mut labels: ShopLabels,
    mut cards: Query<(&ShopBuy, &mut ButtonStyle, &mut BorderColor)>,
) {
    let Ok((equipment, stats, class)) = player.single() else {
        return;
    };
    for mut text in &mut labels.gold {
        text.0 = format!("{} gold   /   Equipment", equipment.gold);
    }
    for (slot, mut text, mut color) in &mut labels.inventory {
        text.0 = equipment
            .inventory
            .get(slot.0)
            .map_or_else(|| "-".to_owned(), |id| short_item_name(*id).to_owned());
        *color = TextColor(if slot.0 < equipment.inventory.len() {
            ui::GOLD
        } else {
            ui::MUTED
        });
    }
    for mut text in &mut labels.summary {
        text.0 = format!(
            "{} gold  |  {} recommendations  |  {}",
            equipment.gold,
            class.0.display_name(),
            if !stats.is_alive() {
                "Wait for respawn"
            } else if equipment.shop_available {
                "In sanctuary: click an item to buy"
            } else {
                "Browse only: return to your base to buy"
            }
        );
    }
    for mut text in &mut labels.feedback {
        text.0.clone_from(&state.feedback);
    }
    for (label, mut text) in &mut labels.cards {
        let reason = unavailable_reason(equipment, stats, label.0);
        let recommended = shop::recommended_items(class.0)[..3].contains(&label.0);
        text.0 = if equipment.inventory.contains(&label.0) {
            "OWNED".into()
        } else if state.pending.as_ref().is_some_and(|p| p.item == label.0) {
            "AWAITING SERVER...".into()
        } else {
            format!(
                "{}g  {}\n{}",
                shop::item(label.0).cost,
                if recommended { "RECOMMENDED" } else { "" },
                reason.unwrap_or_else(|| if mobile.as_ref().is_some_and(|m| m.enabled) {
                    "TAP TO BUY".into()
                } else {
                    "CLICK TO PURCHASE".into()
                })
            )
        };
    }
    for (card, mut style, mut border) in &mut cards {
        let owned = equipment.inventory.contains(&card.0);
        // Owned cards keep `SHOP_OWNED` under the pointer; the kit paints.
        ButtonStyle::set_selected(&mut style, owned);
        *border = BorderColor::all(
            if owned || shop::recommended_items(class.0)[..3].contains(&card.0) {
                ui::GOLD
            } else {
                ui::EDGE
            },
        );
    }
}
fn short_item_name(id: ItemId) -> &'static str {
    match id {
        ItemId::EmberBlade => "Blade",
        ItemId::SwiftGrip => "Grip",
        ItemId::TrailBoots => "Boots",
        ItemId::VitalityGem => "Gem",
        ItemId::FocusCharm => "Charm",
        ItemId::GuardianCrest => "Crest",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equipment_resizes_in_logical_pixels_and_keeps_three_slots_per_row() {
        let mut app = App::new();
        app.add_systems(Startup, (setup_shop, setup_quick_buy))
            .add_systems(Update, adapt_desktop_equipment_width);
        let window = app
            .world_mut()
            .spawn((Window::default(), bevy::window::PrimaryWindow))
            .id();
        for (width, height, scale) in [
            (960, 540, 1.0),
            (1280, 720, 1.0),
            (1600, 1000, 1.0),
            (1920, 1080, 2.0),
        ] {
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .resolution = bevy::window::WindowResolution::new(width, height)
                .with_scale_factor_override(scale);
            app.update();
            let mut parts = app.world_mut().query::<(&EquipmentLayoutPart, &Node)>();
            let mut panel_width = 0.0;
            let mut slot_widths = Vec::new();
            for (part, node) in parts.iter(app.world()) {
                let Val::Px(value) = node.width else {
                    panic!("equipment uses logical pixels")
                };
                match part {
                    EquipmentLayoutPart::Panel => panel_width = value,
                    EquipmentLayoutPart::Slot => slot_widths.push(value),
                }
            }
            assert_eq!(slot_widths.len(), 6);
            assert!(panel_width + 32.0 <= width as f32 / scale + 0.01);
            assert!(slot_widths[0] * 3.0 + 10.0 <= panel_width - 22.0 + 0.01);
            assert!(slot_widths[0] >= 64.0);
            if scale == 1.0 && width >= 1280 {
                assert_eq!(panel_width, 224.0);
                assert_eq!(slot_widths[0], 64.0);
            }
        }
        // Mobile's separate layout must retain its own width after this system runs.
        let mut controls = crate::mobile_controls::MobileControls::default();
        controls.enabled = true;
        app.insert_resource(controls);
        let panel = app
            .world_mut()
            .query::<(Entity, &EquipmentLayoutPart)>()
            .iter(app.world())
            .find(|(_, part)| matches!(part, EquipmentLayoutPart::Panel))
            .unwrap()
            .0;
        app.world_mut().get_mut::<Node>(panel).unwrap().width = Val::Px(132.0);
        app.update();
        assert_eq!(
            app.world().get::<Node>(panel).unwrap().width,
            Val::Px(132.0)
        );
    }

    #[test]
    fn production_shop_bootstraps_distinct_close_label_and_footer() {
        let mut app = App::new();
        app.add_systems(Startup, (setup_shop, setup_quick_buy));
        app.update();
        let mut labels = app.world_mut().query::<(&Name, &Text)>();
        let close: Vec<_> = labels
            .iter(app.world())
            .filter(|(name, _)| name.as_str() == "ShopCloseLabel")
            .map(|(_, text)| text.0.clone())
            .collect();
        let footer: Vec<_> = labels
            .iter(app.world())
            .filter(|(name, _)| name.as_str() == "ShopFooter")
            .map(|(_, text)| text.0.clone())
            .collect();
        assert_eq!(close, ["ESC  CLOSE"]);
        assert_eq!(footer.len(), 1);
        assert!(footer[0].starts_with("Unique permanent items."));
    }
    #[test]
    fn browse_reasons_prioritize_life_ownership_base_and_price() {
        let mut gear = PlayerEquipment {
            gold: 80,
            shop_available: true,
            ..default()
        };
        let mut stats = CombatStats {
            hp: 100.0,
            max_hp: 100.0,
            mana: 100.0,
            max_mana: 100.0,
        };
        assert_eq!(unavailable_reason(&gear, &stats, ItemId::EmberBlade), None);
        gear.gold = 0;
        assert!(
            unavailable_reason(&gear, &stats, ItemId::EmberBlade)
                .unwrap()
                .contains("80")
        );
        gear.shop_available = false;
        assert_eq!(
            unavailable_reason(&gear, &stats, ItemId::EmberBlade).unwrap(),
            "Return to your base"
        );
        gear.inventory.push(ItemId::EmberBlade);
        assert_eq!(
            unavailable_reason(&gear, &stats, ItemId::EmberBlade).unwrap(),
            "Owned"
        );
        stats.hp = 0.0;
        assert_eq!(
            unavailable_reason(&gear, &stats, ItemId::EmberBlade).unwrap(),
            "Wait for respawn"
        );
    }
    fn interaction_app() -> App {
        let mut app = App::new();
        let mut snapshot = GameStateSnapshot {
            state: GameState::Running,
            ..default()
        };
        snapshot.meta.match_id = 1;
        snapshot.meta.server_epoch = 1;
        app.insert_resource(snapshot)
            .insert_resource(ClientSession::admitted_for_test())
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<HelpOverlayVisible>()
            .init_resource::<PauseMenuState>()
            .init_resource::<ShopState>()
            .init_resource::<ActionFeedback>()
            .init_resource::<Time>()
            .add_message::<NetworkCommand>()
            .add_ui_action::<ShopAction>()
            .add_plugins(crate::input_context::InputContextPlugin)
            .configure_sets(
                Update,
                crate::ui::UiSet::Dispatch.in_set(InputContextSet::Modal),
            )
            .add_systems(
                Update,
                toggle_shop
                    .after(crate::ui::UiSet::Dispatch)
                    .in_set(ShopModalSet)
                    .in_set(InputContextSet::Modal),
            )
            .add_systems(
                Update,
                crate::pause_menu::toggle_pause_menu
                    .after(ShopModalSet)
                    .in_set(InputContextSet::Modal),
            )
            .add_systems(
                Update,
                (reconcile_purchase, purchase_buttons)
                    .chain()
                    .in_set(InputContextSet::Actions),
            );
        app
    }

    #[test]
    fn shop_open_stops_path_blocks_actions_and_escape_does_not_open_pause() {
        let mut app = interaction_app();
        let hero = app
            .world_mut()
            .spawn((Player, MovementTarget { target: Vec3::ONE }))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyP);
        app.update();
        assert!(app.world().resource::<ShopState>().open);
        assert!(
            !app.world()
                .get_entity(hero)
                .unwrap()
                .contains::<MovementTarget>()
        );
        assert!(
            !app.world()
                .resource::<crate::input_context::GameplayInputContext>()
                .gameplay_allowed()
        );
        assert!(
            !app.world()
                .resource::<crate::input_context::GameplayInputContext>()
                .camera_allowed()
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        assert!(!app.world().resource::<ShopState>().open);
        assert!(!app.world().resource::<PauseMenuState>().open);
        assert!(
            app.world()
                .resource::<crate::input_context::GameplayInputContext>()
                .gameplay_allowed()
        );
    }

    #[test]
    fn purchase_retries_same_round_and_id_and_waits_for_authoritative_receipt() {
        use bevy::ecs::message::MessageCursor;
        use std::time::Duration;
        let mut app = interaction_app();
        let hero = app
            .world_mut()
            .spawn((
                Player,
                CombatStats {
                    hp: 100.0,
                    max_hp: 100.0,
                    mana: 100.0,
                    max_mana: 100.0,
                },
                PlayerEquipment {
                    gold: 80,
                    shop_available: true,
                    ..default()
                },
            ))
            .id();
        app.update();
        app.world_mut().resource_mut::<ShopState>().open = true;
        app.world_mut().spawn((
            ShopBuy(ItemId::EmberBlade),
            UiAction(ShopAction::Buy(ItemId::EmberBlade)),
            Interaction::Pressed,
        ));
        let mut cursor = MessageCursor::<NetworkCommand>::default();
        app.update();
        let first: Vec<_> = cursor
            .read(app.world().resource::<Messages<NetworkCommand>>())
            .cloned()
            .collect();
        assert!(
            matches!(&first[..], [NetworkCommand::BuyItem { request_id: 1, match_id: 1, item_id, server_epoch: 1 }] if item_id == "ember_blade")
        );
        assert_eq!(app.world().get::<PlayerEquipment>(hero).unwrap().gold, 80);
        assert!(
            app.world()
                .get::<PlayerEquipment>(hero)
                .unwrap()
                .inventory
                .is_empty()
        );
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(800));
        app.update();
        let retry: Vec<_> = cursor
            .read(app.world().resource::<Messages<NetworkCommand>>())
            .cloned()
            .collect();
        assert!(matches!(
            &retry[..],
            [NetworkCommand::BuyItem {
                request_id: 1,
                match_id: 1,
                ..
            }]
        ));
        app.world_mut().entity_mut(hero).insert(PlayerEquipment {
            gold: 0,
            inventory: vec![ItemId::EmberBlade],
            shop_available: true,
            item_bonuses: shop::item_bonuses(&[ItemId::EmberBlade]),
            last_purchase: Some(shop::PurchaseReceipt {
                request_id: 1,
                match_id: 1,
                item_id: Some(ItemId::EmberBlade),
                error: None,
            }),
        });
        app.update();
        assert!(app.world().resource::<ShopState>().pending.is_none());
        assert!(
            app.world()
                .resource::<ShopState>()
                .feedback
                .contains("Purchased Ember Blade")
        );
        assert_eq!(
            cursor
                .read(app.world().resource::<Messages<NetworkCommand>>())
                .count(),
            0
        );
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .meta
            .match_id = 2;
        app.update();
        assert!(app.world().resource::<ShopState>().pending.is_none());
        assert_eq!(app.world().resource::<ShopState>().next_request, 0);
    }
    #[test]
    fn lobby_shop_retries_and_help_overtake_does_not_purchase() {
        use bevy::ecs::message::MessageCursor;
        use std::time::Duration;
        let mut app = interaction_app();
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Lobby;
        app.world_mut().spawn((
            Player,
            CombatStats {
                hp: 100.0,
                max_hp: 100.0,
                mana: 100.0,
                max_mana: 100.0,
            },
            PlayerEquipment {
                gold: 80,
                shop_available: true,
                ..default()
            },
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyP);
        app.update();
        assert!(app.world().resource::<ShopState>().open);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut().spawn((
            ShopBuy(ItemId::TrailBoots),
            UiAction(ShopAction::Buy(ItemId::TrailBoots)),
            Interaction::Pressed,
        ));
        app.update();
        let mut cursor = MessageCursor::<NetworkCommand>::default();
        assert_eq!(
            cursor
                .read(app.world().resource::<Messages<NetworkCommand>>())
                .count(),
            1
        );
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(800));
        app.update();
        assert!(matches!(
            cursor
                .read(app.world().resource::<Messages<NetworkCommand>>())
                .next(),
            Some(NetworkCommand::BuyItem {
                request_id: 1,
                match_id: 1,
                ..
            })
        ));
        app.world_mut().resource_mut::<ShopState>().pending = None;
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        app.world_mut().resource_mut::<HelpOverlayVisible>().0 = true;
        app.world_mut().spawn((
            ShopBuy(ItemId::EmberBlade),
            UiAction(ShopAction::Buy(ItemId::EmberBlade)),
            Interaction::Pressed,
        ));
        app.update();
        assert!(!app.world().resource::<ShopState>().open);
        assert_eq!(
            cursor
                .read(app.world().resource::<Messages<NetworkCommand>>())
                .count(),
            0
        );
    }
    #[test]
    fn reused_round_after_server_restart_cancels_pending_and_hidden_help_does_not_block_lobby_shop()
    {
        let mut app = interaction_app();
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Lobby;
        app.world_mut().resource_mut::<HelpOverlayVisible>().0 = true;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyP);
        app.update();
        assert!(
            app.world().resource::<ShopState>().open,
            "Help is not rendered in lobby"
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut().resource_mut::<ShopState>().pending = Some(PendingPurchase {
            item: ItemId::EmberBlade,
            request: 9,
            round: 1,
            epoch: 1,
            retry: 0.0,
        });
        app.world_mut().resource_mut::<ShopState>().next_request = 9;
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .meta
            .server_epoch = 2;
        app.update();
        assert!(app.world().resource::<ShopState>().pending.is_none());
        assert_eq!(app.world().resource::<ShopState>().next_request, 0);
        assert_eq!(app.world().resource::<ShopState>().epoch, 2);
    }
    #[test]
    fn quick_buy_uses_authoritative_receipt_before_advancing_and_respects_modals() {
        use bevy::ecs::message::MessageCursor;
        let mut app = interaction_app();
        let hero = app
            .world_mut()
            .spawn((
                Player,
                NetworkHeroClass(shared::HeroClass::Warrior),
                CombatStats {
                    hp: 100.0,
                    max_hp: 100.0,
                    mana: 100.0,
                    max_mana: 100.0,
                },
                PlayerEquipment {
                    gold: 80,
                    shop_available: true,
                    ..default()
                },
            ))
            .id();
        app.update();
        let button = app
            .world_mut()
            .spawn((
                QuickBuySlot(0),
                UiAction(ShopAction::QuickBuy(0)),
                Interaction::Pressed,
            ))
            .id();
        let mut cursor = MessageCursor::<NetworkCommand>::default();
        app.update();
        let sent: Vec<_> = cursor
            .read(app.world().resource::<Messages<NetworkCommand>>())
            .cloned()
            .collect();
        assert!(
            matches!(&sent[..],[NetworkCommand::BuyItem {item_id,..}] if item_id=="vitality_gem")
        );
        let before = app.world().get::<PlayerEquipment>(hero).unwrap();
        assert_eq!(before.gold, 80);
        assert_eq!(
            quick_offers(shared::HeroClass::Warrior, &before.inventory)[0],
            Some(ItemId::VitalityGem)
        );
        assert!(!app.world().resource::<ShopState>().open);
        app.world_mut()
            .get_mut::<Interaction>(button)
            .unwrap()
            .clone_from(&Interaction::None);
        let request = app
            .world()
            .resource::<ShopState>()
            .pending
            .as_ref()
            .unwrap()
            .request;
        app.world_mut().entity_mut(hero).insert(PlayerEquipment {
            gold: 0,
            inventory: vec![ItemId::VitalityGem],
            shop_available: true,
            last_purchase: Some(shop::PurchaseReceipt {
                request_id: request,
                match_id: 1,
                item_id: Some(ItemId::VitalityGem),
                error: None,
            }),
            ..default()
        });
        app.update();
        assert!(!app.world().resource::<ShopState>().purchase_pending());
        let after = app.world().get::<PlayerEquipment>(hero).unwrap();
        assert!(
            quick_offers(shared::HeroClass::Warrior, &after.inventory)
                .iter()
                .all(|item| *item != Some(ItemId::VitalityGem))
        );
        app.world_mut()
            .get_mut::<PlayerEquipment>(hero)
            .unwrap()
            .gold = 999;
        app.insert_resource(crate::edge_hud::ScoreboardState::default());
        app.world_mut()
            .resource_mut::<crate::edge_hud::ScoreboardState>()
            .open = true;
        app.world_mut()
            .get_mut::<Interaction>(button)
            .unwrap()
            .clone_from(&Interaction::Pressed);
        app.update();
        assert_eq!(
            cursor
                .read(app.world().resource::<Messages<NetworkCommand>>())
                .count(),
            0
        );
        assert!(!app.world().resource::<ShopState>().purchase_pending());
    }

    #[test]
    fn quick_offers_have_two_stable_slots_and_do_not_offer_owned_items() {
        let mut owned = Vec::new();
        for _ in 0..shop::INVENTORY_CAPACITY {
            let offers = quick_offers(shared::HeroClass::Mage, &owned);
            let next = offers[0].unwrap();
            assert!(!owned.contains(&next));
            assert!(offers[1].is_none_or(|id| id != next && !owned.contains(&id)));
            owned.push(next);
        }
        assert_eq!(quick_offers(shared::HeroClass::Mage, &owned), [None, None]);
        assert_eq!(compact_gold(80), "80");
        assert_eq!(compact_gold(10_000), "10k");
    }
}
