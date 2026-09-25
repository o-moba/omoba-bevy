//! One server-owned countdown and an asset barrier shared by the whole roster.
use bevy::{
    asset::RecursiveDependencyLoadState,
    ecs::system::SystemParam,
    prelude::*,
    scene::{SceneInstance, SceneSpawner},
    window::PrimaryWindow,
};
use shared::prematch::PrematchPhase;

use super::{
    AppScreen,
    draft::{self, DraftClient, DraftScroll, DraftScrollMemory, DraftSet},
    widgets,
};
use crate::{
    model_scale::ModelScaleSource,
    net::{
        GameStateSnapshot, NetworkAvatar, NetworkCharacterChoice, NetworkPlayerId,
        NetworkSpriteCharacter, SessionUiCommand,
    },
    sprite::{PlayerSpriteVisual, PlayerVisualMode},
    team::{AvatarThumbnails, CharacterChoice},
    ui::{Activated, UiActionAppExt, theme},
    verdant3d::{VerdantEnvironment, VerdantFoliage, VerdantStructureVisual},
    world::AvatarAssetCache,
    world2d::World2dStatic,
};

const TIPS: [&str; 5] = [
    "Last-hitting minions is the safest gold in the lane.",
    "Towers hit harder than you do early. Bring minions with you.",
    "Jungle camps respawn: clear them between waves.",
    "Watch the minimap before you rotate; a missing enemy is a warning.",
    "Your ultimate is not an escape. Buy the item that is.",
];
pub fn tip_for(index: usize) -> &'static str {
    TIPS[index % TIPS.len()]
}

pub struct LoadingScreenPlugin;
impl Plugin for LoadingScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_ui_action::<LoadingAction>()
            .add_systems(
                Update,
                (loading_actions, assess_readiness)
                    .chain()
                    .in_set(DraftSet::Input)
                    .run_if(in_state(AppScreen::Loading)),
            )
            .add_systems(
                Update,
                render_loading
                    .in_set(DraftSet::Draw)
                    .run_if(in_state(AppScreen::Loading)),
            );
    }
}

#[derive(Component)]
struct LoadingRoot;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoadingAction {
    Cancel,
}

/// Runs in `DraftSet::Input`, after `UiSet::Dispatch`.
fn loading_actions(
    mut activated: MessageReader<Activated<LoadingAction>>,
    mut state: ResMut<DraftClient>,
    mut session: MessageWriter<SessionUiCommand>,
) {
    if activated
        .read()
        .any(|pressed| pressed.action == LoadingAction::Cancel)
    {
        state.reset();
        session.write(SessionUiCommand::LeaveMatch);
    }
}

#[derive(SystemParam)]
struct LoadingAssets<'w, 's> {
    server: Res<'w, AssetServer>,
    spawner: Res<'w, SceneSpawner>,
    cache: Res<'w, AvatarAssetCache>,
    mode: Res<'w, PlayerVisualMode>,
    meshes: Res<'w, Assets<Mesh>>,
    materials: Res<'w, Assets<StandardMaterial>>,
    procedural: Query<'w, 's, (&'static Mesh3d, &'static MeshMaterial3d<StandardMaterial>)>,
    players: Query<
        'w,
        's,
        (
            Entity,
            &'static NetworkPlayerId,
            &'static NetworkAvatar,
            &'static NetworkCharacterChoice,
            Option<&'static NetworkSpriteCharacter>,
            Option<&'static ModelScaleSource>,
        ),
    >,
    scenes: Query<'w, 's, (&'static SceneRoot, Option<&'static SceneInstance>)>,
    children: Query<'w, 's, &'static Children>,
    map: Query<
        'w,
        's,
        Entity,
        Or<(
            With<VerdantEnvironment>,
            With<VerdantFoliage>,
            With<VerdantStructureVisual>,
        )>,
    >,
    environment: Query<'w, 's, Entity, With<VerdantEnvironment>>,
    foliage: Query<'w, 's, Entity, With<VerdantFoliage>>,
    map_sprites: Query<'w, 's, &'static Sprite, With<World2dStatic>>,
    player_sprites: Query<'w, 's, (&'static PlayerSpriteVisual, &'static Sprite)>,
}
impl LoadingAssets<'_, '_> {
    fn scene_ready(&self, entity: Entity) -> bool {
        self.scenes.get(entity).is_ok_and(|(root, instance)| {
            instance.is_some_and(|instance| self.spawner.instance_is_ready(**instance))
                && matches!(
                    self.server.recursive_dependency_load_state(root.0.id()),
                    RecursiveDependencyLoadState::Loaded
                )
        })
    }
    fn hero_scene_ready(&self, entity: Entity) -> bool {
        self.scene_ready(entity)
            || self
                .children
                .get(entity)
                .is_ok_and(|children| children.iter().any(|child| self.scene_ready(child)))
    }
    fn image_ready(&self, image: &Handle<Image>) -> bool {
        matches!(
            self.server.recursive_dependency_load_state(image.id()),
            RecursiveDependencyLoadState::Loaded
        )
    }
    fn cube_ready(&self, entity: Entity) -> bool {
        let ready = |entity| {
            procedural_assets_ready(
                CharacterChoice::Cube,
                None,
                self.procedural.get(entity).ok(),
                &self.meshes,
                &self.materials,
            )
        };
        ready(entity)
            || self
                .children
                .get(entity)
                .is_ok_and(|children| children.iter().any(ready))
    }
}

