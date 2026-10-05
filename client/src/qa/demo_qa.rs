//! Opt-in demo director for showcase videos. Enabled by OMOBA_DEMO_QA_DIR and
//! OMOBA_DEMO_SCRIPT (`desktop`, `jungle`, `phone`, `lane` or `showcase`). It
//! plays a timeline of ordinary player inputs against a live server or the
//! in-process offline practice: class buttons, the shop,
//! move orders, left-click target lock and right-click attacks at the target's
//! screen position (nearest enemy by the Tab rule),
//! ability keys and, on phones, real TouchInput on the joystick and buttons.
//! Nothing is simulated locally; damage, gold and levels stay authoritative.
//! Caption events are written to `demo-events.json` with the recorder clock
//! (see `record_qa`), so the encoder can title each part of the video. Acts
//! are paced on that clock too, so frame-stepped captures keep game-time pacing.
//!
//! Presentation knobs: OMOBA_QA_SCALE renders the OMOBA_QA_WIDTH x
//! OMOBA_QA_HEIGHT pixels at that scale factor (a 2532x1170 window at scale 3
//! is the 844x390 phone layout), OMOBA_DEMO_ZOOM sets the camera zoom a player
//! would pick with the wheel, and OMOBA_DEMO_LANE (`top`, `mid`, `bot`) picks
//! the lane of the `lane` and `showcase` scripts. The FPS readout is hidden.
//! OMOBA_QA_SYNTHETIC_FOCUS=1 keeps the window focused for the game (as in the
//! other harnesses), so a long capture does not stop when the desktop focus
//! moves on: a phone HUD hides and blocks input while its window is unfocused.
//! In that mode scripted clicks also leave the real pointer alone: the cursor
//! position is the window's for the frame of the click and is put back before
//! the window system would move the pointer.
use std::{path::PathBuf, time::Instant};

use bevy::{
    app::AppExit,
    input::touch::{TouchInput, TouchPhase},
    prelude::*,
    window::PrimaryWindow,
};

use crate::{
    help_overlay::HelpOverlayVisible,
    net::{ClientSession, GameState, GameStateSnapshot, NetworkCommand},
};

/// Seconds an act may take before the director moves on; walks cross the map.
fn act_limit(act: Act) -> f64 {
    match act {
        Act::Go(_) => 150.0,
        Act::Fight { .. } => 180.0,
        _ => 60.0,
    }
}

/// Walks are sped up by this factor in the edit (marked in `demo-events.json`).
const WALK_TIMELAPSE: f32 = 4.0;
/// Enemies farther than this are left for a later beat.
const ENGAGE_RANGE: f32 = 22.0;
/// How close a walk ends to its anchor, and how close the phone thumb walks
/// up to a hero, minion or camp creature before tapping (structures are
/// fought from where the hero stands).
const ANCHOR_REACH: f32 = 8.0;
const PHONE_REACH: f32 = 6.0;
/// How far the phone joystick thumb is pushed from its centre, in pixels.
const STICK_REACH: f32 = 55.0;
/// The thumb replans its way around trees, walls and towers this often.
const STEER_REPLAN_SECONDS: f64 = 0.5;
/// How far ahead on its lane the thumb aims, as lane progress.
const LANE_LOOKAHEAD: f32 = 0.04;
/// A thumb that made no headway for the first span walks by a move order (the
/// order a minimap tap gives) for the second.
const STUCK_SECONDS: f64 = 1.0;
const ORDER_SECONDS: f64 = 8.0;
const STICK_TOUCH: u64 = 7;
/// The finger that scrolls the phone class list, and the frames a drag takes.
const SCROLL_TOUCH: u64 = 9;
const SCROLL_FRAMES: u32 = 16;
/// With a lane quiet around the anchor, a fight pushes on by this much of the
/// lane at a time, up to the limit (measured from the hero's own base).
const LANE_PUSH_STEP: f32 = 0.05;
const LANE_PUSH_LIMIT: f32 = 0.8;

#[derive(Clone, Copy, Debug)]
enum Anchor {
    /// A point on a lane, measured from the hero's own base.
    Lane(shared::map::Lane, f32),
    /// The jungle camps on the hero's own half, nearest first; a fight moves
    /// on to the next camp once the current one is clear.
    HomeCamps,
}

#[derive(Clone, Copy, Debug)]
enum Act {
    /// Title and subtitle for the next part of the video.
    Caption(&'static str, &'static str),
    /// Presses each class button in turn, ending on the requested class. On
    /// phones a finger drags the class list to bring the next button into view.
    ClassTour {
        per_class: f32,
    },
    Join,
    DismissHelp,
    /// Opens the shop, buys `item` (a `ShopBuy-*` id) and closes it again.
    Shop {
        item: &'static str,
    },
    /// Walks to the anchor: move orders on desktop, the joystick on phones.
    /// The walk is marked for timelapse and becomes the anchor for fights.
    Go(Anchor),
    /// Fights for `seconds` of contact time. Desktop aims the real cursor at
    /// the nearest enemy (the Tab rule): left-click locks, right-click attacks,
    /// ability keys cast. Phones tap ATK and the ability buttons. Only abilities
    /// off cooldown are used. With nobody in reach (after a death, a cleared
    /// camp, a quiet lane) the hero returns to the anchor, then moves on to the
    /// next camp or further up the lane; that time does not count.
    Fight {
        seconds: f32,
    },
}

