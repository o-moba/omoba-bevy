//! Browser-local renderer for the real OMOBA practice authority.
//! No socket, account, identity key, career write, or simulated online session.
use bevy::{camera::ScalingMode, prelude::*};
use common::offline::{EPOCH, LOCAL_ADDR, PracticeSession};
use shared::{HeroClass, clock::Instant, map::Team, wire::*};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU32, Ordering},
};

const CLASSES: [HeroClass; 7] = [
    HeroClass::Dawnweaver,
    HeroClass::Wildspark,
    HeroClass::Cinderforge,
    HeroClass::Edgeweaver,
    HeroClass::Stormfist,
    HeroClass::Veilstalker,
    HeroClass::Emberveil,
];
const SKINS: [&str; 7] = [
    "mossback-teapot",
    "neon-axolotl-courier",
    "origami-storm-heron",
    "clockwork-turnip-oracle",
    "void-jelly-astronaut",
    "cathedral-moth-bellringer",
    "tidal-typewriter-crab",
];
const GREEN: Color = Color::srgb(0.57, 0.85, 0.48);
const BLUE: Color = Color::srgb(0.41, 0.65, 0.98);
const GOLD: Color = Color::srgb(0.94, 0.80, 0.44);
static FRAME: AtomicU32 = AtomicU32::new(0);
static ACTIONS: AtomicU32 = AtomicU32::new(0);
static LOCAL_X: AtomicU32 = AtomicU32::new(0);
static LOCAL_Z: AtomicU32 = AtomicU32::new(0);
static ROUND: AtomicU32 = AtomicU32::new(1);
static ACCEPTED: AtomicU32 = AtomicU32::new(0);
static MANA: AtomicU32 = AtomicU32::new(0);
// Read-only build smoke diagnostics, available through Trunk's wasm exports.
#[unsafe(no_mangle)]
pub extern "C" fn omoba_practice_frames() -> u32 {
    FRAME.load(Ordering::Relaxed)
}
#[unsafe(no_mangle)]
pub extern "C" fn omoba_practice_actions() -> u32 {
    ACTIONS.load(Ordering::Relaxed)
}
#[unsafe(no_mangle)]
pub extern "C" fn omoba_practice_x() -> f32 {
    f32::from_bits(LOCAL_X.load(Ordering::Relaxed))
}
#[unsafe(no_mangle)]
pub extern "C" fn omoba_practice_z() -> f32 {
    f32::from_bits(LOCAL_Z.load(Ordering::Relaxed))
}
#[unsafe(no_mangle)]
pub extern "C" fn omoba_practice_round() -> u32 {
    ROUND.load(Ordering::Relaxed)
}
#[unsafe(no_mangle)]
pub extern "C" fn omoba_practice_accepted_actions() -> u32 {
    ACCEPTED.load(Ordering::Relaxed)
}
#[unsafe(no_mangle)]
pub extern "C" fn omoba_practice_mana() -> f32 {
    f32::from_bits(MANA.load(Ordering::Relaxed))
}

