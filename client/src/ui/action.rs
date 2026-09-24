//! Typed button actions: a button carries `UiAction<T>`, the kit turns each
//! press into one `Activated<T>` message and the owning module handles it
//! without touching `Interaction`, gestures or colours.
use bevy::prelude::*;

use super::{Pressable, UiSet};

/// Anything a screen wants to attach to a button. Blanket-implemented.
pub(crate) trait UiActionT: Clone + Send + Sync + 'static {}
impl<T: Clone + Send + Sync + 'static> UiActionT for T {}

/// The action a button performs; requires [`Pressable`] so the recognizer
/// and the painter see every kit button.
#[derive(Component, Clone, Debug)]
#[require(Pressable)]
pub(crate) struct UiAction<T: UiActionT>(pub T);

/// One message per press, edge triggered: a held mouse button or a finger
/// resting on the button does not repeat it.
#[derive(Message, Clone, Debug)]
pub(crate) struct Activated<T: UiActionT> {
    pub action: T,
    pub source: Entity,
}

/// Runs in `UiSet::Dispatch`. Fires when the effective interaction became
/// `Pressed` this frame (desktop click, completed tap or synthetic press).
pub(crate) fn dispatch_actions<T: UiActionT>(
    buttons: Query<
        (Entity, &Interaction, &Pressable, &UiAction<T>),
        Or<(Changed<Interaction>, Changed<Pressable>)>,
    >,
    mut activated: MessageWriter<Activated<T>>,
) {
    for (entity, interaction, pressable, action) in &buttons {
        if pressable.effective(*interaction) == Interaction::Pressed {
            activated.write(Activated {
                action: action.0.clone(),
                source: entity,
            });
        }
    }
}

pub(crate) trait UiActionAppExt {
    /// Registers the `Activated<T>` message and its dispatcher.
    fn add_ui_action<T: UiActionT>(&mut self) -> &mut Self;
}

impl UiActionAppExt for App {
    fn add_ui_action<T: UiActionT>(&mut self) -> &mut Self {
        self.add_message::<Activated<T>>()
            .add_systems(Update, dispatch_actions::<T>.in_set(UiSet::Dispatch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Probe {
        Go,
    }

    #[test]
    fn a_press_dispatches_once_and_a_touch_hold_does_not() {
        let mut app = App::new();
        app.add_ui_action::<Probe>();
        let button = app
            .world_mut()
            .spawn((Button, UiAction(Probe::Go), Interaction::None))
            .id();
        app.update();
        let count = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Messages<Activated<Probe>>>()
                .drain()
                .count()
        };
        assert_eq!(count(&mut app), 0);
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();
        assert_eq!(count(&mut app), 1);
        app.update();
        assert_eq!(count(&mut app), 0, "a held button does not repeat");
        // Touch mode: a resting finger is a hover, the completed tap fires.
        app.world_mut().get_mut::<Pressable>(button).unwrap().touch_mode = true;
        app.update();
        assert_eq!(count(&mut app), 0);
        app.world_mut().get_mut::<Pressable>(button).unwrap().activated = true;
        app.update();
        let fired: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<Activated<Probe>>>()
            .drain()
            .collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].source, button);
        assert_eq!(fired[0].action, Probe::Go);
    }
}