fn script(name: &str, lane: shared::map::Lane) -> Option<Vec<Act>> {
    use Act::*;
    use shared::map::Lane::Mid;
    Some(match name {
        // Class-agnostic scripts: the layout (desktop or phone) follows
        // OMOBA_TOUCH_CONTROLS and the hero follows OMOBA_DEMO_CLASS.
        "lane" => vec![
            Join,
            DismissHelp,
            Caption("To the lane", "Waves meet in the middle"),
            Go(Anchor::Lane(lane, 0.5)),
            Caption("Lane fight", "Attacks and the four abilities"),
            Fight { seconds: 36.0 },
        ],
        "showcase" => vec![
            Caption("Hero roster", "Every class, one tap each"),
            ClassTour { per_class: 0.55 },
            Join,
            DismissHelp,
            Caption("To the lane", "Waves meet in the middle"),
            Go(Anchor::Lane(lane, 0.5)),
            Caption("Lane fight", "Attacks and the four abilities"),
            Fight { seconds: 24.0 },
        ],
        "desktop" => vec![
            Caption("Five classes", "Warrior · Mage · Ranger · Cleric · Warden"),
            ClassTour { per_class: 1.1 },
            Join,
            DismissHelp,
            Caption("Sanctuary shop", "Gold buys permanent items at your base"),
            Shop { item: "ShopBuy-EB" },
            Caption("Push the lane", "Move orders path around the map"),
            Go(Anchor::Lane(Mid, 0.5)),
            Caption("Targeting", "Click locks an enemy · right-click attacks"),
            Fight { seconds: 14.0 },
            Caption(
                "Abilities",
                "Q W E R on the locked target · level up to unlock",
            ),
            Fight { seconds: 16.0 },
        ],
        "jungle" => vec![
            Join,
            DismissHelp,
            Caption("Warden jungler", "Forest Tracker: faster camps, more gold"),
            Go(Anchor::HomeCamps),
            Caption("Jungle camps", "Neutral creatures fight back and respawn"),
            Fight { seconds: 20.0 },
        ],
        "phone" => vec![
            Join,
            DismissHelp,
            Caption("Phone controls", "The left thumb steers the joystick"),
            Go(Anchor::Lane(Mid, 0.47)),
            Caption(
                "Touch combat",
                "ATK and ability buttons around the right thumb",
            ),
            Fight { seconds: 20.0 },
        ],
        _ => return None,
    })
}

pub(crate) struct DemoQaPlugin;

impl Plugin for DemoQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_DEMO_QA_DIR")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        let name = std::env::var("OMOBA_DEMO_SCRIPT").unwrap_or_else(|_| "desktop".into());
        let lane = match std::env::var("OMOBA_DEMO_LANE").as_deref() {
            Ok("top") => shared::map::Lane::Top,
            Ok("bot") => shared::map::Lane::Bot,
            _ => shared::map::Lane::Mid,
        };
        let acts = script(&name, lane).unwrap_or_else(|| {
            panic!("OMOBA_DEMO_SCRIPT must be desktop, jungle, phone, lane or showcase")
        });
        let class = std::env::var("OMOBA_DEMO_CLASS")
            .ok()
            .map(|id| shared::HeroClass::from_id(&id).expect("OMOBA_DEMO_CLASS must be a class"));
        let pixels = |key: &str, fallback| {
            std::env::var(key)
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(fallback)
        };
        let factor = |key: &str| {
            std::env::var(key)
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite() && *value > 0.0)
        };
        app.insert_resource(DemoQa {
            directory,
            script: name,
            acts,
            class,
            width: pixels("OMOBA_QA_WIDTH", 1280),
            height: pixels("OMOBA_QA_HEIGHT", 720),
            scale: factor("OMOBA_QA_SCALE").unwrap_or(1.0).clamp(0.5, 4.0),
            zoom: factor("OMOBA_DEMO_ZOOM"),
            synthetic_focus: std::env::var("OMOBA_QA_SYNTHETIC_FOCUS").as_deref() == Ok("1"),
            started: Instant::now(),
            index: 0,
            act_started: 0.0,
            act_step: 0,
            last_action: None,
            ability: 0,
            keys: Vec::new(),
            click: None,
            cursor_before: None,
            touches: Vec::new(),
            drag: std::collections::VecDeque::new(),
            dragged_for: None,
            events: Vec::new(),
            pace: Vec::new(),
            pace_open: false,
            anchor: None,
            camp_tour: None,
            lane: None,
            route: Vec::new(),
            route_goal: None,
            route_at: 0.0,
            headway: (Vec2::ZERO, 0.0),
            order_until: 0.0,
            stick: false,
            stick_idle: 0,
            engaged: 0.0,
            last_now: 0.0,
            finishing: false,
            focus_requested: false,
            now: 0.0,
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(PreUpdate, inject_touch.before(bevy::input::InputSystems))
        .add_systems(
            PreUpdate,
            inject_keys_and_mouse
                .after(bevy::input::InputSystems)
                .before(bevy::ui::UiSystems::Focus),
        )
        .add_systems(Update, (screens, direct).chain())
        .add_systems(PostUpdate, restore_cursor);
    }
}