#[derive(Resource)]
struct Practice {
    simulation: PracticeSession,
    frame: ServerPacket,
    class: usize,
    paused: bool,
    request: u64,
    route: Vec<[f32; 2]>,
    aim: Vec2,
}
impl Practice {
    fn new(class: usize, round: u64) -> Self {
        shared::catalog::ensure_loaded();
        // The pure host advances its own clock. Give it a virtual epoch with
        // room for backdated cooldowns/wave timers even just after page load,
        // when performance.now() may still be smaller than one second.
        let epoch = Instant::now() + std::time::Duration::from_secs(86_400);
        let mut simulation = PracticeSession::new(epoch);
        simulation.match_id = round;
        simulation.set_bot_difficulty(shared::bot_profile::BotDifficulty::Regular);
        simulation.command(ClientPacket::Join {
            handheld: Default::default(),
            prematch: false,
            team: Team::Green,
            character: CharacterChoice::Ipfs,
            hero_class: CLASSES[class],
            avatar: None,
            sprite_character: Some(SKINS[class].into()),
            session_id: None,
            passport_ticket: None,
        });
        let frame = simulation.snapshot();
        ROUND.store(round as u32, Ordering::Relaxed);
        Self {
            simulation,
            frame,
            class,
            paused: false,
            request: 0,
            route: Vec::new(),
            aim: Vec2::ZERO,
        }
    }
    fn restart(&mut self, class: usize) {
        *self = Self::new(class, self.simulation.match_id + 1);
    }
    fn send(&mut self, packet: ClientPacket) {
        ACTIONS.fetch_add(1, Ordering::Relaxed);
        self.simulation.command(packet);
    }
    fn next_request(&mut self) -> u64 {
        self.request += 1;
        self.request
    }
}
#[derive(Resource)]
struct Art {
    heroes: Vec<Handle<Image>>,
    frames: Handle<TextureAtlasLayout>,
    props: Handle<Image>,
    prop_frames: Handle<TextureAtlasLayout>,
}
#[derive(Component)]
struct Actor(u64);
#[derive(Component)]
struct Hud;
#[derive(Component)]
struct Skills;
#[derive(Component)]
struct ClassButton(usize);
#[derive(Component)]
struct RestartButton;
#[derive(Component)]
struct PauseButton;

