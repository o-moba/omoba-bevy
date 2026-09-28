//! Non-button kit surfaces: panels (plain, framed 9-slice, screen ornament
//! frame, modal over a scrim), list rows, badges, tooltips and toasts
//! (`omoba-ui/handoff/components/{panel,list-row,badge,tooltip,toast}.md`).
// i18n-strict
// Kit parts that screen steps adopt; the kit gallery (`qa` feature) uses all of them.
#![cfg_attr(not(feature = "qa"), allow(dead_code))]
use std::time::Duration;

use bevy::prelude::*;

use super::{KitParts, KitSkin, button_bundle, controls, icon_node};
use crate::i18n::UiLabel;
use crate::ui::{
    TestId,
    action::UiActionT,
    kit_assets::{Frame, Icon, KitImage},
    theme::{self, ButtonKind, Form, TextStyle},
    tokens::{Metric, TextRole, border, color, motion, radius, size, space},
};

/// The plain panel: `color.surface.1`, hairline `color.border.subtle`,
/// `radius.lg`, padding `space.16`.
pub(crate) fn plain_panel() -> impl Bundle {
    (
        theme::panel_node(),
        BackgroundColor(color::SURFACE_1),
        BorderColor::all(color::BORDER_SUBTLE),
    )
}

/// Framed panel content padding (`space.24` desktop / `space.16` phone).
pub(crate) const FRAMED_PADDING: Metric = Metric::new(space::S24, space::S16);

/// The framed panel: the `frames/panel` 9-slice (insets 20, fill baked in).
pub(crate) fn framed_panel(form: Form) -> impl Bundle {
    (
        Node {
            padding: UiRect::all(Val::Px(FRAMED_PADDING.at(form))),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(space::S12),
            ..default()
        },
        KitImage::frame(Frame::Panel),
    )
}

/// `eyebrow` (optional, `type.eyebrow`) over a `type.heading` gold title and
/// a hairline divider: the framed panel header.
pub(crate) fn panel_header(
    parent: &mut ChildSpawnerCommands,
    eyebrow: Option<impl UiLabel>,
    heading: impl UiLabel,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(space::S4),
            padding: UiRect::bottom(Val::Px(space::S12)),
            border: UiRect::bottom(Val::Px(border::HAIRLINE)),
            ..default()
        })
        .insert(BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)))
        .with_children(|header| {
            if let Some(eyebrow) = eyebrow {
                header.spawn((
                    eyebrow.into_text(),
                    theme::role_text(TextRole::Eyebrow),
                    TextColor(color::TEXT_SECONDARY),
                ));
            }
            header.spawn((
                heading.into_text(),
                theme::role_text(TextRole::Heading),
                TextColor(color::TEXT_GOLD),
            ));
        });
}

/// The screen ornament frame (`frames/ornament`, insets 56, centre
/// transparent), `space.8` inside its parent: desktop full-screen menus only.
pub(crate) fn ornament_frame() -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(space::S8),
            right: Val::Px(space::S8),
            top: Val::Px(space::S8),
            bottom: Val::Px(space::S8),
            ..default()
        },
        KitImage::frame(Frame::Ornament),
        Pickable::IGNORE,
    )
}

/// A modal's open animation: scale from `motion.panel_open.scale_from` and
/// the scrim fading in over `motion.duration.panel_open`.
#[derive(Component, Clone, Copy, Default)]
pub(crate) struct PanelOpen {
    pub elapsed: f32,
}

/// The modal's scrim, faded in with its panel.
#[derive(Component)]
pub(crate) struct ModalScrim;