#[derive(Resource)]
struct DemoQa {
    directory: PathBuf,
    script: String,
    acts: Vec<Act>,
    class: Option<shared::HeroClass>,
    width: u32,
    height: u32,
    /// Window scale factor: the pixels above show `pixels / scale` points.
    scale: f32,
    /// Camera zoom applied once the match is on screen.
    zoom: Option<f32>,
    /// Explicit QA fixture input: the window counts as focused every frame.
    synthetic_focus: bool,
    started: Instant,
    index: usize,
    /// When the current act began, on the recorder clock (`now`).
    act_started: f64,
    /// Progress inside the current act (class index, shop step, touch phase).
    act_step: u32,
    last_action: Option<f64>,
    ability: usize,
    keys: Vec<KeyCode>,
    click: Option<(Vec2, MouseButton)>,
    /// Synthetic focus: the window's cursor position before a scripted click.
    cursor_before: Option<Option<Vec2>>,
    touches: Vec<(u64, TouchPhase, Vec2)>,
    /// A touch drag being played, one sample per frame.
    drag: std::collections::VecDeque<(u64, TouchPhase, Vec2)>,
    /// The class-tour step whose button a drag already tried to reveal.
    dragged_for: Option<u32>,
    events: Vec<serde_json::Value>,
    /// Timelapse markers: `{"seconds", "speed"}` on the recorder clock.
    pace: Vec<serde_json::Value>,
    pace_open: bool,
    /// Where fights return to: the last `Go` destination.
    anchor: Option<Vec2>,
    /// Index into the home camps while a jungle fight tours them.
    camp_tour: Option<usize>,
    /// The lane and progress of a lane anchor; a quiet fight pushes it on.
    lane: Option<(shared::map::Lane, f32)>,
    /// The route the phone thumb steers along, where it leads and when it
    /// was planned (recorder clock).
    route: Vec<Vec3>,
    route_goal: Option<Vec2>,
    route_at: f64,
    /// Where and when the walking hero last made headway, and until when a
    /// stuck thumb walks by a move order instead.
    headway: (Vec2, f64),
    order_until: f64,
    /// True while the scripted thumb holds the phone joystick.
    stick: bool,
    /// Frames the held joystick has produced no movement.
    stick_idle: u32,
    /// Contact seconds in the current fight.
    engaged: f32,
    last_now: f64,
    finishing: bool,
    focus_requested: bool,
    /// The recorder clock of the current frame.
    now: f64,
}

impl DemoQa {
    fn next_act(&mut self) {
        if self.pace_open {
            self.set_pace(1.0);
        }
        self.index += 1;
        self.act_started = self.now;
        self.act_step = 0;
        self.engaged = 0.0;
        self.last_action = None;
    }

    fn set_pace(&mut self, speed: f32) {
        let seconds = self.now;
        self.pace
            .push(serde_json::json!({"seconds": seconds, "speed": speed}));
        self.pace_open = speed != 1.0;
    }

    fn act_seconds(&self) -> f32 {
        (self.now - self.act_started) as f32
    }

    /// True at most once per `every` seconds inside an act.
    fn tick(&mut self, every: f32) -> bool {
        if self
            .last_action
            .is_some_and(|last| self.now - last < f64::from(every))
        {
            return false;
        }
        self.last_action = Some(self.now);
        true
    }
}

fn inject_touch(
    mut demo: ResMut<DemoQa>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut events: MessageWriter<TouchInput>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    for (id, phase, position) in demo.touches.drain(..) {
        events.write(TouchInput {
            phase,
            position,
            window,
            force: None,
            id,
        });
    }
}

