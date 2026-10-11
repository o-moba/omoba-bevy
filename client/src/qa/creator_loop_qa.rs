//! Opt-in, file-driven capture for the Ekza creator-loop acceptance.
//!
//! `OMOBA_CREATOR_LOOP_QA=<dir>` turns it on; nothing changes without it. The
//! name avoids the `_QA_DIR`/`_QA_OUTPUT` suffixes on purpose: the shell stays
//! up and the session drives the screens exactly as it does for a player.
//!
//! An external coordinator appends one JSON command per line to
//! `<dir>/commands.jsonl`. The harness answers each in `<dir>/results.jsonl`
//! and rewrites `<dir>/qa-state.json` about four times a second (screen, account
//! line, weapon library, selection, players and every held weapon). Commands:
//!
//! - `press {id}`: a `SyntheticPress` on the UI-kit button with that `TestId`,
//!   the same activation a click or tap produces;
//! - `key {code}`: presses a key through `ButtonInput<KeyCode>` (released two
//!   frames later), e.g. `KeyQ` for Wildspark's Switchfire;
//! - `attack` / `cast {slot}`: the `NetworkCommand` the combat input sends for
//!   a basic attack or an aimed cast at the nearest practice dummy;
//! - `camera {lock, focus_player, zoom}`, `menu {open}`, `move {dx, dz}`,
//!   `scroll {node, y}` (the scroll offset a wheel or drag would leave);
//! - `shot {file}`: a screenshot of the window now;
//! - `shot_when {file, player?, mode?, muzzle?, motion?, action?, item?}`: arms
//!   a gate on one held weapon (the local hero's unless `player` names
//!   another), optionally sends `action` (`attack`, `cast:<slot>` or
//!   `key:<code>`), and takes the screenshot in the frame after the gate holds:
//!   the Wildspark set variant for `mode` is shown and the other hidden, a
//!   muzzle flash was emitted at the shown variant's tip, and the rotor or
//!   slide moved from its pose at arm time. The result carries that readback.
//! - `focus`: asks the OS to focus this window (winit `focus_window`). The
//!   game cancels movement and pending casts of an unfocused window on
//!   purpose, so with two clients on one desktop the coordinator hands focus
//!   to the one that has to act; screenshots and observation need no focus;
//! - `exit`.
//!
//! The coordinator owns the order of the steps; this file only presses,
//! observes and captures. `OMOBA_CREATOR_LOOP_QA_WINDOW=x,y` parks the window
//! (pixels) so two clients can sit side by side without covering each other.

use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::{PrimaryWindow, WindowPosition},
};
use serde::Deserialize;
use shared::loadout::WeaponMode;

use crate::{
    frontend::AppScreen,
    held_weapons::HeldWeapon,
    net::{
        ClientSession, GameStateSnapshot, NetworkCommand, NetworkHeroClass, NetworkPlayerId,
        PlayerHandheld, PlayerLoadout,
    },
    player::Player,
};

