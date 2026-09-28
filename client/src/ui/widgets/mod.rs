//! Kit widgets in the Verdant Crown style (`omoba-ui/handoff/components/`):
//! menu and screen buttons, `-`/`+` steppers, toggle rows and value labels,
//! plus every component of the handoff kit in [`controls`], [`surfaces`] and
//! [`game`]. Every control gets a [`UiAction`], a [`ButtonStyle`] and a
//! [`TestId`]; the module that owns it only handles `Activated<T>`.
//!
//! Painting is data-driven: [`paint_pressables`] fills every `ButtonStyle`
//! from its kind and effective interaction, [`paint_slabs`] draws the 9-slice
//! slab of primary/secondary/danger/team buttons, [`paint_kit`] keeps each
//! kit-spawned control's border, label, icon and parts ([`KitSkin`],
//! [`KitParts`]) in step with its state, and [`paint_focus_ring`] draws the
//! gamepad focus ring. Colours and sizes are tokens ([`super::tokens`]).
//!
//! Captions are `impl UiLabel`: a literal (`"×"`, a formatted `String`) or a
//! `crate::i18n::Localized` key, which the widget spawns filled in the active
//! language and which follows later language changes.
// i18n-strict
use bevy::prelude::*;

use super::{
    Pressable, TestId, UiAction,
    action::UiActionT,
    kit_assets::{Frame, Icon, KitImage},
    theme::{self, ButtonKind, ButtonState, Form, SlabFamily, TextStyle, metric},
    tokens::{TextRole, border, color, motion, radius, size, space},
};
use crate::i18n::UiLabel;

pub(crate) mod controls;
pub(crate) mod game;
pub(crate) mod status;
pub(crate) mod surfaces;

/// Colour role of a kit button. Painted by [`paint_pressables`] from
/// [`kit_state`]: on desktop hover and press follow the pointer; on touch a
/// resting finger shows the pressed look and there is no hover.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ButtonStyle {
    pub kind: ButtonKind,
    /// Selected tiles keep their colour under the pointer.
    pub selected: bool,
}

impl ButtonStyle {
    pub(crate) fn new(kind: ButtonKind) -> Self {
        Self {
            kind,
            selected: false,
        }
    }

    pub(crate) fn idle_color(&self) -> Color {
        theme::button_idle_color(self.kind, self.selected)
    }

    pub(crate) fn hover_color(&self) -> Color {
        theme::button_hover_color(self.kind, self.selected)
    }

    pub(crate) fn pressed_color(&self) -> Color {
        theme::button_pressed_color(self.kind, self.selected)
    }

    /// `selected`, set only when it changes (the painter reacts to changes).
    pub(crate) fn set_selected(style: &mut Mut<Self>, selected: bool) {
        if style.selected != selected {
            style.selected = selected;
        }
    }
}

/// How a kit-spawned control is drawn beyond its fill; [`paint_kit`] reads it.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum KitSkin {
    /// A slab button (primary, secondary, danger, team): label colour.
    Slab,
    /// A tertiary text action (`ButtonKind::Link`): label colour + hover underline.
    Tertiary,
    /// A native tile (legacy screen tiles, chevrons): fill + border by state.
    Native,
    /// A list row: fill by state, `gold.500` border when selected.
    Row,
    /// Round or square icon button (`icon-button.md`, stepper buttons).
    Icon,
    /// Top-bar or rail tab with its selection indicator.
    Tab { rail: bool },
    /// A settings row with a switch.
    Toggle,
    /// A slider row (track, fill, thumb).
    Slider,
    /// The cycle-row control (value between chevrons).
    Cycle,
    /// A hero tile (art, name, source edge).
    HeroTile,
    /// A shop item card or an inventory slot in the shop.
    ShopCard,
    /// A text field.
    Input,
    /// An ability button (circle, rim, cooldown, badges).
    Ability,
}

/// Child entities [`paint_kit`] repaints, recorded at spawn.
#[derive(Component, Clone, Copy, Default, Debug)]
pub(crate) struct KitParts {
    pub label: Option<Entity>,
    pub icon: Option<Entity>,
    /// Tab indicator, toggle track, slider track, ability rim.
    pub track: Option<Entity>,
    /// Toggle knob, slider thumb.
    pub knob: Option<Entity>,
    /// Slider fill, hero tile lock, ability glow.
    pub fill: Option<Entity>,
    /// Secondary text (subtitle, price, placeholder) and cycle chevrons.
    pub extra: [Option<Entity>; 2],
}

/// Pins a control to one state (the kit gallery shows every state at once):
/// the painters use it instead of the pointer, and `focused` draws a static
/// focus ring on the control.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct PreviewState {
    pub state: ButtonState,
    pub focused: bool,
}

/// A button whose kind has a slab but whose owner draws its own image.
#[derive(Component)]
pub(crate) struct NoSlab;

/// The state a control is drawn in.
pub(crate) fn kit_state(
    interaction: Interaction,
    pressable: &Pressable,
    preview: Option<&PreviewState>,
) -> ButtonState {
    if let Some(preview) = preview {
        return preview.state;
    }
    if pressable.disabled {
        return ButtonState::Disabled;
    }
    if pressable.touch_mode {
        // Touch has no hover: a held finger shows the pressed look, even
        // though the press only activates on release.
        if pressable.effective(interaction) == Interaction::None {
            return ButtonState::Idle;
        }
        return if pressable.activated || interaction == Interaction::Pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Idle
        };
    }
    match pressable.effective(interaction) {
        Interaction::Pressed => ButtonState::Pressed,
        Interaction::Hovered => ButtonState::Hover,
        Interaction::None => ButtonState::Idle,
    }
}

