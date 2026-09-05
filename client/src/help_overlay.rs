//! Toggleable controls and onboarding copy. Does not despawn gameplay entities.

use bevy::prelude::*;

use crate::input_bindings::{
    HELP_TOGGLE_KEY, help_key_display, skill_keys_display, upgrade_key_display,
};
use crate::net::{GameState, GameStateSnapshot};

pub struct HelpOverlayPlugin;

impl Plugin for HelpOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HelpOverlayVisible>()
            .init_resource::<HelpAutoShowState>()
            .add_systems(Startup, setup_help_overlay)
            .add_systems(
                Update,
                (
                    auto_show_help_on_first_match_start,
                    toggle_help_overlay,
                    sync_help_overlay_visibility,
                )
                    .chain()
                    .in_set(crate::input_context::InputContextSet::Modal),
            );
    }
}

/// One-time prompt when the match first enters Running (session scope).
#[derive(Resource)]
struct HelpAutoShowState {
    pending: bool,
    was_running: bool,
}

impl Default for HelpAutoShowState {
    fn default() -> Self {
        Self {
            pending: true,
            was_running: false,
        }
    }
}

fn auto_show_help_on_first_match_start(
    snapshot: Res<GameStateSnapshot>,
    mut state: ResMut<HelpAutoShowState>,
    mut visible: ResMut<HelpOverlayVisible>,
) {
    let running = matches!(snapshot.state, GameState::Running);
    if running && !state.was_running && state.pending {
        visible.0 = true;
        state.pending = false;
    }
    state.was_running = running;
}

#[derive(Resource, Default)]
pub struct HelpOverlayVisible(pub bool);

#[derive(Component)]
struct HelpOverlayRoot;

#[derive(Component)]
struct HelpOverlayPanel;

fn setup_help_overlay(mut commands: Commands) {
    let body = help_overlay_body();

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                display: Display::None,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
            ZIndex(40),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            HelpOverlayRoot,
            Name::new("HelpOverlayRoot"),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        width: Val::Percent(88.0),
                        max_width: Val::Px(760.0),
                        padding: UiRect::all(Val::Px(18.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.04, 0.05, 0.08, 0.92)),
                    BorderColor::all(Color::srgba(0.5, 0.55, 0.62, 0.85)),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new(body),
                        TextFont {
                            font_size: 16.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        HelpOverlayPanel,
                    ));
                });
        });
}

fn toggle_help_overlay(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut visible: ResMut<HelpOverlayVisible>,
) {
    if !keyboard.just_pressed(HELP_TOGGLE_KEY) {
        return;
    }
    visible.0 = !visible.0;
}

fn sync_help_overlay_visibility(
    visible: Res<HelpOverlayVisible>,
    snapshot: Res<GameStateSnapshot>,
    mut root: Query<(&mut Visibility, &mut Node), With<HelpOverlayRoot>>,
) {
    let in_running_match = matches!(snapshot.state, GameState::Running);
    if !visible.is_changed() && !snapshot.is_changed() {
        return;
    }
    let Ok((mut v, mut node)) = root.single_mut() else {
        return;
    };
    // Keep lobby/victory overlays readable (game state UI sits below this z-order).
    let show_panel = visible.0 && in_running_match;
    node.display = if show_panel {
        Display::Flex
    } else {
        Display::None
    };
    *v = if show_panel {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
}

fn help_overlay_body() -> String {
    let help_key = help_key_display();
    let skills = skill_keys_display();
    let upgrade = upgrade_key_display();
    format!(
        "Quick guide (press {help_key} to close)\n\n\
MOVE: Click or tap the ground to walk.\n\
CAMERA: Mouse wheel zoom. Y toggles hero follow; Space returns to your hero. Hold Alt + right mouse to rotate in 3D. Arrow keys pan in free 2D.\n\
MINIMAP: Top-left — click to move the camera focus.\n\
ATTACK: Click or tap a hostile to select it and use Q; your hero approaches if needed.\n\
TEAMS: You: double ring · Ally: square · Enemy: triangle.\n\
TARGET: Tab selects the nearest hostile, middle-click selects without attacking, Backspace clears.\n\
CAST: Skill keys {skills} or the on-screen buttons cast at the selected target (W/E/R unlock by level).\n\
SKILL POINTS: Spend with {upgrade} or the arrows above the hotbar to rank up abilities.\n\
OBJECTIVE: Destroy any enemy lane tower to unlock its base, then destroy the base.\n\
MENU: Escape opens settings; the online match continues.\n\
DEBUG: F8 enables flight only with debug controls enabled; Space exits."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_overlay_copy_covers_core_first_match_actions() {
        let body = help_overlay_body();
        assert!(body.contains("MOVE:"));
        assert!(body.contains("TARGET:"));
        assert!(body.contains("Click or tap a hostile"));
        assert!(body.contains("on-screen buttons"));
        assert!(body.contains("CAST:"));
        assert!(body.contains("OBJECTIVE:"));
        assert!(body.contains("Y toggles hero follow"));
        assert!(body.contains(&skill_keys_display()));
    }

    #[test]
    fn help_overlay_copy_includes_toggle_hint() {
        let body = help_overlay_body();
        assert!(body.contains("press F1"));
    }
}
