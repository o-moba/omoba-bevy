//! Interactive kit components: icon buttons, stepper buttons, tabs (top bar
//! and side rail), the toggle switch, the slider, the cycle row and the text
//! field (`omoba-ui/handoff/components/{icon-button,stepper,tab,toggle,
//! slider,cycle-row,text-input}.md`). Every one is a focusable kit button, so
//! the gamepad focus ring reaches it; the slider and the cycle row take
//! Left/Right themselves ([`super::super::focus::FocusAdjustable`]).
// i18n-strict
// Kit parts that screen steps adopt; the kit gallery (`qa` feature) uses all of them.
#![cfg_attr(not(feature = "qa"), allow(dead_code))]
use std::time::Duration;

use bevy::prelude::*;

use super::{ButtonStyle, KitParts, KitSkin, NoSlab, button_bundle, icon_node};
use crate::i18n::UiLabel;
use crate::ui::{
    Pressable, SyntheticPress, TestId,
    action::UiActionT,
    focus::{FocusAdjust, FocusAdjustable, FocusSkip},
    kit_assets::Icon,
    theme::{self, ButtonKind, ButtonState, Form, TextStyle},
    tokens::{TextRole, border, color, motion, radius, size, space},
};

/// Stepper value column (`components/stepper.md`: min width 64).
pub(crate) const STEPPER_VALUE_MIN: f32 = space::S64;
/// Legacy stepper label column of the pause/sandbox rows.
pub(crate) const STEPPER_LABEL_W: f32 = 110.0;
/// A compact screen tile's minimum width (collection clip strip).
pub(crate) const COMPACT_TILE_MIN_W: f32 = 86.0;
/// Settings row height (`toggle.md`, `cycle-row.md`).
pub(crate) const SETTINGS_ROW_H: f32 = space::S48;
/// Label column of settings rows (`slider.md`: max 160).
pub(crate) const SETTINGS_LABEL_W: f32 = 160.0;
/// Cycle control and chevron sizes (`cycle-row.md`).
pub(crate) const CYCLE_W: super::super::tokens::Metric =
    super::super::tokens::Metric::new(280.0, 240.0);
pub(crate) const CHEVRON: f32 = 36.0;
/// Slider default width and the value column (`slider.md`).
pub(crate) const SLIDER_W: super::super::tokens::Metric =
    super::super::tokens::Metric::new(280.0, 240.0);
pub(crate) const SLIDER_VALUE_W: f32 = space::S48;
/// Slider step for Left/Right (5 %) and its focus ring offset.
pub(crate) const SLIDER_STEP: f32 = 0.05;
pub(crate) const SLIDER_RING_OFFSET: f32 = 6.0;
/// Thumb scale while dragged.
pub(crate) const THUMB_PRESSED_SCALE: f32 = 1.15;
/// Toggle knob and its inset (`toggle.md`: knob 18 inset 3; press stretches 4).
pub(crate) const KNOB: f32 = 18.0;
pub(crate) const KNOB_INSET: f32 = 3.0;
pub(crate) const KNOB_STRETCH: f32 = space::S4;
/// Hold-to-repeat for Left/Right on a focused slider or cycle row.
const REPEAT_DELAY: Duration = Duration::from_millis(400);
const REPEAT_EVERY: Duration = Duration::from_millis(80);

pub(crate) fn icon_button_node(side: f32, corner: f32) -> Node {
    Node {
        width: Val::Px(side),
        height: Val::Px(side),
        flex_shrink: 0.0,
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        border: UiRect::all(Val::Px(border::HAIRLINE)),
        border_radius: BorderRadius::all(Val::Px(corner)),
        ..default()
    }
}

fn spawn_icon_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    icon: Icon,
    side: f32,
    corner: f32,
    kind: ButtonKind,
    action: T,
    id: TestId,
) -> Entity {
    let mut parts = KitParts::default();
    let mut button = parent.spawn((
        button_bundle(icon_button_node(side, corner), kind, action, id),
        KitSkin::Icon,
        NoSlab,
    ));
    button.with_children(|button| {
        parts.icon = Some(
            button
                .spawn(icon_node(icon, size::ICON_MD, color::TEXT_GOLD))
                .id(),
        );
    });
    button.insert(parts).id()
}

/// A round icon button (`icon-button.md`): 40 desktop (the phone layout
/// raises it to 44), `color.surface.2`, gold hairline, 20 px gold icon.
pub(crate) fn icon_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    icon: Icon,
    kind: ButtonKind,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    sized_icon_button(parent, icon, Form::Desktop, kind, action, id)
}

/// [`icon_button`] at a layout family's size (`size.icon_button.*`).
pub(crate) fn sized_icon_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    icon: Icon,
    form: Form,
    kind: ButtonKind,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    spawn_icon_button(
        parent,
        icon,
        size::ICON_BUTTON.at(form),
        radius::PILL,
        kind,
        action,
        id.into(),
    )
}