/// Runs in `UiSet::Paint`; the one place kit buttons change fill colour.
#[allow(clippy::type_complexity)]
pub(crate) fn paint_pressables(
    mut buttons: Query<
        (
            &Interaction,
            &Pressable,
            &ButtonStyle,
            &mut BackgroundColor,
            Option<&PreviewState>,
            Option<&KitSkin>,
        ),
        Or<(
            Changed<Interaction>,
            Changed<Pressable>,
            Changed<ButtonStyle>,
            Changed<PreviewState>,
        )>,
    >,
) {
    for (interaction, pressable, style, mut color, preview, skin) in &mut buttons {
        // A kit skin with its own fill rule owns the fill.
        if let Some(next) = skin
            .and_then(|skin| skin_fill(*skin, style, kit_state(*interaction, pressable, preview)))
        {
            if color.0 != next {
                color.0 = next;
            }
            continue;
        }
        let next = match preview.map(|preview| preview.state) {
            Some(ButtonState::Pressed) => style.pressed_color(),
            Some(ButtonState::Hover) => style.hover_color(),
            Some(_) => style.idle_color(),
            None => match kit_state(*interaction, pressable, None) {
                ButtonState::Pressed => style.pressed_color(),
                ButtonState::Hover => style.hover_color(),
                _ => style.idle_color(),
            },
        };
        if color.0 != next {
            color.0 = next;
        }
    }
}

/// The slab image for a family in a state (`frames/button-*`); a selected
/// secondary shows its hover slab (`components/button.md`).
pub(crate) fn slab_frame(family: SlabFamily, state: ButtonState, selected: bool) -> Frame {
    use ButtonState::*;
    let state = if selected && family == SlabFamily::Secondary && state == Idle {
        Hover
    } else {
        state
    };
    match (family, state) {
        (SlabFamily::Primary, Idle) => Frame::ButtonPrimary,
        (SlabFamily::Primary, Hover) => Frame::ButtonPrimaryHover,
        (SlabFamily::Primary, Pressed) => Frame::ButtonPrimaryPressed,
        (SlabFamily::Primary, Disabled) => Frame::ButtonPrimaryDisabled,
        (SlabFamily::Secondary, Idle) => Frame::ButtonSecondary,
        (SlabFamily::Secondary, Hover) => Frame::ButtonSecondaryHover,
        (SlabFamily::Secondary, Pressed) => Frame::ButtonSecondaryPressed,
        (SlabFamily::Secondary, Disabled) => Frame::ButtonSecondaryDisabled,
        (SlabFamily::Danger, Idle) => Frame::ButtonDanger,
        (SlabFamily::Danger, Hover) => Frame::ButtonDangerHover,
        (SlabFamily::Danger, Pressed) => Frame::ButtonDangerPressed,
        (SlabFamily::Danger, Disabled) => Frame::ButtonDangerDisabled,
    }
}

/// The team-colour bar on a team lock-in button (4 px, left edge).
#[derive(Component)]
pub(crate) struct TeamBar;

/// Draws the slab of every primary, secondary, danger and team button,
/// kit-spawned or screen-owned (screens pick up the Verdant look), unless the
/// owner already draws an image on it ([`NoSlab`], or an `ImageNode` that is
/// not a [`KitImage`]). Team buttons get their bar once.
#[allow(clippy::type_complexity)]
pub(crate) fn paint_slabs(
    mut commands: Commands,
    mut buttons: Query<
        (
            Entity,
            &ButtonStyle,
            &Interaction,
            &Pressable,
            Option<&PreviewState>,
            Option<&mut KitImage>,
            Has<ImageNode>,
            Option<&Children>,
        ),
        (
            Without<NoSlab>,
            Or<(
                Changed<Interaction>,
                Changed<Pressable>,
                Changed<ButtonStyle>,
                Changed<PreviewState>,
            )>,
        ),
    >,
    bars: Query<(), With<TeamBar>>,
) {
    for (entity, style, interaction, pressable, preview, kit, has_image, children) in &mut buttons {
        let Some(family) = style.kind.slab() else {
            continue;
        };
        let state = kit_state(*interaction, pressable, preview);
        let next = KitImage::frame(slab_frame(family, state, style.selected));
        match kit {
            Some(mut kit) if *kit != next => *kit = next,
            Some(_) => {}
            None if !has_image => {
                commands.entity(entity).insert(next);
            }
            None => continue,
        }
        if let ButtonKind::Team(team) = style.kind {
            let has_bar =
                children.is_some_and(|children| children.iter().any(|c| bars.contains(c)));
            if !has_bar {
                commands.entity(entity).with_child((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(border::FRAME),
                        top: Val::Px(border::FRAME),
                        bottom: Val::Px(border::FRAME),
                        width: Val::Px(space::S4),
                        border_radius: BorderRadius::left(Val::Px(radius::SM)),
                        ..default()
                    },
                    BackgroundColor(match team {
                        crate::domain::Team::Green => color::TEAM_GREEN,
                        crate::domain::Team::Blue => color::TEAM_BLUE,
                    }),
                    Pickable::IGNORE,
                    TeamBar,
                ));
            }
        }
    }
}

/// Fill a skin draws instead of its kind's (tabs are transparent until
/// hovered or selected, rows and fields keep their surface).
fn skin_fill(skin: KitSkin, style: &ButtonStyle, state: ButtonState) -> Option<Color> {
    use ButtonState::*;
    match skin {
        KitSkin::Tab { rail } => Some(match (state, style.selected) {
            (Disabled, _) => Color::NONE,
            (_, true) if rail => color::SURFACE_SELECTED,
            (Hover, _) => color::SURFACE_HOVER,
            (Pressed, _) => color::SURFACE_0,
            _ => Color::NONE,
        }),
        KitSkin::Row | KitSkin::ShopCard => Some(match (state, style.selected) {
            (_, true) => color::SURFACE_SELECTED,
            (Hover, _) => color::SURFACE_HOVER,
            (Pressed, _) => color::SURFACE_0,
            _ => color::SURFACE_2,
        }),
        KitSkin::Icon => Some(match (state, style.selected) {
            (Disabled, _) => color::SURFACE_2,
            (_, true) => color::SURFACE_SELECTED,
            (Hover, _) => color::SURFACE_HOVER,
            (Pressed, _) => color::SURFACE_0,
            _ => color::SURFACE_2,
        }),
        KitSkin::Toggle | KitSkin::Slider | KitSkin::Ability | KitSkin::Tertiary => {
            Some(Color::NONE)
        }
        KitSkin::Cycle | KitSkin::HeroTile => Some(color::SURFACE_2),
        KitSkin::Input => Some(color::SURFACE_0),
        KitSkin::Slab | KitSkin::Native => None,
    }
}

