//! Active votes also reach teammates who did not open the game menu.
use super::*;
#[derive(Component)]
pub(super) struct Prompt;
type Signature = (Option<View>, u32, u32, u32);
#[allow(clippy::too_many_arguments)]
pub(super) fn render_prompt(
    mut commands: Commands,
    career: Res<CareerClient>,
    session: Res<ClientSession>,
    menu: Res<super::super::PauseMenuState>,
    screen: Res<State<crate::frontend::AppScreen>>,
    modals: Option<Res<crate::ui::ModalStack>>,
    locale: Option<Res<Locale>>,
    roots: Query<Entity, With<Prompt>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    scale: Option<Res<UiScale>>,
    mut previous: Local<Option<Signature>>,
) {
    let width = windows.single().map_or(852., |w| w.width());
    let unit = 1. / scale.as_ref().map_or(1., |s| s.0).max(0.1);
    let show = *screen.get() == crate::frontend::AppScreen::InMatch
        && session.join_confirmed()
        && !session.is_offline()
        && !menu.open
        && modals.as_ref().is_none_or(|m| m.top().is_none());
    let view = career
        .view
        .match_vote
        .clone()
        .filter(|v| show && v.active.is_some());
    let signature = (
        view.as_ref().map(layout_key),
        locale.as_deref().map_or(0, Locale::generation),
        width.to_bits(),
        unit.to_bits(),
    );
    if previous.as_ref() == Some(&signature) {
        return;
    }
    *previous = Some(signature);
    for root in &roots {
        commands.entity(root).despawn();
    }
    let Some(view) = view else {
        return;
    };
    let active = view.active.as_ref().unwrap();
    let panel_width = 300_f32.min((width - 24.).max(44.));
    commands
        .spawn((
            Prompt,
            Name::new("MatchVotePrompt"),
            crate::ui::TestId::new("MatchVotePrompt"),
            GlobalZIndex(31),
            BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 0.95)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.),
                margin: UiRect::left(Val::Px(-panel_width * 0.5 * unit)),
                top: Val::Px(82. * unit),
                width: Val::Px(panel_width * unit),
                padding: UiRect::all(Val::Px(8. * unit)),
                row_gap: Val::Px(6. * unit),
                flex_direction: FlexDirection::Column,
                ..default()
            },
        ))
        .with_children(|parent| {
            label(parent, Caption::Active, &view, unit);
            if let Some(vote) = active.my_vote {
                label(parent, Caption::Voted(vote), &view, unit);
            } else {
                for (action, key, id) in [
                    (Action::Yes, "pause.vote.yes", "MatchVotePromptYes"),
                    (Action::No, "pause.vote.no", "MatchVotePromptNo"),
                ] {
                    button(
                        parent,
                        tr(key),
                        VoteAction {
                            prompt: true,
                            epoch: view.server_epoch,
                            match_id: view.match_id,
                            generation: active.generation,
                            kind: active.kind,
                            action,
                        },
                        id,
                        unit,
                    );
                }
            }
        });
}