/// A stepper `−`/`+` button: 44 × 44 on both profiles, `radius.md`.
pub(crate) fn stepper_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    icon: Icon,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    spawn_icon_button(
        parent,
        icon,
        theme::metric::ADJUST_BTN,
        radius::MD,
        ButtonKind::Secondary,
        action,
        id.into(),
    )
}

/// Where a tab sits (`tab.md`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TabPlacement {
    /// Top bar: sized to its label (≥ 96), 3 px gold bar under the selected one.
    Top,
    /// Side rail: `size.tab.rail_width.*`, 3 px bar on the left edge, filled
    /// when selected.
    Rail,
}

/// Top tab minimum width (`tab.md`: top tabs size to content, ≥ 96).
pub(crate) const TOP_TAB_MIN_W: f32 = 96.0;
/// The selection indicator thickness.
pub(crate) const TAB_INDICATOR: f32 = 3.0;

/// A tab; the screen owns the selection (`ButtonStyle::selected`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn tab<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    label: impl UiLabel,
    icon: Option<Icon>,
    placement: TabPlacement,
    form: Form,
    selected: bool,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    let rail = placement == TabPlacement::Rail;
    let node = Node {
        height: Val::Px(size::TAB_HEIGHT.at(form)),
        width: if rail {
            Val::Px(size::TAB_RAIL_WIDTH.at(form))
        } else {
            Val::Auto
        },
        min_width: Val::Px(if rail { 0.0 } else { TOP_TAB_MIN_W }),
        padding: UiRect::horizontal(Val::Px(space::S16)),
        column_gap: Val::Px(space::S8),
        flex_shrink: 0.0,
        align_items: AlignItems::Center,
        justify_content: if rail {
            JustifyContent::FlexStart
        } else {
            JustifyContent::Center
        },
        border_radius: if rail {
            BorderRadius::right(Val::Px(radius::MD))
        } else {
            BorderRadius::all(Val::Px(radius::SM))
        },
        ..default()
    };
    let style = ButtonStyle {
        kind: ButtonKind::Tile,
        selected,
    };
    let mut parts = KitParts::default();
    let mut button = parent.spawn((
        button_bundle(node, ButtonKind::Tile, action, id.into()),
        KitSkin::Tab { rail },
    ));
    button.insert(style);
    button.with_children(|button| {
        if let Some(icon) = icon {
            parts.icon = Some(
                button
                    .spawn(icon_node(icon, size::ICON_MD, color::TEXT_MUTED))
                    .id(),
            );
        }
        parts.label = Some(
            button
                .spawn((
                    label.into_text(),
                    theme::role_text(TextRole::Button),
                    TextColor(color::TEXT_SECONDARY),
                ))
                .id(),
        );
        let bar = if rail {
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                width: Val::Px(TAB_INDICATOR),
                display: Display::None,
                ..default()
            }
        } else {
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Px(0.0),
                height: Val::Px(TAB_INDICATOR),
                display: Display::None,
                ..default()
            }
        };
        parts.track = Some(
            button
                .spawn((bar, BackgroundColor(color::GOLD_500), Pickable::IGNORE))
                .id(),
        );
    });
    button.insert(parts).id()
}

/// A settings row with a switch (`toggle.md`): the whole 48 px row is the
/// hit target and the focusable control; `ButtonStyle::selected` is "on".
/// Activating it sends `action`; the owner flips `selected`.
pub(crate) fn toggle<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    label: impl UiLabel,
    on: bool,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    let node = Node {
        height: Val::Px(SETTINGS_ROW_H),
        width: Val::Percent(100.0),
        padding: UiRect::horizontal(Val::Px(space::S8)),
        justify_content: JustifyContent::SpaceBetween,
        align_items: AlignItems::Center,
        border_radius: BorderRadius::all(Val::Px(radius::MD)),
        ..default()
    };
    let mut parts = KitParts::default();
    let mut button = parent.spawn((
        button_bundle(node, ButtonKind::Secondary, action, id.into()),
        KitSkin::Toggle,
        NoSlab,
    ));
    button.insert(ButtonStyle {
        kind: ButtonKind::Secondary,
        selected: on,
    });
    button.with_children(|row| {
        parts.label = Some(
            row.spawn((
                label.into_text(),
                theme::role_text(TextRole::Label),
                TextColor(color::TEXT_SECONDARY),
            ))
            .id(),
        );
        let mut knob = None;
        let track = row
            .spawn((
                Node {
                    width: Val::Px(size::TOGGLE_WIDTH),
                    height: Val::Px(size::TOGGLE_HEIGHT),
                    flex_shrink: 0.0,
                    border: UiRect::all(Val::Px(border::HAIRLINE)),
                    border_radius: BorderRadius::all(Val::Px(size::TOGGLE_HEIGHT / 2.0)),
                    ..default()
                },
                BackgroundColor(color::SURFACE_3),
                BorderColor::all(color::BORDER_SUBTLE),
                Pickable::IGNORE,
            ))
            .with_children(|track| {
                let left = knob_left(on, false);
                knob = Some(
                    track
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(left),
                                top: Val::Px(KNOB_INSET - border::HAIRLINE),
                                width: Val::Px(KNOB),
                                height: Val::Px(KNOB),
                                border_radius: BorderRadius::all(Val::Px(KNOB / 2.0)),
                                ..default()
                            },
                            BackgroundColor(color::TEXT_MUTED),
                            Slide { target: left },
                            Pickable::IGNORE,
                        ))
                        .id(),
                );
            })
            .id();
        parts.track = Some(track);
        parts.knob = knob;
    });
    button.insert(parts).id()
}

