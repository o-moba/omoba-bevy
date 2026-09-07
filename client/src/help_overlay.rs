//! Toggleable controls and onboarding copy. Does not despawn gameplay entities.

use bevy::prelude::*;

use crate::input_bindings::{
    HELP_TOGGLE_KEY, help_key_display, skill_keys_display, upgrade_key_display,
};
use crate::net::{ClientSession, GameState, GameStateSnapshot};

pub struct HelpOverlayPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum HelpOverlaySet {
    Input,
}

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
                    dismiss_help_button,
                    sync_help_overlay_visibility,
                )
                    .chain()
                    .in_set(HelpOverlaySet::Input)
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
    session: Option<Res<ClientSession>>,
    mut state: ResMut<HelpAutoShowState>,
    mut visible: ResMut<HelpOverlayVisible>,
) {
    let running = matches!(snapshot.state, GameState::Running)
        && session
            .as_ref()
            .is_none_or(|session| session.join_confirmed());
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

#[derive(Component)]
struct HelpDismissButton;

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
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(18.0),
                        padding: UiRect::all(Val::Px(22.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(crate::ui_theme::PANEL),
                    BorderColor::all(crate::ui_theme::GOLD),
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
                    panel
                        .spawn((
                            Button,
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Px(44.0),
                                flex_shrink: 0.0,
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.15, 0.42, 0.29)),
                            HelpDismissButton,
                            Name::new("HelpDismissButton"),
                        ))
                        .with_children(|button| {
                            button.spawn((
                                Text::new("Got it - play  [Escape / F1]"),
                                TextFont {
                                    font_size: 18.0,
                                    ..default()
                                },
                                TextColor::WHITE,
                            ));
                        });
                });
        });
}

fn toggle_help_overlay(
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    game: Res<GameStateSnapshot>,
    mut visible: ResMut<HelpOverlayVisible>,
) {
    if visible.0
        && matches!(game.state, GameState::Running)
        && keyboard.just_pressed(KeyCode::Escape)
    {
        visible.0 = false;
        // Consume only this dismissal so the same key does not open Pause.
        keyboard.clear_just_pressed(KeyCode::Escape);
    } else if keyboard.just_pressed(HELP_TOGGLE_KEY) {
        visible.0 = !visible.0;
    }
}

fn dismiss_help_button(
    buttons: Query<&Interaction, (With<HelpDismissButton>, Changed<Interaction>)>,
    mut visible: ResMut<HelpOverlayVisible>,
) {
    if buttons
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        visible.0 = false;
    }
}

fn sync_help_overlay_visibility(
    visible: Res<HelpOverlayVisible>,
    snapshot: Res<GameStateSnapshot>,
    session: Option<Res<ClientSession>>,
    mut root: Query<(&mut Visibility, &mut Node), With<HelpOverlayRoot>>,
) {
    let in_running_match = matches!(snapshot.state, GameState::Running)
        && session
            .as_ref()
            .is_none_or(|session| session.join_confirmed());
    if !visible.is_changed()
        && !snapshot.is_changed()
        && session.as_ref().is_none_or(|session| !session.is_changed())
    {
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
        "Quick guide (press {help_key} or Escape to close)\n\n\
MOVE: Click or tap the ground to walk.\n\
ATTACK: Click or tap a hostile to select it and use Q; your hero approaches if needed.\n\
CAST: Use {skills} or the on-screen buttons. W/E/R unlock by level.\n\
SKILL POINTS: Use {upgrade} or the arrows above the hotbar to rank up abilities.\n\
TARGET: Tab selects the nearest hostile; Backspace clears.\n\
TEAMS: You have a double ring; allies have squares; enemies have triangles.\n\n\
OBJECTIVE: Follow a lane with your minions. Destroy an enemy lane tower to unlock its base, then destroy the base to win.\n\
RECOVER: Let minions take tower fire. If defeated, wait for your respawn.\n\
SHOP: Press P or click Open shop. Buy recommended items at your base with earned gold.\n\n\
CAMERA: Y toggles hero follow; Space returns to your hero. Wheel zooms. Hold Alt + right mouse to orbit in 3D. Click the minimap to look around.\n\
MENU: Escape opens settings. The online match continues; a rematch starts automatically after victory."
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
    #[test]
    fn first_match_help_button_dismisses_and_does_not_reopen_on_rematch() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .add_plugins(HelpOverlayPlugin);
        app.update();
        assert!(app.world().resource::<HelpOverlayVisible>().0);
        let button = app
            .world_mut()
            .query_filtered::<Entity, With<HelpDismissButton>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();
        assert!(!app.world().resource::<HelpOverlayVisible>().0);
        let mut root = app
            .world_mut()
            .query_filtered::<(&Node, &Visibility), With<HelpOverlayRoot>>();
        let (node, visibility) = root.single(app.world()).unwrap();
        assert_eq!(node.display, Display::None);
        assert_eq!(*visibility, Visibility::Hidden);
        app.world_mut().entity_mut(button).insert(Interaction::None);
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Lobby;
        app.update();
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        app.update();
        assert!(!app.world().resource::<HelpOverlayVisible>().0);
    }

    #[test]
    fn running_server_does_not_show_first_match_help_before_local_admission() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ClientSession>()
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .add_plugins(HelpOverlayPlugin);
        app.world_mut().resource_mut::<ClientSession>().state =
            crate::net::ClientConnectionState::Connected;
        app.update();
        assert!(!app.world().resource::<HelpOverlayVisible>().0);
        assert!(app.world().resource::<HelpAutoShowState>().pending);
    }
}
