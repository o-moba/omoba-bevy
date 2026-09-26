//! The one "back" signal: `Esc` on a keyboard and any other source that
//! means "close what is in front" (a gamepad's East button).
//!
//! The modules that close something on back (social, help, supporter,
//! career, scoreboard, shop, the lobby's server field, the Combat Test panel,
//! the pause menu) read [`BackInput`] in the same frame order they always
//! had. The first one that acts calls [`BackInput::consume`], so the same
//! press cannot also close the next overlay or open the pause menu. A source
//! other than the keyboard writes [`BackPress`] for one frame; it is cleared
//! at the start of every frame, before input is read.
use bevy::{ecs::system::SystemParam, prelude::*};

/// A back press from a non-keyboard source (a gamepad's East), live for the
/// frame it was written in.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BackPress {
    pending: bool,
}

impl BackPress {
    pub(crate) fn press(&mut self) {
        self.pending = true;
    }

    pub(crate) fn pending(&self) -> bool {
        self.pending
    }
}

/// Clears an unconsumed [`BackPress`] before the next frame's input.
pub(crate) fn clear_back_press(mut back: ResMut<BackPress>) {
    if back.pending {
        back.pending = false;
    }
}

/// `Esc` plus [`BackPress`]. Holds the keyboard mutably, so a system that
/// also reads other keys goes through [`BackInput::keys`] /
/// [`BackInput::keys_mut`] instead of a second `ButtonInput<KeyCode>`.
#[derive(SystemParam)]
pub(crate) struct BackInput<'w> {
    keys: ResMut<'w, ButtonInput<KeyCode>>,
    other: Option<ResMut<'w, BackPress>>,
}

impl BackInput<'_> {
    /// A back press is waiting this frame (not consumed yet).
    pub(crate) fn just_pressed(&self) -> bool {
        self.from_keyboard() || self.other.as_ref().is_some_and(|back| back.pending)
    }

    /// The waiting press came from `Esc`. The pause menu opens only on this;
    /// a gamepad's back button closes the menu but never opens it.
    pub(crate) fn from_keyboard(&self) -> bool {
        self.keys.just_pressed(KeyCode::Escape)
    }

    /// Takes the press, so later readers this frame see none.
    pub(crate) fn consume(&mut self) {
        self.keys.clear_just_pressed(KeyCode::Escape);
        if let Some(back) = self.other.as_mut().filter(|back| back.pending) {
            back.pending = false;
        }
    }

    pub(crate) fn keys(&self) -> &ButtonInput<KeyCode> {
        &self.keys
    }

    pub(crate) fn keys_mut(&mut self) -> &mut ButtonInput<KeyCode> {
        &mut self.keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<BackPress>();
        app
    }

    fn read(app: &mut App) -> (bool, bool) {
        app.world_mut()
            .run_system_once(|back: BackInput| (back.just_pressed(), back.from_keyboard()))
            .unwrap()
    }

    #[test]
    fn escape_and_a_back_press_are_one_signal_and_consume_takes_both() {
        let mut app = app();
        assert_eq!(read(&mut app), (false, false));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        assert_eq!(read(&mut app), (true, true));
        app.world_mut().resource_mut::<BackPress>().press();
        app.world_mut()
            .run_system_once(|mut back: BackInput| back.consume())
            .unwrap();
        assert_eq!(read(&mut app), (false, false));
        assert!(!app.world().resource::<BackPress>().pending());
        // Consuming leaves the key held: only the edge is taken.
        assert!(
            app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::Escape)
        );

        app.world_mut().resource_mut::<BackPress>().press();
        assert_eq!(read(&mut app), (true, false), "not from the keyboard");
        app.world_mut().run_system_once(clear_back_press).unwrap();
        assert_eq!(read(&mut app), (false, false), "lives one frame");
    }

    #[test]
    fn works_without_the_back_press_resource() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        assert_eq!(read(&mut app), (true, true));
        app.world_mut()
            .run_system_once(|mut back: BackInput| {
                back.consume();
                back.keys_mut().press(KeyCode::KeyP);
                back.keys().just_pressed(KeyCode::KeyP)
            })
            .unwrap();
        assert_eq!(read(&mut app), (false, false));
    }

    /// The order the modules consume back in is the order they are scheduled
    /// in: the first overlay that is open takes the press.
    #[test]
    fn a_consumed_press_does_not_reach_the_next_reader() {
        #[derive(Resource, Default)]
        struct Closed(Vec<&'static str>);
        let mut app = app();
        app.init_resource::<Closed>().add_systems(
            Update,
            (
                |mut back: BackInput, mut closed: ResMut<Closed>| {
                    if back.just_pressed() {
                        back.consume();
                        closed.0.push("front");
                    }
                },
                |back: BackInput, mut closed: ResMut<Closed>| {
                    if back.just_pressed() {
                        closed.0.push("behind");
                    }
                },
            )
                .chain(),
        );
        app.world_mut().resource_mut::<BackPress>().press();
        app.update();
        assert_eq!(app.world().resource::<Closed>().0, ["front"]);
    }
}