/// Border a skin draws on the control itself.
fn skin_border(skin: KitSkin, style: &ButtonStyle, state: ButtonState) -> Option<Color> {
    use ButtonState::*;
    Some(match skin {
        KitSkin::Native => theme::native_border_color(style.selected, state),
        KitSkin::Row => match (state, style.selected) {
            (_, true) => color::GOLD_500,
            _ => color::BORDER_SUBTLE,
        },
        KitSkin::ShopCard => match (state, style.selected) {
            (_, true) => color::GOLD_400,
            _ => color::BORDER_SUBTLE,
        },
        KitSkin::Icon => match (state, style.selected) {
            (Disabled, _) => color::BORDER_DISABLED,
            (_, true) => color::GOLD_400,
            (Hover, _) => color::GOLD_500,
            _ => color::GOLD_600,
        },
        KitSkin::Cycle => match state {
            Hover => color::GOLD_600,
            _ => color::BORDER_SUBTLE,
        },
        KitSkin::HeroTile => match (state, style.selected) {
            (_, true) => color::GOLD_400,
            (Hover, _) => color::GOLD_600,
            _ => color::BORDER_SUBTLE,
        },
        KitSkin::Input => match (state, style.selected) {
            (Disabled, _) => color::BORDER_SUBTLE,
            (_, true) => color::GOLD_400,
            (Hover, _) => color::GOLD_600,
            _ => color::BORDER_SUBTLE,
        },
        KitSkin::Slab
        | KitSkin::Tertiary
        | KitSkin::Tab { .. }
        | KitSkin::Toggle
        | KitSkin::Slider
        | KitSkin::Ability => return None,
    })
}

/// Tint of a control's icon part.
fn icon_tint(skin: KitSkin, style: &ButtonStyle, state: ButtonState) -> Color {
    use ButtonState::*;
    match (skin, state) {
        (_, Disabled) => color::TEXT_DISABLED,
        (KitSkin::Icon, Hover) => color::GOLD_300,
        (KitSkin::Icon | KitSkin::Cycle, _) => color::TEXT_GOLD,
        (KitSkin::ShopCard, _) => color::GOLD_400,
        (KitSkin::Tab { .. }, _) if style.selected => color::TEXT_GOLD,
        (KitSkin::Tab { .. }, Hover) => color::TEXT_PRIMARY,
        (KitSkin::Tab { .. }, _) => color::TEXT_MUTED,
        (KitSkin::Slab, _) if style.kind == ButtonKind::Primary => color::TEXT_ON_PRIMARY,
        (KitSkin::Tertiary, _) => color::STATE_INFO,
        _ => theme::button_label_color(style.kind, style.selected, state),
    }
}

/// Scale a pressed control shrinks to (`motion.press.scale`; abilities 0.94).
fn press_scale(skin: KitSkin, state: ButtonState) -> f32 {
    match (skin, state) {
        (KitSkin::Ability, ButtonState::Pressed) => game::ABILITY_PRESS_SCALE,
        (
            KitSkin::Slab
            | KitSkin::Icon
            | KitSkin::Native
            | KitSkin::Row
            | KitSkin::HeroTile
            | KitSkin::ShopCard,
            ButtonState::Pressed,
        ) => motion::PRESS_SCALE,
        _ => 1.0,
    }
}

type SkinItem<'a> = (
    &'a ButtonStyle,
    &'a KitSkin,
    &'a KitParts,
    &'a Interaction,
    &'a Pressable,
    Option<&'a PreviewState>,
    &'a mut BorderColor,
    Option<&'a mut UiTransform>,
);

type SkinChanged = Or<(
    Changed<Interaction>,
    Changed<Pressable>,
    Changed<ButtonStyle>,
    Changed<PreviewState>,
    Added<KitParts>,
)>;