/// The knob's resting `left` inside the track's border box.
fn knob_left(on: bool, pressed: bool) -> f32 {
    let inner = size::TOGGLE_WIDTH - 2.0 * border::HAIRLINE;
    let width = KNOB + if pressed { KNOB_STRETCH } else { 0.0 };
    if on {
        inner - (KNOB_INSET - border::HAIRLINE) - width
    } else {
        KNOB_INSET - border::HAIRLINE
    }
}

/// A node sliding its `left` to `target` over `motion.duration.hover`.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub(crate) struct Slide {
    pub target: f32,
}

pub(crate) fn slide_nodes(time: Res<Time>, mut nodes: Query<(&Slide, &mut Node)>) {
    let span = size::TOGGLE_WIDTH - KNOB;
    let speed = span / motion::DURATION_HOVER.as_secs_f32();
    for (slide, mut node) in &mut nodes {
        let Val::Px(left) = node.left else {
            node.left = Val::Px(slide.target);
            continue;
        };
        if (left - slide.target).abs() < 0.01 {
            continue;
        }
        let step = speed * time.delta_secs();
        let next = if left < slide.target {
            (left + step).min(slide.target)
        } else {
            (left - step).max(slide.target)
        };
        node.left = Val::Px(next);
    }
}

/// Track and knob colours of the switch (`toggle.md`).
pub(crate) fn paint_switch(
    on: bool,
    state: ButtonState,
    parts: &KitParts,
    nodes: &mut Query<
        (
            &mut Node,
            Option<&mut BackgroundColor>,
            Option<&mut BorderColor>,
            Option<&mut Slide>,
        ),
        (Without<KitSkin>, Without<TextColor>),
    >,
) {
    use ButtonState::*;
    let (track, edge, knob) = match (state, on) {
        (Disabled, _) => (color::SURFACE_2, color::BORDER_SUBTLE, color::TEXT_DISABLED),
        (_, true) => (color::EMERALD_600, color::GOLD_600, color::GOLD_300),
        (_, false) => (color::SURFACE_3, color::BORDER_SUBTLE, color::TEXT_MUTED),
    };
    let edge = if state == Hover {
        color::GOLD_500
    } else {
        edge
    };
    if let Some((_, Some(mut fill), Some(mut border), _)) =
        parts.track.and_then(|track| nodes.get_mut(track).ok())
    {
        if fill.0 != track {
            fill.0 = track;
        }
        if *border != BorderColor::all(edge) {
            *border = BorderColor::all(edge);
        }
    }
    if let Some((mut node, Some(mut fill), _, Some(mut slide))) =
        parts.knob.and_then(|knob| nodes.get_mut(knob).ok())
    {
        if fill.0 != knob {
            fill.0 = knob;
        }
        let pressed = state == Pressed;
        let width = Val::Px(KNOB + if pressed { KNOB_STRETCH } else { 0.0 });
        if node.width != width {
            node.width = width;
        }
        let target = knob_left(on, pressed);
        if slide.target != target {
            slide.target = target;
        }
    }
}

/// A slider's value in `0..=1`; `step` is the Left/Right increment.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub(crate) struct Slider {
    pub value: f32,
    pub step: f32,
}

/// A slider moved (drag, tap on the track, Left/Right on a focused one).
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub(crate) struct SliderChanged {
    pub slider: Entity,
    pub value: f32,
}

