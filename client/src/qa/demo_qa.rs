//! Opt-in demo director for showcase videos. Enabled by OMOBA_DEMO_QA_DIR and
//! OMOBA_DEMO_SCRIPT (`desktop`, `jungle` or `phone`). It plays a timeline of
//! ordinary player inputs against a live server: class buttons, the shop,
//! move orders, left-click target lock and right-click attacks at the target's
//! screen position (nearest enemy by the Tab rule),
//! ability keys and, on phones, real TouchInput on the joystick and buttons.
//! Nothing is simulated locally; damage, gold and levels stay authoritative.
//! Caption events are written to `demo-events.json` with the recorder clock
//! (see `record_qa`), so the encoder can title each part of the video.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

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

/// How long an act may take before the director moves on; walks cross the map.
fn act_limit(act: Act) -> Duration {
    Duration::from_secs(match act {
        Act::Go(_) => 150,
        Act::Fight { .. } => 180,
        _ => 60,
    })
}

/// Walks are sped up by this factor in the edit (marked in `demo-events.json`).
const WALK_TIMELAPSE: f32 = 4.0;
/// Enemies farther than this are left for a later beat.
const ENGAGE_RANGE: f32 = 22.0;
/// How far the phone joystick thumb is pushed from its centre, in pixels.
const STICK_REACH: f32 = 55.0;
const STICK_TOUCH: u64 = 7;

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
    /// Presses each class button in turn, ending on the requested class.
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
    /// ability keys cast. Phones tap ATK and the ability buttons. With nobody
    /// in reach (after a death, a cleared camp) the hero returns to the anchor.
    Fight {
        seconds: f32,
    },
}