/// Repaints every kit-spawned control's own fill/border/scale and its parts
/// (label, icon, indicator, switch, thumb) from its state. Runs in
/// `UiSet::Paint` after [`paint_pressables`].
#[allow(clippy::type_complexity)]
pub(crate) fn paint_kit(
    mut controls: Query<SkinItem, SkinChanged>,
    mut labels: Query<(&mut TextColor, Option<&mut bevy::text::UnderlineColor>), Without<KitSkin>>,
    mut images: Query<&mut KitImage, Without<KitSkin>>,
    mut nodes: Query<
        (
            &mut Node,
            Option<&mut BackgroundColor>,
            Option<&mut BorderColor>,
            Option<&mut controls::Slide>,
        ),
        (Without<KitSkin>, Without<TextColor>),
    >,
) {
    for (style, skin, parts, interaction, pressable, preview, mut edge, transform) in &mut controls
    {
        let state = kit_state(*interaction, pressable, preview);
        if let Some(next) = skin_border(*skin, style, state) {
            let next = BorderColor::all(next);
            if *edge != next {
                *edge = next;
            }
        }
        if let Some(mut transform) = transform {
            let scale = Vec2::splat(press_scale(*skin, state));
            if transform.scale != scale {
                transform.scale = scale;
            }
        }
        let label_color = match skin {
            KitSkin::Input | KitSkin::Cycle | KitSkin::Row | KitSkin::ShopCard
                if state != ButtonState::Disabled =>
            {
                color::TEXT_PRIMARY
            }
            KitSkin::Toggle | KitSkin::Slider if state != ButtonState::Disabled => {
                color::TEXT_SECONDARY
            }
            KitSkin::HeroTile if state != ButtonState::Disabled => color::TEXT_PRIMARY,
            _ => theme::button_label_color(style.kind, style.selected, state),
        };
        if let Some((mut text, underline)) =
            parts.label.and_then(|label| labels.get_mut(label).ok())
        {
            if text.0 != label_color {
                text.0 = label_color;
            }
            if let (KitSkin::Tertiary, Some(mut underline)) = (skin, underline) {
                let next = if state == ButtonState::Hover {
                    color::GOLD_600
                } else {
                    Color::NONE
                };
                if underline.0 != next {
                    underline.0 = next;
                }
            }
        }
        for secondary in parts.extra.into_iter().flatten() {
            if let Ok((mut text, _)) = labels.get_mut(secondary) {
                let next = if state == ButtonState::Disabled {
                    color::TEXT_DISABLED
                } else {
                    color::TEXT_MUTED
                };
                if *skin != KitSkin::ShopCard && text.0 != next {
                    text.0 = next;
                }
            }
            if let Ok((mut node, ..)) = nodes.get_mut(secondary) {
                // Cycle chevrons hide when the control is disabled.
                let display = if state == ButtonState::Disabled {
                    Display::None
                } else {
                    Display::Flex
                };
                if *skin == KitSkin::Cycle && node.display != display {
                    node.display = display;
                }
            }
        }
        if let Some(mut icon) = parts.icon.and_then(|icon| images.get_mut(icon).ok()) {
            let tint = icon_tint(*skin, style, state);
            if icon.tint != tint {
                icon.tint = tint;
            }
        }
        match skin {
            KitSkin::Tab { .. } => {
                if let Some((mut node, ..)) = parts.track.and_then(|bar| nodes.get_mut(bar).ok()) {
                    let display = if style.selected {
                        Display::Flex
                    } else {
                        Display::None
                    };
                    if node.display != display {
                        node.display = display;
                    }
                }
            }
            KitSkin::Toggle => controls::paint_switch(style.selected, state, parts, &mut nodes),
            KitSkin::Slider => controls::paint_slider_thumb(state, parts, &mut nodes),
            KitSkin::HeroTile => game::paint_hero_tile(style.selected, state, parts, &mut nodes),
            KitSkin::Ability => {
                if let Some(mut rim) = parts.track.and_then(|rim| images.get_mut(rim).ok()) {
                    let tint = if state == ButtonState::Hover {
                        color::GOLD_300
                    } else {
                        color::GOLD_500
                    };
                    if rim.tint != tint {
                        rim.tint = tint;
                    }
                }
                if let Some((mut node, ..)) = parts.fill.and_then(|glow| nodes.get_mut(glow).ok()) {
                    let display = if style.selected {
                        Display::Flex
                    } else {
                        Display::None
                    };
                    if node.display != display {
                        node.display = display;
                    }
                }
            }
            _ => {}
        }
    }
}

/// The focus ring: one overlay node drawn around the focused kit button
/// (`border.focus` at `border.focus.offset` in `color.focus.ring`) with a
/// `color.focus.halo` child filling the 6 px around the control. An overlay
/// (rather than an `Outline` on the button) leaves outlines that screens own,
/// such as hero select's selected tiles, untouched.
#[derive(Component)]
pub(crate) struct FocusRing;

/// The halo under the ring.
#[derive(Component)]
pub(crate) struct FocusHalo;

/// A control whose ring sits further out (the slider's track row: 6).
#[derive(Component, Clone, Copy)]
pub(crate) struct FocusRingOffset(pub f32);

/// Halo width around a focused control (`components/button.md`: 6 px).
pub(crate) const FOCUS_HALO: f32 = space::S4 + border::FOCUS;

/// The ring (and halo) bundle for a node covering the focused control.
fn ring_bundle(offset: f32, corner: f32) -> impl Bundle {
    (
        Outline::new(Val::Px(border::FOCUS), Val::Px(offset), color::FOCUS_RING),
        bevy::ui::FocusPolicy::Pass,
        Pickable::IGNORE,
        children![(
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                border_radius: BorderRadius::all(Val::Px(corner)),
                ..default()
            },
            Outline::new(
                Val::Px(FOCUS_HALO + offset - border::FOCUS_OFFSET),
                Val::Px(0.0),
                theme::perceptual(color::FOCUS_HALO),
            ),
            Pickable::IGNORE,
            FocusHalo,
        )],
    )
}

