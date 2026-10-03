//! Direct hero locks from the visible, nearby enemy entities in the snapshot.
// i18n-strict
use bevy::{prelude::*, window::PrimaryWindow};

use crate::{
    hud_layout::{HudLayout, ui_viewport},
    input_context::GameplayInputContext,
    mobile_controls::MobileControls,
    net::{
        GameState, GameStateSnapshot, NetworkAvatar, NetworkHeroClass, NetworkPlayerId,
        PlayerProgression, RemotePlayer, TargetId, TargetKind,
    },
    player::Player,
    targeting::{BasicAttackState, TargetValidity},
    team::Team,
    ui::{
        Activated, TestId, UiAction,
        tokens::{color, space},
        widgets::game::{self, PortraitSpec, PortraitView},
    },
};

use super::{CombatStats, PendingCast, TargetState, WorldPointerState};

#[derive(Component, Clone, Copy, Debug)]
pub(super) struct EnemyPortraitAction {
    entity: Entity,
    target: TargetId,
}

#[derive(Component)]
pub(super) struct EnemyPortraitArt(Entity);

fn nearby(origin: Vec3, point: Vec3) -> bool {
    origin.is_finite()
        && point.is_finite()
        && origin.xz().distance_squared(point.xz()) <= shared::vision::HERO_SIGHT_RADIUS.powi(2)
}

