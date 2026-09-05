//! Manifest-driven hero animation proxies for the true XY render world.

use bevy::prelude::*;
use shared::{
    DEFAULT_SPRITE_CHARACTER_ID, PlayerActionKind, SpriteAnimationDefinition,
    SpriteAnimationPlayback, SpriteCharacterDefinition, SpriteSheetKind,
    normalize_sprite_character_id, sprite_character_definition, sprite_character_roster,
};
use std::collections::HashMap;

use crate::combat::CombatStats;
use crate::net::{NetworkSpriteCharacter, PlayerCosmeticAction, RemotePlayer};
use crate::player::Player;
use crate::world2d::{layer, simulation_xz_to_render_xy, y_sorted_z};

const MOVEMENT_EPSILON: f32 = 0.002;
const IDLE_GRACE_SECONDS: f32 = 0.25;
const PORTRAIT_CELL_SIZE: u32 = 256;
const NAMEPLATE_FONT_SIZE: f32 = 14.0;
const NAMEPLATE_MAX_SCALE: f32 = 0.035;
const NAMEPLATE_MIN_SCALE: f32 = 0.012;
const NAMEPLATE_MAX_WIDTH_HERO_RATIO: f32 = 1.8;
const NAMEPLATE_GLYPH_WIDTH_RATIO: f32 = 0.55;

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayerVisualMode {
    #[default]
    Models3d,
    Sprite2d,
}

impl PlayerVisualMode {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Models3d => "models3d",
            Self::Sprite2d => "sprite2d",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "models3d" => Some(Self::Models3d),
            "sprite2d" => Some(Self::Sprite2d),
            _ => None,
        }
    }

    fn from_environment() -> Self {
        let Ok(raw) = std::env::var("OMOBA_PLAYER_VISUAL_MODE") else {
            return Self::Models3d;
        };
        Self::parse(&raw).unwrap_or_else(|| {
            warn!("Invalid OMOBA_PLAYER_VISUAL_MODE={raw:?}; falling back to models3d");
            Self::Models3d
        })
    }
}

#[derive(Clone)]
pub struct SpriteRenderSet {
    image: Handle<Image>,
    layout: Handle<TextureAtlasLayout>,
    action_image: Option<Handle<Image>>,
    action_layout: Option<Handle<TextureAtlasLayout>>,
}

#[derive(Resource, Default)]
pub struct SpriteVisualAssets {
    sets: HashMap<String, SpriteRenderSet>,
    portrait_image: Handle<Image>,
    portrait_layout: Handle<TextureAtlasLayout>,
    ui_frame_image: Handle<Image>,
    ui_frame_layout: Handle<TextureAtlasLayout>,
}

impl SpriteVisualAssets {
    pub fn get(&self, id: &str) -> Option<&SpriteRenderSet> {
        self.sets.get(id)
    }

    pub fn portrait(&self, index: usize) -> (Handle<Image>, Handle<TextureAtlasLayout>, usize) {
        (
            self.portrait_image.clone(),
            self.portrait_layout.clone(),
            index,
        )
    }

    pub fn ui_frame(&self) -> (Handle<Image>, Handle<TextureAtlasLayout>) {
        (self.ui_frame_image.clone(), self.ui_frame_layout.clone())
    }
}

#[derive(Component)]
pub struct PlayerSpriteVisual {
    owner: Entity,
    character_id: String,
    state: SpriteAnimationState,
    frame_offset: usize,
    elapsed_in_frame: f32,
    last_owner_position: Vec3,
    seconds_since_movement: f32,
    last_hp: f32,
    last_action_sequence: u64,
    pending_action: Option<SpriteAnimationState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpriteAnimationState {
    Idle,
    Run,
    Attack,
    Cast,
    Hit,
    Death,
}

pub struct SpriteVisualsPlugin;

impl Plugin for SpriteVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PlayerVisualMode::from_environment())
            .init_resource::<SpriteVisualAssets>()
            .add_systems(
                Startup,
                load_sprite_visual_assets
                    .after(crate::persistence::load_persistent_client_settings),
            )
            .add_systems(
                Update,
                (
                    reconcile_sprite_identity,
                    attach_sprite_visuals,
                    animate_sprite_visuals,
                )
                    .chain(),
            );
    }
}

fn reconcile_sprite_identity(
    mut commands: Commands,
    changed_owners: Query<(Entity, &NetworkSpriteCharacter), Changed<NetworkSpriteCharacter>>,
    visuals: Query<(Entity, &PlayerSpriteVisual)>,
) {
    for (owner, selected) in &changed_owners {
        let expected = normalize_sprite_character_id(selected.0.as_deref());
        for (entity, visual) in &visuals {
            if visual.owner == owner && visual.character_id != expected {
                commands
                    .entity(entity)
                    .despawn_related::<Children>()
                    .despawn();
            }
        }
    }
}