/// Runs in `UiSet::Paint` next to [`paint_pressables`]: places the ring on
/// [`super::UiFocus::focused`] (clipped like the button, following its
/// corner radius) or hides it.
#[allow(clippy::type_complexity)]
pub(crate) fn paint_focus_ring(
    mut commands: Commands,
    focus: Option<Res<super::UiFocus>>,
    buttons: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        Option<&bevy::ui::CalculatedClip>,
        Option<&FocusRingOffset>,
    )>,
    mut rings: Query<(&mut Node, &mut Outline), With<FocusRing>>,
    mut halos: Query<(&mut Node, &mut Outline), (With<FocusHalo>, Without<FocusRing>)>,
) {
    let placed = focus
        .as_ref()
        .and_then(|focus| focus.focused())
        .and_then(|entity| buttons.get(entity).ok())
        .and_then(|(node, transform, clip, offset)| {
            let mut rect = super::focus::node_rect(node, transform);
            if let Some(clip) = clip {
                rect = rect.intersect(clip.clip);
            }
            let scale = node.inverse_scale_factor();
            let offset = offset.map_or(border::FOCUS_OFFSET, |offset| offset.0);
            let corner = node.border_radius().top_left * scale;
            (!rect.is_empty() && rect.min.is_finite() && rect.max.is_finite()).then(|| {
                (
                    Rect::from_corners(rect.min * scale, rect.max * scale),
                    offset,
                    corner,
                )
            })
        });
    let Ok((mut ring, mut outline)) = rings.single_mut() else {
        if placed.is_some() {
            commands.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    display: Display::None,
                    ..default()
                },
                ring_bundle(border::FOCUS_OFFSET, radius::MD),
                GlobalZIndex(5000),
                FocusRing,
                Name::new("UiFocusRing"),
            ));
        }
        return;
    };
    let next = match placed {
        Some((rect, offset, corner)) => {
            let next = Outline::new(Val::Px(border::FOCUS), Val::Px(offset), color::FOCUS_RING);
            if *outline != next {
                *outline = next;
            }
            for (mut halo, mut glow) in &mut halos {
                let radius = BorderRadius::all(Val::Px(corner));
                if halo.border_radius != radius {
                    halo.border_radius = radius;
                }
                let next = Outline::new(
                    Val::Px(FOCUS_HALO + offset - border::FOCUS_OFFSET),
                    Val::Px(0.0),
                    theme::perceptual(color::FOCUS_HALO),
                );
                if *glow != next {
                    *glow = next;
                }
            }
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(rect.min.x),
                top: Val::Px(rect.min.y),
                width: Val::Px(rect.width()),
                height: Val::Px(rect.height()),
                border_radius: BorderRadius::all(Val::Px(corner)),
                ..default()
            }
        }
        None => Node {
            position_type: PositionType::Absolute,
            display: Display::None,
            ..default()
        },
    };
    if *ring != next {
        *ring = next;
    }
}

/// A static ring on every control whose [`PreviewState`] says focused.
#[derive(Component)]
pub(crate) struct PreviewRing;

pub(crate) fn paint_preview_rings(
    mut commands: Commands,
    previews: Query<
        (Entity, &PreviewState, Option<&FocusRingOffset>, &Node),
        Changed<PreviewState>,
    >,
) {
    for (entity, preview, offset, node) in &previews {
        if !preview.focused {
            continue;
        }
        let offset = offset.map_or(border::FOCUS_OFFSET, |offset| offset.0);
        // The control's own corner (pills stay round); `radius.md` otherwise.
        let corner = match node.border_radius.top_left {
            Val::Px(corner) if corner > 0.0 => corner,
            _ => radius::MD,
        };
        commands.entity(entity).with_child((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                border_radius: BorderRadius::all(Val::Px(corner)),
                ..default()
            },
            ring_bundle(offset, corner),
            GlobalZIndex(4900),
            PreviewRing,
        ));
    }
}

fn button_bundle<T: UiActionT>(node: Node, kind: ButtonKind, action: T, id: TestId) -> impl Bundle {
    let style = ButtonStyle::new(kind);
    (
        Button,
        node,
        BorderColor::all(Color::NONE),
        BackgroundColor(style.idle_color()),
        style,
        UiAction(action),
        id,
    )
}

/// The skin of a button kind spawned by a kit button helper.
pub(crate) fn button_skin(kind: ButtonKind) -> KitSkin {
    match kind {
        ButtonKind::Link => KitSkin::Tertiary,
        kind if kind.slab().is_some() => KitSkin::Slab,
        _ => KitSkin::Native,
    }
}

/// The role of a button label by size (`type.button_lg` for the large and
/// hero primary, `type.button` otherwise).
fn label_style(size: ButtonSize, kind: ButtonKind) -> TextStyle {
    match (size, kind) {
        (_, ButtonKind::Link) => TextStyle::new(TextRole::Button).sized(TERTIARY_LABEL),
        (ButtonSize::Large | ButtonSize::Hero, _) => TextStyle::new(TextRole::ButtonLg),
        _ => TextStyle::new(TextRole::Button),
    }
}

/// Tertiary label: `type.button` at 14 px (`type.label` size on phone).
pub(crate) const TERTIARY_LABEL: super::tokens::Metric =
    super::tokens::Metric::new(14.0, TextRole::Label.style().size.phone);

/// Button sizes of `components/button.md`.
#[cfg_attr(not(feature = "qa"), allow(dead_code))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ButtonSize {
    /// `size.button.*` (46 / 44), min width 120.
    Regular,
    /// `size.button_lg.*` (60 / 56), min width 240 / 200: the one call to action.
    Large,
    /// `size.button_hero.*` (360×80 / 220×64): the screen-defining action.
    Hero,
}

/// Node of a kit button of `size` on a layout family (tertiary buttons are
/// `size.button_sm`).
pub(crate) fn button_node(size: ButtonSize, kind: ButtonKind, form: Form) -> Node {
    let (height, min_width, padding) = match (size, kind) {
        (_, ButtonKind::Link) => (size::BUTTON_SM_HEIGHT.at(form), 0.0, space::S12),
        (ButtonSize::Regular, _) => (
            size::BUTTON_HEIGHT.at(form),
            size::BUTTON_MIN_WIDTH,
            space::S16,
        ),
        (ButtonSize::Large, _) => (
            size::BUTTON_LG_HEIGHT.at(form),
            size::BUTTON_LG_MIN_WIDTH.at(form),
            space::S24,
        ),
        (ButtonSize::Hero, _) => (
            size::BUTTON_HERO_HEIGHT.at(form),
            size::BUTTON_HERO_WIDTH.at(form),
            space::S24,
        ),
    };
    Node {
        height: Val::Px(height),
        min_width: Val::Px(min_width),
        padding: UiRect::horizontal(Val::Px(padding)),
        column_gap: Val::Px(space::S8),
        flex_shrink: 0.0,
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        border_radius: BorderRadius::all(Val::Px(radius::MD)),
        ..default()
    }
}