fn procedural_assets_ready(
    character: CharacterChoice,
    avatar: Option<&str>,
    rendered: Option<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
    meshes: &Assets<Mesh>,
    materials: &Assets<StandardMaterial>,
) -> bool {
    character == CharacterChoice::Cube
        && avatar.is_none()
        && rendered.is_some_and(|(mesh, material)| {
            meshes.contains(&mesh.0) && materials.contains(&material.0)
        })
}

fn assess_readiness(
    game: Res<GameStateSnapshot>,
    mut state: ResMut<DraftClient>,
    assets: LoadingAssets,
) {
    let Some(draft) = &game.prematch else {
        return;
    };
    if !matches!(
        draft.phase,
        PrematchPhase::Countdown | PrematchPhase::Loading
    ) {
        return;
    }
    let map_ready = if *assets.mode == PlayerVisualMode::Models3d {
        !assets.environment.is_empty()
            && !assets.foliage.is_empty()
            && assets.map.iter().all(|entity| assets.scene_ready(entity))
    } else {
        !assets.map_sprites.is_empty()
            && assets
                .map_sprites
                .iter()
                .all(|sprite| assets.image_ready(&sprite.image))
    };
    if !map_ready {
        state.local_assets = "Loading the battlefield…".into();
        return;
    }
    for selected in &draft.players {
        let Some((entity, _, avatar, character, sprite_character, source)) = assets
            .players
            .iter()
            .find(|(_, id, _, _, _, _)| id.0 == selected.player_id)
        else {
            state.local_assets = "Preparing the team’s heroes…".into();
            return;
        };
        if avatar.0 != selected.avatar || character.0 != selected.character {
            state.local_assets = "Applying the accepted team choices…".into();
            return;
        }
        if *assets.mode == PlayerVisualMode::Sprite2d {
            if sprite_character.is_none_or(|sprite| sprite.0 != selected.sprite_character)
                || !assets.player_sprites.iter().any(|(visual, sprite)| {
                    visual.owner() == entity && assets.image_ready(&sprite.image)
                })
            {
                state.local_assets = "Loading the team’s sprites…".into();
                return;
            }
            continue;
        }
        // Cube is an intentional built-in mesh, not an unready model fallback.
        // This exemption can never apply to a selected SDK/roster avatar.
        if selected.character == CharacterChoice::Cube && selected.avatar.is_none() {
            if assets.cube_ready(entity) {
                continue;
            }
            state.local_assets = "Preparing built-in hero materials…".into();
            return;
        }
        if let Some(slug) = &selected.avatar {
            if omoba_passport::avatars::avatar_definition(slug).is_none() {
                omoba_passport::store::request_refresh();
                state.local_assets = "Refreshing the approved Studio heroes…".into();
                return;
            }
            if omoba_passport::store::knows(slug) {
                match omoba_passport::store::model_state(slug) {
                    omoba_passport::store::ModelState::Pending => {
                        state.local_assets = "Downloading and verifying Studio heroes…".into();
                        return;
                    }
                    omoba_passport::store::ModelState::Unavailable => {
                        state.local_assets = "A Studio hero is unavailable. Cancel to choose another, or wait for the draft to reopen.".into();
                        return;
                    }
                    omoba_passport::store::ModelState::Ready => {}
                }
            }
            // NetworkAvatar can already name the choice while a fallback is
            // still instantiated. Require the final cached asset as well.
            let expected = assets
                .cache
                .requested()
                .find(|(cached, _)| *cached == slug)
                .map(|(_, handle)| handle);
            if !matching_avatar_asset(expected, source.map(|source| &source.gltf)) {
                state.local_assets = "Preparing the selected avatar models…".into();
                return;
            }
        }
        if !assets.hero_scene_ready(entity)
            || source.is_some_and(|source| {
                !matches!(
                    assets
                        .server
                        .recursive_dependency_load_state(source.gltf.id()),
                    RecursiveDependencyLoadState::Loaded
                )
            })
        {
            state.local_assets = "Loading hero materials and animations…".into();
            return;
        }
    }
    state.local_assets = "Your battlefield and all selected heroes are ready.".into();
    if draft.phase == PrematchPhase::Loading && !draft.players.is_empty() {
        state.request_loaded();
    }
}