pub(crate) fn load_sprite_visual_assets(
    mode: Res<PlayerVisualMode>,
    mut assets: ResMut<SpriteVisualAssets>,
    asset_server: Res<AssetServer>,
    mut atlas_layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    if *mode != PlayerVisualMode::Sprite2d {
        return;
    }
    assets.portrait_image = asset_server.load("presentation2d/portraits.png");
    assets.ui_frame_image = asset_server.load("presentation2d/ui-frame.png");
    let mut ui_frame_layout = TextureAtlasLayout::new_empty(UVec2::splat(1024));
    ui_frame_layout.add_texture(URect::new(0, 0, 1024, 470));
    assets.ui_frame_layout = atlas_layouts.add(ui_frame_layout);
    assets.portrait_layout = atlas_layouts.add(TextureAtlasLayout::from_grid(
        UVec2::splat(PORTRAIT_CELL_SIZE),
        portrait_layout_columns(),
        1,
        None,
        None,
    ));
    for definition in sprite_character_roster() {
        let image: Handle<Image> = asset_server.load(format!("sprites/{}", definition.sheet));
        let layout = atlas_layouts.add(TextureAtlasLayout::from_grid(
            UVec2::new(definition.frame_size[0], definition.frame_size[1]),
            definition.columns,
            definition.rows,
            None,
            None,
        ));
        let action_columns = definition.action_columns.unwrap_or(8);
        let action_rows = definition.action_rows.unwrap_or(4);
        let (action_image, action_layout) = definition
            .action_sheet
            .as_ref()
            .map(|sheet| {
                let image: Handle<Image> = asset_server.load(format!("sprites/{sheet}"));
                let layout = atlas_layouts.add(TextureAtlasLayout::from_grid(
                    UVec2::from_array(definition.frame_size),
                    action_columns,
                    action_rows,
                    None,
                    None,
                ));
                (Some(image), Some(layout))
            })
            .unwrap_or_default();
        assets.sets.insert(
            definition.id.clone(),
            SpriteRenderSet {
                image,
                layout,
                action_image,
                action_layout,
            },
        );
    }
}

fn portrait_layout_columns() -> u32 {
    u32::try_from(sprite_character_roster().len())
        .expect("sprite character roster must fit a texture-atlas column count")
}

fn attach_sprite_visuals(
    mut commands: Commands,
    mode: Res<PlayerVisualMode>,
    assets: Res<SpriteVisualAssets>,
    players: Query<
        (
            Entity,
            &Transform,
            &CombatStats,
            Option<&NetworkSpriteCharacter>,
            Option<&PlayerCosmeticAction>,
        ),
        (
            Or<(With<Player>, With<RemotePlayer>)>,
            Without<PlayerSpriteVisual>,
        ),
    >,
    existing_visuals: Query<&PlayerSpriteVisual>,
) {
    if *mode != PlayerVisualMode::Sprite2d {
        return;
    }
    for (owner, transform, stats, selected, action) in &players {
        if existing_visuals.iter().any(|visual| visual.owner == owner) {
            continue;
        }
        let requested = selected.and_then(|selected| selected.0.as_deref());
        let id = normalize_sprite_character_id(requested);
        if requested.is_some_and(|requested| requested != id) {
            warn!("Unknown sprite character {requested:?}; using {id:?}");
        }
        let Some(definition) = sprite_character_definition(id) else {
            error!("Default sprite character {DEFAULT_SPRITE_CHARACTER_ID:?} is unavailable");
            continue;
        };
        let Some(render_set) = assets.get(id) else {
            warn!("Sprite render assets for {id:?} are not ready");
            continue;
        };
        let anchor = Vec2::new(
            (0.5 - definition.pivot[0]) * definition.world_height,
            (0.5 - definition.pivot[1]) * definition.world_height,
        );
        let xy = simulation_xz_to_render_xy(transform.translation);
        let mut sprite = Sprite::from_atlas_image(
            render_set.image.clone(),
            TextureAtlas {
                layout: render_set.layout.clone(),
                index: definition.animations.idle.start,
            },
        );
        sprite.custom_size = Some(Vec2::splat(definition.world_height));
        let visual_entity = commands
            .spawn((
                sprite,
                Transform::from_xyz(
                    xy.x + anchor.x,
                    xy.y + anchor.y,
                    y_sorted_z(layer::ACTOR, xy.y, owner),
                ),
                PlayerSpriteVisual {
                    owner,
                    character_id: id.to_owned(),
                    state: SpriteAnimationState::Idle,
                    frame_offset: 0,
                    elapsed_in_frame: 0.0,
                    last_owner_position: transform.translation,
                    seconds_since_movement: IDLE_GRACE_SECONDS,
                    last_hp: stats.hp,
                    last_action_sequence: action.map_or(0, |action| action.sequence),
                    pending_action: None,
                },
                Name::new(format!("PlayerSprite-{id}")),
            ))
            .id();
        commands.entity(visual_entity).with_children(|parent| {
            let nameplate_scale =
                player_nameplate_scale(&definition.display_name, definition.world_height);
            parent.spawn((
                Text2d::new(definition.display_name.clone()),
                TextFont {
                    font_size: NAMEPLATE_FONT_SIZE,
                    ..default()
                },
                TextColor(Color::WHITE),
                Transform::from_xyz(0.0, definition.world_height * 0.62, 0.1)
                    .with_scale(Vec3::splat(nameplate_scale)),
                Name::new(format!("PlayerNameplate2d-{id}")),
            ));
        });
    }
}