fn script(name: &str) -> Option<Vec<Act>> {
    use Act::*;
    use shared::map::Lane::Mid;
    Some(match name {
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
        let acts = script(&name)
            .unwrap_or_else(|| panic!("OMOBA_DEMO_SCRIPT must be desktop, jungle or phone"));
        let class = std::env::var("OMOBA_DEMO_CLASS")
            .ok()
            .map(|id| shared::HeroClass::from_id(&id).expect("OMOBA_DEMO_CLASS must be a class"));
        let pixels = |key: &str, fallback| {
            std::env::var(key)
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(fallback)
        };
        app.insert_resource(DemoQa {
            directory,
            script: name,
            acts,
            class,
            width: pixels("OMOBA_QA_WIDTH", 1280),
            height: pixels("OMOBA_QA_HEIGHT", 720),
            started: Instant::now(),
            index: 0,
            act_started: Instant::now(),
            act_step: 0,
            last_action: None,
            ability: 0,
            keys: Vec::new(),
            click: None,
            touches: Vec::new(),
            events: Vec::new(),
            pace: Vec::new(),
            pace_open: false,
            anchor: None,
            camp_tour: None,
            stick: false,
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
        .add_systems(Update, (screens, direct).chain());
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
    started: Instant,
    index: usize,
    act_started: Instant,
    /// Progress inside the current act (class index, shop step, touch phase).
    act_step: u32,
    last_action: Option<Instant>,
    ability: usize,
    keys: Vec<KeyCode>,
    click: Option<(Vec2, MouseButton)>,
    touches: Vec<(u64, TouchPhase, Vec2)>,
    events: Vec<serde_json::Value>,
    /// Timelapse markers: `{"seconds", "speed"}` on the recorder clock.
    pace: Vec<serde_json::Value>,
    pace_open: bool,
    /// Where fights return to: the last `Go` destination.
    anchor: Option<Vec2>,
    /// Index into the home camps while a jungle fight tours them.
    camp_tour: Option<usize>,
    /// True while the scripted thumb holds the phone joystick.
    stick: bool,
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
        self.act_started = Instant::now();
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
        self.act_started.elapsed().as_secs_f32()
    }

    /// True at most once per `every` seconds inside an act.
    fn tick(&mut self, every: f32) -> bool {
        if self
            .last_action
            .is_some_and(|last| last.elapsed().as_secs_f32() < every)
        {
            return false;
        }
        self.last_action = Some(Instant::now());
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
    if !window.focused && !demo.focus_requested {
        window.focused = true;
        demo.focus_requested = true;
    }
    window.resolution.set_scale_factor_override(Some(1.0));
    if window.physical_width() != demo.width || window.physical_height() != demo.height {
        window
            .resolution
            .set_physical_resolution(demo.width, demo.height);
    }
    for key in std::mem::take(&mut demo.keys) {
        keyboard.press(key);
    }
    if let Some((position, button)) = demo.click.take() {
        window.set_cursor_position(Some(position));
        mouse.press(button);
    }
}

/// Owns the two screens this legacy-style run needs: hero select, then match.
fn screens(
    demo: Res<DemoQa>,
    session: Res<ClientSession>,
    screen: Res<State<crate::frontend::AppScreen>>,
    mut next: ResMut<NextState<crate::frontend::AppScreen>>,
) {
    use crate::frontend::AppScreen;
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
    nodes: Query<'w, 's, (crate::qa::QaName, &'static UiGlobalTransform)>,
    candidates: crate::combat::TargetCandidates<'w, 's>,
    validity: crate::targeting::TargetValidity<'w, 's>,
    mobile: Option<Res<'w, crate::mobile_controls::MobileControls>>,
}

impl World3d<'_, '_> {
    fn node_center(&self, name: &str) -> Option<Vec2> {
        self.nodes
            .iter()
            .find(|(key, _)| key.as_str() == name)
            .map(|(_, transform)| transform.translation)
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

    fn skill_points(&self) -> bool {
        self.player
            .single()
            .is_ok_and(|(_, _, _, progression, _)| progression.is_some_and(|p| p.skill_points > 0))
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
                "method": "scripted production inputs against a live server; recorder readbacks",
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
    if demo.act_started.elapsed() > act_limit(act) {
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
            buttons.press(&format!("ClassButton-{}", class.id()));
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
            if demo.act_step == 0 {
                demo.act_step = 1;
                demo.set_pace(WALK_TIMELAPSE);
            }
            if approach(&mut demo, &mut commands, &world, point) {
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
            let enemy = world
                .target
                .selected_entity
                .filter(|entity| world.targets.get(*entity).is_ok())
                .or_else(|| world.nearest_enemy());
            if enemy.is_none()
                && let Some(anchor) = demo.anchor
            {
                // Nobody in reach: walk back to the fight (timelapsed).
                if !approach(&mut demo, &mut commands, &world, anchor) {
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
            }
            if demo.pace_open {
                demo.set_pace(1.0);
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

/// Moves toward `point` like a player would; true once the hero is there.
fn approach(demo: &mut DemoQa, commands: &mut Commands, world: &World3d, point: Vec2) -> bool {
    let Ok((entity, transform, _, _, order)) = world.player.single() else {
        return false;
    };
    let hero = transform.translation;
    if hero.xz().distance(point) <= 8.0 {
        release_stick(demo, world);
        return true;
    }
    let goal = Vec3::new(point.x, hero.y, point.y);
    if world.phone() {
        // Steer the joystick toward the goal as it appears on screen.
        let (Some(center), Some(from), Some(to)) = (
            world.node_center("MobileJoystick"),
            world.project(hero),
            world.project(goal),
        ) else {
            return false;
        };
        let thumb = center + (to - from).normalize_or_zero() * STICK_REACH;
        if !demo.stick {
            demo.touches
                .push((STICK_TOUCH, TouchPhase::Started, center));
            demo.stick = true;
        }
        demo.touches.push((STICK_TOUCH, TouchPhase::Moved, thumb));
    } else if order.is_none() && demo.tick(1.0) {
        // One ordinary order; re-issued only when combat or a respawn cleared it.
        commands
            .entity(entity)
            .insert(crate::player::MovementTarget { target: goal });
    }
    false
}

fn release_stick(demo: &mut DemoQa, world: &World3d) {
    if !demo.stick {
        return;
    }
    let center = world.node_center("MobileJoystick").unwrap_or_default();
    demo.touches.push((STICK_TOUCH, TouchPhase::Ended, center));
    demo.stick = false;
}

/// One phone combat beat: tap ATK, and every few beats an ability button.
fn tap_step(demo: &mut DemoQa, world: &World3d) {
    let step = demo.act_step;
    demo.act_step += 1;
    let ability = step % 4 == 3;
    let control = if ability {
        format!("MobileAbility-{}", demo.ability % 4)
    } else {
        "MobileAttack".to_owned()
    };
    let Some(position) = world.node_center(&control) else {
        return;
    };
    if ability {
        demo.ability += 1;
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
    let locked = world
        .target
        .selected_entity
        .filter(|entity| world.targets.get(*entity).is_ok());
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
            let key = crate::input_bindings::SKILL_CAST_KEYS[demo.ability % 4];
            demo.ability += 1;
            demo.keys.push(key);
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
        for name in ["desktop", "jungle", "phone"] {
            let acts = script(name).unwrap();
            assert!(acts.iter().any(|act| matches!(act, Act::Join)), "{name}");
            assert!(
                acts.iter().any(|act| matches!(act, Act::Caption(..))),
                "{name}"
            );
        }
        assert!(script("other").is_none());
    }
}