/// `label · slider · value` (`slider.md`). The row is the focusable control
/// (ring offset 6), the thumb follows [`Slider::value`] and the value shows
/// it in percent. The owner reads [`SliderChanged`] or the component.
pub(crate) fn slider(
    parent: &mut ChildSpawnerCommands,
    label: impl UiLabel,
    value: f32,
    form: Form,
    id: impl Into<TestId>,
) -> Entity {
    let mut parts = KitParts::default();
    let thumb = size::SLIDER_THUMB.at(form);
    let mut row = parent.spawn((
        Button,
        Pressable::default(),
        Node {
            min_height: Val::Px(if form == Form::Phone {
                size::SLIDER_HIT_PHONE
            } else {
                SETTINGS_ROW_H
            }),
            column_gap: Val::Px(space::S16),
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(Color::NONE),
        BorderColor::all(Color::NONE),
        ButtonStyle::new(ButtonKind::Secondary),
        NoSlab,
        KitSkin::Slider,
        FocusAdjustable,
        super::FocusRingOffset(SLIDER_RING_OFFSET),
        Slider {
            value: value.clamp(0.0, 1.0),
            step: SLIDER_STEP,
        },
        id.into(),
    ));
    row.with_children(|row| {
        parts.label = Some(
            row.spawn((
                label.into_text(),
                Node {
                    max_width: Val::Px(SETTINGS_LABEL_W),
                    ..default()
                },
                theme::role_text(TextRole::Label),
                TextColor(color::TEXT_SECONDARY),
            ))
            .id(),
        );
        let mut fill = None;
        let mut knob = None;
        parts.track = Some(
            row.spawn((
                Node {
                    width: Val::Px(SLIDER_W.at(form)),
                    height: Val::Px(size::SLIDER_TRACK),
                    flex_shrink: 0.0,
                    border_radius: BorderRadius::all(Val::Px(size::SLIDER_TRACK / 2.0)),
                    ..default()
                },
                BackgroundColor(color::BAR_TRACK),
                Pickable::IGNORE,
            ))
            .with_children(|track| {
                fill = Some(
                    track
                        .spawn((
                            Node {
                                height: Val::Percent(100.0),
                                width: Val::Percent(value * 100.0),
                                border_radius: BorderRadius::all(Val::Px(size::SLIDER_TRACK / 2.0)),
                                ..default()
                            },
                            BackgroundColor(color::EMERALD_400),
                            Pickable::IGNORE,
                        ))
                        .id(),
                );
                knob = Some(
                    track
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Percent(value * 100.0),
                                top: Val::Px((size::SLIDER_TRACK - thumb) / 2.0),
                                width: Val::Px(thumb),
                                height: Val::Px(thumb),
                                margin: UiRect::left(Val::Px(-thumb / 2.0)),
                                border: UiRect::all(Val::Px(border::FRAME)),
                                border_radius: BorderRadius::all(Val::Px(thumb / 2.0)),
                                ..default()
                            },
                            BackgroundColor(color::GOLD_400),
                            BorderColor::all(color::GOLD_700),
                            UiTransform::IDENTITY,
                            Pickable::IGNORE,
                        ))
                        .id(),
                );
            })
            .id(),
        );
        parts.fill = fill;
        parts.knob = knob;
        parts.extra[0] = Some(
            row.spawn((
                Text::new(percent(value)),
                Node {
                    width: Val::Px(SLIDER_VALUE_W),
                    ..default()
                },
                TextLayout::new_with_justify(Justify::Right),
                theme::role_text(TextRole::Number),
                TextColor(color::TEXT_PRIMARY),
            ))
            .id(),
        );
    });
    row.insert(parts).id()
}

fn percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

/// Thumb colour and drag scale (`slider.md`); the fill dims when disabled.
pub(crate) fn paint_slider_thumb(
    state: ButtonState,
    parts: &KitParts,
    nodes: &mut Query<
        (
            &mut Node,
            Option<&mut BackgroundColor>,
            Option<&mut BorderColor>,
            Option<&mut Slide>,
        ),
        (Without<KitSkin>, Without<TextColor>),
    >,
) {
    let (thumb, fill) = match state {
        ButtonState::Disabled => (color::TEXT_DISABLED, color::TEXT_DISABLED),
        ButtonState::Hover => (color::GOLD_300, color::EMERALD_400),
        _ => (color::GOLD_400, color::EMERALD_400),
    };
    if let Some((_, Some(mut color), ..)) = parts.knob.and_then(|knob| nodes.get_mut(knob).ok()) {
        if color.0 != thumb {
            color.0 = thumb;
        }
    }
    if let Some((_, Some(mut color), ..)) = parts.fill.and_then(|fill| nodes.get_mut(fill).ok()) {
        if color.0 != fill {
            color.0 = fill;
        }
    }
}

/// Thumb scale while dragged.
pub(crate) fn scale_dragged_thumbs(
    sliders: Query<
        (
            &Interaction,
            &Pressable,
            &KitParts,
            Option<&super::PreviewState>,
        ),
        (With<Slider>, Or<(Changed<Interaction>, Changed<Pressable>)>),
    >,
    mut thumbs: Query<&mut UiTransform, Without<Slider>>,
) {
    for (interaction, pressable, parts, preview) in &sliders {
        let pressed = super::kit_state(*interaction, pressable, preview) == ButtonState::Pressed;
        if let Some(mut transform) = parts.knob.and_then(|knob| thumbs.get_mut(knob).ok()) {
            let scale = Vec2::splat(if pressed { THUMB_PRESSED_SCALE } else { 1.0 });
            if transform.scale != scale {
                transform.scale = scale;
            }
        }
    }
}

