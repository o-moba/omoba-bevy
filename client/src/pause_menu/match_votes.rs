//! The menu renders server eligibility; requests retain their original round.
// i18n-strict
use crate::{
    career::CareerClient,
    i18n::{Locale, tr, trf},
    net::{ClientSession, NetworkCommand},
    ui::{
        Activated, UiActionAppExt, UiSet,
        theme::{self, ButtonKind},
        tokens::{Metric, TextRole},
        widgets,
    },
};
use bevy::prelude::*;
use shared::match_vote::{Action, Kind, View};
mod prompt;
use prompt::render_prompt;
#[cfg(test)]
mod tests;

#[derive(Component)]
struct Mount;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VoteAction {
    prompt: bool,
    epoch: u64,
    match_id: u64,
    generation: u64,
    kind: Kind,
    action: Action,
}
pub(super) fn configure(app: &mut App) {
    app.add_ui_action::<VoteAction>().add_systems(
        Update,
        (actions, render, render_prompt, update_labels)
            .chain()
            .after(UiSet::Dispatch),
    );
}
pub(super) fn spawn(parent: &mut ChildSpawnerCommands) {
    parent.spawn((
        Mount,
        Node {
            width: Val::Percent(100.0),
            max_width: Val::Px(400.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            flex_shrink: 0.0,
            display: Display::None,
            ..default()
        },
        Name::new("MatchVoteMenu"),
    ));
}
fn actions(
    mut input: MessageReader<Activated<VoteAction>>,
    menu: Res<super::PauseMenuState>,
    session: Res<ClientSession>,
    screen: Res<State<crate::frontend::AppScreen>>,
    modals: Option<Res<crate::ui::ModalStack>>,
    mut outgoing: MessageWriter<NetworkCommand>,
) {
    for Activated { action, .. } in input.read() {
        let allowed = if action.prompt {
            !menu.open && modals.as_ref().is_none_or(|m| m.top().is_none())
        } else {
            menu.open && !menu.in_settings
        };
        if allowed
            && *screen.get() == crate::frontend::AppScreen::InMatch
            && session.join_confirmed()
            && !session.is_offline()
        {
            outgoing.write(NetworkCommand::MatchVote {
                server_epoch: action.epoch,
                match_id: action.match_id,
                generation: action.generation,
                kind: action.kind,
                action: action.action,
            });
        }
    }
}
fn title(kind: Kind) -> &'static str {
    tr(match kind {
        Kind::Remake => "pause.vote.remake",
        Kind::Surrender => "pause.vote.surrender",
    })
}
#[derive(Component, Clone, Copy)]
enum Caption {
    Active,
    Voted(bool),
    Cooldown,
    Unavailable(Kind),
}