fn matching_avatar_asset<T: Asset>(
    expected: Option<&Handle<T>>,
    actual: Option<&Handle<T>>,
) -> bool {
    expected
        .zip(actual)
        .is_some_and(|(expected, actual)| expected == actual)
}

fn render_loading(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    state: Res<DraftClient>,
    windows: Query<&Window, With<PrimaryWindow>>,
    roots: Query<Entity, With<LoadingRoot>>,
    mut thumbnails: ResMut<AvatarThumbnails>,
    assets: Res<AssetServer>,
    scroll: Res<DraftScrollMemory>,
    mut last: Local<String>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let phase = game.prematch.as_ref().map(|p| p.phase);
    let seconds = game
        .prematch
        .as_ref()
        .map_or(0, |p| p.remaining_ms.div_ceil(1000));
    let roster_key = game.prematch.as_ref().map(|p| (&p.players, &p.error));
    let key = format!(
        "{phase:?}:{seconds}:{}:{:?}:{}:{}",
        serde_json::to_string(&roster_key).unwrap_or_default(),
        state.local_assets,
        window.width(),
        window.height()
    );
    if *last == key && !roots.is_empty() {
        return;
    }
    *last = key;
    for root in &roots {
        commands.entity(root).despawn();
    }
    for entry in crate::passport::avatar_catalogue().entries {
        if let Some(path) = crate::passport::thumbnail_asset_path(&entry.avatar) {
            thumbnails
                .0
                .insert(entry.avatar.slug.clone(), assets.load(path));
        }
    }
    let compact = window.height() < 500.0;
    let countdown = phase == Some(PrematchPhase::Countdown);
    let title = if countdown {
        format!("Team ready · {seconds}")
    } else {
        "Entering the Verdant".into()
    };
    let ready = game
        .prematch
        .as_ref()
        .map_or(0, |draft| draft.players.iter().filter(|p| p.loaded).count());
    let total = game
        .prematch
        .as_ref()
        .map_or(0, |draft| draft.players.len());
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect {
                    left: Val::Px(if compact { 32.0 } else { 28.0 }),
                    right: Val::Px(if compact { 32.0 } else { 28.0 }),
                    top: Val::Px(if compact { 12.0 } else { 28.0 }),
                    bottom: Val::Px(if compact { 20.0 } else { 28.0 }),
                },
                row_gap: Val::Px(10.0),
                ..default()
            },
            BackgroundColor(theme::BACKDROP),
            ZIndex(theme::SCREEN_Z),
            DespawnOnExit(AppScreen::Loading),
            LoadingRoot,
            Name::new("LoadingScreen"),
        ))
        .with_children(|root| {
            root.spawn(Node {
                min_height: Val::Px(44.0),
                height: Val::Px(44.0),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|header| {
                header
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        ..default()
                    })
                    .with_children(|heading| {
                        heading.spawn((
                            widgets::heading(&title, if compact { 22.0 } else { 32.0 }),
                            Name::new("LoadingPhaseTitle"),
                        ));
                        heading.spawn((
                            widgets::label(
                                if countdown {
                                    "Everyone shares this countdown · choices are locked"
                                } else {
                                    "The match starts when every player is ready"
                                },
                                12.0,
                                theme::MUTED,
                            ),
                            Name::new("LoadingPhaseSubtitle"),
                        ));
                    });
                draft::action_button(
                    header,
                    "Cancel",
                    LoadingAction::Cancel,
                    "LoadingCancel",
                    86.0,
                    false,
                    false,
                );
            });
            if let Some(prematch) = &game.prematch {
                root.spawn(Node {
                    flex_grow: 1.0,
                    min_height: Val::Px(0.0),
                    column_gap: Val::Px(12.0),
                    ..default()
                })
                .with_children(|teams| {
                    let own_team = prematch
                        .players
                        .iter()
                        .find(|p| p.player_id == game.your_id)
                        .map(|p| p.team);
                    for (index, team) in [shared::map::Team::Green, shared::map::Team::Blue]
                        .into_iter()
                        .enumerate()
                    {
                        teams
                            .spawn(Node {
                                flex_grow: 1.0,
                                flex_basis: Val::Px(0.0),
                                min_width: Val::Px(0.0),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(6.0),
                                ..default()
                            })
                            .with_children(|column| {
                                column.spawn(widgets::label(
                                    if Some(team) == own_team {
                                        "YOUR TEAM"
                                    } else {
                                        "OPPONENTS"
                                    },
                                    12.0,
                                    theme::GOLD,
                                ));
                                let id = index as u8 + 2;
                                column
                                    .spawn((
                                        Node {
                                            flex_grow: 1.0,
                                            min_height: Val::Px(0.0),
                                            flex_direction: FlexDirection::Column,
                                            overflow: Overflow::scroll_y(),
                                            row_gap: Val::Px(5.0),
                                            ..default()
                                        },
                                        ScrollPosition(Vec2::new(
                                            0.0,
                                            *scroll.0.get(&id).unwrap_or(&0.0),
                                        )),
                                        DraftScroll(id),
                                        Name::new(format!("LoadingTeam-{index}")),
                                    ))
                                    .with_children(|rows| {
                                        for player in
                                            prematch.players.iter().filter(|p| p.team == team)
                                        {
                                            draft::roster_row(
                                                rows,
                                                player,
                                                game.your_id,
                                                &thumbnails,
                                                true,
                                                compact,
                                            );
                                        }
                                    });
                            });
                    }
                });
                root.spawn((
                    widgets::label(
                        &format!(
                            "{ready} / {total} players ready{}",
                            if countdown {
                                String::new()
                            } else {
                                format!(" · up to {seconds}s remaining")
                            }
                        ),
                        14.0,
                        theme::GOLD,
                    ),
                    Name::new("LoadingReadyCount"),
                ));
                root.spawn((
                    widgets::label(
                        if state.local_assets.is_empty() {
                            "Preparing the battlefield…"
                        } else {
                            &state.local_assets
                        },
                        12.0,
                        theme::IVORY,
                    ),
                    Name::new("LoadingAssetStatus"),
                ));
            } else {
                root.spawn((
                    Node {
                        flex_grow: 1.0,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    Name::new("LoadingBody"),
                ))
                .with_children(|body| {
                    body.spawn(widgets::label(
                        "Connecting to the battlefield…",
                        18.0,
                        theme::GOLD,
                    ));
                });
            }
            root.spawn((
                widgets::label(tip_for(game.meta.match_id as usize), 12.0, theme::MUTED),
                Name::new("LoadingTip"),
            ));
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tips_wrap_around() {
        assert_eq!(tip_for(0), TIPS[0]);
        assert_eq!(tip_for(TIPS.len()), TIPS[0]);
    }
    #[test]
    fn final_avatar_handle_is_required_even_when_a_fallback_scene_is_ready() {
        let mut assets = Assets::<Image>::default();
        let expected = assets.add(Image::default());
        let fallback = assets.add(Image::default());
        assert!(!matching_avatar_asset::<Image>(None, None));
        assert!(!matching_avatar_asset(Some(&expected), None));
        assert!(!matching_avatar_asset(Some(&expected), Some(&fallback)));
        assert!(matching_avatar_asset(Some(&expected), Some(&expected)));
    }

    #[test]
    fn intentional_cube_requires_actual_mesh_and_material_and_never_exempts_avatar_fallbacks() {
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<StandardMaterial>::default();
        let mesh = Mesh3d(meshes.add(Cuboid::default()));
        let material = MeshMaterial3d(materials.add(StandardMaterial::default()));
        assert!(procedural_assets_ready(
            CharacterChoice::Cube,
            None,
            Some((&mesh, &material)),
            &meshes,
            &materials
        ));
        assert!(!procedural_assets_ready(
            CharacterChoice::Cube,
            Some("sdk-avatar"),
            Some((&mesh, &material)),
            &meshes,
            &materials
        ));
        assert!(!procedural_assets_ready(
            CharacterChoice::Ipfs,
            None,
            Some((&mesh, &material)),
            &meshes,
            &materials
        ));
        assert!(!procedural_assets_ready(
            CharacterChoice::Cube,
            None,
            None,
            &meshes,
            &materials
        ));
        materials.remove(material.0.id());
        assert!(!procedural_assets_ready(
            CharacterChoice::Cube,
            None,
            Some((&mesh, &material)),
            &meshes,
            &materials
        ));
    }

    #[test]
    fn cancel_leaves_once_and_a_disabled_cancel_does_nothing() {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.init_resource::<DraftClient>()
            .add_message::<SessionUiCommand>()
            .add_ui_action::<LoadingAction>()
            .add_systems(Update, loading_actions.after(crate::ui::UiSet::Dispatch));
        harness::spawn_ui(app.world_mut(), |header| {
            draft::action_button(
                header,
                "Cancel",
                LoadingAction::Cancel,
                "LoadingCancel",
                86.0,
                false,
                false,
            );
        });
        app.update();
        let leaves = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Messages<SessionUiCommand>>()
                .drain()
                .count()
        };
        harness::press(app.world_mut(), "LoadingCancel");
        app.update();
        assert_eq!(leaves(&mut app), 1);
        app.update();
        assert_eq!(leaves(&mut app), 0);
        harness::set_disabled(app.world_mut(), "LoadingCancel", true);
        harness::press(app.world_mut(), "LoadingCancel");
        app.update();
        assert_eq!(leaves(&mut app), 0);
    }
}
