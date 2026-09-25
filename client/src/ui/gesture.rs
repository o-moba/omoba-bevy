//! The one tap recognizer for touch menus.
//!
//! Desktop buttons use Bevy's `Interaction` directly. On a phone a menu can
//! scroll, so a press only counts on a short release inside the button it
//! started on: a finger that moves more than 10 px cancels the tap for good
//! (coming back does not revive it), a second finger is ignored, a `Canceled`
//! phase clears it, and app suspension, focus loss, a viewport change or a
//! modal navigation (`GestureEpoch`) drop whatever was held.
use bevy::{
    input::touch::{TouchInput, TouchPhase},
    prelude::*,
    window::{AppLifecycle, PrimaryWindow},
};

/// A button the recognizer tracks. Modules keep reading Bevy's `Interaction`
/// through [`Pressable::effective`], so desktop behaviour is unchanged.
#[derive(Component, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Pressable {
    /// Set by the recognizer every frame from [`crate::ui::UiPlatform`].
    pub touch_mode: bool,
    /// True for the one frame a tap (or a [`SyntheticPress`]) completed.
    pub activated: bool,
    /// Owned by the module: a disabled button is never hit, pressed or lit.
    pub disabled: bool,
    /// Set by the recognizer every frame from the [`super::modal::ModalStack`]:
    /// a button outside the top open modal behaves as disabled.
    pub blocked: bool,
}

impl Pressable {
    /// The interaction a handler should act on. In touch mode a held finger
    /// only hovers; the tap release activates. A synthetic press activates
    /// in either mode.
    pub(crate) fn effective(&self, interaction: Interaction) -> Interaction {
        if self.disabled || self.blocked {
            return Interaction::None;
        }
        if self.activated {
            return Interaction::Pressed;
        }
        if !self.touch_mode {
            return interaction;
        }
        if interaction == Interaction::Pressed {
            Interaction::Hovered
        } else {
            interaction
        }
    }
}

/// How far (logical px) a finger may travel before a tap turns into a drag.
/// The tap recognizer cancels past it and phone scroll areas start past it,
/// so one gesture is never both a press and a scroll.
pub(crate) const TAP_SLOP: f32 = 10.0;

/// Bumped by a modal whenever it navigates (page change, open, close), which
/// drops the tap held across the change so a release on the new page cannot
/// activate a button that was under the finger on the old one.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct GestureEpoch(pub u64);

impl GestureEpoch {
    pub(crate) fn bump(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }
}

/// Activates a [`Pressable`] for one frame regardless of the touch gate.
/// QA harnesses and tests use it in place of writing `Interaction::Pressed`.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SyntheticPress(pub Entity);

#[derive(Default)]
pub(crate) struct TapTracker {
    held: Option<Tap>,
}

struct Tap {
    id: u64,
    button: Entity,
    start: Vec2,
    canceled: bool,
}

impl TapTracker {
    pub(crate) fn clear(&mut self) {
        self.held = None;
    }

    /// Feeds one touch phase; `buttons` are the hit rects in front-to-back
    /// order. Returns the button whose tap completed on this event.
    pub(crate) fn event(
        &mut self,
        id: u64,
        phase: TouchPhase,
        point: Vec2,
        buttons: &[(Entity, Rect)],
    ) -> Option<Entity> {
        if !point.is_finite() {
            self.held = None;
            return None;
        }
        if phase == TouchPhase::Started {
            if self.held.is_none() {
                if let Some((button, _)) = buttons.iter().find(|(_, rect)| rect.contains(point)) {
                    self.held = Some(Tap {
                        id,
                        button: *button,
                        start: point,
                        canceled: false,
                    });
                }
            }
            return None;
        }
        let tap = self.held.as_mut().filter(|tap| tap.id == id)?;
        // Sticky cancellation: scrolling away then back cannot revive a tap.
        tap.canceled |= tap.start.distance(point) > TAP_SLOP;
        let candidate = tap.button;
        let released = phase == TouchPhase::Ended
            && !tap.canceled
            && buttons
                .iter()
                .any(|(entity, rect)| *entity == candidate && rect.contains(point));
        if matches!(phase, TouchPhase::Ended | TouchPhase::Canceled) {
            self.held = None;
        }
        released.then_some(candidate)
    }
}

/// Window-logical rectangle of a UI node, clipped by its scroll ancestors.
pub(crate) fn logical_ui_rect(
    node: &ComputedNode,
    transform: &UiGlobalTransform,
    clip: Option<&bevy::ui::CalculatedClip>,
    dpi: f32,
) -> Rect {
    let mut rect = Rect::from_center_size(
        transform.translation / dpi,
        node.size() * transform.to_scale_angle_translation().0.abs() / dpi,
    );
    if let Some(clip) = clip {
        rect = rect.intersect(Rect::from_corners(clip.clip.min / dpi, clip.clip.max / dpi));
    }
    rect
}