fn player_nameplate_scale(display_name: &str, hero_world_height: f32) -> f32 {
    let glyph_count = display_name.chars().count().max(1) as f32;
    let estimated_unscaled_width = glyph_count * NAMEPLATE_FONT_SIZE * NAMEPLATE_GLYPH_WIDTH_RATIO;
    let width_limited_scale =
        hero_world_height * NAMEPLATE_MAX_WIDTH_HERO_RATIO / estimated_unscaled_width;
    width_limited_scale.clamp(NAMEPLATE_MIN_SCALE, NAMEPLATE_MAX_SCALE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SpriteFrame {
    sheet: SpriteSheetKind,
    index: usize,
}

fn animation_definition(
    definition: &SpriteCharacterDefinition,
    state: SpriteAnimationState,
) -> Option<&SpriteAnimationDefinition> {
    match state {
        SpriteAnimationState::Idle => Some(&definition.animations.idle),
        SpriteAnimationState::Run => Some(&definition.animations.run),
        SpriteAnimationState::Attack => definition.animations.attack.as_ref(),
        SpriteAnimationState::Cast => definition.animations.cast.as_ref(),
        SpriteAnimationState::Hit => definition.animations.hit.as_ref(),
        SpriteAnimationState::Death => definition.animations.death.as_ref(),
    }
}

fn enter_state(animation: &mut PlayerSpriteVisual, state: SpriteAnimationState) {
    if animation.state != state {
        animation.state = state;
        animation.frame_offset = 0;
        animation.elapsed_in_frame = 0.0;
    }
}

fn locomotion_state(animation: &PlayerSpriteVisual, alive: bool) -> SpriteAnimationState {
    if alive && animation.seconds_since_movement < IDLE_GRACE_SECONDS {
        SpriteAnimationState::Run
    } else {
        SpriteAnimationState::Idle
    }
}

fn advance_animation(
    animation: &mut PlayerSpriteVisual,
    definition: &SpriteCharacterDefinition,
    owner_position: Vec3,
    hp: f32,
    action: PlayerCosmeticAction,
    delta_seconds: f32,
) -> SpriteFrame {
    let delta_seconds = delta_seconds.max(0.0);
    let horizontal_delta = owner_position.xz() - animation.last_owner_position.xz();
    let moved = horizontal_delta.length_squared() > MOVEMENT_EPSILON * MOVEMENT_EPSILON;
    animation.last_owner_position = owner_position;
    if moved {
        animation.seconds_since_movement = 0.0;
    } else {
        animation.seconds_since_movement += delta_seconds;
    }
    let alive = hp > 0.0;
    let respawned = animation.last_hp <= 0.0 && alive;
    let took_damage = hp < animation.last_hp && alive;
    animation.last_hp = hp;

    let incoming_action =
        if action.sequence != 0 && action.sequence != animation.last_action_sequence {
            animation.last_action_sequence = action.sequence;
            match action.kind {
                PlayerActionKind::Attack => Some(SpriteAnimationState::Attack),
                PlayerActionKind::Cast => Some(SpriteAnimationState::Cast),
                PlayerActionKind::None => None,
            }
        } else {
            None
        };

    if respawned {
        animation.pending_action = None;
        animation.last_action_sequence = action.sequence;
        enter_state(animation, locomotion_state(animation, true));
    }
    if !alive {
        animation.pending_action = None;
        enter_state(animation, SpriteAnimationState::Death);
    } else if took_damage {
        animation.pending_action = incoming_action;
        enter_state(animation, SpriteAnimationState::Hit);
    } else if let Some(incoming) = incoming_action {
        if animation.state == SpriteAnimationState::Hit {
            animation.pending_action = Some(incoming);
        } else {
            enter_state(animation, incoming);
        }
    } else if matches!(
        animation.state,
        SpriteAnimationState::Idle | SpriteAnimationState::Run
    ) {
        enter_state(animation, locomotion_state(animation, alive));
    }

    let mut remaining = delta_seconds;
    loop {
        let Some(sequence) = animation_definition(definition, animation.state) else {
            // Missing optional action data is cosmetic: finish that action
            // immediately and return to a guaranteed locomotion sequence.
            let next = if animation.state == SpriteAnimationState::Hit {
                animation
                    .pending_action
                    .take()
                    .unwrap_or_else(|| locomotion_state(animation, alive))
            } else if animation.state == SpriteAnimationState::Death {
                SpriteAnimationState::Idle
            } else {
                locomotion_state(animation, alive)
            };
            enter_state(animation, next);
            continue;
        };
        if sequence.count == 0 || !sequence.fps.is_finite() || sequence.fps <= 0.0 {
            enter_state(animation, SpriteAnimationState::Idle);
            return SpriteFrame {
                sheet: SpriteSheetKind::Locomotion,
                index: definition.animations.idle.start,
            };
        }

        let playback = match animation.state {
            SpriteAnimationState::Idle | SpriteAnimationState::Run => SpriteAnimationPlayback::Loop,
            SpriteAnimationState::Death => SpriteAnimationPlayback::HoldLast,
            SpriteAnimationState::Attack
            | SpriteAnimationState::Cast
            | SpriteAnimationState::Hit => SpriteAnimationPlayback::Once,
        };
        let frame_duration = 1.0 / sequence.fps;
        if playback == SpriteAnimationPlayback::Once {
            let time_to_end = (sequence.count - animation.frame_offset) as f32 * frame_duration
                - animation.elapsed_in_frame;
            if remaining >= time_to_end {
                remaining -= time_to_end.max(0.0);
                let next = if animation.state == SpriteAnimationState::Hit {
                    animation
                        .pending_action
                        .take()
                        .unwrap_or_else(|| locomotion_state(animation, alive))
                } else {
                    locomotion_state(animation, alive)
                };
                enter_state(animation, next);
                continue;
            }
        }

        animation.elapsed_in_frame += remaining;
        let frames_advanced = (animation.elapsed_in_frame / frame_duration).floor() as usize;
        animation.elapsed_in_frame -= frames_advanced as f32 * frame_duration;
        animation.frame_offset = match playback {
            SpriteAnimationPlayback::Loop => {
                (animation.frame_offset + frames_advanced) % sequence.count
            }
            SpriteAnimationPlayback::Once | SpriteAnimationPlayback::HoldLast => {
                (animation.frame_offset + frames_advanced).min(sequence.count - 1)
            }
        };
        return SpriteFrame {
            sheet: sequence.sheet,
            index: sequence.start + animation.frame_offset,
        };
    }
}

fn animate_sprite_visuals(
    time: Res<Time>,
    assets: Res<SpriteVisualAssets>,
    owners: Query<
        (&Transform, &CombatStats, Option<&PlayerCosmeticAction>),
        (
            Or<(With<Player>, With<RemotePlayer>)>,
            Without<PlayerSpriteVisual>,
        ),
    >,
    mut commands: Commands,
    mut visuals: Query<(Entity, &mut PlayerSpriteVisual, &mut Sprite, &mut Transform)>,
) {
    for (entity, mut visual, mut sprite, mut visual_transform) in &mut visuals {
        let Ok((transform, stats, action)) = owners.get(visual.owner) else {
            commands
                .entity(entity)
                .despawn_related::<Children>()
                .despawn();
            continue;
        };
        let Some(definition) = sprite_character_definition(&visual.character_id) else {
            continue;
        };
        let previous_owner_position = visual.last_owner_position;
        let frame = advance_animation(
            &mut visual,
            definition,
            transform.translation,
            stats.hp,
            action.copied().unwrap_or_default(),
            time.delta_secs(),
        );
        if let Some(render_set) = assets.get(&visual.character_id) {
            let (image, layout) = resolve_sprite_handles(render_set, frame.sheet);
            sprite.image = image.clone();
            if let Some(atlas) = sprite.texture_atlas.as_mut() {
                atlas.layout = layout.clone();
                atlas.index = frame.index;
            }
        }
        let xy = simulation_xz_to_render_xy(transform.translation);
        let anchor = Vec2::new(
            (0.5 - definition.pivot[0]) * definition.world_height,
            (0.5 - definition.pivot[1]) * definition.world_height,
        );
        visual_transform.translation = Vec3::new(
            xy.x + anchor.x,
            xy.y + anchor.y,
            y_sorted_z(layer::ACTOR, xy.y, visual.owner),
        );
        let delta_x = transform.translation.x - previous_owner_position.x;
        if delta_x.abs() > MOVEMENT_EPSILON {
            sprite.flip_x = delta_x < 0.0;
        }
    }
}

fn resolve_sprite_handles(
    render_set: &SpriteRenderSet,
    sheet: SpriteSheetKind,
) -> (&Handle<Image>, &Handle<TextureAtlasLayout>) {
    match sheet {
        SpriteSheetKind::Actions => render_set
            .action_image
            .as_ref()
            .zip(render_set.action_layout.as_ref())
            .unwrap_or((&render_set.image, &render_set.layout)),
        SpriteSheetKind::Locomotion => (&render_set.image, &render_set.layout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn models3d_startup_does_not_request_optional_sprite_sheets() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<Image>()
            .init_asset::<TextureAtlasLayout>()
            .insert_resource(PlayerVisualMode::Models3d)
            .init_resource::<SpriteVisualAssets>()
            .add_systems(Startup, load_sprite_visual_assets);
        app.update();
        let assets = app.world().resource::<SpriteVisualAssets>();
        assert!(assets.sets.is_empty());
        assert_eq!(assets.portrait_image, Handle::default());
        assert_eq!(assets.ui_frame_image, Handle::default());
        assert!(
            app.world()
                .resource::<Assets<TextureAtlasLayout>>()
                .is_empty()
        );
    }

    use shared::SPRITE_CHARACTER_IDS;
    use std::time::Duration;

    fn test_render_assets() -> SpriteVisualAssets {
        let mut sets = HashMap::new();
        for definition in sprite_character_roster() {
            sets.insert(
                definition.id.clone(),
                SpriteRenderSet {
                    image: Handle::default(),
                    layout: Handle::default(),
                    action_image: definition.action_sheet.as_ref().map(|_| Handle::default()),
                    action_layout: definition.action_sheet.as_ref().map(|_| Handle::default()),
                },
            );
        }
        SpriteVisualAssets {
            sets,
            portrait_image: Handle::default(),
            portrait_layout: Handle::default(),
            ui_frame_image: Handle::default(),
            ui_frame_layout: Handle::default(),
        }
    }

    fn sprite_system_app(mode: PlayerVisualMode) -> App {
        let mut app = App::new();
        app.insert_resource(mode)
            .insert_resource(test_render_assets())
            .insert_resource(Time::<()>::default())
            .add_systems(
                Update,
                (attach_sprite_visuals, animate_sprite_visuals).chain(),
            );
        app
    }

    fn animation() -> PlayerSpriteVisual {
        PlayerSpriteVisual {
            owner: Entity::PLACEHOLDER,
            character_id: DEFAULT_SPRITE_CHARACTER_ID.to_owned(),
            state: SpriteAnimationState::Idle,
            frame_offset: 0,
            elapsed_in_frame: 0.0,
            last_owner_position: Vec3::ZERO,
            seconds_since_movement: IDLE_GRACE_SECONDS,
            last_hp: 100.0,
            last_action_sequence: 0,
            pending_action: None,
        }
    }

    fn action_definition() -> SpriteCharacterDefinition {
        let mut definition = sprite_character_definition(DEFAULT_SPRITE_CHARACTER_ID)
            .unwrap()
            .clone();
        let once = |start| SpriteAnimationDefinition {
            start,
            count: 8,
            fps: 16.0,
            sheet: SpriteSheetKind::Actions,
            playback: SpriteAnimationPlayback::Once,
        };
        definition.animations.attack = Some(once(0));
        definition.animations.cast = Some(once(8));
        definition.animations.hit = Some(once(16));
        definition.animations.death = Some(SpriteAnimationDefinition {
            playback: SpriteAnimationPlayback::HoldLast,
            ..once(24)
        });
        definition
    }

    fn action(sequence: u64, kind: PlayerActionKind, slot: u8) -> PlayerCosmeticAction {
        PlayerCosmeticAction {
            sequence,
            kind,
            slot,
        }
    }

    #[test]
    fn visual_mode_is_default_safe() {
        assert_eq!(
            PlayerVisualMode::parse("models3d"),
            Some(PlayerVisualMode::Models3d)
        );
        assert_eq!(
            PlayerVisualMode::parse("sprite2d"),
            Some(PlayerVisualMode::Sprite2d)
        );
        assert_eq!(PlayerVisualMode::parse("invalid"), None);
        assert_eq!(PlayerVisualMode::default(), PlayerVisualMode::Models3d);
    }

    #[test]
    fn roster_nameplates_are_proportionate_to_each_hero() {
        for definition in sprite_character_roster() {
            let scale = player_nameplate_scale(&definition.display_name, definition.world_height);
            let text_height = NAMEPLATE_FONT_SIZE * scale;
            let estimated_width = definition.display_name.chars().count() as f32
                * NAMEPLATE_FONT_SIZE
                * NAMEPLATE_GLYPH_WIDTH_RATIO
                * scale;
            assert!(
                text_height <= definition.world_height * 0.25,
                "{} nameplate is too tall",
                definition.id
            );
            assert!(
                estimated_width <= definition.world_height * NAMEPLATE_MAX_WIDTH_HERO_RATIO + 0.001,
                "{} nameplate is too wide",
                definition.id
            );
        }
    }

    #[test]
    fn long_nameplates_shrink_below_the_normal_cap() {
        let short = player_nameplate_scale("Paco", 2.4);
        let long = player_nameplate_scale("Orchard Comet Centaur", 2.4);
        assert_eq!(short, NAMEPLATE_MAX_SCALE);
        assert!(long < short);
        assert!(long >= NAMEPLATE_MIN_SCALE);
    }

    #[test]
    fn portrait_layout_and_indices_follow_the_embedded_roster() {
        assert_eq!(portrait_layout_columns(), 10);
        assert_eq!(
            portrait_layout_columns() as usize,
            SPRITE_CHARACTER_IDS.len()
        );

        let assets = test_render_assets();
        for (index, id) in SPRITE_CHARACTER_IDS.into_iter().enumerate() {
            assert_eq!(sprite_character_roster()[index].id, id);
            assert_eq!(assets.portrait(index).2, index);
        }
    }

    #[test]
    fn render_asset_registry_covers_all_ten_six_state_characters() {
        let assets = test_render_assets();
        assert_eq!(assets.sets.len(), SPRITE_CHARACTER_IDS.len());
        for id in SPRITE_CHARACTER_IDS {
            let render_set = assets.get(id).expect("roster render set");
            assert!(render_set.action_image.is_some(), "{id}");
            assert!(render_set.action_layout.is_some(), "{id}");
        }
    }

    #[test]
    fn idle_and_run_frames_wrap_and_large_delta_advances() {
        let definition = sprite_character_definition(DEFAULT_SPRITE_CHARACTER_ID).unwrap();
        let mut state = animation();
        assert_eq!(
            advance_animation(
                &mut state,
                definition,
                Vec3::ZERO,
                100.0,
                PlayerCosmeticAction::default(),
                0.0,
            )
            .index,
            0
        );
        assert_eq!(
            advance_animation(
                &mut state,
                definition,
                Vec3::ZERO,
                100.0,
                PlayerCosmeticAction::default(),
                8.0 / 6.0,
            )
            .index,
            0
        );
        assert_eq!(
            advance_animation(
                &mut state,
                definition,
                Vec3::X,
                100.0,
                PlayerCosmeticAction::default(),
                0.0,
            )
            .index,
            8
        );
        assert_eq!(
            advance_animation(
                &mut state,
                definition,
                Vec3::X * 2.0,
                100.0,
                PlayerCosmeticAction::default(),
                10.1 / 12.0,
            )
            .index,
            10
        );
    }

    #[test]
    fn movement_grace_returns_to_idle() {
        let definition = sprite_character_definition(DEFAULT_SPRITE_CHARACTER_ID).unwrap();
        let mut state = animation();
        let no_action = PlayerCosmeticAction::default();
        advance_animation(&mut state, definition, Vec3::X, 100.0, no_action, 0.01);
        assert_eq!(state.state, SpriteAnimationState::Run);
        advance_animation(&mut state, definition, Vec3::X, 100.0, no_action, 0.20);
        assert_eq!(state.state, SpriteAnimationState::Run);
        advance_animation(&mut state, definition, Vec3::X, 100.0, no_action, 0.06);
        assert_eq!(state.state, SpriteAnimationState::Idle);
    }

    #[test]
    fn multiple_entities_keep_independent_animation_state() {
        let definition = action_definition();
        let mut attacking = animation();
        let mut idle = animation();
        let no_action = PlayerCosmeticAction::default();
        advance_animation(
            &mut attacking,
            &definition,
            Vec3::ZERO,
            100.0,
            action(1, PlayerActionKind::Attack, 0),
            0.1,
        );
        advance_animation(&mut idle, &definition, Vec3::ZERO, 100.0, no_action, 0.1);
        assert_eq!(attacking.state, SpriteAnimationState::Attack);
        assert_eq!(idle.state, SpriteAnimationState::Idle);
        assert_ne!(attacking.frame_offset, idle.frame_offset);
    }

    #[test]
    fn combat_priority_one_shots_death_hold_and_respawn_are_deterministic() {
        let definition = action_definition();
        let mut state = animation();

        let attack = action(1, PlayerActionKind::Attack, 0);
        let frame = advance_animation(&mut state, &definition, Vec3::X, 100.0, attack, 0.0);
        assert_eq!(state.state, SpriteAnimationState::Attack);
        assert_eq!(
            frame,
            SpriteFrame {
                sheet: SpriteSheetKind::Actions,
                index: 0
            }
        );

        // Hit interrupts attack, and a cast arriving during hit waits until
        // the higher-priority hit one-shot finishes.
        advance_animation(&mut state, &definition, Vec3::X, 80.0, attack, 0.0);
        assert_eq!(state.state, SpriteAnimationState::Hit);
        let cast = action(2, PlayerActionKind::Cast, 2);
        advance_animation(&mut state, &definition, Vec3::X, 80.0, cast, 0.1);
        assert_eq!(state.state, SpriteAnimationState::Hit);
        advance_animation(&mut state, &definition, Vec3::X, 80.0, cast, 0.4);
        assert_eq!(state.state, SpriteAnimationState::Cast);

        // Death overrides every pending/active action and a long frame holds
        // the final authored death frame.
        let death = advance_animation(&mut state, &definition, Vec3::X, 0.0, cast, 10.0);
        assert_eq!(state.state, SpriteAnimationState::Death);
        assert_eq!(death.index, 31);
        let ignored = action(3, PlayerActionKind::Attack, 0);
        advance_animation(&mut state, &definition, Vec3::X, 0.0, ignored, 1.0);
        assert_eq!(state.state, SpriteAnimationState::Death);

        // Respawn consumes the stale replicated action baseline and resumes
        // locomotion without replaying it.
        advance_animation(&mut state, &definition, Vec3::X, 100.0, ignored, 0.0);
        assert_eq!(state.state, SpriteAnimationState::Idle);
        assert!(state.pending_action.is_none());
    }

    #[test]
    fn one_shot_plays_once_and_long_delta_resumes_locomotion() {
        let definition = action_definition();
        let mut state = animation();
        let attack = action(1, PlayerActionKind::Attack, 0);
        advance_animation(&mut state, &definition, Vec3::ZERO, 100.0, attack, 0.0);
        assert_eq!(state.state, SpriteAnimationState::Attack);
        advance_animation(&mut state, &definition, Vec3::ZERO, 100.0, attack, 10.0);
        assert_eq!(state.state, SpriteAnimationState::Idle);
        advance_animation(&mut state, &definition, Vec3::ZERO, 100.0, attack, 0.0);
        assert_eq!(state.state, SpriteAnimationState::Idle);
    }

    #[test]
    fn missing_action_animation_falls_back_without_panicking() {
        let mut definition = sprite_character_definition(DEFAULT_SPRITE_CHARACTER_ID)
            .unwrap()
            .clone();
        definition.animations.cast = None;
        let mut state = animation();
        let frame = advance_animation(
            &mut state,
            &definition,
            Vec3::ZERO,
            100.0,
            action(1, PlayerActionKind::Cast, 3),
            0.1,
        );
        assert_eq!(state.state, SpriteAnimationState::Idle);
        assert_eq!(frame.sheet, SpriteSheetKind::Locomotion);
    }

    #[test]
    fn missing_action_sheet_render_assets_fall_back_without_panicking() {
        let mut render_set = test_render_assets()
            .sets
            .remove(DEFAULT_SPRITE_CHARACTER_ID)
            .expect("default render set");
        render_set.action_image = None;
        render_set.action_layout = None;
        let rendered = resolve_sprite_handles(&render_set, SpriteSheetKind::Actions);
        assert_eq!(rendered.0, &render_set.image);
        assert_eq!(rendered.1, &render_set.layout);
    }

    #[test]
    fn sprite_mode_attaches_all_new_remote_visuals_once() {
        let mut app = sprite_system_app(PlayerVisualMode::Sprite2d);
        let local_id = SPRITE_CHARACTER_IDS[0];
        let local = app
            .world_mut()
            .spawn((
                Player,
                Transform::default(),
                CombatStats::default(),
                NetworkSpriteCharacter(Some(local_id.to_owned())),
            ))
            .id();
        let remotes = SPRITE_CHARACTER_IDS[5..]
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let entity = app
                    .world_mut()
                    .spawn((
                        RemotePlayer,
                        Transform::from_xyz(index as f32 + 2.0, 0.0, 0.0),
                        CombatStats::default(),
                        NetworkSpriteCharacter(Some((*id).to_owned())),
                    ))
                    .id();
                (entity, *id)
            })
            .collect::<Vec<_>>();

        app.update();
        app.update();

        let mut visuals = app.world_mut().query::<(&PlayerSpriteVisual, &Sprite)>();
        let mut attached = visuals
            .iter(app.world())
            .map(|(visual, sprite)| (visual.owner, visual.character_id.clone(), sprite.image.id()))
            .collect::<Vec<_>>();
        attached.sort_by_key(|(owner, _, _)| owner.index());

        assert_eq!(
            attached.len(),
            1 + remotes.len(),
            "a second update must not duplicate visuals"
        );
        let local_visual = attached
            .iter()
            .find(|(owner, _, _)| *owner == local)
            .expect("local sprite visual");
        assert_eq!(local_visual.1, local_id);
        for (remote, id) in remotes {
            let remote_visual = attached
                .iter()
                .find(|(owner, _, _)| *owner == remote)
                .expect("remote sprite visual");
            assert_eq!(remote_visual.1, id);
        }
    }

    #[test]
    fn models3d_mode_does_not_attach_sprite_visuals() {
        let mut app = sprite_system_app(PlayerVisualMode::default());
        app.world_mut().spawn((
            Player,
            Transform::default(),
            CombatStats::default(),
            NetworkSpriteCharacter(Some(SPRITE_CHARACTER_IDS[0].to_owned())),
        ));
        app.world_mut().spawn((
            RemotePlayer,
            Transform::default(),
            CombatStats::default(),
            NetworkSpriteCharacter(Some(SPRITE_CHARACTER_IDS[1].to_owned())),
        ));

        app.update();

        let mut visuals = app.world_mut().query::<&PlayerSpriteVisual>();
        assert_eq!(visuals.iter(app.world()).count(), 0);
    }

    #[test]
    fn animation_system_changes_mesh_for_position_and_time_deltas() {
        let mut app = sprite_system_app(PlayerVisualMode::Sprite2d);
        let owner = app
            .world_mut()
            .spawn((
                Player,
                Transform::default(),
                CombatStats::default(),
                NetworkSpriteCharacter(Some(DEFAULT_SPRITE_CHARACTER_ID.to_owned())),
            ))
            .id();
        app.update();

        let initial_frame = app
            .world_mut()
            .query::<(&PlayerSpriteVisual, &Sprite)>()
            .single(app.world())
            .expect("one attached sprite visual")
            .1
            .texture_atlas
            .as_ref()
            .unwrap()
            .index;

        app.world_mut()
            .entity_mut(owner)
            .get_mut::<Transform>()
            .expect("owner transform")
            .translation
            .x = 1.0;
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(1.0 / 12.0));
        app.update();

        let (running_state, running_sprite) = app
            .world_mut()
            .query::<(&PlayerSpriteVisual, &Sprite)>()
            .single(app.world())
            .expect("one attached sprite visual");
        assert_eq!(running_state.state, SpriteAnimationState::Run);
        let running_frame = running_sprite.texture_atlas.as_ref().unwrap().index;
        assert_ne!(running_frame, initial_frame);

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(IDLE_GRACE_SECONDS + 0.01));
        app.update();

        let (idle_state, returned_idle_sprite) = app
            .world_mut()
            .query::<(&PlayerSpriteVisual, &Sprite)>()
            .single(app.world())
            .expect("one attached sprite visual");
        assert_eq!(idle_state.state, SpriteAnimationState::Idle);
        assert_ne!(
            returned_idle_sprite.texture_atlas.as_ref().unwrap().index,
            running_frame
        );
    }

    #[test]
    fn sprite_visual_is_independent_xy_proxy() {
        let mut app = sprite_system_app(PlayerVisualMode::Sprite2d);
        let owner = app
            .world_mut()
            .spawn((
                Player,
                Transform::from_xyz(3.0, 7.0, -4.0),
                CombatStats::default(),
                NetworkSpriteCharacter(None),
            ))
            .id();
        app.update();
        let (_, transform) = app
            .world_mut()
            .query::<(&PlayerSpriteVisual, &Transform)>()
            .single(app.world())
            .unwrap();
        assert!((transform.translation.x - 3.0).abs() < 2.0);
        assert!((transform.translation.y - -4.0).abs() < 2.0);
        assert_eq!(
            app.world()
                .entity(owner)
                .get::<Transform>()
                .unwrap()
                .translation,
            Vec3::new(3.0, 7.0, -4.0)
        );
    }
}