fn main() {
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb(0.025, 0.055, 0.045)))
        .insert_resource(Practice::new(1, 1))
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "OMOBA · Verdant Practice".into(),
                        canvas: Some("#game".into()),
                        fit_canvas_to_parent: true,
                        prevent_default_event_handling: true,
                        resolution: (1280, 720).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: "assets".into(),
                    meta_check: bevy::asset::AssetMetaCheck::Never,
                    ..default()
                }),
        )
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (buttons, input, simulate, actors, draw, hud).chain(),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut atlases: ResMut<Assets<TextureAtlasLayout>>,
) {
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: 70.0,
            },
            ..OrthographicProjection::default_2d()
        }),
    ));
    let terrain = assets.load("world2d/terrain-atlas.png");
    let terrain_frames = atlases.add(TextureAtlasLayout::from_grid(
        UVec2::splat(128),
        8,
        4,
        None,
        None,
    ));
    let props = assets.load("world2d/props-atlas.png");
    let art = Art {
        heroes: SKINS
            .iter()
            .map(|skin| assets.load(format!("sprites/{skin}.png")))
            .collect(),
        frames: atlases.add(TextureAtlasLayout::from_grid(
            UVec2::splat(256),
            8,
            2,
            None,
            None,
        )),
        props,
        prop_frames: terrain_frames.clone(),
    };
    let lanes: Vec<_> = [
        shared::map::Lane::Top,
        shared::map::Lane::Mid,
        shared::map::Lane::Bot,
    ]
    .map(shared::map::lane_points)
    .into_iter()
    .collect();
    let geometry = shared::map::geometry();
    let tile = 3.2;
    let half = geometry.bounds.max[0];
    let count = (half * 2.0 / tile).ceil() as i32;
    for y in 0..count {
        for x in 0..count {
            let p = Vec2::new(
                -half + (x as f32 + 0.5) * tile,
                -half + (y as f32 + 0.5) * tile,
            );
            let road = lanes.iter().any(|lane| {
                lane.windows(2).any(|s| {
                    segment_distance(p, Vec2::from(s[0]), Vec2::from(s[1]))
                        < shared::map::LANE_WIDTH * 0.5
                })
            });
            let river = (p.x + p.y).abs() < 7.0;
            let home = p.distance(Vec2::from(geometry.home)) < 16.0;
            let away = p.distance(Vec2::from(geometry.away)) < 16.0;
            let frame = if home {
                20
            } else if away {
                21
            } else if river && road {
                17
            } else if road {
                6 + ((x + y) as usize % 2)
            } else if river {
                8 + ((x + y) as usize % 3)
            } else {
                ((x * 13 + y * 7) as usize) % 4
            };
            let mut sprite = Sprite::from_atlas_image(
                terrain.clone(),
                TextureAtlas {
                    layout: terrain_frames.clone(),
                    index: frame,
                },
            );
            sprite.custom_size = Some(Vec2::splat(tile + 0.01));
            commands.spawn((sprite, Transform::from_xyz(p.x, p.y, -100.0)));
        }
    }
    for (i, obstacle) in shared::navigation::world_navigation()
        .obstacles()
        .iter()
        .enumerate()
    {
        let middle = obstacle
            .vertices
            .iter()
            .map(|v| Vec2::from(*v))
            .sum::<Vec2>()
            / obstacle.vertices.len() as f32;
        let tree = obstacle.kind.contains("tree") || obstacle.kind.contains("forest");
        let mut sprite = Sprite::from_atlas_image(
            art.props.clone(),
            TextureAtlas {
                layout: art.prop_frames.clone(),
                index: if tree { i % 3 } else { 9 },
            },
        );
        sprite.custom_size = Some(if tree {
            Vec2::new(7.0, 8.0)
        } else {
            Vec2::new(4.5, 3.5)
        });
        commands.spawn((
            sprite,
            Transform::from_xyz(middle.x, middle.y + 1.0, -20.0 - middle.y * 0.01),
        ));
    }
    commands.insert_resource(art);
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(16),
            left: px(20),
            right: px(20),
            padding: UiRect::all(px(12)),
            border_radius: BorderRadius::all(px(12)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.025, 0.055, 0.045, 0.92)),
        children![(
            Text::new("OMOBA  /  VERDANT CONFLUENCE"),
            TextFont {
                font_size: FontSize::Px(17.0),
                ..default()
            },
            TextColor(GOLD),
            Hud
        )],
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: px(104),
            left: px(20),
            padding: UiRect::all(px(12)),
            border_radius: BorderRadius::all(px(10)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.025, 0.055, 0.045, 0.94)),
        children![(
            Text::new("Q  W  E  R"),
            TextFont {
                font_size: FontSize::Px(15.0),
                ..default()
            },
            TextColor(Color::WHITE),
            Skills
        )],
    ));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(15),
                left: px(20),
                right: px(20),
                column_gap: px(6),
                row_gap: px(6),
                flex_wrap: FlexWrap::Wrap,
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.055, 0.045, 0.94)),
        ))
        .with_children(|parent| {
            for (i, class) in CLASSES.into_iter().enumerate() {
                parent.spawn((
                    Button,
                    ClassButton(i),
                    Node {
                        padding: UiRect::axes(px(10), px(12)),
                        border_radius: BorderRadius::all(px(6)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.10, 0.19, 0.15)),
                    children![(
                        Text::new(format!("{}  {}", i + 1, class.display_name())),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                        TextColor(Color::WHITE)
                    )],
                ));
            }
            parent.spawn((
                Button,
                PauseButton,
                Node {
                    padding: UiRect::axes(px(10), px(12)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.17, 0.20, 0.16)),
                children![(
                    Text::new("Esc  Pause"),
                    TextFont {
                        font_size: FontSize::Px(12.0),
                        ..default()
                    }
                )],
            ));
            parent.spawn((
                Button,
                RestartButton,
                Node {
                    padding: UiRect::axes(px(10), px(12)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.23, 0.20, 0.11)),
                children![(
                    Text::new("N  Restart"),
                    TextFont {
                        font_size: FontSize::Px(12.0),
                        ..default()
                    }
                )],
            ));
        });
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(0.001)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

fn buttons(
    mut practice: ResMut<Practice>,
    mut query: Query<
        (
            &Interaction,
            Option<&ClassButton>,
            Has<RestartButton>,
            Has<PauseButton>,
            &mut BackgroundColor,
        ),
        (With<Button>, Changed<Interaction>),
    >,
) {
    for (interaction, class, restart, pause, mut color) in &mut query {
        *color = match interaction {
            Interaction::Hovered => Color::srgb(0.22, 0.34, 0.23),
            Interaction::Pressed => GOLD,
            Interaction::None => Color::srgb(0.10, 0.19, 0.15),
        }
        .into();
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Some(class) = class {
            practice.restart(class.0);
        } else if restart {
            let class = practice.class;
            practice.restart(class);
        } else if pause {
            practice.paused = !practice.paused;
        }
    }
}