fn inject_keys_and_mouse(
    mut demo: ResMut<DemoQa>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    // Every scripted press lasts one frame, like a quick tap.
    mouse.release(MouseButton::Left);
    mouse.release(MouseButton::Right);
    for key in [
        KeyCode::KeyQ,
        KeyCode::KeyW,
        KeyCode::KeyE,
        KeyCode::KeyR,
        KeyCode::KeyU,
        KeyCode::Escape,
    ] {
        keyboard.release(key);
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    if !window.focused && (demo.synthetic_focus || !demo.focus_requested) {
        window.focused = true;
        demo.focus_requested = true;
    }
    window
        .resolution
        .set_scale_factor_override(Some(demo.scale));
    if window.physical_width() != demo.width || window.physical_height() != demo.height {
        window
            .resolution
            .set_physical_resolution(demo.width, demo.height);
    }
    for key in std::mem::take(&mut demo.keys) {
        keyboard.press(key);
    }
    if let Some((position, button)) = demo.click.take() {
        if demo.synthetic_focus {
            demo.cursor_before = Some(window.physical_cursor_position());
        }
        window.set_cursor_position(Some(position));
        mouse.press(button);
    }
}

/// Synthetic focus: puts the window's cursor position back after the frame of
/// a scripted click, so the window system has no pointer move to perform.
fn restore_cursor(mut demo: ResMut<DemoQa>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    if let Some(before) = demo.cursor_before.take()
        && let Ok(mut window) = windows.single_mut()
    {
        window.set_physical_cursor_position(before.map(|position| position.as_dvec2()));
    }
}

/// Owns the two screens this legacy-style run needs: hero select, then match.
/// Also keeps the picture clean: no FPS readout, and the requested zoom.
fn screens(
    demo: Res<DemoQa>,
    session: Res<ClientSession>,
    screen: Res<State<crate::frontend::AppScreen>>,
    mut next: ResMut<NextState<crate::frontend::AppScreen>>,
    mut render: ResMut<crate::render_settings::RenderSettings>,
    mut camera: ResMut<crate::camera::CameraSettings>,
) {
    use crate::frontend::AppScreen;
    if render.show_fps {
        render.show_fps = false;
    }
    if let Some(zoom) = demo.zoom
        && (camera.zoom - zoom).abs() > 1e-4
    {
        camera.zoom = zoom;
    }
    let joined = demo.acts[..demo.index.min(demo.acts.len())]
        .iter()
        .any(|act| matches!(act, Act::Join));
    let wanted = if session.join_confirmed() {
        AppScreen::InMatch
    } else if !joined {
        AppScreen::HeroSelect
    } else {
        return;
    };
    if *screen.get() != wanted {
        next.set(wanted);
    }
}

#[derive(bevy::ecs::system::SystemParam)]
struct World3d<'w, 's> {
    player: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static crate::team::Team,
            Option<&'static crate::net::PlayerProgression>,
            Option<&'static crate::player::MovementTarget>,
        ),
        With<crate::player::Player>,
    >,
    targets: Query<'w, 's, &'static GlobalTransform>,
    camera:
        Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<crate::camera::MainCamera>>,
    target: Res<'w, crate::combat::TargetState>,
    layout: Res<'w, crate::maps::MapLayout>,
    nodes: Query<
        'w,
        's,
        (
            crate::qa::QaName,
            &'static UiGlobalTransform,
            &'static ComputedNode,
        ),
    >,
    cooldowns:
        Query<'w, 's, &'static crate::net::PlayerSkillCooldowns, With<crate::player::Player>>,
    structures: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static crate::net::StructureKind,
            Option<&'static crate::domain::CombatStats>,
        ),
        With<crate::net::NetworkStructure>,
    >,
    candidates: crate::combat::TargetCandidates<'w, 's>,
    validity: crate::targeting::TargetValidity<'w, 's>,
    mobile: Option<Res<'w, crate::mobile_controls::MobileControls>>,
}

impl World3d<'_, '_> {
    /// A node's centre in window points, where clicks and touches land.
    fn node_center(&self, name: &str) -> Option<Vec2> {
        self.nodes
            .iter()
            .find(|(key, _, _)| key.as_str() == name)
            .map(|(_, transform, computed)| transform.translation * computed.inverse_scale_factor())
    }

    fn on_screen(&self, entity: Entity) -> Option<Vec2> {
        let world = self.targets.get(entity).ok()?.translation();
        let (camera, transform) = self.camera.single().ok()?;
        camera.world_to_viewport(transform, world).ok()
    }

    fn phone(&self) -> bool {
        self.mobile.as_ref().is_some_and(|mobile| mobile.enabled)
    }

    fn project(&self, world: Vec3) -> Option<Vec2> {
        let (camera, transform) = self.camera.single().ok()?;
        camera.world_to_viewport(transform, world).ok()
    }

    fn anchor_point(&self, anchor: Anchor) -> Option<Vec2> {
        let (_, _, team, _, _) = self.player.single().ok()?;
        Some(match anchor {
            Anchor::Lane(lane, progress) => {
                let progress = if *team == crate::team::Team::Blue {
                    1.0 - progress
                } else {
                    progress
                };
                Vec2::from_array(shared::map::sample_lane(lane, progress))
            }
            Anchor::HomeCamps => self.home_camps(*team).first().copied()?,
        })
    }

    /// Camps on the hero's half of the map, nearest to its base first.
    fn home_camps(&self, team: crate::team::Team) -> Vec<Vec2> {
        let home = self.layout.team_spawn(team).xz();
        let other = if team == crate::team::Team::Blue {
            crate::team::Team::Green
        } else {
            crate::team::Team::Blue
        };
        let away = self.layout.team_spawn(other).xz();
        let mut camps: Vec<_> = self
            .layout
            .camp_centers()
            .into_iter()
            .filter(|camp| camp.distance(home) < camp.distance(away))
            .collect();
        camps.sort_by(|a, b| a.distance(home).total_cmp(&b.distance(home)));
        camps
    }

    /// The next point for a hero at `hero` walking its lane toward `target`
    /// progress: a little ahead of the nearest point of the lane.
    fn lane_waypoint(&self, lane: shared::map::Lane, target: f32, hero: Vec2) -> Option<Vec2> {
        let nearest = (0..=200)
            .map(|step| step as f32 / 200.0)
            .filter_map(|progress| {
                let point = self.anchor_point(Anchor::Lane(lane, progress))?;
                Some((progress, point.distance(hero)))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))?
            .0;
        self.anchor_point(Anchor::Lane(lane, lane_step(nearest, target)))
    }

