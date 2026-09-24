//! Result screen. The match is over, the world is still on screen behind it,
//! and the player decides what happens next.

use bevy::prelude::*;
use shared::career::{MatchOutcome, MatchResult};

use super::AppScreen;
use super::widgets::{self, ButtonKind};
use crate::career::CareerClient;
use crate::net::{GameState, GameStateSnapshot, NetworkCommand, SessionUiCommand};
use crate::team::Team;

pub struct PostMatchScreenPlugin;

impl Plugin for PostMatchScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppScreen::PostMatch), spawn_post_match)
            .add_systems(
                Update,
                (post_match_actions, refresh_post_match).run_if(in_state(AppScreen::PostMatch)),
            );
    }
}

#[derive(Component)]
struct PostMatchRoot(Option<MatchResult>, bool);

#[derive(Component, Clone, Copy)]
enum PostMatchAction {
    PlayAgain,
    BackToMenu,
}

/// Headline for the result, from the player's point of view.
pub fn outcome_headline(winner: Option<Team>, local_team: Option<Team>) -> &'static str {
    match (winner, local_team) {
        (Some(winner), Some(local)) if winner == local => "Victory",
        (Some(_), Some(_)) => "Defeat",
        _ => "Match complete",
    }
}

/// One line of personal numbers for the result panel.
pub fn personal_line(result: &MatchResult, profile_id: Option<&str>) -> Option<String> {
    let profile_id = profile_id?;
    let participant = result
        .participants
        .iter()
        .find(|entry| entry.profile_id.as_deref() == Some(profile_id))?;
    let rating = participant
        .rating
        .as_ref()
        .map(|change| format!(" · rating {:+}", change.delta))
        .unwrap_or_default();
    Some(format!(
        "{}/{}/{} · level {} · +{} XP{rating}",
        participant.stats.kills,
        participant.stats.deaths,
        participant.stats.assists,
        participant.stats.final_level,
        participant.progression_xp_gained,
    ))
}

fn current_result<'a>(
    career: &'a CareerClient,
    game: &GameStateSnapshot,
) -> Option<&'a MatchResult> {
    career.view.last_result.as_ref().filter(|result| {
        result.server_epoch == game.meta.server_epoch && result.match_id == game.meta.match_id
    })
}

fn spawn_post_match(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    career: Res<CareerClient>,
    local_team: Query<&Team, With<crate::player::Player>>,
) {
    let winner = match game.state {
        GameState::Victory { winner } => Some(winner.into()),
        _ => None,
    };
    let headline = outcome_headline(winner, local_team.iter().next().copied());
    let result = current_result(&career, &game).cloned();
    let summary = result.as_ref().map(|result| match result.outcome {
        MatchOutcome::Completed => format!(
            "{} destroyed the enemy base · {} min",
            result.winner.map_or("Nobody", |team| match team {
                shared::map::Team::Green => "Green",
                shared::map::Team::Blue => "Blue",
            }),
            result.duration_ms / 60_000
        ),
        MatchOutcome::Abandoned => "Match abandoned".to_owned(),
        MatchOutcome::Interrupted => "Match interrupted".to_owned(),
    });
    let personal = result
        .as_ref()
        .and_then(|result| personal_line(result, career.public_profile_id.as_deref()));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.01, 0.03, 0.035, 0.72)),
            ZIndex(widgets::SCREEN_Z),
            bevy::state::state_scoped::DespawnOnExit(AppScreen::PostMatch),
            Name::new("PostMatchScreen"),
            PostMatchRoot(result.clone(), career.view.storage_enabled),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Px(600.0),
                    max_width: Val::Percent(92.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(Val::Px(32.0)),
                    row_gap: Val::Px(12.0),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(widgets::PANEL),
                BorderColor::all(widgets::GOLD),
                Name::new("PostMatchPanel"),
            ))
            .with_children(|panel| {
                panel.spawn(widgets::label("THE VERDANT ARENA", 12.0, widgets::GOLD));
                panel.spawn(widgets::heading(headline, 42.0));
                if let Some(summary) = summary.as_deref() {
                    panel.spawn(widgets::label(summary, 15.0, widgets::IVORY));
                }
                if let Some(personal) = personal.as_deref() {
                    panel.spawn(widgets::label(personal, 14.0, widgets::GOLD));
                }
                panel.spawn(widgets::label(
                    if result.as_ref().is_some_and(|result| result.saved) {
                        "Progress saved"
                    } else if !career.view.storage_enabled {
                        "Local practice result"
                    } else {
                        "Saving match results…"
                    },
                    13.0,
                    widgets::MUTED,
                ));
                panel
                    .spawn(Node {
                        column_gap: Val::Px(12.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|row| {
                        widgets::button(
                            row,
                            "Play again",
                            ButtonKind::Primary,
                            PostMatchAction::PlayAgain,
                            "PostMatchPlayAgain",
                        );
                        widgets::button(
                            row,
                            "Back to menu",
                            ButtonKind::Secondary,
                            PostMatchAction::BackToMenu,
                            "PostMatchBackToMenu",
                        );
                    });
            });
        });
}

