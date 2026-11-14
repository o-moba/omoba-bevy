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
    ui::CalculatedClip,
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

    /// Feeds one touch phase. On touch-down, hit-test the front-most button.
    /// On release, check only the button that originally held the tap.
    pub(crate) fn event(
        &mut self,
        id: u64,
        phase: TouchPhase,
        point: Vec2,
        hit: impl FnOnce(Option<Entity>) -> Option<Entity>,
    ) -> Option<Entity> {
        if !point.is_finite() {
            self.held = None;
            return None;
        }
        if phase == TouchPhase::Started {
            if self.held.is_none() {
                if let Some(button) = hit(None) {
                    self.held = Some(Tap {
                        id,
                        button,
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
        let released =
            phase == TouchPhase::Ended && !tap.canceled && hit(Some(candidate)) == Some(candidate);
        if matches!(phase, TouchPhase::Ended | TouchPhase::Canceled) {
            self.held = None;
        }
        released.then_some(candidate)
    }
}

/// Hit test in physical window coordinates, including every transformed scroll clip.
pub(super) fn ui_contains_point(
    node: &ComputedNode,
    transform: &UiGlobalTransform,
    clip: Option<&CalculatedClip>,
    point: Vec2,
) -> bool {
    point.is_finite()
        && node.contains_point(*transform, point)
        && clip.is_none_or(|clip| clip.contains_point(point))
}

/// Window-logical bounds of a UI node, clipped by its scroll ancestors.
pub(crate) fn logical_ui_rect(
    node: &ComputedNode,
    transform: &UiGlobalTransform,
    clip: Option<&CalculatedClip>,
    dpi: f32,
) -> Rect {
    let rect = super::clipped_ui_rect(super::focus::node_rect(node, transform), clip);
    Rect {
        min: rect.min / dpi,
        max: rect.max / dpi,
    }
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
        Option<&bevy::ui::ComputedStackIndex>,
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
    let mut event = |id, phase, point: Vec2| {
        state.tracker.event(id, phase, point, |target| {
            let physical = point * window.scale_factor();
            let hit = |pressable: &Pressable,
                       node: Option<&ComputedNode>,
                       transform: Option<&UiGlobalTransform>,
                       visibility: Option<&InheritedVisibility>,
                       clip| {
                !pressable.disabled
                    && !pressable.blocked
                    && visibility.is_none_or(|visibility| visibility.get())
                    && node.zip(transform).is_some_and(|(node, transform)| {
                        ui_contains_point(node, transform, clip, physical)
                    })
            };
            if let Some(target) = target {
                let (_, pressable, node, transform, visibility, clip, _) =
                    buttons.get(target).ok()?;
                return hit(pressable, node, transform, visibility, clip).then_some(target);
            }
            buttons
                .iter()
                .filter_map(
                    |(entity, pressable, node, transform, visibility, clip, stack)| {
                        hit(pressable, node, transform, visibility, clip)
                            .then_some((stack.map_or(0, |index| index.0), entity))
                    },
                )
                .min_by_key(|(stack, _)| std::cmp::Reverse(*stack))
                .map(|(_, entity)| entity)
        })
    };
    let mut activated = Vec::new();
    for touch in events.read() {
        if touch.window == window_entity
            && let Some(entity) = event(touch.id, touch.phase, touch.position)
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
            && let Some(entity) = event(u64::MAX, phase, point)
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
    fn scroll_y_clipping_respects_dpi_and_ui_scale_on_press_and_release() {
        for (dpi, ui_scale) in [(1.0, 1.0), (2.0, 0.75), (3.0, 1.2)] {
            let (mut app, window, button) = touch_app();
            app.add_systems(PostUpdate, bevy::ui::update::update_clipping_system);
            let world = app.world_mut();
            world
                .get_mut::<Window>(window)
                .unwrap()
                .resolution
                .set_scale_factor_override(Some(dpi));
            let scale = dpi * ui_scale;
            let panel = world
                .spawn((
                    Node {
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                    ComputedNode {
                        size: Vec2::new(100.0, 100.0) * scale,
                        inverse_scale_factor: scale.recip(),
                        ..default()
                    },
                    UiGlobalTransform::from_translation(Vec2::new(100.0, 100.0) * scale),
                ))
                .id();
            // The button extends beyond the panel on both axes. Its bottom
            // half is clipped, but the panel's horizontal overflow is visible.
            world.entity_mut(button).insert((
                Node::default(),
                ChildOf(panel),
                ComputedNode {
                    size: Vec2::new(140.0, 40.0) * scale,
                    inverse_scale_factor: scale.recip(),
                    ..default()
                },
                UiGlobalTransform::from_translation(Vec2::new(100.0, 150.0) * scale),
            ));
            app.update();
            let world = app.world();
            let clip = world.get::<CalculatedClip>(button).unwrap();
            let visible = logical_ui_rect(
                world.get::<ComputedNode>(button).unwrap(),
                world.get::<UiGlobalTransform>(button).unwrap(),
                Some(clip),
                dpi,
            );
            assert!(
                visible
                    .min
                    .abs_diff_eq(Vec2::new(30.0, 130.0) * ui_scale, 0.001)
            );
            assert!(
                visible
                    .max
                    .abs_diff_eq(Vec2::new(170.0, 150.0) * ui_scale, 0.001)
            );

            let inside = Vec2::new(160.0, 148.0) * ui_scale;
            let outside = Vec2::new(160.0, 152.0) * ui_scale;
            for (start, end, activated) in [
                (inside, inside, true),
                (outside, inside, false),
                (inside, outside, false),
            ] {
                for (phase, position) in [(TouchPhase::Started, start), (TouchPhase::Ended, end)] {
                    app.world_mut().write_message(TouchInput {
                        id: 1,
                        phase,
                        position,
                        window,
                        force: None,
                    });
                }
                app.update();
                assert_eq!(
                    app.world().get::<Pressable>(button).unwrap().activated,
                    activated
                );
            }
        }
    }

    #[test]
    fn hits_use_rotated_clip_geometry_and_reject_fully_clipped_nodes() {
        let rect = Rect::new(-50.0, -10.0, 50.0, 10.0);
        let transform =
            UiGlobalTransform::from(bevy::math::Affine2::from_angle(std::f32::consts::FRAC_PI_4));
        let clip = CalculatedClip::default().with_rect(rect, &transform);
        let node = ComputedNode {
            size: Vec2::splat(100.0),
            ..default()
        };
        let origin = UiGlobalTransform::default();
        assert!(ui_contains_point(
            &node,
            &origin,
            Some(&clip),
            Vec2::new(20.0, 20.0)
        ));
        assert!(!ui_contains_point(
            &node,
            &origin,
            Some(&clip),
            Vec2::new(-20.0, 20.0)
        ));
        assert!(!ui_contains_point(
            &node,
            &origin,
            Some(&CalculatedClip::FullyClipped),
            Vec2::ZERO
        ));
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
                    ..default()
                },
                bevy::ui::ComputedStackIndex(5),
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