    /// The way a move order would take from `start` to `goal`, around trees,
    /// walls and standing structures; empty when there is none.
    fn route(&self, start: Vec3, goal: Vec3) -> Vec<Vec3> {
        let structures: Vec<_> = self
            .structures
            .iter()
            .filter(|(_, _, stats)| stats.is_none_or(|stats| stats.is_alive()))
            .map(|(transform, kind, _)| (transform.translation, *kind))
            .collect();
        crate::navigation::plan_route_with_terrain(&self.layout, start, goal, &structures, &[])
            .unwrap_or_default()
    }

    /// The first ability at or after slot `from` (wrapping) that is off
    /// cooldown; any slot while the server has not reported cooldowns yet.
    fn ready_ability(&self, from: usize) -> Option<usize> {
        let Ok(cooldowns) = self.cooldowns.single() else {
            return Some(from % 4);
        };
        if cooldowns.recovery_secs > 0.0 {
            return None;
        }
        next_ready(cooldowns.remaining_secs, from)
    }

    fn skill_points(&self) -> bool {
        self.player
            .single()
            .is_ok_and(|(_, _, _, progression, _)| progression.is_some_and(|p| p.skill_points > 0))
    }

    /// The locked target while it can still be attacked: alive, visible and
    /// not a protected structure.
    fn locked_target(&self) -> Option<Entity> {
        let (entity, id) = self
            .target
            .selected_entity
            .zip(self.target.selected_target)?;
        let (_, _, team, _, _) = self.player.single().ok()?;
        self.validity.valid(entity, id, *team).then_some(entity)
    }

    /// Where the phone thumb should walk to reach `enemy`, if it is a unit
    /// still out of reach.
    fn out_of_reach(&self, enemy: Entity) -> Option<Vec2> {
        if self.structures.get(enemy).is_ok() {
            return None;
        }
        let (_, transform, _, _, _) = self.player.single().ok()?;
        let position = self.targets.get(enemy).ok()?.translation().xz();
        (position.distance(transform.translation.xz()) > PHONE_REACH).then_some(position)
    }

    /// The nearest enemy within engage range, by the same rule as Tab.
    fn nearest_enemy(&self) -> Option<Entity> {
        let (_, transform, team, _, _) = self.player.single().ok()?;
        let entity = crate::combat::nearest_enemy(
            transform.translation,
            *team,
            &self.validity,
            &self.candidates,
        )?;
        let distance = self
            .targets
            .get(entity)
            .ok()?
            .translation()
            .xz()
            .distance(transform.translation.xz());
        (distance <= ENGAGE_RANGE).then_some(entity)
    }
}