/// Moves fill, thumb and value text to [`Slider::value`].
pub(crate) fn paint_slider_values(
    sliders: Query<(&Slider, &KitParts), Changed<Slider>>,
    mut nodes: Query<&mut Node, Without<Slider>>,
    mut texts: Query<&mut Text>,
) {
    for (slider, parts) in &sliders {
        let percent_value = Val::Percent(slider.value * 100.0);
        if let Some(mut fill) = parts.fill.and_then(|fill| nodes.get_mut(fill).ok()) {
            if fill.width != percent_value {
                fill.width = percent_value;
            }
        }
        if let Some(mut thumb) = parts.knob.and_then(|knob| nodes.get_mut(knob).ok()) {
            if thumb.left != percent_value {
                thumb.left = percent_value;
            }
        }
        if let Some(mut text) = parts.extra[0].and_then(|value| texts.get_mut(value).ok()) {
            let next = percent(slider.value);
            if text.0 != next {
                text.0 = next;
            }
        }
    }
}

/// Drag and tap on a slider's track (pointer and touch), Left/Right on a
/// focused one. Disabled or modal-blocked sliders do not move.
#[allow(clippy::type_complexity)]
pub(crate) fn drive_sliders(
    mut sliders: Query<(Entity, &mut Slider, &Interaction, &Pressable, &KitParts)>,
    tracks: Query<(&ComputedNode, &UiGlobalTransform)>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    touches: Option<Res<Touches>>,
    mut adjust: MessageReader<FocusAdjust>,
    mut changed: MessageWriter<SliderChanged>,
) {
    let adjustments: Vec<FocusAdjust> = adjust.read().copied().collect();
    let window = windows.single().ok();
    let pointer = window.and_then(|window| {
        window
            .cursor_position()
            .map(|position| position * window.scale_factor())
    });
    for (entity, mut slider, interaction, pressable, parts) in &mut sliders {
        if pressable.disabled || pressable.blocked {
            continue;
        }
        let mut next = slider.value;
        for step in adjustments.iter().filter(|adjust| adjust.entity == entity) {
            next += f32::from(step.step) * slider.step;
        }
        let track = parts.track.and_then(|track| tracks.get(track).ok());
        if let Some((node, transform)) = track {
            let rect = crate::ui::focus::node_rect(node, transform);
            let touch = touches.as_ref().and_then(|touches| {
                let scale = window.map_or(1.0, Window::scale_factor);
                touches
                    .iter()
                    .map(|touch| touch.position() * scale)
                    .find(|_| pressable.touch_mode && *interaction != Interaction::None)
            });
            let held = if pressable.touch_mode {
                touch
            } else {
                pointer.filter(|_| *interaction == Interaction::Pressed)
            };
            if let Some(position) = held {
                if rect.width() > 0.0 {
                    next = (position.x - rect.min.x) / rect.width();
                }
            }
        }
        let next = next.clamp(0.0, 1.0);
        if (next - slider.value).abs() > f32::EPSILON {
            slider.value = next;
            changed.write(SliderChanged {
                slider: entity,
                value: next,
            });
        }
    }
}

/// The two chevron actions of a cycle row; Left/Right on the focused
/// control press the matching chevron.
#[derive(Component, Clone, Copy)]
pub(crate) struct CycleChevrons {
    pub previous: Entity,
    pub next: Entity,
}

/// `label … [‹ value ›]` (`cycle-row.md`): one focusable 280 × 44 control;
/// Left/Right (gamepad, arrows) or the chevrons send `previous`/`next`; a
/// click on the value does nothing. The value is `{id}Value` (rewritten by
/// the owner, kept as written: language names are shown in their own
/// language), chevrons `{id}Previous`/`{id}Next`. Returns the row.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cycle_row<T: UiActionT, M: Component>(
    parent: &mut ChildSpawnerCommands,
    label: impl UiLabel,
    value: &str,
    value_marker: M,
    previous: T,
    next: T,
    form: Form,
    id: impl Into<TestId>,
) -> Entity {
    let id = id.into();
    parent
        .spawn((
            Node {
                height: Val::Px(SETTINGS_ROW_H),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                column_gap: Val::Px(space::S16),
                ..default()
            },
            id.clone(),
        ))
        .with_children(|row| {
            row.spawn((
                label.into_text(),
                Node {
                    max_width: Val::Px(SETTINGS_LABEL_W),
                    ..default()
                },
                theme::role_text(TextRole::Label),
                TextColor(color::TEXT_SECONDARY),
            ));
            let mut parts = KitParts::default();
            let mut chevrons = [Entity::PLACEHOLDER; 2];
            let mut control = row.spawn((
                Button,
                Pressable::default(),
                Node {
                    width: Val::Px(CYCLE_W.at(form)),
                    height: Val::Px(size::TOUCH_MIN),
                    padding: UiRect::horizontal(Val::Px(space::S4)),
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(border::HAIRLINE)),
                    border_radius: BorderRadius::all(Val::Px(radius::MD)),
                    flex_shrink: 0.0,
                    ..default()
                },
                BackgroundColor(color::SURFACE_2),
                BorderColor::all(color::BORDER_SUBTLE),
                ButtonStyle::new(ButtonKind::Tile),
                KitSkin::Cycle,
                FocusAdjustable,
                id.child("Control"),
            ));
            control.with_children(|control| {
                chevrons[0] = chevron(
                    control,
                    Icon::NavChevronLeft,
                    previous,
                    id.child("Previous"),
                );
                parts.label = Some(
                    control
                        .spawn((
                            Text::new(value),
                            theme::styled_text(TextStyle::keep_case(TextRole::Label)),
                            TextColor(color::TEXT_PRIMARY),
                            TextLayout::new_with_justify(Justify::Center),
                            value_marker,
                            id.child("Value"),
                        ))
                        .id(),
                );
                chevrons[1] = chevron(control, Icon::NavChevronRight, next, id.child("Next"));
            });
            parts.extra = [Some(chevrons[0]), Some(chevrons[1])];
            control.insert((
                parts,
                CycleChevrons {
                    previous: chevrons[0],
                    next: chevrons[1],
                },
            ));
        })
        .id()
}

