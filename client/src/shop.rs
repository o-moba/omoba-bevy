//! Server-authoritative base shop. Browsing is always safe; a receipt confirms payment.
// i18n-strict
use crate::{
    combat::{ActionFeedback, CombatStats},
    help_overlay::HelpOverlayVisible,
    hud_layout::HudRegion,
    i18n::{Locale, Localized, data, tr, trf},
    input_context::InputContextSet,
    net::{
        ClientSession, GameState, GameStateSnapshot, NetworkCommand, NetworkHeroClass,
        PlayerEquipment,
    },
    pause_menu::PauseMenuState,
    player::{MovementTarget, Player},
    ui::{
        Activated, ModalId, ModalRoot, TestId, UiAction, UiActionAppExt,
        kit_assets::{Icon, KitImage},
        theme::{self as ui, ButtonKind, Form},
        tokens::{TextRole, border, color, radius, size, space},
        widgets::{
            ButtonStyle, KitParts, KitSkin, game, icon_node,
            surfaces::{Tooltip, TooltipText},
        },
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
/// An inventory slot of the equipment plate and its glyph.
#[derive(Component)]
struct EquipmentSlot(usize);
#[derive(Component)]
struct EquipmentSlotIcon(usize);
#[derive(Component)]
struct InventoryLabel(usize);
/// Shop text that depends on the language and on the UI platform (phone copy
/// is shorter), rewritten by [`relabel_shop_text`].
#[derive(Component, Clone, Copy)]
enum ShopText {
    ItemName(ItemId),
    ItemDescription(ItemId),
    CloseLabel,
    Footer,
}
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
            .add_systems(Update, relabel_shop_text)
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
                    update_equipment_slots,
                )
                    .chain()
                    .after(crate::combat::CombatPointerInputSet)
                    .before(crate::combat::WorldMovementInputSet)
                    .in_set(InputContextSet::Actions),
            );
    }
}

pub(crate) fn item_code(id: ItemId) -> &'static str {
    match id {
        ItemId::EmberBlade => "EB",      // i18n-allow
        ItemId::SwiftGrip => "SG",       // i18n-allow
        ItemId::TrailBoots => "TB",      // i18n-allow
        ItemId::VitalityGem => "VG",     // i18n-allow
        ItemId::FocusCharm => "FC",      // i18n-allow
        ItemId::GuardianCrest => "GC",   // i18n-allow
        ItemId::CritShard => "CS",       // i18n-allow
        ItemId::SiphonStone => "SS",     // i18n-allow
        ItemId::WindrunnerBoots => "WB", // i18n-allow
        ItemId::DuelistEdge => "DE",     // i18n-allow
        ItemId::VampiricFang => "VF",    // i18n-allow
        ItemId::ArcaneFocus => "AF",     // i18n-allow
        ItemId::Bulwark => "BW",         // i18n-allow
        ItemId::TempestBlade => "TB3",   // i18n-allow
        ItemId::Bloodreaver => "BR",     // i18n-allow
        ItemId::AetherCrown => "AC",     // i18n-allow
    }
}