#[allow(clippy::too_many_arguments)]
fn direct(
    mut demo: ResMut<DemoQa>,
    mut commands: Commands,
    session: Res<ClientSession>,
    game: Res<GameStateSnapshot>,
    help: Res<HelpOverlayVisible>,
    shop: Res<crate::shop::ShopState>,
    selection: Res<crate::team::TeamSelection>,
    world: World3d,
    mut buttons: crate::qa::TestIdPresses,
    mut network: MessageWriter<NetworkCommand>,
    mut recorder: Option<ResMut<super::record_qa::Recorder>>,
    mut exit: MessageWriter<AppExit>,
) {
    let clock = recorder.as_ref().map_or_else(
        || demo.started.elapsed().as_secs_f64(),
        |r| r.elapsed_seconds(),
    );
    let dt = (clock - demo.last_now).clamp(0.0, 0.5) as f32;
    demo.last_now = clock;
    demo.now = clock;
    if demo.finishing {
        if recorder.as_ref().is_none_or(|r| r.idle()) {
            let summary = serde_json::json!({
                "script": demo.script, "class": demo.class.map(|c| c.id()),
                "pixels": [demo.width, demo.height], "events": demo.events, "pace": demo.pace,
                "elapsed_seconds": clock, "version": env!("CARGO_PKG_VERSION"),
                "method": if session.is_offline() {
                    "scripted production inputs against the offline practice; recorder readbacks"
                } else {
                    "scripted production inputs against a live server; recorder readbacks"
                },
            });
            let saved = std::fs::create_dir_all(&demo.directory).and_then(|_| {
                std::fs::write(
                    demo.directory.join("demo-events.json"),
                    serde_json::to_vec_pretty(&summary).unwrap(),
                )
            });
            info!("DEMO_QA completed script={}", demo.script);
            exit.write(if saved.is_ok() {
                AppExit::Success
            } else {
                AppExit::error()
            });
        }
        return;
    }
    let Some(&act) = demo.acts.get(demo.index) else {
        demo.finishing = true;
        if let Some(recorder) = recorder.as_mut() {
            recorder.stop();
        }
        return;
    };
    if demo.now - demo.act_started > act_limit(act) {
        warn!("DEMO_QA act {act:?} timed out; moving on");
        demo.next_act();
        return;
    }
    let playing = session.join_confirmed() && matches!(game.state, GameState::Running);
    match act {
        Act::Caption(title, subtitle) => {
            demo.events
                .push(serde_json::json!({"seconds": clock, "title": title, "subtitle": subtitle}));
            demo.next_act();
        }
        Act::ClassTour { per_class } => {
            if let Some(sample) = demo.drag.pop_front() {
                demo.touches.push(sample);
                return;
            }
            if !session.is_connected() || !demo.tick(per_class) {
                return;
            }
            let classes = shared::HeroClass::ALL;
            let step = demo.act_step as usize;
            let class = if step < classes.len() {
                classes[step]
            } else {
                demo.class.unwrap_or(classes[0])
            };
            let button = format!("ClassButton-{}", class.id());
            if world.phone()
                && demo.dragged_for != Some(demo.act_step)
                && let Some(center) = world.node_center(&button)
            {
                let height = demo.height as f32 / demo.scale;
                if let Some(drag) = reveal_drag(center, height) {
                    demo.dragged_for = Some(demo.act_step);
                    demo.drag = drag;
                    return;
                }
            }
            buttons.press(&button);
            demo.act_step += 1;
            if step > classes.len() && selection.hero_class == class {
                demo.next_act();
            }
        }
        Act::Join => {
            if session.join_confirmed() {
                demo.next_act();
            } else if session.is_connected() && !session.has_committed_join() {
                if let Some(class) = demo.class
                    && selection.hero_class != class
                {
                    buttons.press(&format!("ClassButton-{}", class.id()));
                    return;
                }
                network.write(NetworkCommand::Join {
                    handheld: Default::default(),
                    team: crate::team::Team::Green,
                    character: selection.character,
                    hero_class: selection.hero_class,
                    avatar: selection.avatar.clone(),
                    sprite_character: Some(selection.sprite_character.clone()),
                });
            }
        }
        Act::DismissHelp => {
            if playing && !help.0 && demo.act_seconds() > 1.0 {
                demo.next_act();
            } else if help.0 && demo.tick(0.8) {
                buttons.press("HelpDismissButton");
            }
        }
        Act::Shop { item } => {
            if !playing || !demo.tick(1.4) {
                return;
            }
            match demo.act_step {
                0 if !shop.open => {
                    buttons.press("GoldShopButton");
                }
                0 => demo.act_step = 1,
                1 => {
                    buttons.press(item);
                    demo.act_step = 2;
                }
                2 if shop.purchase_pending() => {}
                2 => {
                    if !buttons.press("ShopCloseButton") {
                        demo.keys.push(KeyCode::Escape);
                    }
                    demo.act_step = 3;
                }
                _ if shop.open => demo.keys.push(KeyCode::Escape),
                _ => demo.next_act(),
            }
        }
        Act::Go(anchor) => {
            if !playing {
                return;
            }
            let Some(point) = world.anchor_point(anchor) else {
                return;
            };
            demo.anchor = Some(point);
            demo.camp_tour = matches!(anchor, Anchor::HomeCamps).then_some(0);
            demo.lane = match anchor {
                Anchor::Lane(lane, progress) => Some((lane, progress)),
                Anchor::HomeCamps => None,
            };
            if demo.act_step == 0 {
                demo.act_step = 1;
                demo.set_pace(WALK_TIMELAPSE);
            }
            if approach(&mut demo, &mut commands, &world, point, ANCHOR_REACH) {
                demo.next_act();
            }
        }
        Act::Fight { seconds } => {
            if demo.engaged >= seconds {
                release_stick(&mut demo, &world);
                demo.next_act();
                return;
            }
            if !playing {
                return;
            }
            let enemy = world.locked_target().or_else(|| world.nearest_enemy());
            if enemy.is_none()
                && let Some(anchor) = demo.anchor
            {
                // Nobody in reach: walk back to the fight (timelapsed).
                if !approach(&mut demo, &mut commands, &world, anchor, ANCHOR_REACH) {
                    if !demo.pace_open {
                        demo.set_pace(WALK_TIMELAPSE);
                    }
                    return;
                }
                // A cleared camp: walk on to the next one on this half.
                if let (Some(index), Ok((_, _, team, _, _))) =
                    (demo.camp_tour, world.player.single())
                {
                    let camps = world.home_camps(*team);
                    if !camps.is_empty() {
                        let next = (index + 1) % camps.len();
                        demo.camp_tour = Some(next);
                        demo.anchor = Some(camps[next]);
                        return;
                    }
                }
                // A quiet lane: push on toward the enemy base.
                if let Some((lane, progress)) = demo.lane
                    && progress < LANE_PUSH_LIMIT
                {
                    let progress = (progress + LANE_PUSH_STEP).min(LANE_PUSH_LIMIT);
                    demo.lane = Some((lane, progress));
                    if let Some(point) = world.anchor_point(Anchor::Lane(lane, progress)) {
                        demo.anchor = Some(point);
                    }
                    return;
                }
            }
            if demo.pace_open {
                demo.set_pace(1.0);
            }
            // A phone hero does not walk to its target by itself: the thumb
            // closes the distance first.
            if world.phone()
                && let Some(position) = enemy.and_then(|enemy| world.out_of_reach(enemy))
            {
                approach(
                    &mut demo,
                    &mut commands,
                    &world,
                    position,
                    PHONE_REACH - 1.0,
                );
                return;
            }
            release_stick(&mut demo, &world);
            demo.engaged += dt;
            if !demo.tick(if world.phone() { 0.35 } else { 0.45 }) {
                return;
            }
            if world.phone() {
                tap_step(&mut demo, &world);
            } else {
                fight_step(&mut demo, &world);
            }
        }
    }
}