fn refresh_post_match(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    career: Res<CareerClient>,
    local_team: Query<&Team, With<crate::player::Player>>,
    roots: Query<(Entity, &PostMatchRoot)>,
) {
    let Ok((entity, root)) = roots.single() else {
        return;
    };
    // A retired worker clears transport state. Keep the already validated
    // terminal receipt and its Victory/Defeat presentation until the player leaves.
    if game.meta.server_epoch == 0
        || root.0.as_ref().is_some_and(|result| {
            result.server_epoch != game.meta.server_epoch || result.match_id != game.meta.match_id
        })
    {
        return;
    }
    if root.0.as_ref() != current_result(&career, &game) || root.1 != career.view.storage_enabled {
        commands
            .entity(entity)
            .despawn_related::<Children>()
            .despawn();
        spawn_post_match(commands, game, career, local_team);
    }
}

fn post_match_actions(
    mut commands: MessageWriter<NetworkCommand>,
    mut session_ui: MessageWriter<SessionUiCommand>,
    buttons: Query<(&Interaction, &PostMatchAction), Changed<Interaction>>,
    flow: Option<Res<crate::match_service::MatchServiceClient>>,
    roots: Query<&PostMatchRoot>,
) {
    for (interaction, action) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            PostMatchAction::PlayAgain => {
                if flow.as_ref().is_some_and(|flow| flow.allocation.is_some()) {
                    if roots
                        .single()
                        .ok()
                        .and_then(|root| root.0.as_ref())
                        .is_some_and(|result| result.saved)
                    {
                        session_ui.write(SessionUiCommand::LeaveMatch);
                    }
                } else {
                    commands.write(NetworkCommand::RequestRematch);
                }
            }
            PostMatchAction::BackToMenu => {
                session_ui.write(SessionUiCommand::LeaveMatch);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved_receipt() -> MatchResult {
        serde_json::from_value(serde_json::json!({
            "result_id":"previous", "server_epoch":7, "match_id":1,
            "started_at_ms":0,"ended_at_ms":1000,"duration_ms":1000,
            "map_profile":"verdant_default","ruleset":"public-casual-v1",
            "outcome":"completed","winner":"green","rated":false,
            "unrated_reason":"allocated_bots","participants":[],"saved":true
        }))
        .unwrap()
    }

    #[test]
    fn prior_saved_receipt_cannot_claim_current_match_saved() {
        let mut career = CareerClient::default();
        let mut game = GameStateSnapshot::default();
        game.meta.server_epoch = 7;
        game.meta.match_id = 2;
        let result = saved_receipt();
        career.view.last_result = Some(result);
        assert!(current_result(&career, &game).is_none());
        let receipt = career.view.last_result.as_mut().unwrap();
        receipt.match_id = 2;
        receipt.saved = false;
        assert!(!current_result(&career, &game).unwrap().saved);
        career.view.last_result.as_mut().unwrap().saved = true;
        assert!(current_result(&career, &game).unwrap().saved);
        game.meta.server_epoch += 1;
        assert!(current_result(&career, &game).is_none());
    }

    #[test]
    fn worker_retirement_preserves_rendered_terminal_panel() {
        let mut app = App::new();
        app.init_resource::<CareerClient>()
            .init_resource::<GameStateSnapshot>()
            .add_message::<NetworkCommand>()
            .add_message::<SessionUiCommand>()
            .add_systems(Update, (refresh_post_match, post_match_actions).chain());
        let mut flow = crate::match_service::MatchServiceClient::default();
        flow.allocation = Some(shared::match_service::MatchAllocation {
            allocation_id: "allocated".into(),
            endpoint: "127.0.0.1:4001".into(),
            preference: shared::match_service::MatchPreference::BotPractice,
            team: shared::map::Team::Green,
            human_count: 1,
            bot_count: 9,
            rated: false,
            join_deadline_ms: 0,
        });
        app.insert_resource(flow);
        let panel = app
            .world_mut()
            .spawn((
                PostMatchRoot(Some(saved_receipt()), true),
                Name::new("RetainedVictory"),
            ))
            .id();
        app.update();
        assert!(
            app.world().get_entity(panel).is_ok(),
            "transport teardown must not erase the terminal panel"
        );
        // A reused UDP port may expose another arena's bootstrap epoch.
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .meta
            .server_epoch = 999;
        app.update();
        assert!(app.world().get_entity(panel).is_ok());
        app.world_mut()
            .spawn((Interaction::Pressed, PostMatchAction::PlayAgain));
        app.update();
        assert_eq!(
            app.world().resource::<Messages<SessionUiCommand>>().len(),
            1,
            "the saved terminal receipt must still permit returning to the lobby"
        );
    }

    #[test]
    fn the_headline_is_written_from_the_local_point_of_view() {
        assert_eq!(
            outcome_headline(Some(Team::Green), Some(Team::Green)),
            "Victory"
        );
        assert_eq!(
            outcome_headline(Some(Team::Green), Some(Team::Blue)),
            "Defeat"
        );
        assert_eq!(outcome_headline(Some(Team::Green), None), "Match complete");
        assert_eq!(outcome_headline(None, Some(Team::Blue)), "Match complete");
    }
}