/// A modal: a framed panel `size.modal.width.*` wide, centred over
/// `color.scrim`, with a `type.heading` title and a close icon button
/// (`{id}Close`) `space.12` from the top-right. Returns the panel; the
/// caller fills it.
pub(crate) fn modal<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    title: impl UiLabel,
    close: T,
    form: Form,
    id: impl Into<TestId>,
) -> Entity {
    let id = id.into();
    let mut panel = Entity::PLACEHOLDER;
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(theme::perceptual(color::SCRIM)),
            ModalScrim,
            id.clone(),
        ))
        .with_children(|scrim| {
            panel = scrim
                .spawn((
                    framed_panel(form),
                    UiTransform::from_scale(Vec2::splat(motion::PANEL_OPEN_SCALE_FROM)),
                    PanelOpen::default(),
                    id.child("Panel"),
                ))
                .insert(Node {
                    width: Val::Px(size::MODAL_WIDTH.at(form)),
                    max_width: Val::Percent(95.0),
                    padding: UiRect::all(Val::Px(FRAMED_PADDING.at(form))),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(space::S12),
                    ..default()
                })
                .with_children(|panel| {
                    panel.spawn((
                        title.into_text(),
                        theme::role_text(TextRole::Heading),
                        TextColor(color::TEXT_GOLD),
                        Node {
                            margin: UiRect::right(Val::Px(size::ICON_BUTTON.at(form))),
                            ..default()
                        },
                    ));
                    panel
                        .spawn(Node {
                            position_type: PositionType::Absolute,
                            top: Val::Px(space::S12),
                            right: Val::Px(space::S12),
                            ..default()
                        })
                        .with_children(|corner| {
                            controls::sized_icon_button(
                                corner,
                                Icon::NavX,
                                form,
                                ButtonKind::Secondary,
                                close,
                                id.child("Close"),
                            );
                        });
                })
                .id();
        });
    panel
}

/// Runs the modal open animation (scale up, scrim fade).
pub(crate) fn animate_panels(
    time: Res<Time>,
    mut panels: Query<(&mut PanelOpen, &mut UiTransform, Option<&ChildOf>)>,
    mut scrims: Query<&mut BackgroundColor, With<ModalScrim>>,
) {
    let duration = motion::DURATION_PANEL_OPEN.as_secs_f32();
    for (mut open, mut transform, parent) in &mut panels {
        if open.elapsed >= duration {
            continue;
        }
        open.elapsed = (open.elapsed + time.delta_secs()).min(duration);
        let t = motion::EASING_ENTER.ease(open.elapsed / duration);
        let from = motion::PANEL_OPEN_SCALE_FROM;
        transform.scale = Vec2::splat(from + (1.0 - from) * t);
        if let Some(mut scrim) = parent.and_then(|parent| scrims.get_mut(parent.parent()).ok()) {
            scrim.0 = theme::perceptual(color::SCRIM.with_alpha(color::SCRIM.alpha() * t));
        }
    }
}

/// What leads a list row.
pub(crate) enum RowLeading {
    None,
    /// Roster art (`avatars/<slug>.jpg`), `size.portrait.sm` circle.
    Portrait(String),
    Icon(Icon),
}

/// A list row (`list-row.md`): 56 high, `color.surface.2`, hairline, leading
/// portrait or icon, title (`type.label`) and subtitle (`type.caption`
/// muted), trailing content from `trailing`. Interactive rows are
/// `ButtonKind::Tile` (selection owned by the screen).
pub(crate) fn list_row<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    leading: RowLeading,
    title: impl UiLabel,
    subtitle: impl UiLabel,
    action: T,
    id: impl Into<TestId>,
    trailing: impl FnOnce(&mut ChildSpawnerCommands),
) -> Entity {
    let mut parts = KitParts::default();
    let mut row = parent.spawn((
        button_bundle(
            Node {
                height: Val::Px(size::LIST_ROW_HEIGHT.desktop),
                width: Val::Percent(100.0),
                padding: UiRect::horizontal(Val::Px(space::S12)),
                column_gap: Val::Px(space::S12),
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::all(Val::Px(radius::MD)),
                flex_shrink: 0.0,
                ..default()
            },
            ButtonKind::Tile,
            action,
            id.into(),
        ),
        KitSkin::Row,
    ));
    row.with_children(|row| {
        match leading {
            RowLeading::None => {}
            RowLeading::Portrait(path) => {
                row.spawn(super::game::round_art(path, size::PORTRAIT_SM));
            }
            RowLeading::Icon(icon) => {
                parts.icon = Some(
                    row.spawn(icon_node(icon, size::ICON_LG, color::TEXT_GOLD))
                        .id(),
                );
            }
        }
        row.spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            min_width: Val::Px(0.0),
            overflow: Overflow::clip_x(),
            ..default()
        })
        .with_children(|text| {
            parts.label = Some(
                text.spawn((
                    title.into_text(),
                    theme::styled_text(TextStyle::keep_case(TextRole::Label)),
                    TextColor(color::TEXT_PRIMARY),
                    TextLayout::new_with_no_wrap(),
                ))
                .id(),
            );
            parts.extra[0] = Some(
                text.spawn((
                    subtitle.into_text(),
                    theme::role_text(TextRole::Caption),
                    TextColor(color::TEXT_MUTED),
                    TextLayout::new_with_no_wrap(),
                ))
                .id(),
            );
        });
        row.spawn(Node {
            column_gap: Val::Px(space::S8),
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(trailing);
    });
    row.insert(parts).id()
}