#[derive(Default)]
pub(crate) struct RecognizerState {
    tracker: TapTracker,
    epoch: Option<u64>,
    viewport: Option<Vec2>,
}

/// Runs first in `InputContextSet::Modal` (`UiSet::Gesture`) for every
/// [`Pressable`] in the world. `activated` is a one-frame flag; `touch_mode`
/// follows the platform; `blocked` follows the modal stack (only the top
/// modal's buttons react); `disabled` is left to the owning module.
pub(crate) fn recognize_presses(
    mut state: Local<RecognizerState>,
    platform: Option<Res<crate::ui::UiPlatform>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    epoch: Option<Res<GestureEpoch>>,
    touches: Option<Res<Touches>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    window: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut events: MessageReader<TouchInput>,
    synthetic: Option<Res<Messages<SyntheticPress>>>,
    mut synthetic_cursor: Local<bevy::ecs::message::MessageCursor<SyntheticPress>>,
    lifecycle: Option<Res<Messages<AppLifecycle>>>,
    mut lifecycle_cursor: Local<bevy::ecs::message::MessageCursor<AppLifecycle>>,
    gate: super::modal::ModalGate,
    mut buttons: Query<(
        Entity,
        &mut Pressable,
        Option<&ComputedNode>,
        Option<&UiGlobalTransform>,
        Option<&InheritedVisibility>,
        Option<&bevy::ui::CalculatedClip>,
    )>,
) {
    let touch_mode = platform
        .as_ref()
        .is_some_and(|platform| platform.is_mobile());
    for (entity, mut pressable, ..) in &mut buttons {
        let disabled = pressable.disabled;
        pressable.set_if_neq(Pressable {
            touch_mode,
            activated: false,
            disabled,
            blocked: !gate.allows(entity),
        });
    }
    if let Some(synthetic) = synthetic.as_ref() {
        for SyntheticPress(entity) in synthetic_cursor.read(synthetic) {
            if let Ok((_, mut pressable, ..)) = buttons.get_mut(*entity)
                && !pressable.disabled
                && !pressable.blocked
            {
                pressable.activated = true;
            }
        }
    }
    let epoch = epoch.as_ref().map_or(0, |epoch| epoch.0);
    if state.epoch != Some(epoch) {
        state.tracker.clear();
        state.epoch = Some(epoch);
    }
    if let Some(mobile) = mobile.as_ref() {
        if state.viewport != Some(mobile.viewport) {
            state.tracker.clear();
            state.viewport = Some(mobile.viewport);
        }
    }
    // Without the mobile HUD resource (unit tests) orientation and focus do
    // not veto; at runtime it always exists on a phone.
    let oriented = mobile
        .as_ref()
        .is_none_or(|mobile| mobile.landscape && mobile.focused);
    let Ok((window_entity, window)) = window.single() else {
        state.tracker.clear();
        events.clear();
        return;
    };
    if !touch_mode || !oriented || !window.focused {
        state.tracker.clear();
        events.clear();
        return;
    }
    let interrupted = lifecycle.as_ref().is_some_and(|events| {
        lifecycle_cursor.read(events).any(|event| {
            matches!(
                event,
                AppLifecycle::WillSuspend | AppLifecycle::Suspended | AppLifecycle::WillResume
            )
        })
    });
    if interrupted {
        state.tracker.clear();
        events.clear();
        return;
    }
    let dpi = window.scale_factor();
    let mut visible: Vec<_> = buttons
        .iter()
        .filter_map(|(entity, pressable, node, transform, visibility, clip)| {
            if pressable.disabled || pressable.blocked || visibility.is_some_and(|v| !v.get()) {
                return None;
            }
            let (Some(node), Some(transform)) = (node, transform) else {
                return None;
            };
            if node.size().min_element() <= 0.0 {
                return None;
            }
            let rect = logical_ui_rect(node, transform, clip, dpi);
            (rect.width() > 0.0 && rect.height() > 0.0).then_some((node.stack_index, entity, rect))
        })
        .collect();
    // Front-most first, so an overlapping button on top wins the touch.
    visible.sort_by(|a, b| b.0.cmp(&a.0));
    let visible: Vec<_> = visible
        .into_iter()
        .map(|(_, entity, rect)| (entity, rect))
        .collect();
    let mut activated = Vec::new();
    for event in events.read() {
        if event.window == window_entity
            && let Some(entity) =
                state
                    .tracker
                    .event(event.id, event.phase, event.position, &visible)
        {
            activated.push(entity);
        }
    }
    // Mouse QA follows the same release rule. Native touch devices ignore
    // synthesized mouse events so a physical release cannot activate twice.
    if !cfg!(any(target_os = "android", target_os = "ios"))
        && let (Some(touches), Some(mouse)) = (touches.as_ref(), mouse.as_ref())
        && touches.iter().next().is_none()
        && !touches.any_just_released()
        && !touches.any_just_canceled()
        && let Some(point) = window.cursor_position()
    {
        let phase = if mouse.just_pressed(MouseButton::Left) {
            Some(TouchPhase::Started)
        } else if mouse.just_released(MouseButton::Left) {
            Some(TouchPhase::Ended)
        } else if mouse.pressed(MouseButton::Left) {
            Some(TouchPhase::Moved)
        } else {
            None
        };
        if let Some(phase) = phase
            && let Some(entity) = state.tracker.event(u64::MAX, phase, point, &visible)
        {
            activated.push(entity);
        }
    }
    for entity in activated {
        if let Ok((_, mut pressable, ..)) = buttons.get_mut(entity) {
            pressable.activated = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_interaction_follows_touch_mode_and_disabled() {
        let desktop = Pressable::default();
        assert_eq!(
            desktop.effective(Interaction::Pressed),
            Interaction::Pressed
        );
        let touch = Pressable {
            touch_mode: true,
            ..default()
        };
        assert_eq!(touch.effective(Interaction::Pressed), Interaction::Hovered);
        assert_eq!(touch.effective(Interaction::Hovered), Interaction::Hovered);
        let tapped = Pressable {
            touch_mode: true,
            activated: true,
            ..default()
        };
        assert_eq!(tapped.effective(Interaction::None), Interaction::Pressed);
        let disabled = Pressable {
            touch_mode: false,
            activated: true,
            disabled: true,
            ..default()
        };
        assert_eq!(disabled.effective(Interaction::Pressed), Interaction::None);
        let blocked = Pressable {
            activated: true,
            blocked: true,
            ..default()
        };
        assert_eq!(blocked.effective(Interaction::Pressed), Interaction::None);
    }

    fn touch_app() -> (App, Entity, Entity) {
        let mut app = App::new();
        app.insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Mobile))
            .init_resource::<GestureEpoch>()
            .add_message::<TouchInput>()
            .add_message::<SyntheticPress>()
            .add_systems(Update, recognize_presses);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let button = app
            .world_mut()
            .spawn((
                Pressable::default(),
                ComputedNode {
                    size: Vec2::new(180.0, 46.0),
                    inverse_scale_factor: 1.0,
                    ..default()
                },
                UiGlobalTransform::from_translation(Vec2::new(300.0, 150.0)),
                InheritedVisibility::VISIBLE,
            ))
            .id();
        (app, window, button)
    }

    #[test]
    fn epoch_bump_drops_the_held_tap_and_synthetic_press_bypasses_the_gate() {
        let (mut app, window, button) = touch_app();
        let event = |id, phase, position| TouchInput {
            id,
            phase,
            position,
            window,
            force: None,
        };
        let center = Vec2::new(300.0, 150.0);
        app.world_mut()
            .write_message(event(1, TouchPhase::Started, center));
        app.update();
        app.world_mut().resource_mut::<GestureEpoch>().bump();
        app.world_mut()
            .write_message(event(1, TouchPhase::Ended, center));
        app.update();
        assert!(!app.world().get::<Pressable>(button).unwrap().activated);
        app.world_mut()
            .write_message(event(2, TouchPhase::Started, center));
        app.world_mut()
            .write_message(event(2, TouchPhase::Ended, center));
        app.update();
        assert!(app.world().get::<Pressable>(button).unwrap().activated);
        app.update();
        assert!(!app.world().get::<Pressable>(button).unwrap().activated);
        app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
        app.world_mut().write_message(SyntheticPress(button));
        app.update();
        assert!(app.world().get::<Pressable>(button).unwrap().activated);
        app.world_mut()
            .get_mut::<Pressable>(button)
            .unwrap()
            .disabled = true;
        app.world_mut().write_message(SyntheticPress(button));
        app.update();
        let pressable = app.world().get::<Pressable>(button).unwrap();
        assert!(!pressable.activated && pressable.disabled);
    }

    #[test]
    fn the_front_most_overlapping_button_takes_the_tap() {
        let (mut app, window, back) = touch_app();
        let front = app
            .world_mut()
            .spawn((
                Pressable::default(),
                ComputedNode {
                    size: Vec2::new(60.0, 46.0),
                    inverse_scale_factor: 1.0,
                    stack_index: 5,
                    ..default()
                },
                UiGlobalTransform::from_translation(Vec2::new(300.0, 150.0)),
                InheritedVisibility::VISIBLE,
            ))
            .id();
        let center = Vec2::new(300.0, 150.0);
        for phase in [TouchPhase::Started, TouchPhase::Ended] {
            app.world_mut().write_message(TouchInput {
                id: 1,
                phase,
                position: center,
                window,
                force: None,
            });
        }
        app.update();
        assert!(app.world().get::<Pressable>(front).unwrap().activated);
        assert!(!app.world().get::<Pressable>(back).unwrap().activated);
    }
}