pub(crate) fn quick_offers(class: shared::HeroClass, inventory: &[ItemId]) -> [Option<ItemId>; 2] {
    // Walk the next build from its missing leaves, so early gold buys a useful
    // component instead of showing only a finished item hundreds of gold away.
    fn next_part(id: ItemId, inventory: &[ItemId]) -> Option<ItemId> {
        if shop::inventory_covers(id, inventory) {
            return None;
        }
        for component in shop::item(id).components {
            if let Some(part) = next_part(*component, inventory) {
                return Some(part);
            }
        }
        Some(id)
    }
    let mut result = [None, None];
    let mut filled = 0;
    for id in shop::recommended_items(class) {
        let Some(mut next) = next_part(*id, inventory) else {
            continue;
        };
        if shop::purchase_quote(next, u32::MAX, inventory).is_err()
            && shop::purchase_quote(*id, u32::MAX, inventory).is_ok()
        {
            next = *id;
        }
        if result.contains(&Some(next)) || shop::purchase_quote(next, u32::MAX, inventory).is_err()
        {
            continue;
        }
        result[filled] = Some(next);
        filled += 1;
        if filled == result.len() {
            break;
        }
    }
    result
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
/// Quick-buy and inventory slot anatomy (`hud.md`: slots `size.item_slot.*`,
/// equipment 3 × 2 with gap 6, price `type.number_sm` bottom-right).
const SLOT_GAP: f32 = 6.0;
const EQUIPMENT_PADDING: f32 = space::S8;
/// Gold row (desktop status plate): 104 × 24 with a keycap.
const GOLD_ROW: Vec2 = Vec2::new(104.0, 24.0);

/// The desktop gold row of the player status plate (`hud.md`
/// `player-status`): a plate button (`ShopAction::Toggle`) with `hud/gold`,
/// the gold (`type.number` gold, compact from 10 000) and the `P` keycap.
pub(crate) fn spawn_gold_row(parent: &mut ChildSpawnerCommands) {
    let mut parts = KitParts::default();
    let mut row = parent.spawn(crate::ui::widgets::plate_button(
        Node {
            width: Val::Px(GOLD_ROW.x),
            height: Val::Px(GOLD_ROW.y),
            padding: UiRect::horizontal(Val::Px(space::S4 + border::FRAME)),
            column_gap: Val::Px(space::S4),
            align_items: AlignItems::Center,
            ..default()
        },
        ShopAction::Toggle,
        "GoldShopButton",
        KitParts::default(),
    ));
    row.insert((
        Tooltip {
            title: None,
            body: "shop.button.open",
        },
        ZIndex(1),
    ));
    row.with_children(|row| {
        parts.icon = Some(
            row.spawn(icon_node(Icon::HudGold, size::ICON_SM, color::TEXT_GOLD))
                .id(),
        );
        row.spawn((
            Text::new("0"),
            ui::role_text(TextRole::Number),
            TextColor(color::TEXT_GOLD),
            Node {
                flex_grow: 1.0,
                ..default()
            },
            QuickGold,
            Name::new("QuickGoldText"),
        ));
        crate::ui::widgets::surfaces::keycap(row, crate::input_bindings::shop_key_display());
    });
    row.insert(parts);
}

/// An item slot button (`item-slot.md` look: `color.surface.0`, subtle
/// border, radius 6) whose icon and price the owner paints: quick-buy
/// offers keep their availability colours, which the kit would repaint.
fn slot_button(parent: &mut ChildSpawnerCommands, form: Form, slot: usize) -> Entity {
    let side = size::ITEM_SLOT.at(form);
    let mut button = parent.spawn((
        crate::ui::widgets::button_bundle(
            Node {
                width: Val::Px(side),
                height: Val::Px(side),
                flex_shrink: 0.0,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::all(Val::Px(game::ITEM_SLOT_RADIUS)),
                ..default()
            },
            ButtonKind::ShopItem,
            ShopAction::QuickBuy(slot),
            TestId::new(format!("QuickBuy-{slot}")),
        ),
        KitSkin::ShopCard,
        KitParts::default(),
        UiTransform::IDENTITY,
        QuickBuySlot(slot),
        Tooltip {
            title: None,
            body: "shop.button.open",
        },
        TooltipText::default(),
    ));
    button.with_children(|slot_node| {
        slot_node.spawn((
            icon_node(
                Icon::NavShoppingBag,
                game::ITEM_ICON.min(side - space::S8),
                color::GOLD_400,
            ),
            Visibility::Hidden,
            QuickBuyIcon { slot, shown: None },
        ));
        slot_node.spawn((
            Text::new("—"),
            ui::role_text(TextRole::NumberSm),
            TextColor(color::TEXT_MUTED),
            TextShadow::default(),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(space::S2),
                bottom: Val::Px(0.0),
                ..default()
            },
            QuickBuyPrice(slot),
            Name::new(format!("QuickBuyPrice-{slot}")),
        ));
    });
    button.id()
}

/// Phone: the quick-buy row under the status plate (`hud.md` phone
/// `quick-buy`): gold button + two offers, 44 each, gap `space.8`. Desktop
/// has the gold row in the status plate and the offers in the equipment
/// plate, so nothing is spawned here.
fn setup_quick_buy(
    mut commands: Commands,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
) {
    if !phone_copy(mobile.as_deref()) {
        return;
    }
    let form = Form::Phone;
    let side = size::ITEM_SLOT.at(form);
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                column_gap: Val::Px(space::S8),
                ..default()
            },
            HudRegion::QuickBuy,
            ZIndex(12),
            Name::new("QuickBuyHud"),
        ))
        .with_children(|row| {
            let mut parts = KitParts::default();
            let mut gold = row.spawn((
                crate::ui::widgets::button_bundle(
                    Node {
                        width: Val::Px(side),
                        height: Val::Px(side),
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(border::HAIRLINE)),
                        border_radius: BorderRadius::all(Val::Px(game::ITEM_SLOT_RADIUS)),
                        ..default()
                    },
                    ButtonKind::ShopItem,
                    ShopAction::Toggle,
                    TestId::new("GoldShopButton"),
                ),
                KitSkin::ShopCard,
                UiTransform::IDENTITY,
            ));
            gold.with_children(|button| {
                parts.icon = Some(
                    button
                        .spawn(icon_node(Icon::HudGold, size::ICON_MD, color::GOLD_400))
                        .id(),
                );
                button.spawn((
                    Text::new("0"),
                    ui::role_text(TextRole::NumberSm),
                    TextColor(color::TEXT_GOLD),
                    QuickGold,
                    Name::new("QuickGoldText"),
                ));
            });
            gold.insert(parts);
            for slot in 0..2 {
                slot_button(row, form, slot);
            }
        });
}