/// Badge colours (`badge.md`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BadgeKind {
    /// MVP, Leader.
    Gold,
    Green,
    Blue,
    Danger,
    /// BOT, off.
    Muted,
}

impl BadgeKind {
    fn colors(self) -> (Color, Color) {
        match self {
            BadgeKind::Gold => (color::GOLD_500, color::TEXT_ON_GOLD),
            BadgeKind::Green => (color::TEAM_GREEN_DIM, color::TEXT_PRIMARY),
            BadgeKind::Blue => (color::TEAM_BLUE_DIM, color::TEXT_PRIMARY),
            BadgeKind::Danger => (color::STATE_DANGER, color::TEXT_PRIMARY),
            BadgeKind::Muted => (color::SURFACE_3, color::TEXT_SECONDARY),
        }
    }
}

/// A pill badge, `size.badge.height` high: `type.caption` semibold text
/// (`number` badges use `type.number_sm`).
pub(crate) fn badge(
    parent: &mut ChildSpawnerCommands,
    text: impl UiLabel,
    kind: BadgeKind,
    number: bool,
) -> Entity {
    let (fill, ink) = kind.colors();
    let style = if number {
        TextStyle::new(TextRole::NumberSm)
    } else {
        // `type.caption` in the semibold body face.
        TextStyle::keep_case(TextRole::Label).sized(TextRole::Caption.style().size)
    };
    parent
        .spawn((
            Node {
                height: Val::Px(size::BADGE_HEIGHT),
                min_width: Val::Px(size::BADGE_HEIGHT),
                padding: UiRect::horizontal(Val::Px(if number { space::S4 } else { space::S8 })),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                flex_shrink: 0.0,
                border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                ..default()
            },
            BackgroundColor(fill),
            Pickable::IGNORE,
        ))
        .with_child((text.into_text(), theme::styled_text(style), TextColor(ink)))
        .id()
}

/// Tooltip limits (`tooltip.md`): max width 280, body `type.body` at 13 px,
/// hover delay 400 ms.
pub(crate) const TOOLTIP_MAX_W: f32 = 280.0;
pub(crate) const TOOLTIP_BODY: Metric = Metric::new(13.0, 13.0);
pub(crate) const TOOLTIP_DELAY: Duration = Duration::from_millis(400);

/// The tooltip panel: `color.surface.glass.strong`, hairline gold, optional
/// title (`type.label`) and body (`type.body` at 13 px, secondary).
pub(crate) fn tooltip_panel(
    parent: &mut ChildSpawnerCommands,
    title: Option<impl UiLabel>,
    body: impl UiLabel,
) -> Entity {
    parent
        .spawn(tooltip_bundle())
        .with_children(|tip| tooltip_content(tip, title, body))
        .id()
}

fn tooltip_bundle() -> impl Bundle {
    (
        Node {
            max_width: Val::Px(TOOLTIP_MAX_W),
            padding: UiRect::axes(Val::Px(space::S12), Val::Px(space::S8 + border::FRAME)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(space::S4),
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            ..default()
        },
        BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
        BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)),
        Pickable::IGNORE,
    )
}

fn tooltip_content(
    tip: &mut ChildSpawnerCommands,
    title: Option<impl UiLabel>,
    body: impl UiLabel,
) {
    if let Some(title) = title {
        tip.spawn((
            title.into_text(),
            theme::styled_text(TextStyle::keep_case(TextRole::Label)),
            TextColor(color::TEXT_PRIMARY),
        ));
    }
    tip.spawn((
        body.into_text(),
        theme::styled_text(TextStyle::new(TextRole::Body).sized(TOOLTIP_BODY)),
        TextColor(color::TEXT_SECONDARY),
    ));
}

/// A control that shows a tooltip on hover (desktop, after 400 ms) and
/// while focused. Keys are i18n keys.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Tooltip {
    pub title: Option<&'static str>,
    pub body: &'static str,
}

/// The one live tooltip.
#[derive(Component)]
pub(crate) struct TooltipRoot {
    anchor: Entity,
}