pub(crate) const ENV: &str = "OMOBA_CREATOR_LOOP_QA";
const WINDOW_ENV: &str = "OMOBA_CREATOR_LOOP_QA_WINDOW";
const STATE_EVERY: Duration = Duration::from_millis(250);
/// The tips `held_weapons` emits the flash at when a model has no hint.
const REPEATER_TIP: [f32; 3] = [0.0, 0.12, 0.735];
const LAUNCHER_TIP: [f32; 3] = [0.0, 0.13, 0.81];
/// A flash this close to a tip (in the weapon's own frame) belongs to it.
const TIP_TOLERANCE: f32 = 0.08;
/// Labels whose text the state file reports.
const LABELS: [&str; 9] = [
    "CollectionAccountStatus",
    "SelectedWeaponStatus",
    "CollectionWeaponMarketHint",
    "CollectionWeaponLibraryHeading",
    "CollectionWeaponCatalogueHeading",
    "DraftWeaponCredit",
    "DraftWeaponStatus",
    "DraftNotice",
    "HomeConnectionStatus",
];

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Command {
    Press {
        id: String,
    },
    Key {
        code: String,
    },
    Menu {
        open: bool,
    },
    Attack,
    Cast {
        slot: u8,
    },
    Move {
        dx: f32,
        dz: f32,
    },
    Camera {
        #[serde(default)]
        lock: Option<bool>,
        #[serde(default)]
        focus_player: Option<u64>,
        #[serde(default)]
        zoom: Option<f32>,
    },
    Scroll {
        node: String,
        y: f32,
    },
    Shot {
        file: String,
    },
    ShotWhen(Gate),
    Focus,
    Exit,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct Gate {
    file: String,
    /// The held weapon's owner; the local hero when absent.
    #[serde(default)]
    player: Option<u64>,
    /// The handheld id the weapon must show (`HeldWeapon::id`).
    #[serde(default)]
    item: Option<String>,
    #[serde(default)]
    mode: Option<WeaponMode>,
    #[serde(default)]
    muzzle: bool,
    #[serde(default)]
    motion: bool,
    #[serde(default)]
    action: Option<String>,
    #[serde(default = "default_timeout")]
    timeout_s: f32,
}

fn default_timeout() -> f32 {
    12.0
}

fn key_code(code: &str) -> Option<KeyCode> {
    Some(match code {
        "KeyQ" => KeyCode::KeyQ,
        "KeyW" => KeyCode::KeyW,
        "KeyE" => KeyCode::KeyE,
        "KeyR" => KeyCode::KeyR,
        "KeyP" => KeyCode::KeyP,
        "Tab" => KeyCode::Tab,
        "Space" => KeyCode::Space,
        "Escape" => KeyCode::Escape,
        "F1" => KeyCode::F1,
        _ => return None,
    })
}

/// `x,y` in pixels.
fn parse_window_at(value: &str) -> Option<IVec2> {
    let (x, y) = value.split_once(',')?;
    Some(IVec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
}

fn epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

/// One glTF node of a held set: what the renderer will do with it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct NodeRead {
    hidden: bool,
    inherited: bool,
    scale: f32,
}

impl NodeRead {
    fn drawn(&self) -> bool {
        !self.hidden && self.inherited && self.scale > 0.5
    }
    fn json(&self) -> serde_json::Value {
        serde_json::json!({"visibility": if self.hidden {"Hidden"} else {"Inherited"},
            "inherited_visible": self.inherited, "scale": self.scale, "drawn": self.drawn()})
    }
}

/// True when exactly the variant for `mode` is drawn and the other is hidden.
fn variant_shown(repeater: Option<NodeRead>, launcher: Option<NodeRead>, mode: WeaponMode) -> bool {
    let (Some(repeater), Some(launcher)) = (repeater, launcher) else {
        return false;
    };
    match mode {
        WeaponMode::Repeater => repeater.drawn() && launcher.hidden && !launcher.inherited,
        WeaponMode::Rockets => launcher.drawn() && repeater.hidden && !repeater.inherited,
    }
}

/// Which tip a flash at `local` (the weapon's own frame) belongs to, if any.
fn classify_tip(local: Vec3, repeater: Vec3, launcher: Vec3) -> Option<(WeaponMode, f32)> {
    let to_repeater = local.distance(repeater);
    let to_launcher = local.distance(launcher);
    let (mode, distance) = if to_repeater <= to_launcher {
        (WeaponMode::Repeater, to_repeater)
    } else {
        (WeaponMode::Rockets, to_launcher)
    };
    (distance <= TIP_TOLERANCE).then_some((mode, distance))
}

/// The tips of one held model: packaged guns their own, SDK models the hint.
fn tips(weapon: &HeldWeapon) -> (Vec3, Vec3) {
    let hint = weapon.hint.unwrap_or_default();
    (
        Vec3::from_array(hint.repeater_muzzle.unwrap_or(REPEATER_TIP)),
        Vec3::from_array(hint.launcher_muzzle.unwrap_or(LAUNCHER_TIP)),
    )
}

#[derive(Clone, Debug)]
struct Flash {
    frame: u64,
    owner: u64,
    tip: WeaponMode,
    distance: f32,
    world: Vec3,
}

#[derive(Clone, Debug, Default)]
struct WeaponRead {
    owner: u64,
    local: bool,
    id: String,
    model: String,
    mode: Option<WeaponMode>,
    loaded: bool,
    repeater: Option<NodeRead>,
    launcher: Option<NodeRead>,
    rotor: Option<Quat>,
    slide: Option<Vec3>,
    screen: Option<Vec2>,
}

impl WeaponRead {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "owner": self.owner, "local": self.local, "id": self.id, "model": self.model,
            "mode": self.mode, "loaded": self.loaded,
            "repeater": self.repeater.map(|n| n.json()), "launcher": self.launcher.map(|n| n.json()),
            "rotor_rotation": self.rotor.map(|q| q.to_array()),
            "slide_translation": self.slide.map(|t| t.to_array()),
            "screen": self.screen.map(|p| p.to_array()),
            "variant_shown_for_mode": self.mode.map(|m| variant_shown(self.repeater, self.launcher, m)),
        })
    }
}

struct Armed {
    n: usize,
    gate: Gate,
    since: Instant,
    frame: u64,
    rotor: Option<Quat>,
    slide: Option<Vec3>,
    matched: Option<(u64, serde_json::Value)>,
}