fn chevron<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    icon: Icon,
    action: T,
    id: TestId,
) -> Entity {
    let mut parts = KitParts::default();
    let mut button = parent.spawn((
        button_bundle(
            Node {
                width: Val::Px(CHEVRON),
                height: Val::Px(CHEVRON),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(radius::SM)),
                ..default()
            },
            ButtonKind::Tile,
            action,
            id,
        ),
        KitSkin::Tab { rail: false },
        FocusSkip,
    ));
    button.with_children(|button| {
        parts.icon = Some(
            button
                .spawn(icon_node(icon, size::ICON_MD, color::TEXT_GOLD))
                .id(),
        );
    });
    button.insert(parts).id()
}

/// Left/Right on a focused cycle row press its chevrons.
pub(crate) fn cycle_on_adjust(
    mut adjust: MessageReader<FocusAdjust>,
    rows: Query<&CycleChevrons>,
    mut presses: MessageWriter<SyntheticPress>,
) {
    for step in adjust.read() {
        if let Ok(chevrons) = rows.get(step.entity) {
            presses.write(SyntheticPress(if step.step < 0 {
                chevrons.previous
            } else {
                chevrons.next
            }));
        }
    }
}

/// Keyboard arrows and a held D-pad/stick repeat Left/Right on a focused
/// adjustable control (5 % per step on a slider).
pub(crate) fn repeat_focus_adjust(
    time: Res<Time>,
    focus: Option<Res<crate::ui::UiFocus>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    pads: Query<&Gamepad>,
    adjustable: Query<(), With<FocusAdjustable>>,
    mut held: Local<Option<(i8, Duration)>>,
    mut adjust: MessageWriter<FocusAdjust>,
) {
    let Some(focused) = focus
        .as_ref()
        .and_then(|focus| focus.focused())
        .filter(|entity| adjustable.contains(*entity))
    else {
        *held = None;
        return;
    };
    let key = |code| keys.as_ref().is_some_and(|keys| keys.pressed(code));
    let just = |code| keys.as_ref().is_some_and(|keys| keys.just_pressed(code));
    if just(KeyCode::ArrowLeft) || just(KeyCode::ArrowRight) {
        adjust.write(FocusAdjust {
            entity: focused,
            step: if just(KeyCode::ArrowLeft) { -1 } else { 1 },
        });
    }
    let pad = |button, axis_sign: f32| {
        pads.iter()
            .any(|pad| pad.pressed(button) || pad.left_stick().x * axis_sign > 0.5)
    };
    let direction = if key(KeyCode::ArrowLeft) || pad(GamepadButton::DPadLeft, -1.0) {
        -1
    } else if key(KeyCode::ArrowRight) || pad(GamepadButton::DPadRight, 1.0) {
        1
    } else {
        *held = None;
        return;
    };
    let now = time.elapsed();
    match *held {
        Some((was, since)) if was == direction => {
            let elapsed = now.saturating_sub(since);
            if elapsed >= REPEAT_DELAY {
                adjust.write(FocusAdjust {
                    entity: focused,
                    step: direction,
                });
                *held = Some((direction, now - REPEAT_DELAY + REPEAT_EVERY));
            }
        }
        _ => *held = Some((direction, now)),
    }
}

/// Validation state of a text field (`text-input.md`: error border and
/// helper line).
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct InputError(pub bool);