/// Shows the tooltip of the hovered (after [`TOOLTIP_DELAY`]) or focused
/// anchor above it with a `space.8` gap, below when it would leave the
/// window; hides it otherwise.
#[allow(clippy::type_complexity)]
pub(crate) fn show_tooltips(
    mut commands: Commands,
    time: Res<Time>,
    focus: Option<Res<crate::ui::UiFocus>>,
    anchors: Query<(
        Entity,
        &Tooltip,
        &Interaction,
        &ComputedNode,
        &UiGlobalTransform,
    )>,
    mut roots: Query<(Entity, &TooltipRoot, &mut Node, &ComputedNode)>,
    mut hovered: Local<Option<(Entity, Duration)>>,
) {
    let now = time.elapsed();
    let focused = focus.as_ref().and_then(|focus| focus.focused());
    let pointer = anchors
        .iter()
        .find(|(_, _, interaction, ..)| **interaction == Interaction::Hovered)
        .map(|(entity, ..)| entity);
    match (pointer, *hovered) {
        (Some(entity), Some((was, _))) if was == entity => {}
        (Some(entity), _) => *hovered = Some((entity, now)),
        (None, _) => *hovered = None,
    }
    let shown = focused
        .filter(|entity| anchors.contains(*entity))
        .or_else(|| {
            hovered
                .filter(|(_, since)| now.saturating_sub(*since) >= TOOLTIP_DELAY)
                .map(|(entity, _)| entity)
        });
    let current = roots
        .iter()
        .next()
        .map(|(entity, root, ..)| (entity, root.anchor));
    match (shown, current) {
        (None, Some((root, _))) => {
            commands.entity(root).despawn();
        }
        (Some(anchor), current) if current.is_none_or(|(_, was)| was != anchor) => {
            if let Some((root, _)) = current {
                commands.entity(root).despawn();
            }
            let Ok((_, tooltip, ..)) = anchors.get(anchor) else {
                return;
            };
            let title = tooltip.title.map(crate::i18n::Localized::new);
            let body = crate::i18n::Localized::new(tooltip.body);
            commands
                .spawn((
                    tooltip_bundle(),
                    GlobalZIndex(4800),
                    Visibility::Hidden,
                    TooltipRoot { anchor },
                    Name::new("UiTooltip"),
                ))
                .insert(Node {
                    position_type: PositionType::Absolute,
                    max_width: Val::Px(TOOLTIP_MAX_W),
                    padding: UiRect::axes(Val::Px(space::S12), Val::Px(space::S8 + border::FRAME)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(space::S4),
                    border: UiRect::all(Val::Px(border::HAIRLINE)),
                    border_radius: BorderRadius::all(Val::Px(radius::MD)),
                    ..default()
                })
                .with_children(|tip| tooltip_content(tip, title, body));
        }
        (Some(anchor), Some(_)) => {
            let Ok((_, _, _, anchor_node, anchor_at)) = anchors.get(anchor) else {
                return;
            };
            for (_, _, mut node, tip) in &mut roots {
                let scale = anchor_node.inverse_scale_factor();
                let rect = crate::ui::focus::node_rect(anchor_node, anchor_at);
                let size = tip.size() * tip.inverse_scale_factor();
                let (min, max) = (rect.min * scale, rect.max * scale);
                let above = min.y - space::S8 - size.y;
                let top = if above >= 0.0 {
                    above
                } else {
                    max.y + space::S8
                };
                let left = ((min.x + max.x) * 0.5 - size.x * 0.5).max(space::S8);
                if node.top != Val::Px(top) {
                    node.top = Val::Px(top);
                }
                if node.left != Val::Px(left) {
                    node.left = Val::Px(left);
                }
            }
        }
        _ => {}
    }
}

/// Reveals a tooltip once it has been measured and placed.
pub(crate) fn reveal_tooltips(
    mut roots: Query<(&ComputedNode, &Node, &mut Visibility), With<TooltipRoot>>,
) {
    for (computed, node, mut visibility) in &mut roots {
        if computed.size().x > 0.0 && node.top != Val::Auto && *visibility == Visibility::Hidden {
            *visibility = Visibility::Inherited;
        }
    }
}

/// Toast kinds (`toast.md`): accent bar and icon colour.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ToastKind {
    Info,
    Success,
    Warning,
    Danger,
}