/// Desktop equipment plate (`hud.md` `equipment`, 200 × 102): six inventory
/// slots 3 × 2 (gap 6), a divider, and the two quick-buy offers.
fn spawn_equipment_plate(commands: &mut Commands) {
    let form = Form::Desktop;
    let side = size::ITEM_SLOT.at(form);
    commands
        .spawn((
            // A Button keeps world clicks off the plate.
            Button,
            game::hud_plate(false),
            HudRegion::Equipment,
            ZIndex(12),
            Name::new("EquipmentHud"),
        ))
        .insert(Node {
            position_type: PositionType::Absolute,
            padding: UiRect::all(Val::Px(EQUIPMENT_PADDING)),
            column_gap: Val::Px(space::S4 + border::HAIRLINE),
            align_items: AlignItems::Stretch,
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            ..default()
        })
        .with_children(|plate| {
            plate
                .spawn((
                    Node {
                        width: Val::Px(3.0 * side + 2.0 * SLOT_GAP),
                        flex_shrink: 0.0,
                        align_content: AlignContent::FlexStart,
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(SLOT_GAP),
                        row_gap: Val::Px(SLOT_GAP),
                        ..default()
                    },
                    Name::new("InventorySlots"),
                ))
                .with_children(|slots| {
                    for index in 0..shop::INVENTORY_CAPACITY {
                        let slot = game::item_slot(slots, None, None, form);
                        slots.commands().entity(slot).insert((
                            // Hover shows the item tooltip.
                            Button,
                            EquipmentSlot(index),
                            Name::new(format!("InventorySlot-{index}")),
                        ));
                        slots.commands().entity(slot).with_children(|slot| {
                            slot.spawn((
                                icon_node(
                                    Icon::NavShoppingBag,
                                    game::ITEM_ICON.min(side - space::S4),
                                    color::GOLD_400,
                                ),
                                Visibility::Hidden,
                                EquipmentSlotIcon(index),
                            ));
                        });
                    }
                });
            plate.spawn((
                Node {
                    width: Val::Px(border::HAIRLINE),
                    flex_shrink: 0.0,
                    ..default()
                },
                BackgroundColor(color::BORDER_SUBTLE),
                Pickable::IGNORE,
            ));
            plate
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    flex_shrink: 0.0,
                    row_gap: Val::Px(SLOT_GAP),
                    ..default()
                })
                .with_children(|column| {
                    for slot in 0..2 {
                        slot_button(column, form, slot);
                    }
                });
        });
}

/// Gold, the two quick-buy offers (icon, price, availability, tooltip).
#[allow(clippy::type_complexity)]
fn update_quick_buy(
    state: Res<ShopState>,
    locale: Option<Res<Locale>>,
    player: Query<(&PlayerEquipment, &CombatStats, &NetworkHeroClass), With<Player>>,
    mut gold: Query<&mut Text, (With<QuickGold>, Without<QuickBuyPrice>)>,
    mut prices: Query<(&QuickBuyPrice, &mut Text, &mut TextColor), Without<QuickGold>>,
    mut icons: Query<(&mut QuickBuyIcon, &mut KitImage, &mut Visibility)>,
    mut slots: Query<(&QuickBuySlot, &mut TooltipText)>,
    mut last: Local<Option<(Vec<ItemId>, u32, bool, u32)>>,
) {
    let Ok((equipment, stats, class)) = player.single() else {
        return;
    };
    let offers = quick_offers(class.0, &equipment.inventory);
    for mut text in &mut gold {
        let next = compact_gold(equipment.gold);
        if text.0 != next {
            text.0 = next;
        }
    }
    let available = |slot: usize| {
        offers[slot].is_some_and(|id| unavailable_reason(equipment, stats, id).is_none())
    };
    for (slot, mut text, mut color) in &mut prices {
        let next = offers[slot.0].map_or_else(
            || "—".into(),
            |id| {
                if state.pending.as_ref().is_some_and(|p| p.item == id) {
                    "...".into()
                } else {
                    shop::upgrade_quote(id, &equipment.inventory)
                        .cost
                        .to_string()
                }
            },
        );
        if text.0 != next {
            text.0 = next;
        }
        let ink = if available(slot.0) {
            color::TEXT_GOLD
        } else {
            color::TEXT_MUTED
        };
        if color.0 != ink {
            color.0 = ink;
        }
    }
    for (mut icon, mut image, mut visibility) in &mut icons {
        let next = offers[icon.slot];
        if icon.shown != next {
            icon.shown = next;
            *visibility = if next.is_some() {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
        // Unavailable offers dim their glyph (Bevy nodes have no opacity).
        let tint = if available(icon.slot) {
            color::GOLD_400
        } else {
            color::TEXT_DISABLED
        };
        let source = KitImage::icon(next.map_or(Icon::NavShoppingBag, game::item_icon), tint);
        if *image != source {
            *image = source;
        }
    }
    // Tooltips: item name + description (+ the unavailable reason).
    let key = (
        equipment.inventory.clone(),
        equipment.gold,
        stats.is_alive() && equipment.shop_available,
        locale.as_ref().map_or(0, |locale| locale.generation()),
    );
    if last.as_ref() == Some(&key) {
        return;
    }
    *last = Some(key);
    for (slot, mut tooltip) in &mut slots {
        let next = offers[slot.0].map_or_else(TooltipText::default, |id| TooltipText {
            title: Some(data::item_name(id).to_owned()),
            body: match unavailable_reason(equipment, stats, id) {
                Some(reason) => format!("{}\n{reason}", item_tooltip(id)),
                None => item_tooltip(id),
            },
        });
        if *tooltip != next {
            *tooltip = next;
        }
    }
}

/// The equipment plate's inventory glyphs and item tooltips.
fn update_equipment_slots(
    mut commands: Commands,
    player: Query<&PlayerEquipment, With<Player>>,
    locale: Option<Res<Locale>>,
    slots: Query<(Entity, &EquipmentSlot)>,
    mut icons: Query<(&EquipmentSlotIcon, &mut KitImage, &mut Visibility)>,
    mut last: Local<Option<(Vec<ItemId>, u32)>>,
) {
    let Ok(equipment) = player.single() else {
        return;
    };
    let key = (
        equipment.inventory.clone(),
        locale.as_ref().map_or(0, |locale| locale.generation()),
    );
    if last.as_ref() == Some(&key) {
        return;
    }
    *last = Some(key);
    for (icon, mut image, mut visibility) in &mut icons {
        let item = equipment.inventory.get(icon.0).copied();
        *visibility = if item.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let Some(item) = item {
            *image = KitImage::icon(game::item_icon(item), color::GOLD_400);
        }
    }
    for (entity, slot) in &slots {
        match equipment.inventory.get(slot.0) {
            Some(item) => {
                commands.entity(entity).insert((
                    Tooltip {
                        title: None,
                        body: "shop.button.open",
                    },
                    TooltipText {
                        title: Some(data::item_name(*item).to_owned()),
                        body: data::item_desc(*item).to_owned(),
                    },
                ));
            }
            None => {
                commands.entity(entity).remove::<(Tooltip, TooltipText)>();
            }
        }
    }
}

/// A phone shows the short shop copy (tap wording, shorter footer).
fn phone_copy(mobile: Option<&crate::mobile_controls::MobileControls>) -> bool {
    mobile.is_some_and(|mobile| mobile.enabled)
}

impl ShopText {
    fn text(self, phone: bool) -> &'static str {
        match self {
            Self::ItemName(id) => data::item_name(id),
            Self::ItemDescription(id) => item_description(id, phone),
            Self::CloseLabel if phone => tr("shop.button.close_phone"),
            Self::CloseLabel => tr("shop.button.close"),
            Self::Footer if phone => tr("shop.footer_phone"),
            Self::Footer => tr("shop.footer"),
        }
    }
}

/// An item's card description; phones use the short form where one exists.
fn item_description(id: ItemId, phone: bool) -> &'static str {
    match (phone, id) {
        (true, ItemId::VitalityGem) => tr("shop.item_phone.vitality_gem"),
        (true, ItemId::GuardianCrest) => tr("shop.item_phone.guardian_crest"),
        _ => data::item_desc(id),
    }
}

