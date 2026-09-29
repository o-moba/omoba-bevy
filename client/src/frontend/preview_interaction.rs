//! Screen-local preview gestures, in logical window pixels (DPI independent).
use super::preview::AvatarPreview;
use bevy::{
    input::touch::{TouchInput, TouchPhase},
    prelude::*,
    window::PrimaryWindow,
};

#[derive(Component)]
pub(crate) struct InteractivePreview;

#[derive(Default)]
pub(super) struct Gesture {
    held: Option<(Option<u64>, Vec2, Vec2, f64)>,
    tap: Option<(Vec2, f64)>,
    dragged: bool,
}

impl Gesture {
    fn end(&mut self, position: Vec2, now: f64, preview: &mut AvatarPreview) {
        if let Some((_, start, _, began)) = self.held.take() {
            if !self.dragged && start.distance(position) < 10.0 && now - began < 0.35 {
                if self.tap.is_some_and(|(point, time)| {
                    now - time < 0.4 && point.distance(position) < 30.0
                }) {
                    preview.greet((now * 1_000_000.0) as u64);
                    self.tap = None;
                } else {
                    self.tap = Some((position, now));
                }
            } else {
                self.tap = None;
            }
        }
    }
}

pub(super) fn interact(
    mut preview: ResMut<AvatarPreview>,
    time: Res<Time>,
    mut gesture: Local<Gesture>,
    mut touches: MessageReader<TouchInput>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    surfaces: Query<(&ComputedNode, &UiGlobalTransform), With<InteractivePreview>>,
    modals: Option<Res<crate::ui::ModalStack>>,
) {
    let Ok(window) = windows.single() else {
        touches.clear();
        return;
    };
    if !window.focused
        || surfaces.is_empty()
        || modals.as_ref().is_some_and(|stack| stack.is_open())
    {
        touches.clear();
        *gesture = Gesture::default();
        return;
    }
    let inside = |position| {
        surfaces.iter().any(|(node, transform)| {
            Rect::from_center_size(
                transform.translation / window.scale_factor(),
                node.size() * transform.to_scale_angle_translation().0.abs()
                    / window.scale_factor(),
            )
            .contains(position)
        })
    };
    let now = time.elapsed_secs_f64();
    let mut touched = false;
    for event in touches.read() {
        touched = true;
        match event.phase {
            TouchPhase::Started if gesture.held.is_none() && inside(event.position) => {
                gesture.held = Some((Some(event.id), event.position, event.position, now));
                gesture.dragged = false;
            }
            TouchPhase::Moved => {
                if let Some((Some(id), start, previous, began)) = gesture.held {
                    if id == event.id {
                        gesture.dragged |= start.distance(event.position) >= 10.0;
                        preview.yaw += (event.position.x - previous.x) * 0.012;
                        preview.auto_spin = false;
                        gesture.held = Some((Some(id), start, event.position, began));
                    }
                }
            }
            TouchPhase::Ended if gesture.held.is_some_and(|(id, ..)| id == Some(event.id)) => {
                gesture.end(event.position, now, &mut preview)
            }
            TouchPhase::Canceled if gesture.held.is_some_and(|(id, ..)| id == Some(event.id)) => {
                *gesture = Gesture::default()
            }
            _ => {}
        }
    }
    if !touched {
        if let (Some(mouse), Some(position)) = (mouse, window.cursor_position()) {
            if mouse.just_pressed(MouseButton::Left) && inside(position) {
                gesture.held = Some((None, position, position, now));
                gesture.dragged = false;
            } else if let Some((None, start, previous, began)) = gesture.held {
                if mouse.just_released(MouseButton::Left) {
                    gesture.end(position, now, &mut preview);
                } else if mouse.pressed(MouseButton::Left) {
                    gesture.dragged |= start.distance(position) >= 10.0;
                    preview.yaw += (position.x - previous.x) * 0.012;
                    preview.auto_spin = false;
                    gesture.held = Some((None, start, position, began));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_tap_is_consumed_but_a_drag_or_long_press_is_not_a_tap() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let mut preview = AvatarPreview::from_world(&mut world);
        let mut gesture = Gesture {
            held: Some((Some(1), Vec2::ZERO, Vec2::ZERO, 1.0)),
            ..Default::default()
        };
        gesture.end(Vec2::ZERO, 1.1, &mut preview);
        assert!(gesture.tap.is_some());
        gesture.held = Some((Some(1), Vec2::ZERO, Vec2::ZERO, 1.2));
        gesture.end(Vec2::ZERO, 1.3, &mut preview);
        assert!(gesture.tap.is_none());
        gesture.held = Some((Some(1), Vec2::ZERO, Vec2::ZERO, 2.0));
        gesture.dragged = true; // dragged away and back to its origin
        gesture.end(Vec2::ZERO, 2.1, &mut preview);
        assert!(gesture.tap.is_none());
        gesture.dragged = false;
        gesture.held = Some((Some(1), Vec2::ZERO, Vec2::ZERO, 3.0));
        gesture.end(Vec2::ZERO, 3.8, &mut preview);
        assert!(gesture.tap.is_none());
    }
}