impl ToastKind {
    fn accent(self) -> Color {
        match self {
            ToastKind::Info => color::STATE_INFO,
            ToastKind::Success => color::STATE_SUCCESS,
            ToastKind::Warning => color::STATE_WARNING,
            ToastKind::Danger => color::STATE_DANGER,
        }
    }

    fn icon(self) -> Icon {
        match self {
            ToastKind::Info => Icon::NavInfo,
            ToastKind::Success => Icon::NavCheck,
            ToastKind::Warning => Icon::NavAlertTriangle,
            ToastKind::Danger => Icon::NavWifiOff,
        }
    }
}

/// Toast geometry (`toast.md`): 320–420 wide, 72 px from the top on
/// desktop, under the score strip on a phone, three at most.
pub(crate) const TOAST_MIN_W: f32 = 320.0;
pub(crate) const TOAST_MAX_W: f32 = 420.0;
pub(crate) const TOAST_TOP: Metric = Metric::new(72.0, size::TOUCH_MIN + space::S12);
pub(crate) const TOAST_ACCENT: f32 = space::S4;
pub(crate) const TOAST_MAX: usize = 3;
/// Slide distance of the entry animation.
pub(crate) const TOAST_SLIDE: f32 = space::S8;

/// A toast panel (static; [`ToastRequest`] shows a timed one).
pub(crate) fn toast_panel(
    parent: &mut ChildSpawnerCommands,
    kind: ToastKind,
    text: impl UiLabel,
) -> Entity {
    parent
        .spawn(toast_bundle())
        .with_children(|toast| toast_content(toast, kind, text))
        .id()
}

fn toast_bundle() -> impl Bundle {
    (
        Node {
            min_width: Val::Px(TOAST_MIN_W),
            max_width: Val::Px(TOAST_MAX_W),
            padding: UiRect::new(
                Val::Px(space::S16 + TOAST_ACCENT),
                Val::Px(space::S16),
                Val::Px(space::S12),
                Val::Px(space::S12),
            ),
            column_gap: Val::Px(space::S12),
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
        BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)),
        Pickable::IGNORE,
    )
}

fn toast_content(toast: &mut ChildSpawnerCommands, kind: ToastKind, text: impl UiLabel) {
    toast.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            width: Val::Px(TOAST_ACCENT),
            ..default()
        },
        BackgroundColor(kind.accent()),
    ));
    toast.spawn(icon_node(kind.icon(), size::ICON_MD, kind.accent()));
    toast.spawn((
        text.into_text(),
        theme::styled_text(TextStyle::keep_case(TextRole::Body)),
        TextColor(color::TEXT_PRIMARY),
        Node {
            max_width: Val::Px(
                TOAST_MAX_W - space::S16 * 2.0 - TOAST_ACCENT - size::ICON_MD - space::S12,
            ),
            ..default()
        },
    ));
}

/// Shows a timed toast.
#[derive(Message, Clone, Debug)]
pub(crate) struct ToastRequest {
    pub kind: ToastKind,
    pub text: String,
}

/// The toast stack root and one live toast's age.
#[derive(Component)]
pub(crate) struct ToastStack;
#[derive(Component, Default)]
pub(crate) struct ToastAge(Duration);