fn item_tooltip(id: ItemId) -> String {
    let definition = shop::item(id);
    let mut body = data::item_desc(id).to_owned();
    if !definition.components.is_empty() {
        let recipe = definition
            .components
            .iter()
            .map(|id| data::item_name(*id))
            .collect::<Vec<_>>()
            .join(" + ");
        body.push('\n');
        body.push_str(&trf(
            "shop.recipe",
            &[("items", &recipe), ("total", &definition.cost)],
        ));
    }
    body
}

/// Rewrites the item cards, close label and footer when the language or the
/// UI platform changes.
fn relabel_shop_text(
    locale: Option<Res<Locale>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut last: Local<Option<(u32, bool)>>,
    mut labels: Query<(&ShopText, &mut Text)>,
    mut cards: Query<(&ShopBuy, &mut TooltipText)>,
) {
    let phone = phone_copy(mobile.as_deref());
    let key = (
        locale.as_ref().map_or(0, |locale| locale.generation()),
        phone,
    );
    if *last == Some(key) {
        return;
    }
    *last = Some(key);
    for (label, mut text) in &mut labels {
        let next = label.text(phone);
        if text.0 != next {
            text.0 = next.to_owned();
        }
    }
    for (buy, mut tooltip) in &mut cards {
        tooltip.title = Some(data::item_name(buy.0).to_owned());
        tooltip.body = item_tooltip(buy.0);
    }
}