/// A kit button with an optional leading icon (`size.icon.lg` for the large
/// sizes, `size.icon.md` otherwise). The label is `{id}Label` when the
/// owner rewrites it (`label_extra`), otherwise unnamed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    node: Node,
    label: impl UiLabel,
    style: TextStyle,
    kind: ButtonKind,
    icon: Option<Icon>,
    action: T,
    id: TestId,
    label_extra: impl Bundle,
) -> Entity {
    let size = if style.role == TextRole::ButtonLg {
        size::ICON_LG
    } else {
        size::ICON_MD
    };
    let mut parts = KitParts::default();
    let mut button = parent.spawn((button_bundle(node, kind, action, id), button_skin(kind)));
    button.with_children(|button| {
        if let Some(icon) = icon {
            parts.icon = Some(
                button
                    .spawn(icon_node(
                        icon,
                        size,
                        theme::button_label_color(kind, false, ButtonState::Idle),
                    ))
                    .id(),
            );
        }
        parts.label = Some(
            button
                .spawn((
                    label.into_text(),
                    theme::styled_text(style),
                    TextColor(theme::button_label_color(kind, false, ButtonState::Idle)),
                    TextLayout::new_with_justify(Justify::Center),
                    label_extra,
                ))
                .id(),
        );
        if kind == ButtonKind::Link {
            // The tertiary hover underline (1 px `gold.600`), hidden until hover.
            if let Some(label) = parts.label {
                button.commands().entity(label).insert((
                    bevy::text::Underline,
                    bevy::text::UnderlineColor(Color::NONE),
                ));
            }
        }
    });
    button.insert(parts).id()
}

/// A tinted icon node `size` square.
pub(crate) fn icon_node(icon: Icon, size: f32, tint: Color) -> impl Bundle {
    (
        Node {
            width: Val::Px(size),
            height: Val::Px(size),
            flex_shrink: 0.0,
            ..default()
        },
        KitImage::icon(icon, tint),
        Pickable::IGNORE,
    )
}

fn menu_button_node(kind: ButtonKind) -> Node {
    Node {
        width: Val::Px(metric::MENU_W),
        max_width: Val::Percent(100.0),
        ..button_node(ButtonSize::Regular, kind, Form::Desktop)
    }
}

/// A full-width menu button with a centred label.
pub(crate) fn button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    label: impl UiLabel,
    kind: ButtonKind,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    spawn_button(
        parent,
        menu_button_node(kind),
        label,
        label_style(ButtonSize::Regular, kind),
        kind,
        None,
        action,
        id.into(),
        (),
    )
}

/// [`button`] whose label carries `label_marker` and its own id, for a
/// caption the owning module rewrites (a mute/unmute toggle). The kit keeps
/// such a label as written (it never fights the owner's writes).
pub(crate) fn button_with_label<T: UiActionT, M: Component>(
    parent: &mut ChildSpawnerCommands,
    label: impl UiLabel,
    kind: ButtonKind,
    action: T,
    id: impl Into<TestId>,
    label_marker: M,
    label_id: impl Into<TestId>,
) -> Entity {
    let style = TextStyle {
        keep_case: true,
        ..label_style(ButtonSize::Regular, kind)
    };
    spawn_button(
        parent,
        menu_button_node(kind),
        label,
        style,
        kind,
        None,
        action,
        id.into(),
        (label_marker, label_id.into()),
    )
}

/// A round icon button (`components/icon-button.md`) for a panel header;
/// the close `×` is the `nav/x` icon, any other glyph is drawn as text in
/// the icon colour. Header buttons keep the 44 px touch minimum on both
/// profiles (the pause menu's reachability contract); new code that wants
/// the 40 px desktop size uses [`controls::icon_button`].
pub(crate) fn icon_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    glyph: &str,
    kind: ButtonKind,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    match glyph {
        "×" => controls::sized_icon_button(parent, Icon::NavX, Form::Phone, kind, action, id),
        glyph => {
            let side = size::ICON_BUTTON.phone;
            let mut parts = KitParts::default();
            let mut button = parent.spawn((
                button_bundle(
                    controls::icon_button_node(side, radius::PILL),
                    kind,
                    action,
                    id.into(),
                ),
                KitSkin::Icon,
                NoSlab,
            ));
            button.with_children(|button| {
                parts.label = Some(
                    button
                        .spawn((
                            Text::new(glyph),
                            theme::role_text(TextRole::Heading),
                            TextColor(color::TEXT_GOLD),
                        ))
                        .id(),
                );
            });
            button.insert(parts).id()
        }
    }
}

/// A centred value the owning module rewrites through `marker`
/// (`type.number`, min width 64: `components/stepper.md`).
pub(crate) fn value_label<M: Component>(
    parent: &mut ChildSpawnerCommands,
    value: String,
    marker: M,
    id: impl Into<TestId>,
) -> Entity {
    parent
        .spawn((
            Text::new(value),
            Node {
                min_width: Val::Px(controls::STEPPER_VALUE_MIN),
                flex_shrink: 0.0,
                ..default()
            },
            TextLayout::new_with_justify(Justify::Center),
            theme::role_text(TextRole::Number),
            TextColor(color::TEXT_PRIMARY),
            marker,
            id.into(),
        ))
        .id()
}

/// `label  [-] value [+]` (`components/stepper.md`); the controls are
/// `{id}-Down`, `{id}-Value` and `{id}-Up`. Returns the row.
pub(crate) fn adjust_row<T: UiActionT, M: Component>(
    parent: &mut ChildSpawnerCommands,
    label: impl UiLabel,
    value: String,
    value_marker: M,
    decrease: T,
    increase: T,
    id: impl Into<TestId>,
) -> Entity {
    let id = id.into();
    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                flex_shrink: 0.0,
                column_gap: Val::Px(space::S8),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            id.clone(),
        ))
        .with_children(|row| {
            row.spawn((
                label.into_text(),
                Node {
                    width: Val::Px(controls::STEPPER_LABEL_W),
                    flex_shrink: 0.0,
                    ..default()
                },
                theme::role_text(TextRole::Label),
                TextColor(color::TEXT_SECONDARY),
            ));
            controls::stepper_button(row, Icon::NavMinus, decrease, id.child("-Down"));
            value_label(row, value, value_marker, id.child("-Value"));
            controls::stepper_button(row, Icon::NavPlus, increase, id.child("-Up"));
        })
        .id()
}

