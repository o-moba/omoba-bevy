//! Small in-match recovery and disconnected-teammate controls.
// i18n-strict
#[cfg(feature = "qa")]
mod qa;

use super::{ClientSession, NetworkCommand, SessionUiCommand};
use crate::i18n::{Locale, tr, trf};
use crate::ui::theme::{ButtonKind, TextStyle};
use crate::ui::tokens::{Metric, TextRole};
use crate::ui::{Activated, ModalStack, ScrollArea, UiActionAppExt, UiSet};
use bevy::prelude::*;
use shared::match_service::{TakeoverPolicy, TakeoverSeatView};

pub(crate) struct RecoveryUiPlugin;
impl Plugin for RecoveryUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RecoveryUi>()
            .add_ui_action::<RecoveryAction>()
            .add_systems(Update, (actions, render).chain().after(UiSet::Dispatch));
        #[cfg(feature = "qa")]
        qa::configure(app);
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryAction {
    Toggle,
    Home,
    Vote {
        player_id: u64,
        generation: u64,
        policy: TakeoverPolicy,
    },
}
#[derive(Resource, Default)]
struct RecoveryUi {
    expanded: bool,
}
#[derive(Component)]
struct RecoveryRoot;
#[derive(Clone, PartialEq)]
struct Signature {
    expanded: bool,
    viewport: (u32, u32),
    scale: u32,
    panel_top: u32,
    home: bool,
    seats: Vec<TakeoverSeatView>,
    locale: u32,
}
fn actions(
    mut input: MessageReader<Activated<RecoveryAction>>,
    mut commands: MessageWriter<NetworkCommand>,
    mut session: MessageWriter<SessionUiCommand>,
    mut ui: ResMut<RecoveryUi>,
) {
    for Activated { action, .. } in input.read() {
        match *action {
            RecoveryAction::Toggle => ui.expanded = !ui.expanded,
            RecoveryAction::Home => {
                session.write(SessionUiCommand::LeaveMatch);
            }
            RecoveryAction::Vote {
                player_id,
                generation,
                policy,
            } => {
                commands.write(NetworkCommand::TakeoverVote {
                    player_id,
                    generation,
                    policy,
                });
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn render(
    mut commands: Commands,
    screen: Res<State<crate::frontend::AppScreen>>,
    session: Res<ClientSession>,
    career: Res<crate::career::CareerClient>,
    locale: Option<Res<Locale>>,
    roots: Query<Entity, With<RecoveryRoot>>,
    mut previous: Local<Option<Signature>>,
    ui: Res<RecoveryUi>,
    modals: Option<Res<ModalStack>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    scale: Option<Res<UiScale>>,
    status: Query<
        (&ComputedNode, &UiGlobalTransform, &Node, &Visibility),
        With<super::status_ui::ConnectionStatusRoot>,
    >,
) {
    let in_match = *screen.get() == crate::frontend::AppScreen::InMatch
        && modals.as_ref().is_none_or(|stack| stack.top().is_none());
    let viewport = windows.single().map_or(Vec2::new(852.0, 393.0), |w| {
        Vec2::new(w.width(), w.height())
    });
    let unit = 1.0 / scale.as_ref().map_or(1.0, |scale| scale.0).max(0.1);
    let home = in_match && !session.join_confirmed();
    // The connection explanation varies with locale and retry state. Read its
    // previous layout and leave a physical gap instead of covering its text.
    let panel_top = if home {
        status
            .iter()
            .find_map(|(computed, transform, node, visibility)| {
                (node.display != Display::None
                    && *visibility != Visibility::Hidden
                    && computed.size().y > 0.0)
                    .then(|| physical_rect(computed, transform).max.y + 12.0)
            })
            .unwrap_or(184.0)
            .max(82.0)
    } else {
        82.0
    };
    let signature = Signature {
        expanded: ui.expanded,
        viewport: (viewport.x.to_bits(), viewport.y.to_bits()),
        scale: unit.to_bits(),
        panel_top: panel_top.to_bits(),
        home,
        seats: if in_match && session.join_confirmed() {
            career.view.takeovers.clone()
        } else {
            Vec::new()
        },
        locale: locale.as_deref().map_or(0, Locale::generation),
    };
    if previous.as_ref() == Some(&signature) {
        return;
    }
    *previous = Some(signature.clone());
    for root in &roots {
        commands.entity(root).despawn();
    }
    if !signature.home && signature.seats.is_empty() {
        return;
    }
    let panel_width = 240.0_f32.min((viewport.x - 24.0).max(44.0));
    commands
        .spawn((
            RecoveryRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                margin: UiRect::left(Val::Px(-panel_width * 0.5 * unit)),
                top: Val::Px(panel_top * unit),
                width: Val::Px(panel_width * unit),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0 * unit),
                padding: UiRect::all(Val::Px(4.0 * unit)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 0.92)),
            GlobalZIndex(30),
            Name::new("MatchRecoveryControls"),
        ))
        .with_children(|root| {
            if signature.home {
                button(
                    root,
                    tr("home.recovery.home"),
                    RecoveryAction::Home,
                    "RecoveryHome",
                    unit,
                );
                return;
            }
            button(
                root,
                &trf("home.recovery.team", &[("count", &signature.seats.len())]),
                RecoveryAction::Toggle,
                "TakeoverToggle",
                unit,
            );
            if !signature.expanded {
                return;
            }
            root.spawn((
                ScrollArea::menu(28.0),
                Node {
                    width: Val::Percent(100.0),
                    max_height: Val::Px((viewport.y - 220.0).clamp(88.0, 168.0) * unit),
                    overflow: Overflow::scroll_y(),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0 * unit),
                    ..default()
                },
                Name::new("TakeoverScroll"),
            ))
            .with_children(|list| {
                for seat in &signature.seats {
                    let policy = match seat.policy {
                        TakeoverPolicy::Bot => tr("home.recovery.bot"),
                        TakeoverPolicy::Idle => tr("home.recovery.idle"),
                    };
                    list.spawn((
                        Text::new(trf(
                            "home.recovery.seat",
                            &[("name", &seat.nickname), ("policy", &policy)],
                        )),
                        TextFont {
                            font_size: (13.0 * unit).into(),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        Node {
                            flex_shrink: 0.0,
                            ..default()
                        },
                    ));
                    list.spawn((Node {
                        column_gap: Val::Px(4.0 * unit),
                        flex_shrink: 0.0,
                        ..default()
                    },))
                        .with_children(|row| {
                            for (policy, label, votes) in [
                                (TakeoverPolicy::Bot, tr("home.recovery.bot"), seat.bot_votes),
                                (
                                    TakeoverPolicy::Idle,
                                    tr("home.recovery.idle"),
                                    seat.idle_votes,
                                ),
                            ] {
                                button(
                                    row,
                                    &format!(
                                        "{} {} ({}/{})",
                                        if seat.my_vote == Some(policy) {
                                            "✓"
                                        } else {
                                            ""
                                        },
                                        label,
                                        votes,
                                        seat.eligible_voters
                                    ),
                                    RecoveryAction::Vote {
                                        player_id: seat.player_id,
                                        generation: seat.generation,
                                        policy,
                                    },
                                    &format!("Takeover{}{:?}", seat.player_id, policy), // i18n-allow: stable test id, never displayed
                                    unit,
                                );
                            }
                        });
                }
            });
        });
}

fn physical_rect(node: &ComputedNode, transform: &UiGlobalTransform) -> Rect {
    Rect::from_center_size(
        transform.translation,
        node.size() * transform.to_scale_angle_translation().0.abs(),
    )
}

fn button(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    action: RecoveryAction,
    id: &str,
    unit: f32,
) {
    crate::ui::widgets::spawn_button(
        parent,
        Node {
            min_width: Val::Px(108.0 * unit),
            height: Val::Px(44.0 * unit),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            padding: UiRect::horizontal(Val::Px(5.0 * unit)),
            ..default()
        },
        label,
        TextStyle::new(TextRole::Button).sized(Metric::new(13.0 * unit, 13.0 * unit)),
        ButtonKind::Secondary,
        None,
        action,
        id.into(),
        (),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_id::harness;
    #[test]
    fn real_recognizer_activates_home_and_collapsed_takeover_controls() {
        let mut app = harness::kit_app();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .insert_state(crate::frontend::AppScreen::InMatch)
            .insert_resource(ClientSession::reconnecting_for_test())
            .init_resource::<crate::career::CareerClient>()
            .add_message::<SessionUiCommand>()
            .add_message::<NetworkCommand>()
            .add_plugins(RecoveryUiPlugin);
        app.update();
        let home = harness::find(app.world_mut(), "RecoveryHome").unwrap();
        assert!(app.world().get::<crate::ui::Pressable>(home).is_some());
        harness::press(app.world_mut(), "RecoveryHome");
        app.update();
        assert!(
            app.world_mut()
                .resource_mut::<Messages<SessionUiCommand>>()
                .drain()
                .any(|event| matches!(event, SessionUiCommand::LeaveMatch))
        );
        app.insert_resource(ClientSession::admitted_for_test());
        app.world_mut()
            .resource_mut::<crate::career::CareerClient>()
            .view
            .takeovers = vec![TakeoverSeatView {
            player_id: 42,
            generation: 9,
            nickname: "Teammate".into(),
            policy: TakeoverPolicy::Bot,
            idle_votes: 0,
            bot_votes: 0,
            eligible_voters: 1,
            my_vote: None,
        }];
        app.update();
        assert!(harness::find(app.world_mut(), "Takeover42Idle").is_none());
        harness::press(app.world_mut(), "TakeoverToggle");
        app.update();
        let vote = harness::find(app.world_mut(), "Takeover42Idle").unwrap();
        assert!(app.world().get::<crate::ui::Pressable>(vote).is_some());
        harness::press(app.world_mut(), "Takeover42Idle");
        app.update();
        assert!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .any(|event| matches!(
                    event,
                    NetworkCommand::TakeoverVote {
                        player_id: 42,
                        generation: 9,
                        policy: TakeoverPolicy::Idle
                    }
                ))
        );
        let mut modals = ModalStack::default();
        modals.push(crate::ui::ModalId::Pause);
        app.insert_resource(modals);
        app.update();
        assert!(harness::find(app.world_mut(), "TakeoverToggle").is_none());
    }
}