fn input(
    mut practice: ResMut<Practice>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        practice.paused = !practice.paused;
    }
    if keys.just_pressed(KeyCode::KeyN) {
        let class = practice.class;
        practice.restart(class);
    }
    let class_keys = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
    ];
    for (i, key) in class_keys.into_iter().enumerate() {
        if keys.just_pressed(key) {
            practice.restart(i);
        }
    }
    if practice.paused {
        return;
    }
    let Some(local) = practice.simulation.world.players.get(&LOCAL_ADDR) else {
        return;
    };
    let position = Vec2::new(local.hero.x, local.hero.z);
    let y = local.hero.y;
    let dash_sequence = local.hero.utility.dash_sequence;
    let speed = common::hero_stats::move_speed(local);
    if let Some(cursor) = window.cursor_position()
        && let Ok(aim) = camera.0.viewport_to_world_2d(camera.1, cursor)
    {
        practice.aim = aim;
    }
    if mouse.just_pressed(MouseButton::Right) {
        let discs: Vec<_> = practice
            .simulation
            .world
            .structures
            .values()
            .filter(|s| s.state.hp > 0.0)
            .map(|s| shared::navigation::Disc {
                center: [s.state.x, s.state.z],
                radius: if s.state.kind == StructureKind::BaseTower {
                    3.2
                } else {
                    1.3
                },
            })
            .collect();
        practice.route = shared::navigation::world_navigation()
            .plan_route(position.into(), practice.aim.into(), &discs)
            .unwrap_or_default();
    }
    let pressed = |key| if keys.pressed(key) { 1.0 } else { 0.0 };
    let mut direction = Vec2::new(
        pressed(KeyCode::ArrowRight) - pressed(KeyCode::ArrowLeft),
        pressed(KeyCode::ArrowUp) - pressed(KeyCode::ArrowDown),
    );
    if direction.length_squared() > 0.0 {
        practice.route.clear();
    } else if let Some(point) = practice.route.first().copied() {
        direction = Vec2::from(point) - position;
        if direction.length() < 0.6 {
            practice.route.remove(0);
        }
    }
    if direction.length_squared() > 0.01 {
        let delta = direction.normalize() * speed * time.delta_secs().min(0.05);
        let next = position + delta;
        practice.send(ClientPacket::Transform {
            x: next.x,
            y,
            z: next.y,
            yaw: delta.x.atan2(delta.y),
            dash_sequence,
        });
    }
    let round = practice.simulation.match_id;
    if mouse.pressed(MouseButton::Left) || keys.pressed(KeyCode::Space) {
        // Never attack through the DOM/HUD: bottom and top bands own their clicks.
        let on_world = window
            .cursor_position()
            .is_none_or(|p| p.y > 85.0 && p.y < window.height() - 95.0);
        if (keys.pressed(KeyCode::Space) || on_world)
            && let Some(target) = nearest_target(&practice, position)
        {
            let request_id = practice.next_request();
            practice.send(ClientPacket::BasicAttack {
                target,
                server_epoch: EPOCH,
                match_id: round,
                request_id,
            });
        }
    }
    for (slot, key) in [KeyCode::KeyQ, KeyCode::KeyW, KeyCode::KeyE, KeyCode::KeyR]
        .into_iter()
        .enumerate()
    {
        if keys.just_pressed(key) {
            let request_id = practice.next_request();
            let aim = practice.aim.into();
            practice.send(ClientPacket::CastSkill {
                slot: slot as u8,
                aim,
                server_epoch: EPOCH,
                match_id: round,
                request_id,
            });
        }
    }
    for (key, action) in [
        (KeyCode::KeyF, shared::utility::UtilityAction::Dash),
        (KeyCode::KeyB, shared::utility::UtilityAction::Recall),
    ] {
        if keys.just_pressed(key) {
            let request_id = practice.next_request();
            let direction = (practice.aim - position).normalize_or_zero().into();
            practice.send(ClientPacket::Utility {
                action,
                direction,
                server_epoch: EPOCH,
                match_id: round,
                request_id,
            });
        }
    }
}