/// A text field: 44 high, `color.surface.0`, hairline border, optional
/// leading icon, `type.body` text, placeholder in `color.text.muted`
/// (`text-input.md`). `ButtonStyle::selected` marks editing (gold border,
/// caret); the owner keeps the text (the value is `{id}Value`).
pub(crate) fn text_input<T: UiActionT, M: Component>(
    parent: &mut ChildSpawnerCommands,
    placeholder: impl UiLabel,
    icon: Option<Icon>,
    value_marker: M,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    let id = id.into();
    let mut parts = KitParts::default();
    let mut field = parent.spawn((
        button_bundle(
            Node {
                height: Val::Px(size::TOUCH_MIN),
                width: Val::Percent(100.0),
                padding: UiRect::horizontal(Val::Px(space::S12)),
                column_gap: Val::Px(space::S8),
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::all(Val::Px(radius::MD)),
                ..default()
            },
            ButtonKind::Tile,
            action,
            id.clone(),
        ),
        KitSkin::Input,
        InputError::default(),
    ));
    field.with_children(|field| {
        if let Some(icon) = icon {
            parts.icon = Some(
                field
                    .spawn(icon_node(icon, size::ICON_MD, color::TEXT_MUTED))
                    .id(),
            );
        }
        parts.label = Some(
            field
                .spawn((
                    Text::new(""),
                    theme::styled_text(TextStyle::keep_case(TextRole::Body)),
                    TextColor(color::TEXT_PRIMARY),
                    value_marker,
                    id.child("Value"),
                ))
                .id(),
        );
        parts.knob = Some(
            field
                .spawn((
                    Node {
                        width: Val::Px(border::HAIRLINE),
                        height: Val::Px(TextRole::Body.style().size.desktop * 1.2),
                        display: Display::None,
                        ..default()
                    },
                    BackgroundColor(color::GOLD_300),
                    Pickable::IGNORE,
                    Caret,
                ))
                .id(),
        );
        parts.extra[0] = Some(
            field
                .spawn((
                    placeholder.into_text(),
                    theme::styled_text(TextStyle::keep_case(TextRole::Body)),
                    TextColor(color::TEXT_MUTED),
                    Placeholder,
                ))
                .id(),
        );
    });
    field.insert(parts).id()
}

/// The blinking caret of an editing field.
#[derive(Component)]
pub(crate) struct Caret;

/// The placeholder text of a field, shown while its value is empty.
#[derive(Component)]
pub(crate) struct Placeholder;

/// Caret (editing only, 1 s blink), placeholder (empty value only) and the
/// error border of text fields.
#[allow(clippy::type_complexity)]
pub(crate) fn paint_text_inputs(
    time: Res<Time>,
    fields: Query<(Entity, &ButtonStyle, &KitParts, &InputError, &Pressable)>,
    texts: Query<&Text, Without<Placeholder>>,
    mut nodes: Query<&mut Node, Or<(With<Caret>, With<Placeholder>)>>,
    mut borders: Query<&mut BorderColor, With<InputError>>,
) {
    let blink = time.elapsed_secs().fract() < 0.5;
    for (entity, style, parts, error, pressable) in &fields {
        let empty = parts
            .label
            .and_then(|label| texts.get(label).ok())
            .is_none_or(|text| text.0.is_empty());
        let editing = style.selected && !pressable.disabled;
        if let Some(mut caret) = parts.knob.and_then(|caret| nodes.get_mut(caret).ok()) {
            let display = if editing && blink {
                Display::Flex
            } else {
                Display::None
            };
            if caret.display != display {
                caret.display = display;
            }
        }
        if let Some(mut placeholder) = parts.extra[0].and_then(|p| nodes.get_mut(p).ok()) {
            let display = if empty { Display::Flex } else { Display::None };
            if placeholder.display != display {
                placeholder.display = display;
            }
        }
        if let Ok(mut border) = borders.get_mut(entity) {
            let danger = BorderColor::all(color::STATE_DANGER);
            if error.0 && *border != danger {
                *border = danger;
            } else if !error.0 && *border == danger {
                *border = BorderColor::all(color::BORDER_SUBTLE);
            }
        }
    }
}