/// Spawns requested toasts (newest at the bottom, oldest dropped past three)
/// and runs their slide-in, hold and exit.
pub(crate) fn run_toasts(
    mut commands: Commands,
    time: Res<Time>,
    form: Option<Res<theme::UiForm>>,
    mut requests: MessageReader<ToastRequest>,
    stacks: Query<Entity, With<ToastStack>>,
    mut toasts: Query<(Entity, &mut ToastAge, &mut Node, &mut Visibility)>,
) {
    let form = form.map_or(Form::Desktop, |form| form.0);
    let mut fresh: Vec<ToastRequest> = requests.read().cloned().collect();
    fresh.drain(..fresh.len().saturating_sub(TOAST_MAX));
    if !fresh.is_empty() {
        let stack = stacks.iter().next().unwrap_or_else(|| {
            commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(TOAST_TOP.at(form)),
                        left: Val::Px(0.0),
                        right: Val::Px(0.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(space::S8),
                        ..default()
                    },
                    GlobalZIndex(4700),
                    Pickable::IGNORE,
                    ToastStack,
                    Name::new("UiToastStack"),
                ))
                .id()
        });
        let live = toasts.iter().count();
        let excess = (live + fresh.len()).saturating_sub(TOAST_MAX);
        let mut ages: Vec<(Entity, Duration)> =
            toasts.iter().map(|(e, age, ..)| (e, age.0)).collect();
        ages.sort_by_key(|(_, age)| std::cmp::Reverse(*age));
        for (entity, _) in ages.into_iter().take(excess) {
            commands.entity(entity).despawn();
        }
        for request in fresh {
            commands.entity(stack).with_children(|stack| {
                stack
                    .spawn((toast_bundle(), ToastAge::default()))
                    .with_children(|toast| toast_content(toast, request.kind, request.text));
            });
        }
    }
    let enter = motion::DURATION_TOAST_IN;
    let hold = motion::DURATION_TOAST_HOLD;
    let exit = motion::DURATION_TOAST_OUT;
    for (entity, mut age, mut node, mut visibility) in &mut toasts {
        age.0 += time.delta();
        let offset = if age.0 < enter {
            let t = motion::EASING_ENTER.ease(age.0.as_secs_f32() / enter.as_secs_f32());
            -TOAST_SLIDE * (1.0 - t)
        } else if age.0 < enter + hold {
            0.0
        } else if age.0 < enter + hold + exit {
            let t = (age.0 - enter - hold).as_secs_f32() / exit.as_secs_f32();
            -TOAST_SLIDE * motion::EASING_EXIT.ease(t)
        } else {
            commands.entity(entity).despawn();
            continue;
        };
        if node.top != Val::Px(offset) {
            node.top = Val::Px(offset);
        }
        if *visibility != Visibility::Inherited {
            *visibility = Visibility::Inherited;
        }
    }
}

/// A keycap: the ability button's key badge (`ability-button.md`) as an
/// inline control legend (`hud-help.md` inputs): `size.badge.height` high,
/// `color.surface.1.opaque`, hairline `color.gold.600`, the key in
/// `type.number_sm` size and the semibold body face, `color.text.gold`.
/// Key names are glyphs, never translated.
pub(crate) fn keycap(parent: &mut ChildSpawnerCommands, key: impl UiLabel) -> Entity {
    parent
        .spawn((
            Node {
                min_width: Val::Px(size::BADGE_HEIGHT),
                height: Val::Px(size::BADGE_HEIGHT),
                padding: UiRect::horizontal(Val::Px(space::S4)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                flex_shrink: 0.0,
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::all(Val::Px(radius::SM)),
                ..default()
            },
            BackgroundColor(color::SURFACE_1_OPAQUE),
            BorderColor::all(color::GOLD_600),
            Pickable::IGNORE,
        ))
        .with_child((
            key.into_text(),
            theme::styled_text(
                TextStyle::keep_case(TextRole::Label).sized(TextRole::NumberSm.style().size),
            ),
            TextColor(color::TEXT_GOLD),
        ))
        .id()
}

/// Info card padding (`space.16` desktop / `space.12` phone, `hud-help.md`).
pub(crate) const INFO_CARD_PADDING: Metric = Metric::new(space::S16, space::S12);
/// The icon disc (`size.icon.xl` / `size.icon.lg`) and its icon
/// (`size.icon.md` / `size.icon.sm`).
pub(crate) const INFO_CARD_DISC: Metric = Metric::new(size::ICON_XL, size::ICON_LG);
pub(crate) const INFO_CARD_ICON: Metric = Metric::new(size::ICON_MD, size::ICON_SM);
/// Card title: `type.heading`, set at 16 px on a phone (`hud-help.md`), below
/// the phone heading minimum, so the card owns that size ([`theme::PhoneSized`]).
pub(crate) const INFO_CARD_TITLE: Metric =
    Metric::new(TextRole::Heading.style().size.desktop, 16.0);
/// Gap between the disc and the title (the redline's 10 px) and between the
/// header and the body (`space.8` / `space.4`).
pub(crate) const INFO_CARD_HEAD_GAP: f32 = 10.0;
pub(crate) const INFO_CARD_BODY_GAP: Metric = Metric::new(space::S8, space::S4);

/// The parts of an [`info_card`] a screen fills or tests.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InfoCard {
    pub card: Entity,
    pub title: Entity,
    pub body: Entity,
}

