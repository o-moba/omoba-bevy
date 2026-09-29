//! Session-only iPad preview. UIKit resizes the real window, so existing
//! phone layouts and window-local touch coordinates need no emulation.
// i18n-strict
use bevy::prelude::*;

use crate::pause_menu::PauseAction;
use crate::ui::{Activated, UiAction, UiSet, widgets::ButtonStyle};

pub(crate) struct PhoneLayoutPreviewPlugin;

#[derive(Resource, Default)]
struct PhoneLayoutPreview {
    available: bool,
    active: bool,
    requested: Option<bool>,
}

impl PhoneLayoutPreview {
    fn toggle(&mut self) {
        // Also reject stale/synthetic actions on an ineligible host.
        if self.available || self.active {
            self.requested = Some(!self.active);
        }
    }
}

#[derive(Component)]
pub(crate) struct PhonePreviewSettings;

impl Plugin for PhoneLayoutPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PhoneLayoutPreview>().add_systems(
            Update,
            (request_preview, sync_preview_control)
                .chain()
                .after(UiSet::Dispatch),
        );
        #[cfg(target_os = "ios")]
        app.add_systems(
            PreUpdate,
            poll_native_preview.after(bevy::input::InputSystems),
        );
    }
}

fn request_preview(
    mut state: ResMut<PhoneLayoutPreview>,
    mut events: MessageReader<Activated<PauseAction>>,
) {
    for event in events.read() {
        if event.action == PauseAction::TogglePhoneLayoutPreview {
            state.toggle();
        }
    }
}

fn sync_preview_control(
    state: Res<PhoneLayoutPreview>,
    mut rows: Query<&mut Node, With<PhonePreviewSettings>>,
    mut buttons: Query<(&UiAction<PauseAction>, &mut ButtonStyle)>,
) {
    for mut node in &mut rows {
        node.display = if state.available || state.active {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (action, mut style) in &mut buttons {
        if action.0 == PauseAction::TogglePhoneLayoutPreview {
            ButtonStyle::set_selected(&mut style, state.active);
        }
    }
}

#[cfg(any(target_os = "ios", test))]
fn poll_native_preview(
    _main_thread: bevy::ecs::system::NonSendMarker,
    mut state: ResMut<PhoneLayoutPreview>,
    mut touches: ResMut<Touches>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    window: Query<Entity, With<bevy::window::PrimaryWindow>>,
    mut events: MessageWriter<bevy::input::touch::TouchInput>,
    mut epoch: ResMut<crate::ui::GestureEpoch>,
    mut mobile: ResMut<crate::mobile_controls::MobileControls>,
) {
    #[cfg(target_os = "ios")]
    unsafe extern "C" {
        fn omoba_phone_preview_available() -> bool;
        fn omoba_phone_preview_active() -> bool;
        fn omoba_phone_preview_set(enabled: bool, title: *const std::ffi::c_char);
    }
    // Compile and exercise the complete native-poll integration in desktop
    // tests too, without linking UIKit or pretending the desktop is an iPad.
    #[cfg(all(test, not(target_os = "ios")))]
    unsafe fn omoba_phone_preview_available() -> bool {
        false
    }
    #[cfg(all(test, not(target_os = "ios")))]
    unsafe fn omoba_phone_preview_active() -> bool {
        false
    }
    #[cfg(all(test, not(target_os = "ios")))]
    unsafe fn omoba_phone_preview_set(_: bool, _: *const std::ffi::c_char) {}
    // Synchronous UIKit calls on Bevy's main thread; the bridge also enforces
    // iPad eligibility. It copies the temporary title before returning.
    state.available = unsafe { omoba_phone_preview_available() };
    if touches.iter().next().is_none()
        && !mouse.pressed(MouseButton::Left)
        && let Some(enabled) = state.requested.take()
        && (!enabled || state.available)
        && let Ok(title) = std::ffi::CString::new(crate::i18n::tr("pause.preview.return"))
    {
        unsafe { omoba_phone_preview_set(enabled, title.as_ptr()) };
    }
    let active = unsafe { omoba_phone_preview_active() };
    if state.active != active {
        state.active = active;
        state.requested = None;
        epoch.bump();
        mobile.clear();
        if let Ok(window) = window.single() {
            for touch in touches.iter() {
                events.write(bevy::input::touch::TouchInput {
                    window,
                    id: touch.id(),
                    position: touch.position(),
                    force: None,
                    phase: bevy::input::touch::TouchPhase::Canceled,
                });
            }
        }
        touches.reset_all();
        mouse.reset_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_hosts_reject_actions_and_preview_is_never_persisted() {
        let mut state = PhoneLayoutPreview::default();
        state.toggle();
        assert_eq!(state.requested, None);
        assert!(!state.active);
        state.available = true;
        state.toggle();
        assert_eq!(state.requested, Some(true));
        state.active = true;
        state.available = false;
        state.toggle();
        assert_eq!(state.requested, Some(false));
        let restarted = PhoneLayoutPreview::default();
        assert!(!restarted.active);
        assert!(!restarted.available);
    }

    #[test]
    fn control_is_hidden_on_ineligible_hosts_and_retains_the_exit_state() {
        let mut app = App::new();
        app.init_resource::<PhoneLayoutPreview>()
            .add_systems(Update, sync_preview_control);
        let row = app
            .world_mut()
            .spawn((Node::default(), PhonePreviewSettings))
            .id();
        app.update();
        assert_eq!(app.world().get::<Node>(row).unwrap().display, Display::None);
        app.world_mut()
            .resource_mut::<PhoneLayoutPreview>()
            .available = true;
        app.update();
        assert_eq!(app.world().get::<Node>(row).unwrap().display, Display::Flex);
        let mut state = app.world_mut().resource_mut::<PhoneLayoutPreview>();
        state.available = false;
        state.active = true;
        app.update();
        assert_eq!(app.world().get::<Node>(row).unwrap().display, Display::Flex);
    }

    #[test]
    fn native_return_cancels_gameplay_capture_and_advances_gesture_epoch() {
        let mut app = App::new();
        app.insert_resource(PhoneLayoutPreview {
            active: true,
            ..default()
        })
        .init_resource::<Touches>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<crate::ui::GestureEpoch>()
        .init_resource::<crate::mobile_controls::MobileControls>()
        .add_message::<bevy::input::touch::TouchInput>()
        .add_systems(Update, poll_native_preview);
        app.world_mut()
            .resource_mut::<crate::mobile_controls::MobileControls>()
            .start_attack_hold_for_test();
        app.update();
        assert!(!app.world().resource::<PhoneLayoutPreview>().active);
        assert_eq!(app.world().resource::<crate::ui::GestureEpoch>().0, 1);
        assert!(
            !app.world()
                .resource::<crate::mobile_controls::MobileControls>()
                .has_active_gesture()
        );
    }
}
