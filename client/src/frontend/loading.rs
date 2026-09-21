//! Loading screen shown between "match found" and the first playable frame.

use bevy::prelude::*;

use super::AppScreen;
use super::widgets;
use crate::team::TeamSelection;

/// Rotating hints; a loading screen is the only place a player reads them.
const TIPS: [&str; 5] = [
    "Last-hitting minions is the safest gold in the lane.",
    "Towers hit harder than you do early. Bring minions with you.",
    "Jungle camps respawn: clear them between waves.",
    "Watch the minimap before you rotate; a missing enemy is a warning.",
    "Your ultimate is not an escape. Buy the item that is.",
];

pub struct LoadingScreenPlugin;

impl Plugin for LoadingScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppScreen::Loading), spawn_loading);
    }
}

pub fn tip_for(index: usize) -> &'static str {
    TIPS[index % TIPS.len()]
}

fn spawn_loading(mut commands: Commands, selection: Res<TeamSelection>, time: Res<Time>) {
    let hero = selection.hero_class;
    let avatar = selection
        .avatar
        .as_deref()
        .and_then(shared::avatar_definition)
        .map_or("Default avatar", |avatar| avatar.display_name.as_str());
    let tip = tip_for(time.elapsed_secs() as usize);
    commands
        .spawn(widgets::screen_root(AppScreen::Loading, "LoadingScreen"))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Px(620.0),
                    max_width: Val::Percent(92.0),
                    margin: UiRect::all(Val::Auto),
                    padding: UiRect::all(Val::Px(40.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(14.0),
                    ..default()
                },
                BackgroundColor(widgets::PANEL),
                BorderColor::all(widgets::PANEL_EDGE),
                Name::new("LoadingBody"),
            ))
            .with_children(|body| {
                body.spawn(widgets::label("MATCH FOUND", 12.0, widgets::GOLD));
                body.spawn((
                    Node {
                        width: Val::Px(48.0),
                        height: Val::Px(3.0),
                        margin: UiRect::bottom(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(widgets::GOLD),
                ));
                body.spawn(widgets::heading("Entering the Verdant", 34.0));
                body.spawn(widgets::label(
                    &format!("{} · {avatar}", hero.display_name()),
                    16.0,
                    widgets::GOLD,
                ));
                body.spawn(widgets::label(hero.tagline(), 13.0, widgets::MUTED));
                body.spawn((
                    widgets::label(tip, 14.0, widgets::IVORY),
                    Name::new("LoadingTip"),
                ));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tips_wrap_around() {
        assert_eq!(tip_for(0), TIPS[0]);
        assert_eq!(tip_for(TIPS.len()), TIPS[0]);
        assert_eq!(tip_for(TIPS.len() + 2), TIPS[2]);
    }
}