fn nearest_target(practice: &Practice, position: Vec2) -> Option<TargetId> {
    let ServerPacket::Snapshot {
        players,
        minions,
        structures,
        neutrals,
        ..
    } = &practice.frame
    else {
        return None;
    };
    let mut targets = Vec::new();
    for p in players
        .iter()
        .filter(|p| p.team != Team::Green && p.hp > 0.0)
    {
        targets.push((
            Vec2::new(p.x, p.z).distance_squared(position),
            TargetId::player(p.id),
        ));
    }
    for p in minions
        .iter()
        .filter(|p| p.team != Team::Green && p.hp > 0.0)
    {
        targets.push((
            Vec2::new(p.x, p.z).distance_squared(position),
            TargetId {
                kind: TargetKind::Minion,
                id: p.id,
            },
        ));
    }
    for p in structures
        .iter()
        .filter(|p| p.team != Team::Green && p.hp > 0.0 && !p.protected)
    {
        targets.push((
            Vec2::new(p.x, p.z).distance_squared(position),
            TargetId {
                kind: TargetKind::Structure,
                id: p.id,
            },
        ));
    }
    for p in neutrals.iter().filter(|p| p.hp > 0.0) {
        targets.push((
            Vec2::new(p.x, p.z).distance_squared(position),
            TargetId {
                kind: TargetKind::Neutral,
                id: p.id,
            },
        ));
    }
    targets
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, target)| target)
}

fn simulate(mut practice: ResMut<Practice>, time: Res<Time>) {
    if !practice.paused {
        practice.simulation.advance(time.delta_secs().min(0.05));
    }
    practice.frame = practice.simulation.snapshot();
    FRAME.fetch_add(1, Ordering::Relaxed);
    if let Some(p) = practice.simulation.world.players.get(&LOCAL_ADDR) {
        LOCAL_X.store(p.hero.x.to_bits(), Ordering::Relaxed);
        LOCAL_Z.store(p.hero.z.to_bits(), Ordering::Relaxed);
        ACCEPTED.store(p.hero.last_action.sequence as u32, Ordering::Relaxed);
        MANA.store(p.hero.mana.to_bits(), Ordering::Relaxed);
    }
}

