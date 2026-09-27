//! Toggleable controls and onboarding copy. Does not despawn gameplay entities.
//!
//! The copy is platform-specific: the desktop guide (keyboard and mouse,
//! `help.body`) and the phone guide (touch controls, `help.phone.body`), both
//! `Localized`, so they follow a language change while the overlay exists.
// i18n-strict

use bevy::prelude::*;

use crate::i18n::Localized;
use crate::input_bindings::{
    HELP_TOGGLE_KEY, help_key_display, skill_keys_display, upgrade_key_display,
};
use crate::net::{ClientSession, GameState, GameStateSnapshot};
use crate::ui::theme::ButtonKind;
use crate::ui::widgets::ButtonStyle;
use crate::ui::{Activated, TestId, UiAction, UiActionAppExt};

pub struct HelpOverlayPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum HelpOverlaySet {
    Input,
}

impl Plugin for HelpOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HelpOverlayVisible>()
            .init_resource::<HelpAutoShowState>()
            .add_ui_action::<HelpAction>()
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
                    .after(crate::ui::UiSet::Dispatch)
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
    if running && !state.was_running && state.pending && !crate::sandbox::requested() {
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

/// The overlay's one button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelpAction {
    Dismiss,
}

fn setup_help_overlay(mut commands: Commands, platform: Option<Res<crate::ui::UiPlatform>>) {
    let phone = platform.is_some_and(|platform| platform.is_mobile());
    let (body, dismiss) = if phone {
        (
            Localized::new("help.phone.body"),
            Localized::new("help.phone.dismiss"),
        )
    } else {
        (help_overlay_text(), Localized::new("help.dismiss"))
    };

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
            ZIndex(crate::ui::theme::SCREEN_Z + 50),
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
                        row_gap: Val::Px(16.0),
                        padding: UiRect::all(Val::Px(24.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(crate::ui::theme::PANEL.with_alpha(1.0)),
                    BorderColor::all(crate::ui::theme::EDGE),
                    Name::new("HelpPanel"),
                ))
                .with_children(|panel| {
                    if !phone {
                        panel.spawn((
                            Localized::new("help.eyebrow").into_text(),
                            crate::ui::theme::text(11.0),
                            TextColor(crate::ui::theme::GOLD),
                        ));
                        panel.spawn((
                            Localized::new("help.title").into_text(),
                            crate::ui::theme::text(28.0),
                            TextColor(crate::ui::theme::IVORY),
                        ));
                    }
                    panel.spawn((
                        body.into_text(),
                        TextFont {
                            font_size: 15.0,
                            ..default()
                        },
                        TextColor(crate::ui::theme::IVORY),
                        HelpOverlayPanel,
                        Name::new("HelpBody"),
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
                            BackgroundColor(crate::ui::theme::PRIMARY),
                            ButtonStyle::new(ButtonKind::Primary),
                            UiAction(HelpAction::Dismiss),
                            HelpDismissButton,
                            TestId::new("HelpDismissButton"),
                        ))
                        .with_children(|button| {
                            button.spawn((
                                dismiss.into_text(),
                                Name::new("HelpDismissLabel"),
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
    mut back: crate::ui::BackInput,
    game: Res<GameStateSnapshot>,
    mut visible: ResMut<HelpOverlayVisible>,
    career: Option<Res<crate::career::CareerClient>>,
    social: Option<Res<crate::social::SocialClient>>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
) {
    if social
        .as_ref()
        .is_some_and(|social| social.blocks_gameplay())
        || career.as_ref().is_some_and(|career| career.modal_open())
    {
        return;
    }
    if visible.0
        && (matches!(game.state, GameState::Running)
            || screen.as_ref().is_some_and(|screen| screen.get().is_menu()))
        && back.just_pressed()
    {
        visible.0 = false;
        // Consume only this dismissal so the same key does not open Pause.
        back.consume();
    } else if back.keys().just_pressed(HELP_TOGGLE_KEY) {
        visible.0 = !visible.0;
    }
}

fn dismiss_help_button(
    mut activated: MessageReader<Activated<HelpAction>>,
    mut visible: ResMut<HelpOverlayVisible>,
) {
    for Activated { action, .. } in activated.read() {
        match action {
            HelpAction::Dismiss => visible.0 = false,
        }
    }
}

fn sync_help_overlay_visibility(
    visible: Res<HelpOverlayVisible>,
    snapshot: Res<GameStateSnapshot>,
    session: Option<Res<ClientSession>>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    mut root: Query<(&mut Visibility, &mut Node), With<HelpOverlayRoot>>,
) {
    let in_running_match = matches!(snapshot.state, GameState::Running)
        && session
            .as_ref()
            .is_none_or(|session| session.join_confirmed());
    if !visible.is_changed()
        && !snapshot.is_changed()
        && session.as_ref().is_none_or(|session| !session.is_changed())
        && screen.as_ref().is_none_or(|screen| !screen.is_changed())
    {
        return;
    }
    let Ok((mut v, mut node)) = root.single_mut() else {
        return;
    };
    // Explicit Help requests are usable before admission too. Automatic
    // first-match onboarding remains gated separately by local admission.
    let shell = screen.as_ref().is_some_and(|screen| screen.get().is_menu());
    let show_panel = visible.0 && (in_running_match || shell);
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

/// The desktop guide with this build's key bindings filled in.
fn help_overlay_text() -> Localized {
    Localized::with_args(
        "help.body",
        [
            ("skills", &skill_keys_display()),
            ("upgrade", &upgrade_key_display()),
            ("help_key", &help_key_display()),
        ],
    )
}

/// The desktop guide in the active language.
#[cfg(test)]
fn help_overlay_body() -> String {
    help_overlay_text().text()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_help_opens_and_dismisses_before_admission_without_auto_onboarding() {
        for state in [GameState::Lobby, GameState::Running] {
            let mut app = App::new();
            app.init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<ClientSession>()
                .insert_resource(State::new(crate::frontend::AppScreen::Home))
                .insert_resource(GameStateSnapshot { state, ..default() })
                .add_plugins(HelpOverlayPlugin);
            app.update();
            assert!(!app.world().resource::<HelpOverlayVisible>().0);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(HELP_TOGGLE_KEY);
            app.update();
            let mut roots = app
                .world_mut()
                .query_filtered::<(&Node, &Visibility), With<HelpOverlayRoot>>();
            let (node, visibility) = roots.single(app.world()).unwrap();
            assert_eq!(node.display, Display::Flex);
            assert_eq!(*visibility, Visibility::Visible);
            assert!(app.world().resource::<HelpAutoShowState>().pending);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .reset_all();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Escape);
            app.update();
            assert!(!app.world().resource::<HelpOverlayVisible>().0);
            let (node, visibility) = roots.single(app.world()).unwrap();
            assert_eq!(node.display, Display::None);
            assert_eq!(*visibility, Visibility::Hidden);
            assert!(
                !app.world()
                    .resource::<ButtonInput<KeyCode>>()
                    .just_pressed(KeyCode::Escape)
            );
        }
    }

    #[test]
    fn help_overlay_copy_covers_core_first_match_actions() {
        let body = help_overlay_body();
        assert!(body.contains("MOVE:"));
        assert!(body.contains("TARGET:"));
        assert!(body.contains("Right-click a hostile"));
        assert!(body.contains("on-screen buttons"));
        assert!(body.contains("CAST:"));
        assert!(body.contains("OBJECTIVE:"));
        assert!(body.contains("Y toggles hero follow"));
        assert!(body.contains(&skill_keys_display()));
    }

    /// The platform guide follows a language change while the overlay
    /// exists, with the key bindings still filled in. Isolated: it switches
    /// the process-wide language.
    #[test]
    fn help_copy_follows_the_language_on_desktop_and_phone() {
        if crate::i18n::testing::isolated(
            "help_overlay::tests::help_copy_follows_the_language_on_desktop_and_phone",
        ) {
            return;
        }
        use crate::i18n::{I18nPlugin, Locale, LocaleId};
        let zh = LocaleId::parse("zh-Hans").unwrap();
        for (profile, body_start, dismiss) in [
            (
                crate::platform::UiProfile::Desktop,
                "移动：",
                "进入竞技场   /   Esc 或 F1",
            ),
            (
                crate::platform::UiProfile::Mobile,
                "你的第一场对局",
                "知道了，开始游戏",
            ),
        ] {
            let mut app = App::new();
            app.add_plugins(I18nPlugin::default())
                .insert_resource(crate::ui::UiPlatform(profile))
                .init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<GameStateSnapshot>()
                .add_plugins(HelpOverlayPlugin);
            app.update();
            let body = |app: &mut App| {
                app.world_mut()
                    .query_filtered::<&Text, With<HelpOverlayPanel>>()
                    .single(app.world())
                    .unwrap()
                    .0
                    .clone()
            };
            let english = body(&mut app);
            assert!(
                english.starts_with(if profile == crate::platform::UiProfile::Mobile {
                    "YOUR FIRST MATCH"
                } else {
                    "MOVE:"
                })
            );
            app.world_mut().resource_mut::<Locale>().set(zh);
            app.update();
            let chinese = body(&mut app);
            assert!(chinese.starts_with(body_start), "{chinese}");
            if profile == crate::platform::UiProfile::Desktop {
                assert!(chinese.contains(&skill_keys_display()));
                assert!(chinese.contains("按 F1"));
            }
            let label = app
                .world_mut()
                .query::<(&Name, &Text)>()
                .iter(app.world())
                .find(|(name, _)| name.as_str() == "HelpDismissLabel")
                .map(|(_, text)| text.0.clone())
                .unwrap();
            assert_eq!(label, dismiss);
        }
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
        app.world_mut()
            .resource_mut::<ClientSession>()
            .set_state_for_test(crate::net::ClientConnectionState::Connected);
        app.update();
        assert!(!app.world().resource::<HelpOverlayVisible>().0);
        assert!(app.world().resource::<HelpAutoShowState>().pending);
    }
}