#[derive(Resource)]
struct Loop {
    dir: PathBuf,
    started: Instant,
    frame: u64,
    processed: usize,
    window_at: Option<IVec2>,
    placed: bool,
    release: Vec<(KeyCode, u64)>,
    armed: Option<Armed>,
    flashes: Vec<Flash>,
    focus: Option<u64>,
    last_state: Option<Instant>,
    reads: Vec<WeaponRead>,
    exit_at: Option<u64>,
    input: serde_json::Value,
    focus_request: Option<usize>,
    focused: bool,
}

impl Loop {
    fn result(&self, n: usize, status: &str, data: serde_json::Value) {
        let line = serde_json::json!({"n": n, "status": status, "t_ms": epoch_ms(),
            "frame": self.frame, "data": data});
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("results.jsonl"))
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

#[derive(Component)]
struct LoopShot {
    n: usize,
    data: serde_json::Value,
}

pub(crate) struct CreatorLoopQaPlugin;

impl Plugin for CreatorLoopQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(dir) = std::env::var_os(ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        app.insert_resource(Loop {
            dir,
            started: Instant::now(),
            frame: 0,
            processed: 0,
            window_at: std::env::var(WINDOW_ENV)
                .ok()
                .and_then(|value| parse_window_at(&value)),
            placed: false,
            release: Vec::new(),
            armed: None,
            flashes: Vec::new(),
            focus: None,
            last_state: None,
            reads: Vec::new(),
            exit_at: None,
            input: serde_json::Value::Null,
            focus_request: None,
            focused: false,
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(
            PreUpdate,
            (keep_window, run_commands, focus_window)
                .chain()
                .after(bevy::input::InputSystems)
                .after(bevy::ui::UiSystems::Focus),
        )
        .add_systems(
            PostUpdate,
            observe
                .after(bevy::transform::TransformSystems::Propagate)
                .after(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
        )
        .add_observer(shot_done);
    }
}

/// 1280x720 at scale 1, parked where the coordinator asked, and continuous
/// updates focused or not (the game's own pacing idles an unfocused window,
/// which would starve the second client of frames).
fn keep_window(
    mut lp: ResMut<Loop>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut pacing: ResMut<bevy::winit::WinitSettings>,
) {
    use bevy::winit::UpdateMode;
    if pacing.focused_mode != UpdateMode::Continuous
        || pacing.unfocused_mode != UpdateMode::Continuous
    {
        pacing.focused_mode = UpdateMode::Continuous;
        pacing.unfocused_mode = UpdateMode::Continuous;
    }
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        if window.physical_width() != 1280 || window.physical_height() != 720 {
            window.resolution.set_physical_resolution(1280, 720);
        }
        if !lp.placed {
            lp.placed = true;
            if let Some(at) = lp.window_at {
                window.position = WindowPosition::At(at);
            }
        }
    }
}

/// Real OS focus for the window, the way a click on it would give it; the
/// production policy that idles an unfocused window's input stays untouched.
fn focus_window(
    mut lp: ResMut<Loop>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    _main: bevy::ecs::system::NonSendMarker,
) {
    let Ok((entity, window)) = windows.single() else {
        return;
    };
    lp.focused = window.focused;
    let Some(n) = lp.focus_request.take() else {
        return;
    };
    let requested = bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        windows
            .get_window(entity)
            .map(|window| window.focus_window())
            .is_some()
    });
    lp.result(
        n,
        if requested { "ok" } else { "unavailable" },
        serde_json::json!({"was_focused": window.focused}),
    );
}