/// A menu-width secondary button showing `label` on the left and the
/// current `value` on the right; pressing it is the toggle. The button is
/// `{id}Button` and the value `{id}Value` (rewritten by the owner, so kept as
/// written). Returns the button.
pub(crate) fn toggle_row<T: UiActionT, M: Component>(
    parent: &mut ChildSpawnerCommands,
    label: impl UiLabel,
    value: &str,
    value_marker: M,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    let id = id.into();
    let node = Node {
        justify_content: JustifyContent::SpaceBetween,
        ..menu_button_node(ButtonKind::Secondary)
    };
    let mut parts = KitParts::default();
    let mut button = parent.spawn((
        button_bundle(node, ButtonKind::Secondary, action, id.child("Button")),
        KitSkin::Slab,
    ));
    button.with_children(|button| {
        parts.label = Some(
            button
                .spawn((
                    label.into_text(),
                    theme::role_text(TextRole::Label),
                    TextColor(color::TEXT_PRIMARY),
                ))
                .id(),
        );
        button.spawn((
            Text::new(value),
            theme::styled_text(TextStyle::keep_case(TextRole::Label)),
            TextColor(color::TEXT_GOLD),
            value_marker,
            id.child("Value"),
        ));
    });
    button.insert(parts).id()
}

/// Font size a front-end label was designed at; `frontend::widgets` applies
/// `metric::menu_font` to it every frame (a readable minimum on a phone).
#[derive(Component)]
pub(crate) struct MenuTypography {
    pub size: f32,
    pub heading: bool,
}

/// Height a front-end control was designed at; `metric::menu_control_height`
/// raises it to the touch minimum on a phone, in the same pass as
/// [`MenuTypography`].
#[derive(Component)]
pub(crate) struct MenuControl {
    pub height: f32,
}

/// A front-end screen label in `color`, with its phone readability metric.
pub(crate) fn screen_label(text: impl UiLabel, size: f32, color: Color) -> impl Bundle {
    (
        text.into_text(),
        theme::text(size),
        TextColor(color),
        MenuTypography {
            size,
            heading: false,
        },
    )
}

/// A front-end screen button: `Primary` is the one large call to action
/// (`size.button_lg`), `Link` a tertiary text action, everything else a
/// regular slab sized to its label.
pub(crate) fn screen_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    text: impl UiLabel,
    kind: ButtonKind,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    spawn_screen_button(
        parent,
        text,
        ButtonStyle::new(kind),
        action,
        id.into(),
        false,
    )
}

/// A grid tile whose selected state is owned by the screen (it flips
/// `ButtonStyle::selected`; the painter repaints).
pub(crate) fn screen_tile<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    text: impl UiLabel,
    selected: bool,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    compact_screen_tile(parent, text, selected, action, id, false)
}

/// [`screen_tile`] that shrinks on a phone (the collection's clip strip).
pub(crate) fn compact_screen_tile<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    text: impl UiLabel,
    selected: bool,
    action: T,
    id: impl Into<TestId>,
    compact: bool,
) -> Entity {
    let style = ButtonStyle {
        kind: ButtonKind::Tile,
        selected,
    };
    spawn_screen_button(parent, text, style, action, id.into(), compact)
}