/// Every system of this module, in `UiSet::Paint` (inputs first).
pub(crate) fn add_systems(app: &mut App) {
    app.add_message::<SliderChanged>().add_systems(
        Update,
        (
            (repeat_focus_adjust, cycle_on_adjust).before(crate::ui::UiSet::Gesture),
            (
                drive_sliders,
                paint_slider_values,
                scale_dragged_thumbs,
                slide_nodes,
            )
                .chain()
                .in_set(crate::ui::UiSet::Paint),
            paint_text_inputs
                .after(super::paint_kit)
                .in_set(crate::ui::UiSet::Paint),
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        UiActionAppExt,
        focus::{FocusAdjust, FocusNav},
        test_id::harness::{drain_actions, find, kit_app},
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Probe {
        Previous,
        Next,
        Toggle,
    }
    #[derive(Component)]
    struct Value;

    #[test]
    fn cycle_row_turns_left_right_into_its_chevron_actions() {
        let mut app = kit_app();
        app.add_message::<FocusAdjust>()
            .add_ui_action::<Probe>()
            .add_systems(Update, cycle_on_adjust.before(crate::ui::UiSet::Gesture));
        let root = app.world_mut().spawn(Node::default()).id();
        app.world_mut()
            .commands()
            .entity(root)
            .with_children(|parent| {
                cycle_row(
                    parent,
                    "Language",
                    "English",
                    Value,
                    Probe::Previous,
                    Probe::Next,
                    Form::Desktop,
                    "Lang",
                );
            });
        app.world_mut().flush();
        app.update();
        let control = find(app.world_mut(), "LangControl").unwrap();
        for id in ["Lang", "LangValue", "LangPrevious", "LangNext"] {
            assert!(find(app.world_mut(), id).is_some(), "{id}");
        }
        assert!(app.world().get::<FocusAdjustable>(control).is_some());
        let next = find(app.world_mut(), "LangNext").unwrap();
        assert!(app.world().get::<FocusSkip>(next).is_some());
        app.world_mut().write_message(FocusAdjust {
            entity: control,
            step: 1,
        });
        app.update();
        assert_eq!(drain_actions::<Probe>(app.world_mut()), [Probe::Next]);
        app.world_mut().write_message(FocusAdjust {
            entity: control,
            step: -1,
        });
        app.update();
        assert_eq!(drain_actions::<Probe>(app.world_mut()), [Probe::Previous]);
        let _ = FocusNav::Left;
    }

    #[test]
    fn slider_steps_five_percent_and_clamps() {
        let mut app = App::new();
        app.add_message::<FocusAdjust>()
            .add_message::<SliderChanged>()
            .add_systems(Update, (drive_sliders, paint_slider_values).chain());
        let root = app.world_mut().spawn(Node::default()).id();
        app.world_mut()
            .commands()
            .entity(root)
            .with_children(|parent| {
                slider(parent, "Master", 0.5, Form::Desktop, "Volume");
            });
        app.world_mut().flush();
        let slider_entity = app
            .world_mut()
            .query_filtered::<Entity, With<Slider>>()
            .single(app.world())
            .unwrap();
        for _ in 0..3 {
            app.world_mut().write_message(FocusAdjust {
                entity: slider_entity,
                step: 1,
            });
        }
        app.update();
        let value = app.world().get::<Slider>(slider_entity).unwrap().value;
        assert!((value - 0.65).abs() < 1e-5, "{value}");
        let parts = *app.world().get::<KitParts>(slider_entity).unwrap();
        let text = app.world().get::<Text>(parts.extra[0].unwrap()).unwrap();
        assert_eq!(text.0, "65%");
        for _ in 0..20 {
            app.world_mut().write_message(FocusAdjust {
                entity: slider_entity,
                step: 1,
            });
        }
        app.update();
        assert_eq!(app.world().get::<Slider>(slider_entity).unwrap().value, 1.0);
        let changed = app
            .world_mut()
            .resource_mut::<Messages<SliderChanged>>()
            .drain()
            .count();
        assert_eq!(changed, 2, "one message per frame that moved it");
        // Disabled sliders ignore input.
        app.world_mut()
            .get_mut::<Pressable>(slider_entity)
            .unwrap()
            .disabled = true;
        app.world_mut().write_message(FocusAdjust {
            entity: slider_entity,
            step: -1,
        });
        app.update();
        assert_eq!(app.world().get::<Slider>(slider_entity).unwrap().value, 1.0);
    }

    #[test]
    fn toggle_switch_follows_on_off_and_state() {
        let mut app = App::new();
        app.add_systems(
            Update,
            (super::super::paint_pressables, super::super::paint_kit).chain(),
        );
        let root = app.world_mut().spawn(Node::default()).id();
        app.world_mut()
            .commands()
            .entity(root)
            .with_children(|parent| {
                toggle(parent, "Mute sound", false, Probe::Toggle, "Mute");
            });
        app.world_mut().flush();
        app.update();
        let row = find(app.world_mut(), "Mute").unwrap();
        let parts = *app.world().get::<KitParts>(row).unwrap();
        let knob = parts.knob.unwrap();
        let track = parts.track.unwrap();
        assert_eq!(
            app.world().get::<BackgroundColor>(track).unwrap().0,
            color::SURFACE_3
        );
        app.world_mut()
            .get_mut::<ButtonStyle>(row)
            .unwrap()
            .selected = true;
        app.update();
        assert_eq!(
            app.world().get::<BackgroundColor>(track).unwrap().0,
            color::EMERALD_600
        );
        assert_eq!(
            app.world().get::<BackgroundColor>(knob).unwrap().0,
            color::GOLD_300
        );
        assert_eq!(
            app.world().get::<Slide>(knob).unwrap().target,
            knob_left(true, false)
        );
        assert!(knob_left(true, false) + KNOB + KNOB_INSET <= size::TOGGLE_WIDTH);
        app.world_mut().get_mut::<Pressable>(row).unwrap().disabled = true;
        app.update();
        assert_eq!(
            app.world().get::<BackgroundColor>(knob).unwrap().0,
            color::TEXT_DISABLED
        );
    }
}