/// Moves toward `point` like a player would; true once the hero is within
/// `reach` of it.
fn approach(
    demo: &mut DemoQa,
    commands: &mut Commands,
    world: &World3d,
    point: Vec2,
    reach: f32,
) -> bool {
    let Ok((entity, transform, _, _, order)) = world.player.single() else {
        return false;
    };
    let hero = transform.translation;
    if hero.xz().distance(point) <= reach {
        release_stick(demo, world);
        return true;
    }
    let goal = Vec3::new(point.x, hero.y, point.y);
    // A thumb that stopped making headway (a rock, a wall) lets go and walks
    // by a move order for a while, as a player would tap the minimap.
    if hero.xz().distance(demo.headway.0) > 1.0 || demo.route_goal != Some(point) {
        demo.headway = (hero.xz(), demo.now);
    } else if world.phone() && demo.now - demo.headway.1 > STUCK_SECONDS {
        demo.headway = (hero.xz(), demo.now);
        demo.order_until = demo.now + ORDER_SECONDS;
    }
    let by_order = !world.phone() || demo.now < demo.order_until;
    if !by_order {
        // Steer the joystick along the lane, or else along the route a move
        // order would take, toward the next point as it appears on screen.
        if demo.route_goal != Some(point) || demo.now - demo.route_at >= STEER_REPLAN_SECONDS {
            demo.route = world.route(hero, goal);
            demo.route_goal = Some(point);
            demo.route_at = demo.now;
        }
        let next = demo
            .lane
            // The lane leads to the anchor; a walk up to an enemy goes direct.
            .filter(|_| demo.anchor == Some(point))
            .and_then(|(lane, target)| world.lane_waypoint(lane, target, hero.xz()))
            .map(|ahead| Vec3::new(ahead.x, hero.y, ahead.y))
            .or_else(|| {
                demo.route
                    .iter()
                    .copied()
                    .find(|corner| corner.xz().distance(hero.xz()) > 1.5)
            })
            .unwrap_or(goal);
        let (Some(center), Some(from), Some(to)) = (
            world.node_center("MobileJoystick"),
            world.project(hero),
            world.project(Vec3::new(next.x, hero.y, next.z)),
        ) else {
            return false;
        };
        let thumb = center + (to - from).normalize_or_zero() * STICK_REACH;
        // The game drops held fingers on a respawn, a new round or a focus
        // change; a real thumb would lift and press the stick again.
        let moving = world
            .mobile
            .as_ref()
            .is_some_and(|mobile| mobile.movement != Vec2::ZERO);
        demo.stick_idle = if demo.stick && !moving {
            demo.stick_idle + 1
        } else {
            0
        };
        if demo.stick_idle > 8 {
            release_stick(demo, world);
            return false;
        }
        if !demo.stick {
            demo.touches
                .push((STICK_TOUCH, TouchPhase::Started, center));
            demo.stick = true;
        }
        demo.touches.push((STICK_TOUCH, TouchPhase::Moved, thumb));
    } else {
        release_stick(demo, world);
        demo.route_goal = Some(point);
        if order.is_none() && demo.tick(1.0) {
            // One ordinary order; re-issued only when combat or a respawn cleared it.
            commands
                .entity(entity)
                .insert(crate::player::MovementTarget { target: goal });
        }
    }
    false
}

/// Lane progress to aim at from `nearest` on the way to `target`.
fn lane_step(nearest: f32, target: f32) -> f32 {
    if nearest < target {
        (nearest + LANE_LOOKAHEAD).min(target)
    } else {
        (nearest - LANE_LOOKAHEAD).max(target)
    }
}

/// The first slot at or after `from` (wrapping) whose cooldown has run out.
fn next_ready(remaining_secs: [f32; 4], from: usize) -> Option<usize> {
    (0..4)
        .map(|offset| (from + offset) % 4)
        .find(|slot| remaining_secs[*slot] <= 0.0)
}

/// The touch drag that scrolls a list so a button at `center` (window points)
/// lands mid-screen; `None` while the button is comfortably in view.
fn reveal_drag(
    center: Vec2,
    height: f32,
) -> Option<std::collections::VecDeque<(u64, TouchPhase, Vec2)>> {
    if (0.3 * height..=0.85 * height).contains(&center.y) {
        return None;
    }
    let travel = (0.55 * height - center.y).clamp(-0.5 * height, 0.5 * height);
    // Start where the finger has room to travel, on the list's own column.
    let from = Vec2::new(center.x, if travel < 0.0 { 0.8 } else { 0.35 } * height);
    let mut drag = std::collections::VecDeque::from([(SCROLL_TOUCH, TouchPhase::Started, from)]);
    for frame in 1..=SCROLL_FRAMES {
        let eased = 1.0 - (1.0 - frame as f32 / SCROLL_FRAMES as f32).powi(2);
        drag.push_back((
            SCROLL_TOUCH,
            TouchPhase::Moved,
            from + Vec2::Y * travel * eased,
        ));
    }
    drag.push_back((SCROLL_TOUCH, TouchPhase::Ended, from + Vec2::Y * travel));
    Some(drag)
}

fn release_stick(demo: &mut DemoQa, world: &World3d) {
    if !demo.stick {
        return;
    }
    let center = world.node_center("MobileJoystick").unwrap_or_default();
    demo.touches.push((STICK_TOUCH, TouchPhase::Ended, center));
    demo.stick = false;
    demo.stick_idle = 0;
}