#[allow(clippy::type_complexity)]
fn nearest_dummy(
    actors: &Query<(
        Entity,
        &NetworkPlayerId,
        &Transform,
        Option<&crate::combat::CombatStats>,
        Has<Player>,
    )>,
    from: Vec3,
) -> Option<(u64, Vec3)> {
    actors
        .iter()
        .filter(|(_, _, _, stats, local)| {
            !*local && stats.is_some_and(|s| s.max_hp >= shared::debug::DUMMY_MAX_HP && s.hp > 0.0)
        })
        .map(|(_, id, t, _, _)| (id.0, t.translation))
        .min_by(|a, b| a.1.distance(from).total_cmp(&b.1.distance(from)))
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_commands(
    mut commands: Commands,
    mut lp: ResMut<Loop>,
    mut buttons: super::TestIdPresses,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut menu: Option<ResMut<crate::pause_menu::PauseMenuState>>,
    mut outgoing: MessageWriter<NetworkCommand>,
    mut camera: ResMut<crate::camera::CameraState>,
    mut settings: ResMut<crate::camera::CameraSettings>,
    mut nav: Option<ResMut<crate::minimap::MinimapNavigationState>>,
    actors: Query<(
        Entity,
        &NetworkPlayerId,
        &Transform,
        Option<&crate::combat::CombatStats>,
        Has<Player>,
    )>,
    mut scrolls: Query<(super::QaName, &mut ScrollPosition)>,
    mut orders: (
        Option<ResMut<crate::combat::PendingCast>>,
        Option<ResMut<crate::targeting::BasicAttackState>>,
    ),
    mut exit: MessageWriter<AppExit>,
) {
    lp.frame += 1;
    let frame = lp.frame;
    if lp.exit_at.is_some_and(|at| frame >= at) {
        exit.write(AppExit::Success);
        return;
    }
    let due: Vec<KeyCode> = lp
        .release
        .iter()
        .filter(|(_, at)| *at <= frame)
        .map(|(key, _)| *key)
        .collect();
    lp.release.retain(|(_, at)| *at > frame);
    for key in due {
        keys.release(key);
    }
    if let Some(id) = lp.focus {
        if let Some((_, _, pose, _, _)) = actors.iter().find(|(_, p, _, _, _)| p.0 == id) {
            if let Some(nav) = nav.as_deref_mut() {
                nav.focus_target = Some(pose.translation);
            }
        }
    }
    let local = actors
        .iter()
        .find(|(_, _, _, _, local)| *local)
        .map(|(entity, _, pose, _, _)| (entity, pose.translation));
    // One command a frame keeps presses and their screens in order.
    if !frame.is_multiple_of(3) || lp.armed.is_some() {
        return;
    }
    let Ok(text) = std::fs::read_to_string(lp.dir.join("commands.jsonl")) else {
        return;
    };
    let Some(line) = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .nth(lp.processed)
    else {
        return;
    };
    let n = lp.processed;
    lp.processed += 1;
    let command = match serde_json::from_str::<Command>(line) {
        Ok(command) => command,
        Err(error) => {
            lp.result(
                n,
                "invalid",
                serde_json::json!({"error": error.to_string()}),
            );
            return;
        }
    };
    let aim_at_dummy = |slot: Option<u8>, outgoing: &mut MessageWriter<NetworkCommand>| {
        let from = local.map_or(Vec3::ZERO, |(_, at)| at);
        let target = nearest_dummy(&actors, from)?;
        match slot {
            None => outgoing.write(NetworkCommand::BasicAttack {
                target: shared::wire::TargetId::player(target.0),
            }),
            Some(slot) => outgoing.write(NetworkCommand::CastSkill {
                slot,
                aim: target.1.xz(),
            }),
        };
        Some(target)
    };
    match command {
        Command::Press { id } => {
            let found = buttons.press(&id);
            lp.result(
                n,
                if found { "ok" } else { "not_found" },
                serde_json::json!({"id": id}),
            );
        }
        Command::Key { code } => match key_code(&code) {
            Some(key) => {
                keys.press(key);
                lp.release.push((key, frame + 2));
                lp.result(n, "ok", serde_json::json!({"code": code}));
            }
            None => lp.result(n, "invalid", serde_json::json!({"code": code})),
        },
        Command::Menu { open } => match menu.as_deref_mut() {
            Some(menu) => {
                menu.open = open;
                lp.result(n, "ok", serde_json::json!({"open": open}));
            }
            None => lp.result(n, "unavailable", serde_json::Value::Null),
        },
        Command::Attack => {
            let target = aim_at_dummy(None, &mut outgoing);
            lp.result(
                n,
                if target.is_some() { "ok" } else { "no_target" },
                serde_json::json!({"target": target.map(|t| t.0)}),
            );
        }
        Command::Cast { slot } => {
            let target = aim_at_dummy(Some(slot), &mut outgoing);
            lp.result(
                n,
                if target.is_some() { "ok" } else { "no_target" },
                serde_json::json!({"slot": slot, "target": target.map(|t| t.0)}),
            );
        }
        Command::Move { dx, dz } => match local {
            Some((entity, at)) => {
                // What a ground click does first (`player::input`): a move
                // order drops any pending cast and basic-attack order.
                if let Some(pending) = orders.0.as_deref_mut() {
                    pending.cancel();
                }
                if let Some(basic) = orders.1.as_deref_mut() {
                    basic.cancel_for_movement();
                }
                commands
                    .entity(entity)
                    .insert(crate::player::MovementTarget {
                        target: at + Vec3::new(dx, 0.0, dz),
                    });
                lp.result(n, "ok", serde_json::json!({"from": at.to_array()}));
            }
            None => lp.result(n, "no_hero", serde_json::Value::Null),
        },
        Command::Camera {
            lock,
            focus_player,
            zoom,
        } => {
            if let Some(lock) = lock {
                camera.locked = lock;
            }
            lp.focus = focus_player;
            if focus_player.is_none()
                && let Some(nav) = nav.as_deref_mut()
            {
                nav.focus_target = None;
            }
            if let Some(zoom) = zoom {
                let zoom = zoom.clamp(
                    crate::camera::CAMERA_MIN_ZOOM,
                    crate::camera::CAMERA_MAX_ZOOM,
                );
                camera.zoom = zoom;
                settings.zoom = zoom;
            }
            lp.result(
                n,
                "ok",
                serde_json::json!({"zoom": camera.zoom, "locked": camera.locked}),
            );
        }
        Command::Scroll { node, y } => {
            let mut found = false;
            for (key, mut scroll) in &mut scrolls {
                if key.as_str() == node {
                    scroll.y = y;
                    found = true;
                }
            }
            lp.result(
                n,
                if found { "ok" } else { "not_found" },
                serde_json::json!({"node": node, "y": y}),
            );
        }
        Command::Shot { file } => {
            spawn_shot(&mut commands, &lp, n, &file, serde_json::Value::Null);
        }
        Command::ShotWhen(gate) => {
            let read = lp.reads.iter().find(|r| match gate.player {
                Some(id) => r.owner == id,
                None => r.local,
            });
            let (rotor, slide) = read.map_or((None, None), |r| (r.rotor, r.slide));
            let mut sent = serde_json::Value::Null;
            if let Some(action) = gate.action.as_deref() {
                sent = match action.split_once(':') {
                    None if action == "attack" => {
                        serde_json::json!({"attack": aim_at_dummy(None, &mut outgoing).map(|t| t.0)})
                    }
                    Some(("cast", slot)) => {
                        let slot = slot.parse().unwrap_or(0);
                        serde_json::json!({"cast": slot, "target": aim_at_dummy(Some(slot), &mut outgoing).map(|t| t.0)})
                    }
                    Some(("key", code)) => match key_code(code) {
                        Some(key) => {
                            keys.press(key);
                            lp.release.push((key, frame + 2));
                            serde_json::json!({"key": code})
                        }
                        None => serde_json::json!({"invalid_key": code}),
                    },
                    _ => serde_json::json!({"invalid_action": action}),
                };
            }
            lp.armed = Some(Armed {
                n,
                gate,
                since: Instant::now(),
                frame,
                rotor,
                slide,
                matched: None,
            });
            let _ = std::fs::write(
                lp.dir.join("armed.json"),
                serde_json::json!({"n": n, "sent": sent, "t_ms": epoch_ms()}).to_string(),
            );
        }
        Command::Focus => lp.focus_request = Some(n),
        Command::Exit => {
            lp.result(n, "ok", serde_json::Value::Null);
            lp.exit_at = Some(frame + 2);
        }
    }
}

fn spawn_shot(commands: &mut Commands, lp: &Loop, n: usize, file: &str, data: serde_json::Value) {
    let path = lp.dir.join(file);
    let _ = std::fs::remove_file(&path);
    commands
        .spawn((Screenshot::primary_window(), LoopShot { n, data }))
        .observe(save_to_disk(path));
}

fn shot_done(event: On<ScreenshotCaptured>, shots: Query<&LoopShot>, lp: Res<Loop>) {
    let Ok(shot) = shots.get(event.entity) else {
        return;
    };
    let mut data = shot.data.clone();
    if let Some(object) = data.as_object_mut() {
        object.insert(
            "pixels".into(),
            serde_json::json!([event.image.width(), event.image.height()]),
        );
    } else {
        data = serde_json::json!({"pixels": [event.image.width(), event.image.height()]});
    }
    lp.result(shot.n, "captured", data);
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn observe(
    mut commands: Commands,
    mut lp: ResMut<Loop>,
    screen: Res<State<AppScreen>>,
    session: Res<ClientSession>,
    snapshot: Res<GameStateSnapshot>,
    selection: Res<crate::team::TeamSelection>,
    assets: Res<AssetServer>,
    mut bursts: MessageReader<crate::game_vfx::SkillBurst>,
    weapons: Query<(Entity, &HeldWeapon, &GlobalTransform, &WorldAssetRoot)>,
    children: Query<&Children>,
    nodes: Query<
        (
            &Name,
            &Transform,
            Option<&Visibility>,
            Option<&InheritedVisibility>,
        ),
        Without<Mesh3d>,
    >,
    actors: Query<(
        &NetworkPlayerId,
        &NetworkHeroClass,
        Option<&PlayerHandheld>,
        Option<&PlayerLoadout>,
        &Transform,
        Has<Player>,
        Option<&crate::combat::CombatStats>,
    )>,
    cameras: Query<(&Camera, &GlobalTransform), With<crate::camera::MainCamera>>,
    labels: Query<(super::QaName, &Text)>,
    pressables: Query<
        (&crate::ui::TestId, Option<&InheritedVisibility>),
        With<crate::ui::Pressable>,
    >,
    context: Res<crate::input_context::GameplayInputContext>,
) {
    let frame = lp.frame;
    lp.input = serde_json::json!({"running": context.running, "modal_open": context.modal_open,
        "gameplay_allowed": context.gameplay_allowed()});
    let camera = cameras.iter().find(|(camera, _)| camera.is_active);
    // Every held weapon, read the way the renderer will draw it.
    let mut reads = Vec::new();
    let mut frames = HashMap::new();
    for (entity, weapon, pose, scene) in &weapons {
        let owner = actors.get(weapon.owner).ok();
        let mut read = WeaponRead {
            owner: owner.map_or(0, |(id, ..)| id.0),
            local: owner.is_some_and(|(_, _, _, _, _, local, _)| local),
            id: weapon.id.clone(),
            model: weapon.model.clone(),
            mode: Some(weapon.mode),
            loaded: matches!(
                assets.get_recursive_dependency_load_state(scene.0.id()),
                Some(bevy::asset::RecursiveDependencyLoadState::Loaded)
            ),
            screen: camera
                .and_then(|(camera, at)| camera.world_to_viewport(at, pose.translation()).ok()),
            ..default()
        };
        for node in children.iter_descendants(entity) {
            let Ok((name, transform, visibility, inherited)) = nodes.get(node) else {
                continue;
            };
            let state = NodeRead {
                hidden: visibility == Some(&Visibility::Hidden),
                inherited: inherited.is_some_and(|v| v.get()),
                scale: transform.scale.min_element(),
            };
            match name.as_str() {
                "WildsparkSetRepeater" => read.repeater = read.repeater.or(Some(state)),
                "WildsparkSetLauncher" => read.launcher = read.launcher.or(Some(state)),
                "WildsparkRotor" => read.rotor = read.rotor.or(Some(transform.rotation)),
                "WildsparkSlide" => read.slide = read.slide.or(Some(transform.translation)),
                _ => {}
            }
        }
        frames.insert(entity, (pose.affine().inverse(), tips(weapon), read.owner));
        reads.push(read);
    }
    // Muzzle flashes: the first particle of `held_weapons::emit_muzzle`.
    for burst in bursts.read() {
        for spec in &burst.0 {
            if spec.source != crate::game_vfx::ParticleSource::Accent
                || (spec.lifetime - 0.09).abs() > 1e-3
                || (spec.size - 0.32).abs() > 1e-3
            {
                continue;
            }
            let best = frames
                .values()
                .filter_map(|(inverse, (repeater, launcher), owner)| {
                    let local = inverse.transform_point3(spec.origin);
                    classify_tip(local, *repeater, *launcher).map(|(tip, d)| (*owner, tip, d))
                })
                .min_by(|a, b| a.2.total_cmp(&b.2));
            if let Some((owner, tip, distance)) = best {
                lp.flashes.push(Flash {
                    frame,
                    owner,
                    tip,
                    distance,
                    world: spec.origin,
                });
            }
        }
    }
    lp.flashes.retain(|flash| flash.frame + 600 > frame);
    // A gated still: evaluate, then capture one frame after it held.
    if let Some(mut armed) = lp.armed.take() {
        let read = reads.iter().find(|r| match armed.gate.player {
            Some(id) => r.owner == id,
            None => r.local,
        });
        if let Some((at, data)) = armed.matched.clone() {
            if frame > at {
                let file = armed.gate.file.clone();
                let mut data = data;
                if let (Some(object), Some(read)) = (data.as_object_mut(), read) {
                    object.insert("weapon_at_capture".into(), read.json());
                }
                spawn_shot(&mut commands, &lp, armed.n, &file, data);
                return_state(
                    &mut lp,
                    reads,
                    &screen,
                    &session,
                    &snapshot,
                    &selection,
                    &actors,
                    &labels,
                    &pressables,
                );
                return;
            }
        } else if let Some(read) = read {
            let item_ok = armed.gate.item.as_ref().is_none_or(|id| *id == read.id);
            let mode_ok = armed.gate.mode.is_none_or(|mode| {
                read.mode == Some(mode)
                    && (read.repeater.is_none() && read.launcher.is_none()
                        || variant_shown(read.repeater, read.launcher, mode))
            });
            let flash = lp
                .flashes
                .iter()
                .filter(|f| f.owner == read.owner && f.frame >= armed.frame)
                .find(|f| read.mode.is_none_or(|mode| f.tip == mode))
                .cloned();
            let muzzle_ok = !armed.gate.muzzle || flash.is_some();
            let rotor_turn = match (armed.rotor, read.rotor) {
                (Some(before), Some(now)) => before.angle_between(now),
                _ => 0.0,
            };
            let slide_kick = match (armed.slide, read.slide) {
                (Some(before), Some(now)) => before.distance(now),
                _ => 0.0,
            };
            let motion_ok = !armed.gate.motion
                || match read.mode {
                    Some(WeaponMode::Rockets) => slide_kick > 0.01,
                    _ => rotor_turn > 0.2,
                };
            if read.loaded && item_ok && mode_ok && muzzle_ok && motion_ok {
                armed.matched = Some((
                    frame,
                    serde_json::json!({
                        "gate": format!("{:?}", armed.gate),
                        "matched_frame": frame,
                        "seconds_after_arm": armed.since.elapsed().as_secs_f32(),
                        "weapon": read.json(),
                        "rotor_turn_since_arm_rad": rotor_turn,
                        "slide_offset_since_arm_m": slide_kick,
                        "flash": flash.map(|f| serde_json::json!({"tip": f.tip, "distance_to_tip_m": f.distance,
                            "world": f.world.to_array(), "frame": f.frame})),
                    }),
                ));
            }
        }
        if armed.since.elapsed().as_secs_f32() > armed.gate.timeout_s && armed.matched.is_none() {
            lp.result(
                armed.n,
                "timeout",
                serde_json::json!({"gate": format!("{:?}", armed.gate), "weapon": read.map(WeaponRead::json),
                    "flashes": lp.flashes.iter().filter(|f| f.frame >= armed.frame).map(|f| serde_json::json!({"owner": f.owner, "tip": f.tip, "distance": f.distance})).collect::<Vec<_>>()}),
            );
        } else {
            lp.armed = Some(armed);
        }
    }
    return_state(
        &mut lp,
        reads,
        &screen,
        &session,
        &snapshot,
        &selection,
        &actors,
        &labels,
        &pressables,
    );
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn return_state(
    lp: &mut Loop,
    reads: Vec<WeaponRead>,
    screen: &State<AppScreen>,
    session: &ClientSession,
    snapshot: &GameStateSnapshot,
    selection: &crate::team::TeamSelection,
    actors: &Query<(
        &NetworkPlayerId,
        &NetworkHeroClass,
        Option<&PlayerHandheld>,
        Option<&PlayerLoadout>,
        &Transform,
        Has<Player>,
        Option<&crate::combat::CombatStats>,
    )>,
    labels: &Query<(super::QaName, &Text)>,
    pressables: &Query<
        (&crate::ui::TestId, Option<&InheritedVisibility>),
        With<crate::ui::Pressable>,
    >,
) {
    lp.reads = reads;
    if lp.last_state.is_some_and(|at| at.elapsed() < STATE_EVERY) {
        return;
    }
    lp.last_state = Some(Instant::now());
    let (revision, status, entries) = omoba_passport::weapon_store::snapshot();
    let selected = match &selection.handheld {
        shared::handheld::HandheldSelection::Item(id) => Some(serde_json::json!({
            "id": id,
            "model_state": format!("{:?}", omoba_passport::weapon_store::model_state(id)),
            "eligible": omoba_passport::weapon_store::eligible(id),
        })),
        _ => None,
    };
    let mut texts = serde_json::Map::new();
    for (key, text) in labels.iter() {
        if LABELS.contains(&key.as_str()) {
            texts.insert(key.as_str().to_owned(), text.0.clone().into());
        }
    }
    let mut buttons: Vec<&str> = pressables
        .iter()
        .filter(|(_, visible)| visible.is_some_and(|v| v.get()))
        .map(|(id, _)| id.as_str())
        .collect();
    buttons.sort_unstable();
    buttons.truncate(300);
    let players: Vec<_> = actors
        .iter()
        .take(24)
        .map(|(id, class, handheld, loadout, pose, local, stats)| {
            serde_json::json!({
                "id": id.0, "class": class.0.id(), "local": local,
                "handheld": handheld.map(|h| serde_json::to_value(&h.0).unwrap_or_default()),
                "weapon_mode": loadout.and_then(|l| l.0.as_ref()).map(|l| l.weapon_mode),
                "root_remaining_secs": loadout.and_then(|l| l.0.as_ref()).map(|l| l.root_remaining_secs),
                "movement_multiplier": loadout.and_then(|l| l.0.as_ref()).map(|l| l.movement_multiplier),
                "position": pose.translation.to_array(),
                "hp": stats.map(|s| [s.hp, s.max_hp]),
            })
        })
        .collect();
    let state = serde_json::json!({
        "t_ms": epoch_ms(),
        "uptime_s": lp.started.elapsed().as_secs_f32(),
        "frame": lp.frame,
        "processed": lp.processed,
        "armed": lp.armed.as_ref().map(|a| a.n),
        "input": lp.input,
        "window_focused": lp.focused,
        "screen": format!("{:?}", screen.get()),
        "session": {"connected": session.is_connected(), "join_confirmed": session.join_confirmed(),
            "your_id": snapshot.your_id, "match_mode": snapshot.match_mode},
        "account": {"connected": crate::passport::account_connected(),
            "status_line": crate::passport::avatar_account_status_line()},
        "weapons": {"revision": revision, "status": format!("{status:?}"),
            "entries": entries.iter().map(|e| serde_json::json!({"id": e.id, "name": e.name,
                "owned": e.owned, "author": e.author, "license": e.license})).collect::<Vec<_>>()},
        "selection": {"handheld": serde_json::to_value(&selection.handheld).unwrap_or_default(),
            "class": selection.hero_class.id(), "item": selected},
        "labels": texts,
        "buttons": buttons,
        "players": players,
        "held": lp.reads.iter().map(WeaponRead::json).collect::<Vec<_>>(),
        "recent_flashes": lp.flashes.iter().rev().take(8).map(|f| serde_json::json!({"frame": f.frame,
            "owner": f.owner, "tip": f.tip, "distance": f.distance})).collect::<Vec<_>>(),
    });
    let path = lp.dir.join("qa-state.json");
    let temp = lp.dir.join("qa-state.json.tmp");
    if std::fs::write(&temp, state.to_string()).is_ok() {
        let _ = std::fs::rename(temp, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(hidden: bool, inherited: bool, scale: f32) -> Option<NodeRead> {
        Some(NodeRead {
            hidden,
            inherited,
            scale,
        })
    }

    #[test]
    fn commands_parse_from_one_json_line_each() {
        assert_eq!(
            serde_json::from_str::<Command>(r#"{"op":"press","id":"HomeCollection"}"#).unwrap(),
            Command::Press {
                id: "HomeCollection".into()
            }
        );
        assert_eq!(
            serde_json::from_str::<Command>(r#"{"op":"attack"}"#).unwrap(),
            Command::Attack
        );
        assert_eq!(
            serde_json::from_str::<Command>(r#"{"op":"scroll","node":"Grid","y":400}"#).unwrap(),
            Command::Scroll {
                node: "Grid".into(),
                y: 400.0
            }
        );
        let Command::ShotWhen(gate) = serde_json::from_str::<Command>(
            r#"{"op":"shot_when","file":"a.png","mode":"rockets","muzzle":true,"action":"key:KeyQ"}"#,
        )
        .unwrap() else {
            panic!("shot_when");
        };
        assert_eq!(gate.mode, Some(WeaponMode::Rockets));
        assert!(gate.muzzle && !gate.motion);
        assert_eq!(gate.player, None);
        assert_eq!(gate.timeout_s, 12.0);
        assert_eq!(
            serde_json::from_str::<Command>(r#"{"op":"focus"}"#).unwrap(),
            Command::Focus
        );
        assert!(serde_json::from_str::<Command>(r#"{"op":"format_disk"}"#).is_err());
    }

    #[test]
    fn keys_and_window_positions_parse() {
        assert_eq!(key_code("KeyQ"), Some(KeyCode::KeyQ));
        assert_eq!(key_code("KeyZ"), None);
        assert_eq!(parse_window_at("10, 40"), Some(IVec2::new(10, 40)));
        assert_eq!(parse_window_at("10"), None);
    }

    #[test]
    fn only_the_variant_for_the_mode_counts_as_shown() {
        let drawn = node(false, true, 1.0);
        let hidden = node(true, false, 1.0);
        assert!(variant_shown(drawn, hidden, WeaponMode::Repeater));
        assert!(!variant_shown(drawn, hidden, WeaponMode::Rockets));
        assert!(variant_shown(hidden, drawn, WeaponMode::Rockets));
        // The authored zero-scale launcher is not drawn even when visible.
        assert!(!variant_shown(
            hidden,
            node(false, true, 0.0),
            WeaponMode::Rockets
        ));
        // Both drawn ("mixed") never passes, and a model without the set never does.
        assert!(!variant_shown(drawn, drawn, WeaponMode::Repeater));
        assert!(!variant_shown(None, drawn, WeaponMode::Rockets));
    }

    #[test]
    fn a_flash_belongs_to_the_nearest_tip_within_tolerance() {
        let repeater = Vec3::from_array(REPEATER_TIP);
        let launcher = Vec3::from_array(LAUNCHER_TIP);
        assert_eq!(
            classify_tip(repeater, repeater, launcher).map(|(m, _)| m),
            Some(WeaponMode::Repeater)
        );
        assert_eq!(
            classify_tip(launcher + Vec3::X * 0.01, repeater, launcher).map(|(m, _)| m),
            Some(WeaponMode::Rockets)
        );
        assert_eq!(classify_tip(Vec3::ZERO, repeater, launcher), None);
    }
}
