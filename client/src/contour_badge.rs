//! The contour marker. A contour build (`scripts/release.py build … --contour
//! dev`) names its contour in a kit badge (`DEV`) beside the Home title and in
//! the match HUD under the score strip, so a tester's screen or screenshot never
//! passes for production. Production builds spawn nothing.
//!
//! The contour is the compile-time [`BUILD_CONTOUR`] that also names the
//! settings folder and the window title (`crate::platform`).
// i18n-strict
use bevy::prelude::*;

use crate::frontend::AppScreen;
use crate::hud_layout::HudRegion;
use crate::i18n::Localized;
use crate::session_config::BUILD_CONTOUR;
use crate::ui::widgets::surfaces::{self, BadgeKind};

pub(crate) struct ContourBadgePlugin;

impl Plugin for ContourBadgePlugin {
    fn build(&self, app: &mut App) {
        if BUILD_CONTOUR.is_some() {
            app.add_systems(OnEnter(AppScreen::InMatch), spawn_hud_badge);
        }
    }
}

/// The badge text: the contour in capitals (`DEV`); none for production.
fn label(contour: Option<&str>) -> Option<Localized> {
    let contour = contour
        .filter(|contour| !contour.is_empty())?
        .to_ascii_uppercase();
    Some(Localized::with_args(
        "common.contour_badge",
        [("contour", &contour)],
    ))
}

/// Adds this build's contour badge to `parent` (a row); nothing in production.
pub(crate) fn spawn(parent: &mut ChildSpawnerCommands) -> Option<Entity> {
    spawn_for(parent, BUILD_CONTOUR)
}

fn spawn_for(parent: &mut ChildSpawnerCommands, contour: Option<&str>) -> Option<Entity> {
    let badge = surfaces::badge(parent, label(contour)?, BadgeKind::Danger, false);
    parent
        .commands()
        .entity(badge)
        .insert(Name::new("ContourBadge"));
    Some(badge)
}

fn spawn_hud_badge(mut commands: Commands) {
    spawn_hud_badge_for(&mut commands, BUILD_CONTOUR);
}

/// The match HUD's badge ([`HudRegion::ContourBadge`]), under the modals like
/// the score strip, for the length of the match.
fn spawn_hud_badge_for(commands: &mut Commands, contour: Option<&str>) {
    if label(contour).is_none() {
        return;
    }
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            HudRegion::ContourBadge,
            ZIndex(14),
            Pickable::IGNORE,
            bevy::state::state_scoped::DespawnOnExit(AppScreen::InMatch),
            Name::new("HudContourBadge"),
        ))
        .with_children(|root| {
            spawn_for(root, contour);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{LocaleId, available_locales};

    /// The text of every spawned badge.
    fn badge_texts(app: &mut App) -> Vec<String> {
        let badges: Vec<Entity> = app
            .world_mut()
            .query::<(Entity, &Name)>()
            .iter(app.world())
            .filter(|(_, name)| name.as_str() == "ContourBadge")
            .map(|(entity, _)| entity)
            .collect();
        badges
            .into_iter()
            .map(|badge| {
                let text = app.world().get::<Children>(badge).unwrap()[0];
                app.world().get::<Text>(text).unwrap().0.clone()
            })
            .collect()
    }

    #[test]
    fn only_a_contour_build_has_a_badge_and_it_names_the_contour() {
        assert!(label(None).is_none());
        assert!(label(Some("")).is_none());
        let dev = label(Some("dev")).unwrap();
        assert_eq!(dev.key, "common.contour_badge");
        assert_eq!(dev.text_in(LocaleId::ENGLISH), "DEV");
        for locale in available_locales() {
            assert!(dev.text_in(locale).contains("DEV"), "{}", locale.code());
        }
        assert_eq!(
            label(Some("dev-2")).unwrap().text_in(LocaleId::ENGLISH),
            "DEV-2"
        );
    }

    #[test]
    fn the_home_row_gets_a_badge_only_in_a_contour_build() {
        let mut app = App::new();
        app.add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Node::default()).with_children(|row| {
                assert!(spawn_for(row, None).is_none());
                assert!(spawn_for(row, Some("dev")).is_some());
            });
        });
        app.update();
        assert_eq!(badge_texts(&mut app), ["DEV"]);
        // The kit badge relabels itself on a language change.
        let text = app
            .world_mut()
            .query::<(&Name, &Children)>()
            .iter(app.world())
            .find(|(name, _)| name.as_str() == "ContourBadge")
            .map(|(_, children)| children[0])
            .unwrap();
        assert!(app.world().get::<Localized>(text).is_some());
    }

    #[test]
    fn the_hud_badge_lives_for_the_match_in_its_region() {
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<AppScreen>()
            .add_systems(OnEnter(AppScreen::InMatch), |mut commands: Commands| {
                spawn_hud_badge_for(&mut commands, None);
                spawn_hud_badge_for(&mut commands, Some("dev"));
            });
        app.update();
        assert!(badge_texts(&mut app).is_empty(), "not before the match");
        app.world_mut()
            .resource_mut::<NextState<AppScreen>>()
            .set(AppScreen::InMatch);
        app.update();
        assert_eq!(badge_texts(&mut app), ["DEV"], "production adds none");
        let regions: Vec<HudRegion> = app
            .world_mut()
            .query::<&HudRegion>()
            .iter(app.world())
            .copied()
            .collect();
        assert_eq!(regions, [HudRegion::ContourBadge]);
        app.world_mut()
            .resource_mut::<NextState<AppScreen>>()
            .set(AppScreen::PostMatch);
        app.update();
        assert!(badge_texts(&mut app).is_empty(), "gone with the match");
    }
}