fn actors(
    mut commands: Commands,
    art: Res<Art>,
    practice: Res<Practice>,
    time: Res<Time>,
    mut actors: Query<(Entity, &Actor, &mut Transform, &mut Sprite)>,
    mut camera: Query<(&mut Transform, &Camera), (With<Camera2d>, Without<Actor>)>,
) {
    let ServerPacket::Snapshot {
        players, your_id, ..
    } = &practice.frame
    else {
        return;
    };
    let mut remaining: HashMap<_, _> = players
        .iter()
        .filter(|p| p.hp > 0.0)
        .map(|p| (p.id, p))
        .collect();
    for (entity, actor, mut transform, mut sprite) in &mut actors {
        if let Some(player) = remaining.remove(&actor.0) {
            update_actor(
                &mut transform,
                &mut sprite,
                player,
                *your_id,
                &art,
                time.elapsed_secs(),
            );
        } else {
            commands.entity(entity).despawn();
        }
    }
    for player in remaining.values() {
        let mut sprite = Sprite::from_atlas_image(
            art.heroes[skin_index(player.hero_class)].clone(),
            TextureAtlas {
                layout: art.frames.clone(),
                index: 0,
            },
        );
        let mut transform = Transform::default();
        update_actor(
            &mut transform,
            &mut sprite,
            player,
            *your_id,
            &art,
            time.elapsed_secs(),
        );
        commands.spawn((sprite, transform, Actor(player.id)));
    }
    if let Some(p) = players.iter().find(|p| p.id == *your_id) {
        for (mut transform, camera) in &mut camera {
            let viewport = camera
                .logical_viewport_size()
                .unwrap_or(Vec2::new(1280.0, 720.0));
            let half = Vec2::new(35.0 * viewport.x / viewport.y.max(1.0), 35.0);
            let bounds = shared::map::geometry().bounds;
            let limit_x = (bounds.max[0] - half.x).max(0.0);
            let limit_y = (bounds.max[1] - half.y).max(0.0);
            let target = Vec3::new(
                p.x.clamp(-limit_x, limit_x),
                (p.z + 4.0).clamp(-limit_y, limit_y),
                transform.translation.z,
            );
            transform.translation = transform.translation.lerp(target, 0.18);
        }
    }
}
fn skin_index(class: HeroClass) -> usize {
    CLASSES
        .iter()
        .position(|c| *c == class)
        .unwrap_or(class as usize % SKINS.len())
}
fn update_actor(
    transform: &mut Transform,
    sprite: &mut Sprite,
    p: &PlayerState,
    your_id: u64,
    art: &Art,
    elapsed: f32,
) {
    let old = transform.translation.truncate();
    let moving = old.distance(Vec2::new(p.x, p.z)) > 0.005;
    transform.translation = Vec3::new(p.x, p.z + 1.5, 10.0 - p.z * 0.01);
    sprite.image = art.heroes[skin_index(p.hero_class)].clone();
    sprite.custom_size = Some(Vec2::splat(if p.id == your_id { 5.5 } else { 5.0 }));
    sprite.color = if p.team == Team::Green {
        Color::WHITE
    } else {
        Color::srgb(0.75, 0.83, 1.0)
    };
    sprite.flip_x = p.yaw.sin() < 0.0;
    if let Some(atlas) = &mut sprite.texture_atlas {
        atlas.index =
            (if moving { 8 } else { 0 }) + (elapsed * if moving { 12.0 } else { 6.0 }) as usize % 8;
    }
}