fn spawn_screen_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    text: impl UiLabel,
    style: ButtonStyle,
    action: T,
    id: TestId,
    compact: bool,
) -> Entity {
    let kind = style.kind;
    let (node, label) = match kind {
        ButtonKind::Tile => (
            Node {
                height: Val::Px(metric::TOUCH_MIN),
                min_width: Val::Px(if compact {
                    controls::COMPACT_TILE_MIN_W
                } else {
                    size::BUTTON_MIN_WIDTH
                }),
                padding: UiRect::axes(
                    Val::Px(if compact { space::S8 } else { space::S16 }),
                    Val::Px(space::S8),
                ),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::all(Val::Px(radius::MD)),
                ..default()
            },
            TextStyle::new(TextRole::Label),
        ),
        ButtonKind::Primary => (
            button_node(ButtonSize::Large, kind, Form::Desktop),
            label_style(ButtonSize::Large, kind),
        ),
        _ => (
            button_node(ButtonSize::Regular, kind, Form::Desktop),
            label_style(ButtonSize::Regular, kind),
        ),
    };
    let height = match node.height {
        Val::Px(height) => height,
        _ => metric::TOUCH_MIN,
    };
    let entity = spawn_button(parent, node, text, label, kind, None, action, id, ());
    parent.commands().entity(entity).insert((
        style,
        BackgroundColor(style.idle_color()),
        MenuControl { height },
    ));
    entity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Probe {
        Down,
        Up,
        Toggle,
    }
    #[derive(Component)]
    struct Value;

    /// Touch has no hover: a held finger paints pressed, a hovering pointer
    /// in touch mode paints idle; a blocked button paints idle.
    #[test]
    fn touch_mode_never_paints_hover() {
        let touch = Pressable {
            touch_mode: true,
            ..Pressable::default()
        };
        assert_eq!(
            kit_state(Interaction::Pressed, &touch, None),
            ButtonState::Pressed
        );
        assert_eq!(
            kit_state(Interaction::Hovered, &touch, None),
            ButtonState::Idle
        );
        assert_eq!(
            kit_state(Interaction::None, &touch, None),
            ButtonState::Idle
        );
        let tapped = Pressable {
            activated: true,
            ..touch
        };
        assert_eq!(
            kit_state(Interaction::None, &tapped, None),
            ButtonState::Pressed
        );
        let blocked = Pressable {
            blocked: true,
            ..touch
        };
        assert_eq!(
            kit_state(Interaction::Pressed, &blocked, None),
            ButtonState::Idle
        );
        let mouse = Pressable::default();
        assert_eq!(
            kit_state(Interaction::Hovered, &mouse, None),
            ButtonState::Hover
        );
    }

    #[test]
    fn widgets_name_their_controls_and_paint_from_the_effective_interaction() {
        let mut app = App::new();
        app.add_message::<super::super::SyntheticPress>()
            .add_systems(Update, (paint_pressables, paint_kit).chain());
        let root = app.world_mut().spawn(Node::default()).id();
        app.world_mut()
            .commands()
            .entity(root)
            .with_children(|parent| {
                adjust_row(
                    parent,
                    "Level",
                    "5".into(),
                    Value,
                    Probe::Down,
                    Probe::Up,
                    "Row",
                );
                toggle_row(parent, "God mode", "OFF", Value, Probe::Toggle, "Practice");
                button(parent, "Resume", ButtonKind::Primary, Probe::Up, "Resume");
            });
        app.world_mut().flush();
        app.update();
        let names: Vec<String> = app
            .world_mut()
            .query::<crate::ui::test_id::NodeKey>()
            .iter(app.world())
            .map(|name| name.as_str().to_owned())
            .collect();
        for wanted in [
            "Row",
            "Row-Down",
            "Row-Value",
            "Row-Up",
            "PracticeButton",
            "PracticeValue",
            "Resume",
        ] {
            assert!(names.iter().any(|name| name == wanted), "{wanted} missing");
        }
        let resume = super::super::test_id::harness::find(app.world_mut(), "Resume").unwrap();
        assert_eq!(
            app.world().get::<BackgroundColor>(resume).unwrap().0,
            theme::PRIMARY
        );
        let label = app.world().get::<KitParts>(resume).unwrap().label.unwrap();
        assert_eq!(
            app.world().get::<TextColor>(label).unwrap().0,
            color::TEXT_ON_PRIMARY
        );
        app.world_mut()
            .entity_mut(resume)
            .insert(Interaction::Hovered);
        app.update();
        assert_eq!(
            app.world().get::<BackgroundColor>(resume).unwrap().0,
            theme::PRIMARY_HOVER
        );
        // A disabled button never lights, and its label dims.
        app.world_mut()
            .get_mut::<Pressable>(resume)
            .unwrap()
            .disabled = true;
        app.update();
        assert_eq!(
            app.world().get::<BackgroundColor>(resume).unwrap().0,
            theme::PRIMARY
        );
        assert_eq!(
            app.world().get::<TextColor>(label).unwrap().0,
            color::TEXT_DISABLED
        );
    }

    #[test]
    fn slabs_follow_the_state_and_team_buttons_get_their_bar() {
        use crate::domain::Team;
        let mut app = App::new();
        app.add_systems(Update, paint_slabs);
        let primary = app
            .world_mut()
            .spawn((
                Button,
                Pressable::default(),
                ButtonStyle::new(ButtonKind::Primary),
            ))
            .id();
        let team = app
            .world_mut()
            .spawn((
                Button,
                Pressable::default(),
                ButtonStyle::new(ButtonKind::Team(Team::Blue)),
            ))
            .id();
        let owned = app
            .world_mut()
            .spawn((
                Button,
                Pressable::default(),
                ButtonStyle::new(ButtonKind::Secondary),
                ImageNode::default(),
            ))
            .id();
        let tile = app
            .world_mut()
            .spawn((
                Button,
                Pressable::default(),
                ButtonStyle::new(ButtonKind::Tile),
            ))
            .id();
        app.update();
        let frame = |app: &App, entity| app.world().get::<KitImage>(entity).map(|kit| kit.source);
        use crate::ui::kit_assets::KitSource;
        assert_eq!(
            frame(&app, primary),
            Some(KitSource::Frame(Frame::ButtonPrimary))
        );
        assert_eq!(frame(&app, owned), None, "a screen-owned image is kept");
        assert_eq!(frame(&app, tile), None, "tiles are native");
        app.world_mut()
            .entity_mut(primary)
            .insert(Interaction::Pressed);
        app.update();
        assert_eq!(
            frame(&app, primary),
            Some(KitSource::Frame(Frame::ButtonPrimaryPressed))
        );
        let bars = app
            .world_mut()
            .query_filtered::<&ChildOf, With<TeamBar>>()
            .iter(app.world())
            .map(ChildOf::parent)
            .collect::<Vec<_>>();
        assert_eq!(bars, [team]);
        assert_eq!(
            slab_frame(SlabFamily::Secondary, ButtonState::Idle, true),
            Frame::ButtonSecondaryHover
        );
        assert_eq!(
            slab_frame(SlabFamily::Danger, ButtonState::Disabled, false),
            Frame::ButtonDangerDisabled
        );
    }

    #[test]
    fn preview_states_pin_the_painters() {
        let mut app = App::new();
        app.add_systems(Update, (paint_pressables, paint_kit).chain());
        let tab = app
            .world_mut()
            .spawn((
                Button,
                Pressable::default(),
                ButtonStyle::new(ButtonKind::Tile),
                KitSkin::Tab { rail: true },
                KitParts::default(),
                BorderColor::all(Color::NONE),
                PreviewState {
                    state: ButtonState::Hover,
                    focused: false,
                },
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<BackgroundColor>(tab).unwrap().0,
            color::SURFACE_HOVER
        );
        app.world_mut()
            .get_mut::<ButtonStyle>(tab)
            .unwrap()
            .selected = true;
        app.update();
        assert_eq!(
            app.world().get::<BackgroundColor>(tab).unwrap().0,
            color::SURFACE_SELECTED
        );
    }
}
