//! Opt-in synthetic controller for existing native screenshot scenarios only.
//! This is presentation evidence, never a physical-controller or iPhone test.
#![cfg(debug_assertions)]

use super::*;

fn requested(flag: Option<&str>, capture_directory: Option<&std::ffi::OsStr>) -> bool {
    flag == Some("1") && capture_directory.is_some_and(|directory| !directory.is_empty())
}

pub(super) fn configure(app: &mut App) {
    if !requested(
        std::env::var("OMOBA_GAMEPAD_QA").ok().as_deref(),
        std::env::var_os("OMOBA_VISUAL_QA_DIR").as_deref(),
    ) {
        return;
    }
    // The ordinary sampler would report no attached hardware and clear the
    // gesture state before every synthetic frame. Skip it only in this explicit
    // fixture; the real resolver, input gates and presentation remain enabled.
    app.configure_sets(PreUpdate, GamepadInputSet::Sample.run_if(|| false))
        .add_systems(Startup, label_fixture)
        .add_systems(
            PreUpdate,
            sample_fixture
                .after(GamepadInputSet::Sample)
                .before(bevy::ui::UiSystems::Focus),
        );
    info!("GAMEPAD_QA synthetic PlayStation snapshot enabled; physical_device_verified=false");
}

fn label_fixture(mut commands: Commands) {
    commands.spawn((
        Text::new("SYNTHETIC CONTROLLER · no hardware test"),
        crate::ui_theme::text(10.0),
        TextColor(crate::ui_theme::GOLD),
        BackgroundColor(crate::ui_theme::PANEL),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(30.0),
            top: Val::Px(2.0),
            padding: UiRect::axes(Val::Px(5.0), Val::Px(2.0)),
            ..default()
        },
        GlobalZIndex(2100),
        bevy::ui::FocusPolicy::Pass,
        Pickable::IGNORE,
        Name::new("QaGamepadFixtureLabel"),
    ));
}

fn sample_fixture(
    mut controller: ResMut<GamepadControls>,
    context: Res<GameplayInputContext>,
    snapshot: Res<GameStateSnapshot>,
    local: Query<&CombatStats, With<Player>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut neutral_frames: Local<u8>,
) {
    let focused = windows.single().is_ok_and(|window| window.focused);
    let ready = focused
        && context.gameplay_allowed()
        && matches!(snapshot.state, crate::net::GameState::Running)
        && local.single().is_ok_and(|stats| stats.is_alive());
    *neutral_frames = if ready {
        neutral_frames.saturating_add(1)
    } else {
        0
    };
    let aiming = *neutral_frames > 3;
    controller.sample(
        Some(PadSnapshot {
            identity: 0x5141_5041_4401,
            playstation: true,
            left: Vec2::ZERO,
            right: if aiming {
                Vec2::new(0.85, 0.35)
            } else {
                Vec2::ZERO
            },
            // Remain held; modal/focus/death clears through the real input gate.
            // No release-to-cast or attack is scripted by the render fixture.
            buttons: if aiming { L1 } else { 0 },
        }),
        false,
        focused,
    );
    // Deliberate opt-in grants fixture ownership even during neutral lobby/menu
    // frames. It does not override native focus or the gameplay/modal resolver.
    controller.active = focused;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_requires_both_explicit_opt_in_and_an_active_capture_directory() {
        let path = Some(std::ffi::OsStr::new("/tmp/controller-capture"));
        assert!(requested(Some("1"), path));
        assert!(!requested(None, path));
        assert!(!requested(Some("0"), path));
        assert!(!requested(Some("1"), None));
        assert!(!requested(Some("1"), Some(std::ffi::OsStr::new(""))));
    }
}