// Only semantic changes replace controls. Timer/count updates must preserve an
// in-flight press on the same entity, including another teammate's vote.
fn layout_key(view: &View) -> View {
    let mut key = view.clone();
    key.cooldown_secs = u64::from(key.cooldown_secs > 0);
    key.remake.starts_in_secs = 0;
    key.surrender.starts_in_secs = 0;
    if let Some(active) = &mut key.active {
        active.yes = 0;
        active.no = 0;
        active.required = 0;
        active.eligible = 0;
        active.remaining_secs = 0;
        active.initiator_name.clear();
    }
    key
}
fn caption_text(caption: Caption, view: &View) -> String {
    match caption {
        Caption::Active => view.active.as_ref().map_or_else(String::new, |active| {
            trf(
                "pause.vote.active",
                &[
                    ("kind", &title(active.kind)),
                    ("name", &active.initiator_name),
                    ("yes", &active.yes),
                    ("required", &active.required),
                    ("no", &active.no),
                    ("seconds", &active.remaining_secs),
                ],
            )
        }),
        Caption::Voted(vote) => tr(if vote {
            "pause.vote.voted_yes"
        } else {
            "pause.vote.voted_no"
        })
        .into(),
        Caption::Cooldown => trf("pause.vote.cooldown", &[("seconds", &view.cooldown_secs)]),
        Caption::Unavailable(kind) => {
            let e = match kind {
                Kind::Remake => &view.remake,
                Kind::Surrender => &view.surrender,
            };
            let reason = match e.code.as_str() {
                "too_early" => trf("pause.vote.too_early", &[("seconds", &e.starts_in_secs)]),
                "window_closed" => tr("pause.vote.window_closed").into(),
                _ => tr("pause.vote.no_absent_teammate").into(),
            };
            trf(
                "pause.vote.unavailable",
                &[("kind", &title(kind)), ("reason", &reason)],
            )
        }
    }
}
fn update_labels(career: Res<CareerClient>, mut labels: Query<(&Caption, &mut Text)>) {
    let Some(view) = career.view.match_vote.as_ref() else {
        return;
    };
    for (caption, mut text) in &mut labels {
        let value = caption_text(*caption, view);
        if text.0 != value {
            text.0 = value;
        }
    }
}
fn label(parent: &mut ChildSpawnerCommands, caption: Caption, view: &View, unit: f32) {
    parent.spawn((
        caption,
        Text::new(caption_text(caption, view)),
        TextFont {
            font_size: (14.0 * unit).into(),
            ..default()
        },
        TextColor(theme::IVORY),
        Node {
            max_width: Val::Percent(100.0),
            flex_shrink: 0.0,
            ..default()
        },
    ));
}
fn button(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    action: VoteAction,
    id: &'static str,
    unit: f32,
) {
    widgets::spawn_button(
        parent,
        Node {
            width: Val::Percent(100.0),
            min_height: Val::Px(44.0 * unit),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        text,
        theme::TextStyle::new(TextRole::Button).sized(Metric::new(14.0 * unit, 14.0 * unit)),
        ButtonKind::Secondary,
        None,
        action,
        id.into(),
        (),
    );
}
fn render(
    mut commands: Commands,
    career: Res<CareerClient>,
    session: Res<ClientSession>,
    locale: Option<Res<Locale>>,
    scale: Option<Res<UiScale>>,
    mut mounts: Query<(Entity, &mut Node), With<Mount>>,
    mut previous: Local<Option<(Option<View>, u32, u32)>>,
) {
    let unit = 1.0 / scale.as_ref().map_or(1.0, |s| s.0).max(0.1);
    let view = career
        .view
        .match_vote
        .clone()
        .filter(|_| session.join_confirmed() && !session.is_offline());
    let signature = (
        view.as_ref().map(layout_key),
        locale.as_deref().map_or(0, Locale::generation),
        unit.to_bits(),
    );
    if previous.as_ref() == Some(&signature) {
        return;
    }
    let Ok((entity, mut node)) = mounts.single_mut() else {
        return;
    };
    *previous = Some(signature);
    commands.entity(entity).despawn_children();
    let Some(view) = view else {
        node.display = Display::None;
        return;
    };
    node.display = Display::Flex;
    commands.entity(entity).with_children(|parent| {
        if let Some(active) = &view.active {
            label(parent, Caption::Active, &view, unit);
            if let Some(vote) = active.my_vote {
                label(parent, Caption::Voted(vote), &view, unit);
            } else {
                for (action, key, id) in [
                    (Action::Yes, "pause.vote.yes", "MatchVoteYes"),
                    (Action::No, "pause.vote.no", "MatchVoteNo"),
                ] {
                    button(
                        parent,
                        tr(key),
                        VoteAction {
                            prompt: false,
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
        } else if view.cooldown_secs > 0 {
            label(parent, Caption::Cooldown, &view, unit);
        } else {
            for (kind, e, id) in [
                (Kind::Remake, &view.remake, "MatchVoteRemake"),
                (Kind::Surrender, &view.surrender, "MatchVoteSurrender"),
            ] {
                if e.available
                    && let Some(generation) = view.next_generation
                {
                    button(
                        parent,
                        title(kind),
                        VoteAction {
                            prompt: false,
                            epoch: view.server_epoch,
                            match_id: view.match_id,
                            generation,
                            kind,
                            action: Action::Start,
                        },
                        id,
                        unit,
                    );
                } else {
                    label(parent, Caption::Unavailable(kind), &view, unit);
                }
            }
        }
    });
}
