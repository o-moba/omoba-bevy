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
                post_match_actions.run_if(in_state(AppScreen::PostMatch)),
            );
    }
}

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
        "{}/{}/{} · level {}{rating}",
        participant.stats.kills,
        participant.stats.deaths,
        participant.stats.assists,
        participant.stats.final_level,
    ))
}

fn spawn_post_match(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    career: Res<CareerClient>,
    local_team: Query<&Team, With<crate::player::Player>>,
) {
    let winner = match game.state {
        GameState::Victory { winner } => Some(winner),
        _ => None,
    };
    let headline = outcome_headline(winner, local_team.iter().next().copied());
    let result = career.view.last_result.clone();
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

fn post_match_actions(
    mut commands: MessageWriter<NetworkCommand>,
    mut session_ui: MessageWriter<SessionUiCommand>,
    buttons: Query<(&Interaction, &PostMatchAction), Changed<Interaction>>,
) {
    for (interaction, action) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            PostMatchAction::PlayAgain => {
                commands.write(NetworkCommand::RequestRematch);
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