fn draw(mut gizmos: Gizmos, practice: Res<Practice>, time: Res<Time>) {
    let ServerPacket::Snapshot {
        players,
        your_id,
        structures,
        minions,
        neutrals,
        projectiles,
        combat_events,
        ..
    } = &practice.frame
    else {
        return;
    };
    for p in players.iter().filter(|p| p.hp > 0.0) {
        let pos = Vec2::new(p.x, p.z);
        let color = if p.team == Team::Green { GREEN } else { BLUE };
        gizmos.circle_2d(pos, if p.id == *your_id { 1.1 } else { 0.85 }, color);
        bar(
            &mut gizmos,
            pos + Vec2::Y * 4.1,
            p.hp / p.max_hp,
            3.6,
            color,
        );
    }
    for s in structures {
        let pos = Vec2::new(s.x, s.z);
        let color = if s.team == Team::Green { GREEN } else { BLUE };
        let radius = if s.kind == StructureKind::BaseTower {
            3.2
        } else {
            1.5
        };
        gizmos.circle_2d(pos, radius, color);
        gizmos.circle_2d(pos, radius * 0.72, color.with_alpha(0.5));
        gizmos.rect_2d(pos, Vec2::splat(radius), GOLD);
        bar(
            &mut gizmos,
            pos + Vec2::Y * (radius + 0.9),
            s.hp / s.max_hp,
            4.0,
            color,
        );
    }
    for m in minions {
        let color = if m.team == Team::Green { GREEN } else { BLUE };
        let pos = Vec2::new(m.x, m.z);
        gizmos.circle_2d(pos, 0.65, color);
        bar(
            &mut gizmos,
            pos + Vec2::Y * 1.1,
            m.hp / m.max_hp,
            1.4,
            color,
        );
    }
    for n in neutrals {
        let pos = Vec2::new(n.x, n.z);
        gizmos.circle_2d(pos, 1.2, GOLD);
        bar(&mut gizmos, pos + Vec2::Y * 2.0, n.hp / n.max_hp, 3.0, GOLD);
    }
    for p in projectiles {
        gizmos.circle_2d(
            Vec2::new(p.x, p.z),
            0.28,
            if p.owner_team == Team::Green {
                GOLD
            } else {
                BLUE
            },
        );
    }
    for event in combat_events.iter().rev().take(24) {
        gizmos.circle_2d(
            Vec2::new(event.x, event.z),
            0.6 + (time.elapsed_secs() * 3.0).fract(),
            Color::srgba(1.0, 0.58, 0.23, 0.65),
        );
    }
    if let Some(p) = players.iter().find(|p| p.id == *your_id) {
        let aim = practice.aim;
        gizmos.circle_2d(aim, 0.5, GOLD.with_alpha(0.7));
        if let Some(next) = practice.route.first() {
            gizmos.line_2d(
                Vec2::new(p.x, p.z),
                Vec2::from(*next),
                GREEN.with_alpha(0.3),
            );
        }
    }
}
fn bar(gizmos: &mut Gizmos, position: Vec2, fraction: f32, width: f32, color: Color) {
    gizmos.line_2d(
        position - Vec2::X * width * 0.5,
        position + Vec2::X * width * 0.5,
        Color::srgb(0.04, 0.06, 0.04),
    );
    gizmos.line_2d(
        position - Vec2::X * width * 0.5,
        position + Vec2::X * width * (fraction.clamp(0.0, 1.0) - 0.5),
        color,
    );
}
fn hud(
    practice: Res<Practice>,
    mut title: Single<&mut Text, With<Hud>>,
    mut skills: Single<&mut Text, (With<Skills>, Without<Hud>)>,
) {
    let ServerPacket::Snapshot {
        players,
        your_id,
        scoreboard,
        game_state,
        ..
    } = &practice.frame
    else {
        return;
    };
    let seconds = practice.simulation.world.match_elapsed_secs as u32;
    let green = scoreboard.as_ref().map_or(0, |s| {
        s.players
            .iter()
            .filter(|p| p.team == Team::Green)
            .map(|p| p.kills)
            .sum::<u32>()
    });
    let blue = scoreboard.as_ref().map_or(0, |s| {
        s.players
            .iter()
            .filter(|p| p.team == Team::Blue)
            .map(|p| p.kills)
            .sum::<u32>()
    });
    let state = if practice.paused {
        "PAUSED | Esc to resume"
    } else if matches!(
        game_state,
        GameState::Victory {
            winner: Team::Green
        }
    ) {
        "VICTORY | N to play again"
    } else if matches!(game_state, GameState::Victory { .. }) {
        "DEFEAT | N to play again"
    } else {
        "LOCAL PRACTICE | BOTS | NO RATING"
    };
    let hero = players.iter().find(|p| p.id == *your_id);
    **title = Text::new(format!(
        "OMOBA  /  VERDANT CONFLUENCE       {green}  :  {blue}       {:02}:{:02}\n{state}       {}{}",
        seconds / 60,
        seconds % 60,
        CLASSES[practice.class].display_name(),
        hero.map_or(String::new(), |p| format!(
            "  |  Lv {}  |  HP {:.0}/{:.0}  |  MP {:.0}  |  {} gold",
            p.level, p.hp, p.max_hp, p.mana, p.gold
        ))
    ));
    let abilities = CLASSES[practice.class].abilities();
    **skills = Text::new(format!(
        "{}\nArrows: move   Right-click: route   Space/click: attack   F: dash   B: recall\n1-7: change class and start a new practice",
        abilities
            .iter()
            .enumerate()
            .map(|(i, a)| format!(
                "{}  {} {}",
                ["Q", "W", "E", "R"][i],
                a.name,
                hero.map_or(String::new(), |p| {
                    if p.skill_cooldown_remaining_secs[i] > 0.1 {
                        format!("({:.1}s)", p.skill_cooldown_remaining_secs[i])
                    } else {
                        "[ready]".into()
                    }
                })
            ))
            .collect::<Vec<_>>()
            .join("    ")
    ));
}