/// An illustrated info card (`hud-help.md` help cards): a plain panel in
/// `color.surface.2` with `color.border.subtle`, `radius.md`; a header with an
/// icon disc (`color.surface.3`, hairline `color.gold.600`, gold icon) and a
/// `type.heading` gold title; a `type.body` secondary body. The caller sizes
/// the card (`node`) and may add an input legend row (keycaps, badges) at the
/// bottom. Not interactive.
pub(crate) fn info_card(
    parent: &mut ChildSpawnerCommands,
    node: Node,
    icon: Icon,
    title: impl UiLabel,
    body: impl UiLabel,
    form: Form,
) -> InfoCard {
    let phone = form == Form::Phone;
    let padding = INFO_CARD_PADDING.at(form);
    let disc = INFO_CARD_DISC.at(form);
    let mut parts = InfoCard {
        card: Entity::PLACEHOLDER,
        title: Entity::PLACEHOLDER,
        body: Entity::PLACEHOLDER,
    };
    let mut card = parent.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(padding)),
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            overflow: Overflow::clip(),
            ..node
        },
        BackgroundColor(color::SURFACE_2),
        BorderColor::all(color::BORDER_SUBTLE),
    ));
    parts.card = card.id();
    card.with_children(|card| {
        card.spawn(Node {
            height: Val::Px(disc),
            column_gap: Val::Px(INFO_CARD_HEAD_GAP),
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|head| {
            head.spawn((
                Node {
                    width: Val::Px(disc),
                    height: Val::Px(disc),
                    flex_shrink: 0.0,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(border::HAIRLINE)),
                    border_radius: BorderRadius::all(Val::Percent(50.0)),
                    ..default()
                },
                BackgroundColor(color::SURFACE_3),
                BorderColor::all(color::GOLD_600),
            ))
            .with_child(icon_node(icon, INFO_CARD_ICON.at(form), color::TEXT_GOLD));
            let mut title = head.spawn((
                title.into_text(),
                theme::styled_text(TextStyle::new(TextRole::Heading).sized(INFO_CARD_TITLE)),
                TextColor(color::TEXT_GOLD),
                TextLayout::new_with_no_wrap(),
            ));
            if phone {
                // Below the phone heading minimum: the card owns the size.
                title.insert((
                    theme::PhoneSized,
                    TextFont::from_font_size(INFO_CARD_TITLE.phone),
                ));
            }
            parts.title = title.id();
        });
        parts.body = card
            .spawn((
                body.into_text(),
                theme::role_text(TextRole::Body),
                TextColor(color::TEXT_SECONDARY),
                Node {
                    margin: UiRect::top(Val::Px(INFO_CARD_BODY_GAP.at(form))),
                    ..default()
                },
            ))
            .id();
    });
    parts
}

/// Every system of this module.
pub(crate) fn add_systems(app: &mut App) {
    app.add_message::<ToastRequest>().add_systems(
        Update,
        (animate_panels, show_tooltips, run_toasts).in_set(crate::ui::UiSet::Paint),
    );
    app.add_systems(
        PostUpdate,
        reveal_tooltips.after(bevy::ui::UiSystems::Layout),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toasts_stack_at_most_three_and_expire() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .add_message::<ToastRequest>()
            .add_systems(Update, run_toasts);
        for index in 0..5 {
            app.world_mut().write_message(ToastRequest {
                kind: ToastKind::Info,
                text: format!("toast {index}"),
            });
        }
        app.update();
        app.update();
        let count = app
            .world_mut()
            .query::<&ToastAge>()
            .iter(app.world())
            .count();
        assert_eq!(count, TOAST_MAX, "the newest three");
        app.world_mut().write_message(ToastRequest {
            kind: ToastKind::Danger,
            text: "late".into(),
        });
        app.update();
        app.update();
        let count = app
            .world_mut()
            .query::<&ToastAge>()
            .iter(app.world())
            .count();
        assert_eq!(count, TOAST_MAX, "the oldest is dropped");
        // Age everything past its life.
        for mut age in app
            .world_mut()
            .query::<&mut ToastAge>()
            .iter_mut(app.world_mut())
        {
            age.0 = motion::DURATION_TOAST_IN
                + motion::DURATION_TOAST_HOLD
                + motion::DURATION_TOAST_OUT;
        }
        app.update();
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ToastAge>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn badge_kinds_use_their_token_pairs() {
        assert_eq!(
            BadgeKind::Gold.colors(),
            (color::GOLD_500, color::TEXT_ON_GOLD)
        );
        assert_eq!(BadgeKind::Muted.colors().0, color::SURFACE_3);
    }
}