/// Source this list from replicated world entities, never the global scoreboard:
/// snapshot omission despawns hidden enemies, while the scoreboard is identity-only.
#[allow(clippy::type_complexity)]
pub(super) fn sync_enemy_portraits(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mobile: Option<Res<MobileControls>>,
    ui_scale: Option<Res<UiScale>>,
    local: Query<(&Transform, &Team, &CombatStats), With<Player>>,
    enemies: Query<
        (
            Entity,
            &Transform,
            &NetworkPlayerId,
            Option<&NetworkHeroClass>,
            Option<&NetworkAvatar>,
            Option<&PlayerProgression>,
            Option<&Visibility>,
        ),
        (With<RemotePlayer>, Without<Player>),
    >,
    validity: TargetValidity,
    selected: Res<TargetState>,
    mut buttons: Query<(Entity, &EnemyPortraitAction, &mut Node, &mut BorderColor)>,
    mut portraits: Query<(&EnemyPortraitArt, &mut PortraitView)>,
) {
    let local = local.single().ok().filter(|(_, _, stats)| stats.is_alive());
    let mut visible = Vec::new();
    if matches!(game.state, GameState::Running)
        && screen
            .as_ref()
            .is_none_or(|screen| *screen.get() == crate::frontend::AppScreen::InMatch)
        && let Some((origin, team, _)) = local
    {
        for (entity, transform, id, class, avatar, progression, visibility) in &enemies {
            let target = TargetId {
                kind: TargetKind::Player,
                id: id.0,
            };
            if visibility == Some(&Visibility::Hidden)
                || !validity.valid(entity, target, *team)
                || !nearby(origin.translation, transform.translation)
            {
                continue;
            }
            visible.push((
                EnemyPortraitAction { entity, target },
                PortraitView {
                    art: avatar
                        .and_then(|avatar| avatar.0.as_deref())
                        .and_then(omoba_passport::avatars::avatar_definition)
                        .and_then(crate::passport::thumbnail_asset_path),
                    fallback: game::class_icon(
                        class.map_or(shared::HeroClass::default(), |class| class.0),
                    ),
                    level: progression.map(|progression| progression.level),
                    xp: 0.0,
                    grey: false,
                    strong_rim: selected.selected_entity == Some(entity),
                },
            ));
        }
    }
    // Stable identities avoid reshuffling portraits as enemies move around.
    visible.sort_by_key(|(action, _)| action.target.id);
    visible.truncate(5);
    let viewport = window.single().ok().map_or_else(
        || {
            mobile
                .as_ref()
                .map_or(Vec2::new(1280.0, 720.0), |mobile| mobile.viewport)
        },
        |window| ui_viewport(window, ui_scale.as_deref()),
    );
    let slots = HudLayout::resolve(viewport, mobile.as_deref(), false).enemy_portrait_slots();
    for (button, action, mut node, mut border) in &mut buttons {
        let Some(slot) = visible
            .iter()
            .position(|(entry, _)| entry.entity == action.entity)
        else {
            commands.entity(button).despawn();
            continue;
        };
        node.left = Val::Px(slots[slot].min.x);
        node.top = Val::Px(slots[slot].min.y);
        *border = BorderColor::all(if selected.selected_entity == Some(action.entity) {
            color::GOLD_400
        } else {
            color::STATE_DANGER
        });
    }
    for (art, mut view) in &mut portraits {
        if let Some((_, next)) = visible.iter().find(|(entry, _)| entry.entity == art.0) {
            view.set_if_neq(next.clone());
        }
    }
    for (slot, (action, view)) in visible.into_iter().enumerate() {
        if buttons
            .iter()
            .any(|(_, existing, _, _)| existing.entity == action.entity)
        {
            continue;
        }
        // Each button owns one immutable target identity for its entire life.
        // An enemy disappearing during a tap cannot turn that tap into a lock on its replacement.
        commands
            .spawn((
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(slots[slot].min.x),
                    top: Val::Px(slots[slot].min.y),
                    width: Val::Px(44.0),
                    height: Val::Px(44.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(color::SURFACE_1_OPAQUE),
                BorderColor::all(color::STATE_DANGER),
                ZIndex(18),
                UiAction(action),
                action,
                TestId::new(format!("EnemyPortrait-{}", action.target.id)),
                Name::new(format!("EnemyPortrait-{}", action.target.id)),
            ))
            .with_children(|button| {
                game::live_portrait(
                    button,
                    view,
                    PortraitSpec {
                        side: 36.0,
                        ring: false,
                        disc: 14.0,
                        disc_at: Vec2::new(-space::S4, -space::S4),
                        icon: 24.0,
                    },
                    EnemyPortraitArt(action.entity),
                );
            });
    }
}

pub(super) fn activate_enemy_portrait(
    mut activated: MessageReader<Activated<EnemyPortraitAction>>,
    context: Res<GameplayInputContext>,
    local: Query<(&Transform, &Team, &CombatStats), With<Player>>,
    validity: TargetValidity,
    visibility: Query<&Visibility>,
    mut selected: ResMut<TargetState>,
    mut pending: ResMut<PendingCast>,
    mut basic: ResMut<BasicAttackState>,
    mut pointer: ResMut<WorldPointerState>,
    mut buttons: Query<&mut Node, With<EnemyPortraitAction>>,
) {
    // Resolve after modal toggles so portraits disappear on the same frame
    // as Settings/shop/scoreboard opens, then reappear when gameplay resumes.
    for mut node in &mut buttons {
        node.display = if context.gameplay_allowed() {
            Display::Flex
        } else {
            Display::None
        };
    }
    for Activated { action, .. } in activated.read() {
        pointer.consumed_primary_press = true;
        pointer.consumed_secondary_press = true;
        if !context.gameplay_allowed() {
            continue;
        }
        let Ok((origin, team, stats)) = local.single() else {
            continue;
        };
        if !stats.is_alive()
            || visibility
                .get(action.entity)
                .is_ok_and(|visibility| *visibility == Visibility::Hidden)
            || !validity.valid(action.entity, action.target, *team)
            || !validity
                .position(action.entity)
                .is_some_and(|point| nearby(origin.translation, point))
        {
            continue;
        }
        selected.selected_entity = Some(action.entity);
        selected.selected_target = Some(action.target);
        pending.cancel();
        basic.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        let mut mobile = MobileControls::default();
        mobile.enabled = true;
        mobile.viewport = Vec2::new(852.0, 393.0);
        app.insert_resource(mobile)
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .init_resource::<TargetState>()
            .init_resource::<GameplayInputContext>()
            .init_resource::<PendingCast>()
            .init_resource::<BasicAttackState>()
            .init_resource::<WorldPointerState>()
            .add_message::<Activated<EnemyPortraitAction>>()
            .add_systems(
                Update,
                (sync_enemy_portraits, activate_enemy_portrait).chain(),
            );
        app.world_mut().spawn((
            Player,
            Team::Green,
            Transform::default(),
            CombatStats::default(),
        ));
        app
    }

    fn enemy(app: &mut App, id: u64, x: f32) -> Entity {
        app.world_mut()
            .spawn((
                RemotePlayer,
                NetworkPlayerId(id),
                Team::Blue,
                Transform::from_xyz(x, 0.0, 0.0),
                CombatStats::default(),
                InheritedVisibility::VISIBLE,
            ))
            .id()
    }

    fn portraits(app: &mut App) -> Vec<(Entity, EnemyPortraitAction)> {
        app.world_mut()
            .query::<(Entity, &EnemyPortraitAction)>()
            .iter(app.world())
            .map(|(entity, action)| (entity, *action))
            .collect()
    }

    #[test]
    fn portraits_require_live_visible_nearby_enemy_entities_and_expire_with_them() {
        let mut app = app();
        let live = enemy(&mut app, 1, 4.0);
        let ally = enemy(&mut app, 2, 3.0);
        *app.world_mut().get_mut::<Team>(ally).unwrap() = Team::Green;
        let dead = enemy(&mut app, 3, 5.0);
        app.world_mut().get_mut::<CombatStats>(dead).unwrap().hp = 0.0;
        let hidden = enemy(&mut app, 4, 6.0);
        app.world_mut()
            .entity_mut(hidden)
            .insert(InheritedVisibility::HIDDEN);
        enemy(&mut app, 5, shared::vision::HERO_SIGHT_RADIUS + 0.1);
        app.update();
        let entries = portraits(&mut app);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1.entity, live);
        app.world_mut()
            .entity_mut(live)
            .insert(InheritedVisibility::HIDDEN);
        app.update();
        assert!(portraits(&mut app).is_empty());
        app.world_mut()
            .entity_mut(live)
            .insert(InheritedVisibility::VISIBLE);
        app.update();
        assert_eq!(portraits(&mut app).len(), 1);
        app.world_mut().despawn(live);
        app.update();
        assert!(portraits(&mut app).is_empty());
    }

    #[test]
    fn portrait_tap_locks_exact_enemy_consumes_pointer_and_never_retargets_stale_press() {
        let mut app = app();
        let first = enemy(&mut app, 17, 3.0);
        let second = enemy(&mut app, 18, 2.0);
        app.update();
        let entries = portraits(&mut app);
        let (source, action) = *entries
            .iter()
            .find(|(_, action)| action.entity == first)
            .unwrap();
        app.world_mut().write_message(Activated { source, action });
        app.update();
        assert_eq!(
            app.world().resource::<TargetState>().selected_entity,
            Some(first)
        );
        assert_eq!(
            app.world().resource::<TargetState>().selected_target,
            Some(action.target)
        );
        let pointer = app.world().resource::<WorldPointerState>();
        assert!(pointer.consumed_primary_press && pointer.consumed_secondary_press);
        assert!(app.world().resource::<BasicAttackState>().order.is_none());
        app.world_mut()
            .resource_mut::<TargetState>()
            .selected_entity = None;
        app.world_mut()
            .resource_mut::<TargetState>()
            .selected_target = None;
        app.world_mut().despawn(first);
        app.world_mut().write_message(Activated { source, action });
        app.update();
        assert!(
            app.world()
                .resource::<TargetState>()
                .selected_entity
                .is_none()
        );
        assert_eq!(portraits(&mut app)[0].1.entity, second);
        let (source, action) = portraits(&mut app)[0];
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .modal_open = true;
        app.world_mut().write_message(Activated { source, action });
        app.update();
        assert!(
            app.world()
                .resource::<TargetState>()
                .selected_entity
                .is_none()
        );
        assert_eq!(
            app.world().get::<Node>(source).unwrap().display,
            Display::None
        );
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .modal_open = false;
        app.update();
        assert_eq!(
            app.world().get::<Node>(source).unwrap().display,
            Display::Flex
        );
    }
}