/// One phone combat beat: tap ATK, and every few beats an ability button.
fn tap_step(demo: &mut DemoQa, world: &World3d) {
    let step = demo.act_step;
    demo.act_step += 1;
    let ability = (step % 4 == 3)
        .then(|| world.ready_ability(demo.ability))
        .flatten();
    let control = match ability {
        Some(slot) => format!("MobileAbility-{slot}"),
        None => "MobileAttack".to_owned(),
    };
    let Some(position) = world.node_center(&control) else {
        return;
    };
    if let Some(slot) = ability {
        demo.ability = slot + 1;
    }
    // A tap: press and release; the id stays distinct from the joystick.
    let id = 20 + u64::from(step % 1000);
    demo.touches.push((id, TouchPhase::Started, position));
    demo.touches.push((id, TouchPhase::Ended, position));
    if world.skill_points() {
        demo.keys.push(KeyCode::KeyU);
    }
}

/// One desktop combat beat: aim at an enemy and lock it with a left-click,
/// right-click it to attack, then keep casting and re-issuing the attack.
fn fight_step(demo: &mut DemoQa, world: &World3d) {
    if world.skill_points() {
        demo.keys.push(KeyCode::KeyU);
    }
    let locked = world.locked_target();
    let Some(target) = locked.or_else(|| world.nearest_enemy()) else {
        demo.act_step = 0;
        return;
    };
    let Some(on_screen) = world.on_screen(target) else {
        return;
    };
    if locked.is_none() {
        demo.click = Some((on_screen, MouseButton::Left));
        demo.act_step = 0;
        return;
    }
    demo.act_step += 1;
    match demo.act_step {
        1 => demo.click = Some((on_screen, MouseButton::Right)),
        step if step % 3 == 0 => {
            if let Some(slot) = world.ready_ability(demo.ability) {
                demo.ability = slot + 1;
                demo.keys.push(crate::input_bindings::SKILL_CAST_KEYS[slot]);
            }
        }
        step if step % 8 == 0 => demo.click = Some((on_screen, MouseButton::Right)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_script_exists_joins_and_captions() {
        for name in ["desktop", "jungle", "phone", "lane", "showcase"] {
            let acts = script(name, shared::map::Lane::Mid).unwrap();
            assert!(acts.iter().any(|act| matches!(act, Act::Join)), "{name}");
            assert!(
                acts.iter().any(|act| matches!(act, Act::Caption(..))),
                "{name}"
            );
        }
        assert!(script("other", shared::map::Lane::Mid).is_none());
    }

    #[test]
    fn class_list_drag_brings_an_off_screen_button_to_the_middle() {
        let height = 390.0;
        assert!(reveal_drag(Vec2::new(120.0, 200.0), height).is_none());
        // Below the fold: the finger travels up by the distance to mid-screen.
        let below = reveal_drag(Vec2::new(120.0, 400.0), height).unwrap();
        let (first, last) = (below.front().unwrap(), below.back().unwrap());
        assert_eq!((first.1, last.1), (TouchPhase::Started, TouchPhase::Ended));
        assert_eq!(below.len() as u32, SCROLL_FRAMES + 2);
        assert!((last.2.y - first.2.y - (0.55 * height - 400.0)).abs() < 1e-3);
        assert!(
            below
                .iter()
                .all(|sample| (0.0..=height).contains(&sample.2.y))
        );
        // Above the list: the finger travels down, never past half a screen.
        let above = reveal_drag(Vec2::new(120.0, -600.0), height).unwrap();
        let travel = above.back().unwrap().2.y - above.front().unwrap().2.y;
        assert!((travel - 0.5 * height).abs() < 1e-3);
        assert!(
            above
                .iter()
                .all(|sample| (0.0..=height).contains(&sample.2.y))
        );
    }

    #[test]
    fn lane_walk_aims_a_little_ahead_and_stops_at_the_target() {
        assert!((lane_step(0.1, 0.5) - (0.1 + LANE_LOOKAHEAD)).abs() < 1e-6);
        assert_eq!(lane_step(0.49, 0.5), 0.5);
        // Past the target (a pushed lane, a retreat): walk back the same way.
        assert!((lane_step(0.7, 0.5) - (0.7 - LANE_LOOKAHEAD)).abs() < 1e-6);
        assert_eq!(lane_step(0.5, 0.5), 0.5);
    }

    #[test]
    fn abilities_on_cooldown_are_skipped() {
        assert_eq!(next_ready([0.0; 4], 2), Some(2));
        assert_eq!(next_ready([0.0, 3.0, 1.5, 0.0], 1), Some(3));
        assert_eq!(next_ready([0.0, 3.0, 1.5, 9.0], 5), Some(0));
        assert_eq!(next_ready([2.0, 3.0, 1.5, 9.0], 0), None);
    }

    #[test]
    fn lane_scripts_walk_to_the_requested_lane() {
        use shared::map::Lane;
        for name in ["lane", "showcase"] {
            for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
                let acts = script(name, lane).unwrap();
                assert!(
                    acts.iter().any(|act| matches!(
                        act,
                        Act::Go(Anchor::Lane(to, _)) if *to == lane
                    )),
                    "{name} {lane:?}"
                );
            }
        }
    }
}