fn setup_shop(mut commands: Commands, mobile: Option<Res<crate::mobile_controls::MobileControls>>) {
    let phone = phone_copy(mobile.as_deref());
    if !phone {
        spawn_equipment_plate(&mut commands);
    }
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
            BackgroundColor(Color::srgba(0.005, 0.025, 0.025, 0.68)),
            Visibility::Hidden,
            ZIndex(45),
            ShopRoot,
            ModalRoot(ModalId::Shop),
            Name::new("ShopRoot"),
        ))
        .with_children(|overlay| {
            overlay
                .spawn((
                    Node {
                        width: Val::Px(864.0),
                        max_width: Val::Percent(94.0),
                        max_height: Val::Percent(94.0),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(12.0),
                        padding: UiRect::all(Val::Px(22.0)),
                        ..ui::panel_node()
                    },
                    BackgroundColor(ui::PANEL.with_alpha(1.0)),
                    BorderColor::all(ui::EDGE),
                    Name::new("ShopPanel"),
                ))
                .with_children(|panel| {
                    panel
                        .spawn((Node {
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::SpaceBetween,
                            ..default()
                        },))
                        .with_children(|row| {
                            row.spawn((
                                Localized::new("shop.title").into_text(),
                                ui::text(26.0),
                                TextColor(ui::IVORY),
                            ));
                            row.spawn((
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(12.0), Val::Px(7.0)),
                                    ..default()
                                },
                                BackgroundColor(ui::TILE),
                                UiAction(ShopAction::Close),
                                TestId::new("ShopCloseButton"),
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new(ShopText::CloseLabel.text(phone)),
                                    ShopText::CloseLabel,
                                    Name::new("ShopCloseLabel"),
                                    ui::text(13.0),
                                    TextColor(ui::MUTED),
                                ));
                            });
                        });
                    panel.spawn((
                        Text::new(""),
                        ui::text(16.0),
                        TextColor(ui::GOLD),
                        ShopSummary,
                        Name::new("ShopSummary"),
                    ));
                    panel
                        .spawn((
                            Node {
                                flex_wrap: FlexWrap::Wrap,
                                column_gap: Val::Px(10.0),
                                row_gap: Val::Px(10.0),
                                max_height: Val::Px(480.0),
                                min_height: Val::Px(0.0),
                                overflow: Overflow::scroll_y(),
                                ..default()
                            },
                            Name::new("ShopCards"),
                            crate::ui::ScrollArea::menu(28.0),
                        ))
                        .with_children(|cards| {
                            for definition in shop::items() {
                                cards
                                    .spawn((
                                        Button,
                                        Node {
                                            width: Val::Px(264.0),
                                            height: Val::Px(150.0),
                                            min_height: Val::Px(if definition.tier > 1 {
                                                132.0
                                            } else {
                                                0.0
                                            }),
                                            flex_shrink: 0.0,
                                            padding: UiRect::all(Val::Px(13.0)),
                                            flex_direction: FlexDirection::Column,
                                            row_gap: Val::Px(7.0),
                                            border: UiRect::all(Val::Px(1.0)),
                                            border_radius: BorderRadius::all(Val::Px(7.0)),
                                            ..default()
                                        },
                                        BackgroundColor(ui::TILE),
                                        BorderColor::all(ui::EDGE),
                                        ShopBuy(definition.id),
                                        Tooltip {
                                            title: None,
                                            body: "shop.title",
                                        },
                                        TooltipText {
                                            title: Some(data::item_name(definition.id).into()),
                                            body: item_tooltip(definition.id),
                                        },
                                        ButtonStyle::new(ButtonKind::ShopItem),
                                        UiAction(ShopAction::Buy(definition.id)),
                                        TestId::new(format!(
                                            "ShopBuy-{}",
                                            item_code(definition.id)
                                        )),
                                    ))
                                    .with_children(|card| {
                                        card.spawn((Node {
                                            align_items: AlignItems::Center,
                                            column_gap: Val::Px(10.0),
                                            ..default()
                                        },))
                                            .with_children(|row| {
                                                spawn_item_icon(row, definition.id, 36.0);
                                                row.spawn((
                                                    Text::new(
                                                        ShopText::ItemName(definition.id)
                                                            .text(phone),
                                                    ),
                                                    ShopText::ItemName(definition.id),
                                                    ui::text(18.0),
                                                    TextColor(ui::IVORY),
                                                ));
                                            });
                                        card.spawn((
                                            Text::new(
                                                ShopText::ItemDescription(definition.id)
                                                    .text(phone),
                                            ),
                                            ShopText::ItemDescription(definition.id),
                                            ui::text(14.0),
                                            TextColor(ui::MUTED),
                                            Name::new(format!(
                                                "ShopDescription-{}",
                                                item_code(definition.id)
                                            )),
                                            Node {
                                                flex_grow: 1.0,
                                                ..default()
                                            },
                                        ));
                                        card.spawn((
                                            Text::new(""),
                                            ui::text(14.0),
                                            TextColor(ui::GOLD),
                                            ShopCardLabel(definition.id),
                                            Name::new(format!(
                                                "ShopDetails-{}",
                                                item_code(definition.id)
                                            )),
                                        ));
                                    });
                            }
                        });
                    panel
                        .spawn((
                            Node {
                                column_gap: Val::Px(6.0),
                                flex_wrap: FlexWrap::Wrap,
                                ..default()
                            },
                            Name::new("ShopInventory"),
                        ))
                        .with_children(|row| {
                            for index in 0..shop::INVENTORY_CAPACITY {
                                row.spawn((
                                    Node {
                                        width: Val::Px(74.0),
                                        height: Val::Px(28.0),
                                        align_items: AlignItems::Center,
                                        column_gap: Val::Px(3.0),
                                        ..default()
                                    },
                                    BackgroundColor(ui::TILE),
                                ))
                                .with_children(|slot| {
                                    slot.spawn((
                                        Node {
                                            width: Val::Px(22.0),
                                            height: Val::Px(22.0),
                                            ..default()
                                        },
                                        InventoryIcon { index, shown: None },
                                    ));
                                    slot.spawn((
                                        Text::new("-"),
                                        ui::text(12.0),
                                        TextColor(ui::MUTED),
                                        InventoryLabel(index),
                                    ));
                                });
                            }
                        });
                    panel.spawn((
                        Text::new(""),
                        ui::text(15.0),
                        TextColor(ui::JADE),
                        ShopFeedback,
                        Node {
                            min_height: Val::Px(21.0),
                            ..default()
                        },
                        Name::new("ShopFeedback"),
                    ));
                    panel.spawn((
                        Text::new(ShopText::Footer.text(phone)),
                        ShopText::Footer,
                        ui::text(13.0),
                        TextColor(ui::MUTED),
                        Name::new("ShopFooter"),
                    ));
                });
        });
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
                ItemId::EmberBlade
                | ItemId::DuelistEdge
                | ItemId::VampiricFang
                | ItemId::TempestBlade
                | ItemId::Bloodreaver => {
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
                ItemId::TrailBoots | ItemId::WindrunnerBoots => {
                    part(6.0, 7.0, 9.0, 18.0, 0.0, ui::JADE, 2.0);
                    part(6.0, 22.0, 16.0, 7.0, 0.0, ui::JADE, 3.0);
                    part(20.0, 4.0, 9.0, 18.0, 0.0, ui::IVORY, 2.0);
                    part(20.0, 19.0, 14.0, 7.0, 0.0, ui::IVORY, 3.0);
                    part(6.0, 28.0, 16.0, 3.0, 0.0, ui::GOLD, 1.0);
                }
                ItemId::VitalityGem | ItemId::CritShard | ItemId::SiphonStone => {
                    part(7.0, 7.0, 22.0, 22.0, 45.0, ui::JADE, 3.0);
                    part(13.0, 10.0, 8.0, 14.0, 45.0, ui::IVORY, 1.0);
                }
                ItemId::FocusCharm | ItemId::ArcaneFocus | ItemId::AetherCrown => {
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
                ItemId::GuardianCrest | ItemId::Bulwark => {
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
    mut back: crate::ui::BackInput,
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
    gamepad: Option<Res<crate::gamepad::GamepadControls>>,
) {
    // Every press is read, so one made while the shop is gated cannot fire later.
    let (mut toggle, mut close) = (false, false);
    // A controller's D-pad right is the shop button (it only reaches here in
    // play; inside the shop the D-pad navigates and East closes it).
    toggle |= gamepad
        .as_ref()
        .is_some_and(|pad| pad.active && pad.shop_pressed);
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
    if shop.open && back.just_pressed() {
        shop.open = false;
        back.consume();
    } else if back.keys().just_pressed(KeyCode::KeyP) || toggle {
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
        Some(tr("shop.reason.respawn").into())
    } else if equipment.inventory.contains(&id) {
        Some(tr("shop.reason.owned").into())
    } else if !equipment.shop_available {
        Some(tr("shop.reason.return_base").into())
    } else if shop::purchase_quote(id, u32::MAX, &equipment.inventory)
        == Err(PurchaseError::InventoryFull)
    {
        Some(tr("shop.reason.inventory_full").into())
    } else if equipment.gold < shop::upgrade_quote(id, &equipment.inventory).cost {
        Some(trf(
            "shop.reason.need_gold",
            &[(
                "gold",
                &(shop::upgrade_quote(id, &equipment.inventory).cost - equipment.gold),
            )],
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
    mut pointer: Option<ResMut<crate::combat::WorldPointerState>>,
    mut outgoing: MessageWriter<NetworkCommand>,
) {
    // Every press is read, so one made while a purchase is pending cannot
    // fire when it resolves.
    let presses: Vec<ShopAction> = activated.read().map(|activated| activated.action).collect();
    // The HUD owns even an unavailable/pending offer's tap. Buying cannot
    // also issue a movement command later in this frame.
    if !presses.is_empty()
        && let Some(pointer) = pointer.as_deref_mut()
    {
        pointer.consumed_primary_press = true;
        pointer.consumed_secondary_press = true;
    }
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
        state.feedback = trf(
            "shop.feedback.purchasing",
            &[("item", &data::item_name(item))],
        );
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
                        trf(
                            "shop.feedback.purchased",
                            &[("item", &data::item_name(pending.item))],
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
    tr(match error {
        PurchaseError::Dead => "shop.error.dead",
        PurchaseError::OutsideBase => "shop.error.outside_base",
        PurchaseError::InsufficientGold => "shop.error.insufficient_gold",
        PurchaseError::AlreadyOwned => "shop.error.already_owned",
        PurchaseError::InventoryFull => "shop.error.inventory_full",
        PurchaseError::UnknownItem => "shop.error.unknown_item",
        _ => "shop.error.unavailable",
    })
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
    for (slot, mut text, mut color) in &mut labels.inventory {
        text.0 = equipment
            .inventory
            .get(slot.0)
            .map_or_else(|| "-".to_owned(), |id| data::item_short(*id).to_owned());
        *color = TextColor(if slot.0 < equipment.inventory.len() {
            ui::GOLD
        } else {
            ui::MUTED
        });
    }
    let phone = phone_copy(mobile.as_deref());
    for mut text in &mut labels.summary {
        let status = tr(if !stats.is_alive() {
            "shop.reason.respawn"
        } else if !equipment.shop_available {
            "shop.status.browse"
        } else if phone {
            "shop.status.buy_tap"
        } else {
            "shop.status.buy_click"
        });
        text.0 = trf(
            "shop.summary",
            &[
                ("gold", &equipment.gold),
                ("class", &data::hero_name(class.0)),
                ("status", &status),
            ],
        );
    }
    for mut text in &mut labels.feedback {
        text.0.clone_from(&state.feedback);
    }
    for (label, mut text) in &mut labels.cards {
        let reason = unavailable_reason(equipment, stats, label.0);
        let recommended = shop::recommended_items(class.0)[..3].contains(&label.0);
        text.0 = if equipment.inventory.contains(&label.0) {
            tr("shop.card.owned").into()
        } else if state.pending.as_ref().is_some_and(|p| p.item == label.0) {
            tr("shop.card.awaiting").into()
        } else {
            let tag = if recommended {
                tr("shop.card.recommended")
            } else {
                ""
            };
            let action = reason.unwrap_or_else(|| {
                tr(if phone {
                    "shop.card.tap_to_buy"
                } else {
                    "shop.card.click_to_buy"
                })
                .into()
            });
            let quote = shop::upgrade_quote(label.0, &equipment.inventory);
            let tier = trf("shop.tier", &[("tier", &shop::item(label.0).tier)]);
            let credit = shop::item(label.0).cost - quote.cost;
            let tag = if credit > 0 {
                trf("shop.credit", &[("tier", &tier), ("credit", &credit)])
            } else {
                trf("shop.tier_tag", &[("tier", &tier), ("tag", &tag)])
            };
            trf(
                "shop.card.details",
                &[("cost", &quote.cost), ("tag", &tag), ("action", &action)],
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

#[cfg(test)]
mod tests {
    use super::*;

    /// hud.md `equipment` (replaces the hidden 224 px panel this test used
    /// to size): desktop spawns the 200 × 102 plate with six 40 px slots in
    /// rows of three and the two quick-buy offers; a phone spawns the
    /// quick-buy row instead, and each TestId exists once.
    #[test]
    fn equipment_plate_is_the_redline_grid_and_each_profile_spawns_its_own_shop_shortcuts() {
        for phone in [false, true] {
            let mut app = App::new();
            let mut controls = crate::mobile_controls::MobileControls::default();
            controls.enabled = phone;
            app.insert_resource(controls)
                .add_systems(Startup, (setup_shop, setup_quick_buy));
            app.update();
            let mut ids = app.world_mut().query::<&TestId>();
            let ids: Vec<String> = ids.iter(app.world()).map(|id| id.0.to_string()).collect();
            for id in ["QuickBuy-0", "QuickBuy-1"] {
                assert_eq!(ids.iter().filter(|name| *name == id).count(), 1, "{id}");
            }
            // Desktop's gold row lives in the status plate (match_hud).
            assert_eq!(
                ids.iter().filter(|name| *name == "GoldShopButton").count(),
                usize::from(phone)
            );
            let mut names = app.world_mut().query::<(&Name, &Node)>();
            let mut find = |app: &mut App, wanted: &str| {
                names
                    .iter(app.world())
                    .find(|(name, _)| name.as_str() == wanted)
                    .map(|(_, node)| node.clone())
            };
            let equipment = find(&mut app, "EquipmentHud");
            let row = find(&mut app, "QuickBuyHud");
            assert_eq!(equipment.is_some(), !phone);
            assert_eq!(row.is_some(), phone);
            if !phone {
                let mut slots = app.world_mut().query::<(&EquipmentSlot, &Node)>();
                let slots: Vec<_> = slots
                    .iter(app.world())
                    .map(|(_, node)| node.width)
                    .collect();
                assert_eq!(slots.len(), shop::INVENTORY_CAPACITY);
                assert!(slots.iter().all(|width| *width == Val::Px(40.0)));
                let grid = find(&mut app, "InventorySlots").unwrap();
                assert_eq!(grid.width, Val::Px(3.0 * 40.0 + 2.0 * SLOT_GAP));
                // 8 + 132 + 5 + 1 + 5 + 40 + 8 = 199 ≤ 200: the plate fits.
                let inner = 2.0 * EQUIPMENT_PADDING + 4.0 * 40.0 + 2.0 * SLOT_GAP;
                assert!(inner + 2.0 * (space::S4 + border::HAIRLINE) + border::HAIRLINE <= 200.0);
            }
        }
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
        assert!(footer[0].starts_with("Components become upgrades."));
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
            .init_resource::<crate::combat::WorldPointerState>()
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
    fn controller_dpad_right_opens_the_shop_and_east_closes_it_without_pause() {
        let mut app = interaction_app();
        app.init_resource::<crate::ui::BackPress>()
            .add_systems(Last, crate::ui::back::clear_back_press);
        let mut pad = crate::gamepad::GamepadControls::default();
        pad.active = true;
        pad.shop_pressed = true;
        app.insert_resource(pad);
        app.update();
        assert!(app.world().resource::<ShopState>().open);
        app.world_mut()
            .resource_mut::<crate::gamepad::GamepadControls>()
            .shop_pressed = false;
        app.update();
        assert!(app.world().resource::<ShopState>().open);
        app.world_mut()
            .resource_mut::<crate::ui::BackPress>()
            .press();
        app.update();
        assert!(!app.world().resource::<ShopState>().open);
        assert!(!app.world().resource::<PauseMenuState>().open);
        // An idle (not owning) controller cannot open it.
        {
            let mut pad = app
                .world_mut()
                .resource_mut::<crate::gamepad::GamepadControls>();
            pad.active = false;
            pad.shop_pressed = true;
        }
        app.update();
        assert!(!app.world().resource::<ShopState>().open);
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
        let pointer = app.world().resource::<crate::combat::WorldPointerState>();
        assert!(pointer.consumed_primary_press && pointer.consumed_secondary_press);
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

    /// The phone copy replaces the English patches `mobile_ui` used to apply
    /// (`CLOSE`, the short footer, `max HP`, `tap an item`).
    #[test]
    fn phone_copy_reproduces_the_former_phone_wording() {
        assert_eq!(ShopText::CloseLabel.text(false), "ESC  CLOSE");
        assert_eq!(ShopText::CloseLabel.text(true), "CLOSE");
        assert_eq!(
            ShopText::Footer.text(true),
            "Buy at your base. Items survive respawn and reset next round."
        );
        for id in ItemId::ALL {
            assert_eq!(
                ShopText::ItemDescription(id).text(true),
                shop::item(id).description.replace("maximum HP", "max HP")
            );
            assert_eq!(
                ShopText::ItemDescription(id).text(false),
                shop::item(id).description
            );
            assert_eq!(ShopText::ItemName(id).text(true), shop::item(id).name);
        }
        assert_eq!(
            tr("shop.status.buy_tap"),
            tr("shop.status.buy_click").replace("click an item", "tap an item")
        );
    }

    #[test]
    fn shop_relabels_on_a_language_switch_and_on_a_phone() {
        if crate::i18n::testing::isolated(
            "shop::tests::shop_relabels_on_a_language_switch_and_on_a_phone",
        ) {
            return;
        }
        use crate::i18n::{I18nPlugin, Locale, LocaleId};
        let mut app = App::new();
        app.add_plugins(I18nPlugin::default())
            .add_systems(Startup, (setup_shop, setup_quick_buy))
            .add_systems(Update, relabel_shop_text);
        app.update();
        let named = |app: &mut App, wanted: &str| {
            app.world_mut()
                .query::<(&Name, &Text)>()
                .iter(app.world())
                .find(|(name, _)| name.as_str() == wanted)
                .map(|(_, text)| text.0.clone())
                .unwrap()
        };
        let item_name = |app: &mut App| {
            app.world_mut()
                .query::<(&ShopText, &Text)>()
                .iter(app.world())
                .find(|(label, _)| matches!(label, ShopText::ItemName(ItemId::EmberBlade)))
                .map(|(_, text)| text.0.clone())
                .unwrap()
        };
        let keyed = |app: &mut App, key: &str| {
            app.world_mut()
                .query::<(&Localized, &Text)>()
                .iter(app.world())
                .find(|(label, _)| label.key == key)
                .map(|(_, text)| text.0.clone())
                .unwrap()
        };
        assert_eq!(named(&mut app, "ShopCloseLabel"), "ESC  CLOSE");
        assert_eq!(item_name(&mut app), "Ember Blade");
        assert_eq!(keyed(&mut app, "shop.title"), "Sanctuary shop");

        let zh = LocaleId::parse("zh-Hans").unwrap();
        app.world_mut().resource_mut::<Locale>().set(zh);
        app.update();
        assert_eq!(named(&mut app, "ShopCloseLabel"), "ESC  关闭");
        assert_eq!(item_name(&mut app), "余烬之刃");
        assert_eq!(keyed(&mut app, "shop.title"), "圣所商店");
        assert!(named(&mut app, "ShopFooter").starts_with("组件可合成升级装备"));

        let mut controls = crate::mobile_controls::MobileControls::default();
        controls.enabled = true;
        app.insert_resource(controls);
        app.update();
        assert_eq!(named(&mut app, "ShopCloseLabel"), "关闭");
        assert!(named(&mut app, "ShopFooter").starts_with("在基地购买"));
    }

    #[test]
    fn quick_offers_have_two_stable_slots_and_do_not_offer_owned_items() {
        use ItemId::*;
        let mut owned = Vec::new();
        for _ in 0..20 {
            let offers = quick_offers(shared::HeroClass::Mage, &owned);
            let Some(next) = offers[0] else { break };
            assert!(!owned.contains(&next));
            assert!(offers[1].is_none_or(|id| id != next && !owned.contains(&id)));
            let quote = shop::purchase_quote(next, u32::MAX, &owned).unwrap();
            owned.retain(|id| !quote.consumed.contains(id));
            owned.push(next);
            assert!(owned.len() <= shop::INVENTORY_CAPACITY);
        }
        let ranger = quick_offers(shared::HeroClass::Ranger, &[SwiftGrip]);
        assert_eq!(ranger[0], Some(EmberBlade));
        let upgrade = quick_offers(
            shared::HeroClass::Ranger,
            &[SwiftGrip, EmberBlade, CritShard],
        );
        assert_eq!(upgrade[0], Some(DuelistEdge));
        let upgrade = quick_offers(shared::HeroClass::Ranger, &[SwiftGrip, DuelistEdge]);
        assert_eq!(upgrade[0], Some(TempestBlade));
        assert!(
            !quick_offers(shared::HeroClass::Ranger, &[TempestBlade]).contains(&Some(SwiftGrip))
        );
        assert_eq!(compact_gold(80), "80");
        assert_eq!(compact_gold(10_000), "10k");
    }
}
