//! Focused, opt-in native captures using live sandbox setup and normal skill commands.
//!
//! `OMOBA_STANDARD_QA_OFFSCREEN=1` hides the window and renders the main
//! camera to an image, so a locked or covered desktop still yields frames.
//! `OMOBA_STANDARD_QA_PHASES=1` replaces the single frame per skill by three
//! stills (windup, release, impact or settled) gated on replicated state; see
//! the phase capture section at the end of this file. For a look at a motion
//! clip, `OMOBA_STANDARD_QA_AVATAR=<slug>` picks the hero's rig of a phase run
//! and `OMOBA_STANDARD_QA_RELEASE_AT=contact|<seconds>` moves the release still
//! of a skill without a telegraph to the clip's contact time or a fixed time.
//! For a look at projectile bodies, `OMOBA_STANDARD_QA_FLIGHT=1` stands the
//! target of every unit-target ability far enough for its projectile to be in
//! flight at the release still and adds the basic attack: one still of its
//! projectile in flight (a melee core throws none) and one of its hit. A
//! repeater then casts its weapon switch and shows the same two stills of its
//! rocket round. `OMOBA_STANDARD_QA_INTERLEAVE=1` orders one basic
//! attack as soon as the cast of a skill with a telegraph is accepted, so the
//! stills show what an action accepted during the telegraph does to the pose.
//! `OMOBA_STANDARD_QA_DISPLACE=1` moves the hero across its aim through the
//! sandbox as soon as the cast of a skill whose telegraph the server keeps on
//! its caster is accepted, so the stills show whether the telegraph stays under
//! a caster that is moved. `OMOBA_STANDARD_QA_BLOCK=1` makes the target cast
//! its first skill at the hero as soon as the cast of a skill that raises a
//! shield wall is accepted, so the later stills show a wall that has stopped a
//! projectile.
//! `OMOBA_STANDARD_QA_AIM=1` adds one still of the aim preview of every modular
//! skill, taken with its key held before the cast, and a second one when the
//! slot offers a recast after the cast.
//! `OMOBA_STANDARD_QA_RECIPE=<skill>,<skill>,<skill>,<skill>` gives the hero of a
//! phase run an authored kit on the core of its class, so that skills of other
//! classes are captured on its Q, W, E and R.
use crate::{
    frontend::{AppScreen, ScreenDriverPaused},
    help_overlay::HelpOverlayVisible,
    net::{
        ClientSession, GameStateSnapshot, NetworkCommand, NetworkHeroClass, PlayerLoadout,
        PlayerProgression,
    },
    player::Player,
    team::{CharacterChoice, Team, TeamSelection},
};
use bevy::{
    app::AppExit,
    camera::RenderTarget,
    ecs::system::SystemParam,
    prelude::*,
    render::{
        render_resource::{TextureFormat, TextureUsages},
        view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    },
    window::PrimaryWindow,
};
use shared::{
    HeroClass, SkillSlot, TargetingMode,
    loadout::{EffectVisualKind, SkillEffect, SkillEffectState, SkillId, Technique, WeaponMode},
    sandbox::{
        ActorConfig, SandboxActor, SandboxCommand, SandboxConfig, SandboxRequest, SandboxSnapshot,
    },
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    f32::consts::TAU,
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) struct StandardKitsQaPlugin;
impl Plugin for StandardKitsQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_STANDARD_QA_DIR").map(PathBuf::from) else {
            return;
        };
        let class = std::env::var("OMOBA_STANDARD_QA_CLASS")
            .ok()
            .and_then(|s| HeroClass::from_id(&s))
            .unwrap_or(HeroClass::Dawnweaver);
        let flag = |name: &str| std::env::var(name).as_deref() == Ok("1");
        let phases = flag("OMOBA_STANDARD_QA_PHASES");
        let offscreen = flag("OMOBA_STANDARD_QA_OFFSCREEN");
        let flight = flag("OMOBA_STANDARD_QA_FLIGHT");
        let interleave = flag("OMOBA_STANDARD_QA_INTERLEAVE");
        let displace = flag("OMOBA_STANDARD_QA_DISPLACE");
        let block = flag("OMOBA_STANDARD_QA_BLOCK");
        let aim = flag("OMOBA_STANDARD_QA_AIM");
        let recipe = std::env::var("OMOBA_STANDARD_QA_RECIPE")
            .ok()
            .map(|ids| authored_kit(class, &ids).expect("OMOBA_STANDARD_QA_RECIPE"));
        let release_at = match std::env::var("OMOBA_STANDARD_QA_RELEASE_AT").as_deref() {
            Err(_) => ReleaseAt::Window,
            Ok("contact") => ReleaseAt::Contact,
            Ok(secs) => ReleaseAt::Secs(
                secs.parse()
                    .ok()
                    .filter(|secs| (0.0..=2.0).contains(secs))
                    .expect("OMOBA_STANDARD_QA_RELEASE_AT is `contact` or 0..=2 seconds"),
            ),
        };
        if offscreen {
            // Hidden before winit creates it: an invisible window makes the
            // runner update on its timer instead of waiting for a redraw that
            // a locked or covered desktop never requests.
            let world = app.world_mut();
            let mut windows = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
            for mut window in windows.iter_mut(world) {
                window.visible = false;
            }
        }
        app.insert_resource(Qa {
            directory,
            class,
            avatar: std::env::var("OMOBA_STANDARD_QA_AVATAR").unwrap_or_else(|_| "agnes".into()),
            release_at,
            phases,
            offscreen,
            flight,
            interleave,
            displace,
            block,
            aim,
            recipe,
            zoom: crate::camera::CAMERA_MIN_ZOOM,
            idles: HashMap::new(),
            target: None,
            black: None,
            ux: std::env::var_os("OMOBA_COMBAT_UX_QA").is_some(),
            hud: std::env::var_os("OMOBA_COMBAT_UX_HUD_QA").is_some(),
            stage: 0,
            roster: std::env::var_os("OMOBA_ROSTER_SKILLS_QA").is_some(),
            weapons: std::env::var_os("OMOBA_HANDHELD_QA").is_some(),
            slot: 0,
            action_before: 0,
            last_cast_simulation: -10.0,
            prepared: false,
            frames: 0,
            aim_ack: 0,
            started: Instant::now(),
            captures: Vec::new(),
            readbacks: Vec::new(),
            requests: Vec::new(),
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .insert_resource(ScreenDriverPaused(true))
        .add_systems(Startup, label)
        .add_systems(
            PreUpdate,
            prepare
                .after(bevy::ui::UiSystems::Focus)
                .after(bevy::input::InputSystems),
        )
        .add_systems(
            PostUpdate,
            observe
                .after(bevy::ui::UiSystems::Layout)
                .after(bevy::transform::TransformSystems::Propagate),
        );
        if offscreen {
            app.add_systems(PreUpdate, (pace_offscreen, retarget_main_camera));
        } else {
            app.add_systems(PreUpdate, super::combat_qa::focus_capture_window);
        }
        if phases {
            app.init_resource::<Phases>().add_systems(
                PostUpdate,
                drive_phases
                    .after(observe)
                    .after(crate::game_vfx::VfxPresentation)
                    .after(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
            );
            if aim {
                app.add_systems(PreUpdate, hold_aim.after(bevy::input::InputSystems))
                    .add_systems(PostUpdate, restore_cursor);
            }
        }
    }
}
#[derive(Resource)]
struct Qa {
    directory: PathBuf,
    class: HeroClass,
    /// Rig of the hero in a phase run; the other runs stage their own rigs.
    avatar: String,
    release_at: ReleaseAt,
    /// Three state-gated stills per skill instead of the roster frame.
    phases: bool,
    /// Hidden window; the main camera renders to `target`.
    offscreen: bool,
    /// Phase run staged for a look at projectiles in flight.
    flight: bool,
    /// Phase run that orders a basic attack during every telegraph.
    interleave: bool,
    /// Phase run that moves the hero during a telegraph that follows its caster.
    displace: bool,
    /// Phase run in which the target shoots at a shield wall as it is raised.
    block: bool,
    /// Phase run that also takes stills of the aim previews.
    aim: bool,
    /// Phase run on an authored kit instead of the preset of the class.
    recipe: Option<shared::loadout::BuildRecipe>,
    /// Camera distance of a phase or handheld run; a phase run widens it for a skill
    /// whose area or hit the closest view does not hold.
    zoom: f32,
    /// The stage crop of every idle still, by file: what later stills are compared with.
    idles: HashMap<String, Vec<u8>>,
    target: Option<Handle<Image>>,
    /// A frame that read back black; the run fails instead of keeping it.
    black: Option<String>,
    ux: bool,
    hud: bool,
    stage: u8,
    roster: bool,
    weapons: bool,
    slot: u8,
    action_before: u64,
    last_cast_simulation: f64,
    prepared: bool,
    frames: u32,
    aim_ack: u64,
    started: Instant,
    captures: Vec<serde_json::Value>,
    readbacks: Vec<usize>,
    requests: Vec<serde_json::Value>,
}
impl Qa {
    fn pixels(&self) -> (u32, u32) {
        if self.ux { (1180, 820) } else { (1280, 720) }
    }
}
fn label(mut commands: Commands, qa: Res<Qa>) {
    commands.spawn((
        Text::new(if qa.hud {
            "QA · live practice · authoritative bots and kill notices"
        } else if qa.phases {
            "QA · live sandbox · scripted casts · stills paused at 0.25x"
        } else {
            "QA · live sandbox · scripted commands and held-key input"
        }),
        TextFont {
            font_size: (10.0).into(),
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::BLACK),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(3.0),
            left: Val::Percent(35.0),
            ..default()
        },
        ZIndex(200),
        bevy::ui::FocusPolicy::Pass,
    ));
}
/// The kit of `class` with four named skills on its Q, W, E and R, as the server admits it.
fn authored_kit(class: HeroClass, ids: &str) -> Result<shared::loadout::BuildRecipe, String> {
    let skills = ids
        .split(',')
        .map(|id| SkillId::from_id(id.trim()).ok_or_else(|| format!("unknown skill `{id}`")))
        .collect::<Result<Vec<_>, _>>()?;
    let mut recipe = shared::loadout::preset_for_class(class)
        .ok_or("an authored kit needs the core of a modular class")?
        .recipe();
    recipe.skills = <[SkillId; 4]>::try_from(skills).map_err(|_| "name four skills")?;
    shared::loadout::resolve(&recipe).map_err(|error| error.to_string())?;
    Ok(recipe)
}
fn prepare(
    mut qa: ResMut<Qa>,
    mut selection: ResMut<TeamSelection>,
    mut screen: ResMut<NextState<AppScreen>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    help: Res<HelpOverlayVisible>,
    mut buttons: crate::qa::TestIdPresses,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    sandbox: Res<crate::sandbox::SandboxClient>,
    mut camera: ResMut<crate::camera::CameraState>,
    mut camera_settings: ResMut<crate::camera::CameraSettings>,
) {
    if qa.weapons || qa.phases {
        // Use the closest supported gameplay view for grip and pose inspection.
        *camera = crate::camera::CameraState {
            zoom: qa.zoom,
            ..default()
        };
        camera_settings.zoom = qa.zoom;
    }
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        let (width, height) = qa.pixels();
        if window.physical_width() != width || window.physical_height() != height {
            window.resolution.set_physical_resolution(width, height);
        }
        if (9..=11).contains(&qa.stage) {
            window.set_cursor_position(Some(Vec2::new(850.0, 300.0)));
            let key = if qa.ux {
                KeyCode::KeyR
            } else if qa.class == HeroClass::Dawnweaver {
                KeyCode::KeyQ
            } else {
                KeyCode::KeyW
            };
            if qa.stage == 11 {
                keyboard.release(key);
            } else {
                keyboard.press(key);
            }
        }
    }
    if !qa.prepared {
        qa.prepared = true;
        selection.hero_class = qa.class;
        selection.character = CharacterChoice::Cube;
        selection.avatar = Some(if qa.phases {
            qa.avatar.clone()
        } else {
            "agnes".into()
        });
        screen.set(AppScreen::HeroSelect);
    }
    if help.0 {
        buttons.press("HelpDismissButton");
    }
    if qa.stage >= 2 && sandbox.enabled && sandbox.open {
        if sandbox.overlay {
            if !buttons.press("CombatTestToggle-Overlay") {
                buttons.press("CombatTestTab-World");
            }
        } else {
            buttons.press("CombatTestClose");
        }
    }
}
fn observe(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    snapshot: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    screen: Res<State<AppScreen>>,
    mut next_screen: ResMut<NextState<AppScreen>>,
    local: Query<
        (
            &Transform,
            &NetworkHeroClass,
            &PlayerProgression,
            &PlayerLoadout,
            &crate::net::PlayerCosmeticAction,
        ),
        With<Player>,
    >,
    cooldowns: Query<&crate::net::PlayerSkillCooldowns, With<Player>>,
    nodes: Query<(crate::qa::QaName, &ComputedNode, &InheritedVisibility)>,
    help: Res<HelpOverlayVisible>,
    mut outgoing: MessageWriter<NetworkCommand>,
    mut exit: MessageWriter<AppExit>,
    assets: Res<AssetServer>,
    scenes: Query<&WorldAssetRoot>,
    context: Res<crate::input_context::GameplayInputContext>,
    (
        animations,
        vfx,
        palette,
        camera,
        map_materials,
        materials,
        windows,
        facings,
        held,
        bones,
        player_entities,
        rigs,
    ): (
        Res<crate::sandbox::AnimationReadout>,
        Query<(
            &Name,
            &Visibility,
            &crate::skill_presentation::SkillEffectVisual,
        )>,
        Option<Res<crate::verdant3d::VerdantPaletteMaterials>>,
        Query<
            (
                &bevy::core_pipeline::tonemapping::Tonemapping,
                &bevy::post_process::bloom::Bloom,
            ),
            With<crate::camera::MainCamera>,
        >,
        Query<(
            &bevy::gltf::GltfMaterialName,
            &MeshMaterial3d<StandardMaterial>,
        )>,
        Res<Assets<StandardMaterial>>,
        Query<Entity, With<PrimaryWindow>>,
        Query<(
            &crate::net::NetworkPlayerId,
            &Transform,
            &crate::net::PlayerActionFacing,
        )>,
        Query<(
            &crate::held_weapons::HeldWeapon,
            &GlobalTransform,
            &Transform,
            &WorldAssetRoot,
        )>,
        Query<&GlobalTransform>,
        Query<Entity, With<Player>>,
        Query<(
            &crate::net::NetworkPlayerId,
            &crate::net::NetworkAvatar,
            &crate::model_scale::ModelScaleSource,
            &crate::humanoid::RuntimeHumanoidPlayer,
        )>,
    ),
) {
    if qa.stage == 255 || qa.stage == PHASE_STAGE {
        return;
    }
    if let Some(file) = qa.black.take() {
        fail(
            &mut qa,
            &mut exit,
            &format!("Captured frame {file} is black"),
            serde_json::Value::Null,
        );
        return;
    }
    if qa.started.elapsed() > Duration::from_secs(120) {
        fail(
            &mut qa,
            &mut exit,
            "Timed out waiting for live server state or screenshot readback",
            serde_json::Value::Null,
        );
        return;
    }
    qa.frames += 1;
    let paving = map_materials
        .iter()
        .find(|(name, _)| name.0 == "VC / worn ceremonial paving")
        .and_then(|(_, binding)| {
            let color = materials.get(&binding.0)?.base_color.to_linear();
            Some(serde_json::json!({
                "linear_rgba": [color.red, color.green, color.blue, color.alpha],
                "tuned_binding": palette.as_ref().is_some_and(|p| p.contains_tuned(&binding.0)),
            }))
        });
    let frame = serde_json::json!({
        "snapshot_tick": snapshot.meta.snapshot_tick,
        "server_epoch": snapshot.meta.server_epoch,
        "match_id": snapshot.meta.match_id,
        "screen": format!("{:?}", screen.get()),
        "gameplay_allowed": context.gameplay_allowed(),
        "scripted_aim_held": matches!(qa.stage, 9 | 10),
        "effects": snapshot.skill_effects,
        "action": local.single().ok().map(|(_,_,_,_,a)| serde_json::json!({"sequence":a.sequence,"slot":a.slot})),
        "roster_slot": qa.slot,
        "rigs": rigs.iter().map(|(id, avatar, source, rig)| serde_json::json!({
            "owner":id.0,"avatar":avatar.0,
            "model":assets.get_path(source.gltf.id()).map(|p|p.to_string()),
            "bound_to_model":rig.model == source.gltf.id(),
        })).collect::<Vec<_>>(),
        "handhelds": held.iter().map(|(w,world,local,scene)| serde_json::json!({
            "id":w.id,"owner":facings.get(w.owner).ok().map(|(id,_,_)|id.0),
            "position":world.translation().to_array(),
            "hand":bones.get(w.hand).ok().map(|p|p.translation().to_array()),
            "attachment_error":bones.get(w.hand).ok().map(|p|p.mul_transform(*local).translation().distance(world.translation())),
            "loaded":matches!(assets.get_recursive_dependency_load_state(scene.0.id()),Some(bevy::asset::RecursiveDependencyLoadState::Loaded)),
        })).collect::<Vec<_>>(),
        "facings": facings.iter().map(|(id, pose, facing)| serde_json::json!({"id":id.0,"sequence":facing.sequence,"action_yaw":facing.yaw,"visual_forward":(pose.rotation * Vec3::NEG_Z).to_array()})).collect::<Vec<_>>(),
        "animations": animations.0,
        "rendering": {
            "environment_palette_materials": palette.as_ref().map_or(0, |p| p.count()),
            "paving_material": paving,
            "postprocess": camera.single().ok().map(|(tonemapping, bloom)| serde_json::json!({
                "tonemapping": format!("{tonemapping:?}"),
                "bloom_intensity": bloom.intensity,
                "bloom_threshold": bloom.prefilter.threshold,
            })),
        },
        "skill_vfx": vfx.iter().map(|(name,visibility,visual)| serde_json::json!({
            "name":name.as_str(), "visible":*visibility != Visibility::Hidden,
            "effect_id":visual.id, "model_ready":visual.model_ready,
        })).collect::<Vec<_>>(),
        "loadout": local.single().ok().and_then(|(_,_,_,l,_)| l.0.as_ref()),
        "nodes": nodes.iter().filter(|(n,_,_)| n.as_str().starts_with("ClassButton") || ["StandardKitStatus","HeroOverheadPlate","AlliedVitals","MinimapSkillVector","KillFeed","MinimapTrap"].contains(&n.as_str()))
            .map(|(n,c,v)| serde_json::json!({"name":n.as_str(),"size":c.size().to_array(),"visible":v.get()})).collect::<Vec<_>>()
    });
    if qa.stage == 8
        && qa.class == HeroClass::Dawnweaver
        && !qa.captures.iter().any(|c| c["file"] == FILES[5])
        && snapshot
            .skill_effects
            .iter()
            .any(|e| e.kind == EffectVisualKind::Beam)
    {
        capture(&mut commands, &mut qa, 5, frame.clone());
    }
    match qa.stage {
        30 if qa.readbacks.contains(&5) => {
            if let (Ok(entity), Ok((pose, _, _, _, _))) = (player_entities.single(), local.single())
            {
                commands
                    .entity(entity)
                    .insert(crate::player::MovementTarget {
                        target: pose.translation + Vec3::new(-3., 0., 0.),
                    });
                qa.stage = 31;
                qa.frames = 0;
            }
        }
        31 if qa.frames >= 10
            && animations
                .0
                .get(&snapshot.your_id)
                .is_some_and(|(name, _)| name == "Run") =>
        {
            capture(&mut commands, &mut qa, 6, frame);
            if let Ok(entity) = player_entities.single() {
                commands
                    .entity(entity)
                    .remove::<(crate::player::MovementTarget, crate::player::MovementRoute)>();
            }
            qa.stage = 32;
        }
        32 if qa.readbacks.contains(&6) => {
            qa.stage = 3;
            qa.frames = 0;
        }
        20 if local.single().is_ok_and(|(_, _, _, _, a)| {
            a.sequence > qa.action_before
                && a.slot
                    == if qa.weapons {
                        shared::BASIC_ATTACK_ACTION_SLOT
                    } else {
                        qa.slot
                    }
        }) =>
        {
            qa.last_cast_simulation = snapshot.sandbox.as_ref().map_or(0.0, |s| s.simulation_secs);
            qa.stage = 21;
            qa.frames = 0;
        }
        21 if qa.frames >= 2
            && snapshot.sandbox.as_ref().is_some_and(|s| {
                s.simulation_secs
                    >= qa.last_cast_simulation
                        + if qa.weapons {
                            0.18
                        } else if qa.class == HeroClass::Cinderforge && qa.slot == 3 {
                            1.75
                        } else {
                            0.06
                        }
            })
            && (qa.class != HeroClass::Cinderforge
                || qa.slot != 0
                || snapshot
                    .skill_effects
                    .iter()
                    .any(|e| e.skill == shared::loadout::SkillId::FaultLine && e.armed)) =>
        {
            let index = usize::from(qa.slot) + 1;
            capture(&mut commands, &mut qa, index, frame);
            qa.stage = 22;
        }
        22 if qa.readbacks.contains(&(usize::from(qa.slot) + 1)) => {
            if qa.slot < 3 {
                qa.slot += 1;
                qa.stage = 2;
            } else {
                if qa.class == HeroClass::Riftshot && !qa.readbacks.contains(&5) {
                    if !qa.captures.iter().any(|c| c["file"] == "06-release.png")
                        && snapshot.skill_effects.iter().any(|e| {
                            e.skill == shared::loadout::SkillId::HorizonWave
                                && e.kind == EffectVisualKind::Bolt
                        })
                    {
                        capture(&mut commands, &mut qa, 5, frame);
                    }
                    return;
                }
                let summary = serde_json::json!({"pass":true,"scenario":if qa.weapons {"handheld_pilot"} else {"roster_skills"},"class":qa.class.id(),"locale":"en","pixels":[1280,720],"manual_interaction_verified":false,"physical_device_verified":false,"setup":"live sandbox; level 10; infinite resources/cooldowns; stationary invulnerable enemy; ally-only casts target self","requests":qa.requests,"captures":qa.captures});
                if std::fs::write(
                    qa.directory.join("qa-summary.json"),
                    serde_json::to_vec_pretty(&summary).unwrap(),
                )
                .is_err()
                {
                    fail(
                        &mut qa,
                        &mut exit,
                        "Cannot write roster evidence",
                        serde_json::Value::Null,
                    );
                    return;
                }
                qa.stage = 255;
                if let Ok(window) = windows.single() {
                    commands.entity(window).despawn();
                }
                exit.write(AppExit::Success);
            }
        }
        0 if *screen.get() == AppScreen::HeroSelect && qa.frames >= 120 && !help.0 => {
            if scenes.is_empty()
                || !scenes.iter().all(|root| {
                    matches!(
                        assets.get_recursive_dependency_load_state(root.0.id()),
                        Some(bevy::asset::RecursiveDependencyLoadState::Loaded)
                    )
                })
            {
                return;
            }
            capture(&mut commands, &mut qa, 0, frame);
            qa.stage = 1;
        }
        1 if qa.readbacks.contains(&0) && session.is_connected() => {
            outgoing.write(NetworkCommand::Join {
                handheld: Default::default(),
                team: Team::Green,
                character: CharacterChoice::Cube,
                hero_class: qa.class,
                avatar: Some("agnes".into()),
                sprite_character: None,
            });
            let class_id = qa.class.id();
            qa.requests
                .push(serde_json::json!({"command":"join","class":class_id}));
            next_screen.set(AppScreen::InMatch);
            qa.stage = 2;
        }
        2 if session.join_confirmed() && local.single().is_ok() => {
            if qa.hud {
                qa.stage = 40;
                qa.frames = 0;
                return;
            }
            if qa.phases {
                // `drive_phases` owns the sandbox from here.
                qa.stage = PHASE_STAGE;
                return;
            }
            let Some(sandbox) = snapshot.sandbox.as_ref() else {
                return;
            };
            let mut config = SandboxConfig {
                player: ActorConfig {
                    hero: qa.class,
                    avatar: Some("agnes".into()),
                    level: 10,
                    no_cooldowns: qa.ux,
                    position: [-8.0, -8.0],
                    max_hp: shared::hero_balance::base_hp(qa.class),
                    ..default()
                },
                ..default()
            };
            config.dummy.enabled = true;
            config.dummy.position = [-2.0, -8.0];
            if qa.roster {
                config.player.infinite_resource = true;
                config.player.no_cooldowns = true;
                config.player.god_mode = true;
                config.enemy.enabled = true;
                config.enemy.actor.position = [-3.0, -8.0];
                config.enemy.actor.god_mode = true;
                config.enemy.actor.avatar = Some("agnes".into());
                config.dummy.enabled = false;
            }
            if qa.weapons {
                config.player.handheld = match qa.slot {
                    0 => Default::default(),
                    1 => shared::handheld::HandheldSelection::Item("forge-hammer".into()),
                    2 => shared::handheld::HandheldSelection::Item(
                        std::env::var("OMOBA_HANDHELD_QA_IMPORT")
                            .unwrap_or_else(|_| "dawn-scepter".into()),
                    ),
                    _ => shared::handheld::HandheldSelection::Unequipped,
                };
                if qa.slot >= 2 {
                    config.player.avatar = Some("orion".into());
                }
                config.enemy.actor.hero = HeroClass::Warrior;
                config.enemy.actor.position = [-8.0, -6.5];
            }
            let request = SandboxRequest {
                server_epoch: snapshot.meta.server_epoch,
                match_id: snapshot.meta.match_id,
                request_id: sandbox.last_request_id.saturating_add(1),
                command: SandboxCommand::ApplyConfig { config },
            };
            qa.requests
                .push(serde_json::json!({"command":"sandbox_setup","request":request}));
            outgoing.write(NetworkCommand::Sandbox(request));
            qa.stage = 3;
            qa.frames = 0;
        }
        3 => {
            let Ok((pose, class, progression, _, action)) = local.single() else {
                return;
            };
            if class.0 != qa.class
                || progression.level != 10
                || qa.frames < 60
                || (qa.ux && qa.started.elapsed() < Duration::from_secs(14))
                || help.0
                || !context.gameplay_allowed()
            {
                return;
            }
            if qa.weapons {
                let avatar = if qa.slot >= 2 { "orion" } else { "agnes" };
                if !rigs.iter().any(|(id, _, source, rig)| {
                    id.0 == snapshot.your_id
                        && rig.model == source.gltf.id()
                        && assets.get_path(source.gltf.id()).is_some_and(|p| {
                            p.path() == std::path::Path::new(&format!("avatars/{avatar}.glb"))
                        })
                }) {
                    return;
                }
                let expected = if qa.slot == 3 { 1 } else { 2 };
                if held.iter().count() != expected
                    || held.iter().any(|(_, _, _, scene)| {
                        !matches!(
                            assets.get_recursive_dependency_load_state(scene.0.id()),
                            Some(bevy::asset::RecursiveDependencyLoadState::Loaded)
                        )
                    })
                {
                    return;
                }
                if qa.slot == 0 && !qa.readbacks.contains(&5) {
                    capture(&mut commands, &mut qa, 5, frame);
                    qa.stage = 30;
                    return;
                }
                let Some(enemy) = snapshot.sandbox.as_ref().and_then(|s| {
                    s.actors
                        .iter()
                        .find(|a| a.actor == shared::sandbox::SandboxActor::Enemy)
                }) else {
                    return;
                };
                qa.action_before = action.sequence;
                outgoing.write(NetworkCommand::BasicAttack {
                    target: shared::wire::TargetId::player(enemy.id),
                });
                let weapon_step = qa.slot;
                qa.requests.push(serde_json::json!({"command":"basic_attack","target":enemy.id,"weapon_step":weapon_step}));
                qa.stage = 20;
                qa.frames = 0;
                return;
            }
            if qa.roster {
                if snapshot
                    .sandbox
                    .as_ref()
                    .is_none_or(|s| s.simulation_secs < qa.last_cast_simulation + 1.2)
                {
                    return;
                }
                qa.action_before = action.sequence;
                let slot = qa.slot;
                let self_target = matches!(
                    (qa.class, slot),
                    (HeroClass::Frostguard, 1) | (HeroClass::Orbitwright, 2)
                );
                let range = qa
                    .class
                    .ability(shared::SkillSlot::from_index(slot).unwrap())
                    .cast_range;
                let target_position = snapshot
                    .sandbox
                    .as_ref()
                    .and_then(|s| {
                        s.actors
                            .iter()
                            .find(|a| a.actor == shared::sandbox::SandboxActor::Enemy)
                    })
                    .map_or(pose.translation.xz() + Vec2::X * 5.0, |a| {
                        Vec2::from_array(a.position)
                    });
                let delta = target_position - pose.translation.xz();
                let aim = if self_target {
                    pose.translation.xz()
                } else if delta.length() < 0.1 {
                    pose.translation.xz() + Vec2::X
                } else {
                    pose.translation.xz() + delta.normalize() * delta.length().min(range.max(0.1))
                };
                if qa.class.is_standard() {
                    cast(
                        &mut qa,
                        &mut outgoing,
                        slot,
                        aim,
                        snapshot.meta.snapshot_tick,
                    );
                } else {
                    let target = if qa
                        .class
                        .ability(shared::SkillSlot::from_index(slot).unwrap())
                        .targeting
                        == shared::TargetingMode::SelfTarget
                    {
                        snapshot.your_id
                    } else {
                        let Some(target) = snapshot.sandbox.as_ref().and_then(|s| {
                            s.actors
                                .iter()
                                .find(|a| a.actor == shared::sandbox::SandboxActor::Enemy)
                        }) else {
                            return;
                        };
                        target.id
                    };
                    outgoing.write(NetworkCommand::Cast {
                        slot,
                        target: shared::wire::TargetId::player(target),
                    });
                    qa.requests.push(serde_json::json!({"command":"cast","slot":slot,"target":target,"snapshot_tick":snapshot.meta.snapshot_tick}));
                }
                qa.stage = 20;
                qa.frames = 0;
                return;
            }
            cast(
                &mut qa,
                &mut outgoing,
                2,
                pose.translation.xz() + Vec2::new(3.0, 0.0),
                snapshot.meta.snapshot_tick,
            );
            qa.stage = 4;
        }
        4 => {
            if !cooldowns.single().is_ok_and(|cd| cd.recovery_secs <= 0.0) {
                return;
            }
            let kind = if qa.class == HeroClass::Dawnweaver {
                EffectVisualKind::Field
            } else {
                EffectVisualKind::Trap
            };
            if !snapshot
                .skill_effects
                .iter()
                .any(|e| e.kind == kind && e.armed)
            {
                return;
            }
            let Ok((pose, _, _, _, _)) = local.single() else {
                return;
            };
            let slot = if qa.class == HeroClass::Dawnweaver {
                1
            } else {
                0
            };
            cast(
                &mut qa,
                &mut outgoing,
                slot,
                pose.translation.xz() + Vec2::new(10.0, 0.0),
                snapshot.meta.snapshot_tick,
            );
            qa.stage = 5;
        }
        5 => {
            let ready = if qa.class == HeroClass::Dawnweaver {
                snapshot
                    .skill_effects
                    .iter()
                    .any(|e| e.kind == EffectVisualKind::Barrier)
            } else {
                local
                    .single()
                    .ok()
                    .and_then(|(_, _, _, l, _)| l.0.as_ref())
                    .is_some_and(|s| s.weapon_mode == WeaponMode::Rockets)
            };
            if ready {
                capture(&mut commands, &mut qa, 1, frame);
                qa.stage = 6;
            }
        }
        6 if qa.readbacks.contains(&1) => {
            if !cooldowns.single().is_ok_and(|cd| cd.recovery_secs <= 0.0) {
                return;
            }
            let Ok((pose, _, _, _, _)) = local.single() else {
                return;
            };
            cast(
                &mut qa,
                &mut outgoing,
                3,
                pose.translation.xz() + Vec2::new(12.0, 0.0),
                snapshot.meta.snapshot_tick,
            );
            qa.stage = 7;
            qa.frames = 0;
        }
        7 => {
            let kind = if qa.class == HeroClass::Dawnweaver {
                EffectVisualKind::BeamWarning
            } else {
                EffectVisualKind::Rocket
            };
            if qa.frames >= 3
                && snapshot.skill_effects.iter().any(|e| {
                    e.kind == kind
                        && (qa.class == HeroClass::Dawnweaver
                            || vfx
                                .iter()
                                .any(|(_, _, visual)| visual.id == e.id && visual.model_ready))
                })
            {
                capture(&mut commands, &mut qa, 2, frame);
                qa.stage = 8;
            }
        }
        8 if qa.readbacks.contains(&2)
            && (qa.class != HeroClass::Dawnweaver || qa.readbacks.contains(&5))
            && context.gameplay_allowed()
            && cooldowns.single().is_ok_and(|cd| cd.recovery_secs <= 0.0) =>
        {
            qa.aim_ack = local
                .single()
                .ok()
                .and_then(|(_, _, _, l, _)| l.0.as_ref())
                .map_or(0, |l| l.cast_request_id);
            let key = if qa.ux {
                "R"
            } else if qa.class == HeroClass::Dawnweaver {
                "Q"
            } else {
                "W"
            };
            qa.requests
                .push(serde_json::json!({"input":"hold_key","key":key,"cursor":[850,300]}));
            qa.stage = 9;
            qa.frames = 0;
        }
        9 if qa.frames >= 5 && context.gameplay_allowed() => {
            capture(&mut commands, &mut qa, 3, frame);
            qa.stage = 10;
        }
        10 if qa.readbacks.contains(&3) => {
            qa.requests.push(serde_json::json!({"input":"release_key","snapshot_tick":snapshot.meta.snapshot_tick}));
            qa.stage = 11;
        }
        11 if snapshot.skill_effects.iter().any(|e| {
            (qa.ux
                && ((qa.class == HeroClass::Wildspark && e.kind == EffectVisualKind::Rocket)
                    || (qa.class == HeroClass::Dawnweaver && e.kind == EffectVisualKind::Beam)))
                || e.kind == EffectVisualKind::Bolt
                    && matches!(
                        e.skill,
                        shared::loadout::SkillId::DawnBind | shared::loadout::SkillId::WildZap
                    )
        }) && local
            .single()
            .ok()
            .and_then(|(_, _, _, l, _)| l.0.as_ref())
            .is_some_and(|l| l.cast_request_id > qa.aim_ack) =>
        {
            capture(&mut commands, &mut qa, 4, frame);
            qa.stage = 12;
        }
        12 if qa.readbacks.len()
            == if qa.class == HeroClass::Dawnweaver {
                6
            } else {
                5
            }
            && FILES
                .iter()
                .take(if qa.class == HeroClass::Dawnweaver {
                    6
                } else {
                    5
                })
                .all(|f| qa.directory.join(f).metadata().is_ok_and(|m| m.len() > 32)) =>
        {
            let summary = serde_json::json!({"pass":true,"scenario":"standard_kits","class":qa.class.id(),"locale":"en","pixels":if qa.ux {vec![1180,820]} else {vec![1280,720]},"touch_hud":qa.ux,"scripted_commands":true,"scripted_key_release_accepted":true,"manual_interaction_verified":false,"physical_device_verified":false,"setup":if qa.ux {"live sandbox; level 10; rank 1; infinite dummy; cooldowns disabled; keyboard-held aim on touch HUD"} else {"live development sandbox; level 10; rank 1; stationary infinite-health dummy; normal costs, cooldowns and time"},"requests":qa.requests,"captures":qa.captures});
            if std::fs::write(
                qa.directory.join("qa-summary.json"),
                serde_json::to_vec_pretty(&summary).unwrap(),
            )
            .is_err()
            {
                fail(
                    &mut qa,
                    &mut exit,
                    "Cannot write evidence summary",
                    serde_json::Value::Null,
                );
                return;
            }
            qa.stage = 255;
            // Match the production Exit button and the other native capture
            // harnesses: release the window's render surface before AppExit.
            if let Ok(window) = windows.single() {
                commands.entity(window).despawn();
            }
            exit.write(AppExit::Success);
        }
        40 if qa.frames > 90
            && nodes
                .iter()
                .any(|(n, c, v)| n.as_str() == "AlliedVitals" && v.get() && c.size().x > 0.0) =>
        {
            capture(&mut commands, &mut qa, 1, frame);
            qa.stage = 41;
        }
        41 if qa.readbacks.contains(&1)
            && nodes
                .iter()
                .any(|(n, c, v)| n.as_str() == "KillFeed" && v.get() && c.size().y > 0.0) =>
        {
            capture(&mut commands, &mut qa, 2, frame);
            qa.stage = 42;
        }
        42 if qa.readbacks.contains(&2) => {
            let summary = serde_json::json!({"pass":true,"scenario":"combat_ux_hud",
                "locale":"en","pixels":[1180,820],"touch_hud":true,
                "physical_device_verified":false,"manual_interaction_verified":false,
                "setup":"ordinary practice; authoritative lane bots; real hero kills; no synthetic damage or HUD state",
                "captures":qa.captures,"scoreboard":snapshot.scoreboard});
            std::fs::write(
                qa.directory.join("qa-summary.json"),
                serde_json::to_vec_pretty(&summary).unwrap(),
            )
            .unwrap();
            qa.stage = 255;
            if let Ok(window) = windows.single() {
                commands.entity(window).despawn();
            }
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
fn cast(qa: &mut Qa, outgoing: &mut MessageWriter<NetworkCommand>, slot: u8, aim: Vec2, tick: u64) {
    qa.requests.push(serde_json::json!({"command":"cast_skill","slot":slot,"aim":aim.to_array(),"snapshot_tick":tick}));
    outgoing.write(NetworkCommand::CastSkill { slot, aim });
}
const FILES: [&str; 6] = [
    "01-selection.png",
    "02-persistent-effects.png",
    "03-ultimate-flight.png",
    "04-held-aim.png",
    "05-skill-flight.png",
    "06-ray-release.png",
];
#[derive(Component)]
struct Shot {
    index: usize,
    file: String,
}
fn capture(commands: &mut Commands, qa: &mut Qa, index: usize, frame: serde_json::Value) {
    let file = if qa.weapons {
        [
            "01-selection.png",
            "02-sword-attack.png",
            "03-hammer-attack.png",
            "04-scepter-avatar-swap.png",
            "05-empty-hands.png",
            "06-idle.png",
            "07-running.png",
        ][index]
    } else if qa.roster {
        [
            "01-selection.png",
            "02-q.png",
            "03-w.png",
            "04-e.png",
            "05-r.png",
            "06-release.png",
        ][index]
    } else {
        FILES[index]
    };
    shoot(commands, qa, index, file.into(), frame);
}
/// One screenshot of the window, or of the offscreen target, per call; the
/// caller waits for its read-back before the next one.
fn shoot(
    commands: &mut Commands,
    qa: &mut Qa,
    index: usize,
    file: String,
    mut frame: serde_json::Value,
) {
    frame["file"] = file.as_str().into();
    qa.captures.push(frame);
    let screenshot = match &qa.target {
        Some(target) => Screenshot::image(target.clone()),
        None => Screenshot::primary_window(),
    };
    commands
        .spawn((
            screenshot,
            Shot {
                index,
                file: file.clone(),
            },
        ))
        .observe(save_to_disk(qa.directory.join(file)))
        .observe(readback);
}
fn readback(event: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<Qa>) {
    let Ok(shot) = shots.get(event.entity) else {
        return;
    };
    let (width, height) = qa.pixels();
    if event.image.width() != width
        || event.image.height() != height
        || qa.readbacks.contains(&shot.index)
    {
        return;
    }
    // `None` for a format the PNG writer cannot convert either.
    let rgb = event
        .image
        .clone()
        .try_into_dynamic()
        .ok()
        .map(|image| image.to_rgb8());
    let mean = rgb.as_ref().and_then(|rgb| mean_pixel(rgb.as_raw()));
    let qa = &mut *qa;
    if let Some(capture) = qa
        .captures
        .iter_mut()
        .find(|capture| capture["file"] == shot.file.as_str())
    {
        capture["mean_pixel"] = mean.into();
        // A phase still is measured against the idle still of its own framing.
        let stage = rgb
            .as_ref()
            .filter(|_| capture["phase"].is_string())
            .and_then(|rgb| stage_crop(rgb.as_raw(), rgb.width(), rgb.height()));
        if let Some(stage) = stage {
            if matches!(capture["phase"].as_str(), Some("idle" | "slot_idle")) {
                qa.idles.insert(shot.file.clone(), stage);
            } else if let Some(idle) = capture["idle_file"]
                .as_str()
                .and_then(|file| qa.idles.get(file))
            {
                capture["changed_pixels"] = changed_pixels(&stage, idle).into();
            }
        }
    }
    // A locked or hidden desktop presents nothing: such a frame is not evidence.
    if mean == Some(0.0) {
        qa.black = Some(shot.file.clone());
    } else {
        qa.readbacks.push(shot.index);
    }
}
/// Mean of the colour channels (0 to 255).
fn mean_pixel(bytes: &[u8]) -> Option<f64> {
    (!bytes.is_empty())
        .then(|| bytes.iter().map(|&value| f64::from(value)).sum::<f64>() / bytes.len() as f64)
}
/// The stage of a phase still: the staged pair without the HUD, as left, top, right and
/// bottom pixel. `scripts/build_skill_contact_sheets.py` crops its tiles to the same box.
const STAGE_CROP: [u32; 4] = [330, 55, 1130, 505];
/// A pixel has changed when one of its colour channels differs by more than this.
const CHANGED_LEVEL: u8 = 32;
/// The RGB bytes of the stage crop of a frame, or `None` when the frame does not hold it.
fn stage_crop(rgb: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let [left, top, right, bottom] = STAGE_CROP;
    if right > width || bottom > height || rgb.len() != (width * height * 3) as usize {
        return None;
    }
    Some(
        (top..bottom)
            .flat_map(|row| {
                let start = ((row * width + left) * 3) as usize;
                &rgb[start..start + ((right - left) * 3) as usize]
            })
            .copied()
            .collect(),
    )
}
/// Pixels of a stage crop that differ from the same crop of another frame: the changed
/// area the look-alike report calls the energy of a still.
fn changed_pixels(stage: &[u8], idle: &[u8]) -> usize {
    stage
        .chunks_exact(3)
        .zip(idle.chunks_exact(3))
        .filter(|(a, b)| {
            a.iter()
                .zip(b.iter())
                .any(|(a, b)| a.abs_diff(*b) > CHANGED_LEVEL)
        })
        .count()
}
fn fail(qa: &mut Qa, exit: &mut MessageWriter<AppExit>, reason: &str, detail: serde_json::Value) {
    let _=std::fs::write(qa.directory.join("qa-failure.json"),serde_json::to_vec_pretty(&serde_json::json!({"stage":qa.stage,"reason":reason,"detail":detail,"captures":qa.captures,"requests":qa.requests})).unwrap());
    error!("STANDARD_KITS_QA stage={}: {reason}", qa.stage);
    qa.stage = 255;
    exit.write(AppExit::error());
}

/// Offscreen mode: the frame loop must not depend on focus. Render settings
/// rewrite the pacing whenever they change (one update per second without
/// focus), so it is pinned every frame.
fn pace_offscreen(mut pacing: ResMut<bevy::winit::WinitSettings>) {
    let mode = bevy::winit::UpdateMode::Reactive {
        wait: Duration::from_secs_f64(1.0 / 60.0),
        react_to_device_events: false,
        react_to_user_events: false,
        react_to_window_events: false,
    };
    if pacing.focused_mode != mode || pacing.unfocused_mode != mode {
        pacing.focused_mode = mode;
        pacing.unfocused_mode = mode;
    }
}
/// Offscreen mode: the main camera, and the UI it carries, render to an image
/// that screenshots read back without a window surface.
fn retarget_main_camera(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<(Entity, &RenderTarget), With<crate::camera::MainCamera>>,
) {
    for (camera, current) in &cameras {
        if matches!(current, RenderTarget::Image(_)) {
            continue;
        }
        let (width, height) = qa.pixels();
        let target = qa
            .target
            .get_or_insert_with(|| {
                let mut image = Image::new_target_texture(
                    width,
                    height,
                    TextureFormat::Rgba8Unorm,
                    Some(TextureFormat::Rgba8UnormSrgb),
                );
                image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
                images.add(image)
            })
            .clone();
        commands.entity(camera).insert((
            RenderTarget::Image(target.into()),
            bevy::ui::IsDefaultUiCamera,
        ));
    }
}

// ---------------------------------------------------------------------------
// Phase capture (`OMOBA_STANDARD_QA_PHASES=1`)
//
// Every Q/W/E/R is cast twice on a clean slate (`ResetDuel` and its
// acknowledgement before each cast). The first cast runs at normal speed and
// only records what the server replicated (the probe). The second runs at
// 0.25x and is paused for three stills:
//   windup   the accepted edge; for a skill with a telegraph, 40 % of the
//            telegraph the probe observed;
//   release  the release window after the edge; for a skill with a telegraph,
//            the warning's kind flip or the disappearance of the fuse effect;
//   impact   a damage receipt for the slot whose burst is 0.03 to 0.2 s old,
//            or `settled` 0.6 s after the release when no receipt arrived.
// Every gate reads replicated state only; a miss is never shown as a hit.
//
// The staging of a cast follows from the catalog (`Staging`): where the target
// stands, where the cast is aimed, whether the kit's orb is parked or an allied
// wave is walked in first, and whether the staged cast can hit at all. A skill
// whose area or hit the closest view does not hold is captured from farther
// away and gets an idle still of its own to be compared with.
// ---------------------------------------------------------------------------

/// `Qa::stage` while `drive_phases` owns the run.
const PHASE_STAGE: u8 = 50;
/// Read-back index of the first phase still (the roster indexes stay below).
const PHASE_SHOT: usize = 100;
/// Whole-run limit, below the launcher's default limit for a phase run.
const PHASE_DEADLINE: Duration = Duration::from_secs(280);
/// No single step waits longer than this before the run fails with its name.
const STEP_STALL: Duration = Duration::from_secs(30);
/// The hero's spot on the mid lane and the lane direction its target stands in.
const STAGE_HOME: Vec2 = Vec2::new(-5.0, -5.0);
/// How far a displace run moves the hero across its aim during a telegraph: far enough to
/// tell the telegraph from the place of the cast, near enough for the target to stay in
/// the cone.
const DISPLACE_UNITS: f32 = 2.5;
const STAGE_LANE: Vec2 = Vec2::new(
    std::f32::consts::FRAC_1_SQRT_2,
    std::f32::consts::FRAC_1_SQRT_2,
);
/// Target distance of pick and melee skills.
const CLOSE_RANGE: f32 = 2.5;
/// Farthest target the close zoom still frames next to the hero.
const LANE_LIMIT: f32 = 7.0;
/// Time scale of the cast that is captured.
const SLOW_MOTION: f32 = 0.25;
/// Simulated seconds the probe cast is observed for.
const PROBE_SECS: f64 = 2.5;
/// An effect first seen this soon after the edge, as a warning or unarmed, is
/// the cast's telegraph.
const TELEGRAPH_GRACE: f64 = 0.25;
/// Share of the telegraph the probe observed that has passed at the windup still.
const TELEGRAPH_SHARE: f64 = 0.4;
/// Release window of a skill without a telegraph, in simulated seconds after
/// the accepted edge. A row that keeps the contact rule shows its contact pose
/// inside it; `ReleaseAt` moves the window for a look at a clip played from
/// its first key.
const RELEASE_WINDOW: (f64, f64) = (0.10, 0.20);

/// Where the release window of a skill without a telegraph starts.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ReleaseAt {
    /// `RELEASE_WINDOW`.
    Window,
    /// The library contact time of the profile's release clip.
    Contact,
    /// A fixed time after the edge.
    Secs(f64),
}
impl ReleaseAt {
    /// The window for a release clip whose library contact is `contact`.
    fn window(self, contact: Option<f32>) -> (f64, f64) {
        let start = match self {
            Self::Window => return RELEASE_WINDOW,
            Self::Contact => contact.map_or(RELEASE_WINDOW.0, f64::from),
            Self::Secs(secs) => secs,
        };
        (start, start + RELEASE_WINDOW.1 - RELEASE_WINDOW.0)
    }
}
/// Age window of the impact burst in the impact still.
const IMPACT_AGE: (f32, f32) = (0.03, 0.2);
/// Simulated seconds between the release still and a `settled` still.
const SETTLED_AFTER_RELEASE: f64 = 0.6;
/// Extra simulated seconds the capture cast waits for the receipt the probe saw.
const RECEIPT_GRACE: f64 = 1.0;
/// Share of the way to the target the basic attack's projectile has gone when the pause
/// for its still is requested.
const FLIGHT_SHARE: f64 = 0.3;
/// How far along the lane the orb of a kit is parked: for its aim stills, so that a
/// preview drawn from the orb is told apart from one drawn from the hero, and before a
/// cast that calls it home, so that its way crosses the target.
const ORB_PARK: f32 = 5.0;
/// The accepted edge is this fresh, in simulated seconds, in the windup still of a skill
/// without a telegraph.
const EDGE_WINDOW: f64 = 0.06;
/// How often the cast of one skill is staged again because a stalled frame made its
/// windup still late.
const RETAKES: u8 = 2;
/// Distance of the free point a leap to open ground is aimed at.
const FREE_POINT: f32 = 6.0;
/// Time scale while an allied wave walks from its base to the stage.
const MARCH_SCALE: f32 = 2.0;
/// The wave is stopped when its first minion is this near the hero: out of the reach of
/// the casts staged there and too far from the target to start a fight.
const ALLY_STOP: f32 = 5.5;
/// Reach of the burst of a hit around its receipt (the bound of a hit without area).
const HIT_REACH: f32 = 1.5;
/// Share of the stage crop, from the hero to its edge, that what a skill must show may
/// take; the rest is room for the width of what is drawn there.
const FRAME_FILL: f32 = 0.92;
/// Frames the camera has to rest before a framing is measured.
const FRAME_REST: u32 = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Pass {
    #[default]
    Probe,
    Capture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Windup,
    Release,
    Impact,
    Settled,
    /// The projectile of the basic attack in flight (`OMOBA_STANDARD_QA_FLIGHT`).
    Flight,
    /// The aim preview with the key held before the cast (`OMOBA_STANDARD_QA_AIM`).
    Aim,
    /// The aim preview with the key held while the slot offers a recast.
    RecastAim,
    /// The clean slate of one slot whose stills are taken from farther away.
    SlotIdle,
}
impl Phase {
    fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::SlotIdle => "slot_idle",
            Self::Windup => "windup",
            Self::Release => "release",
            Self::Impact => "impact",
            Self::Settled => "settled",
            Self::Flight => "flight",
            Self::Aim => "aim",
            Self::RecastAim => "recast_aim",
        }
    }
    /// Position of the still in the skill's row: the three stills of the cast, the aim
    /// before them and the recast aim after them.
    fn order(self) -> u8 {
        match self {
            Self::Idle | Self::SlotIdle | Self::Flight | Self::Aim => 0,
            Self::Windup => 1,
            Self::Release => 2,
            Self::Impact | Self::Settled => 3,
            Self::RecastAim => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Step {
    /// Place the target for this slot at normal speed.
    #[default]
    Arrange,
    /// Placement acknowledged: reset the duel.
    Reset,
    /// Reset acknowledged: order an allied wave for a cast that needs an ally.
    Wave,
    /// The wave marches: wait for it to reach the hero, then stop it.
    March,
    /// Reset acknowledged: wait until nothing of an earlier cast is left.
    Settle,
    /// Capture pass: wait for the camera to rest and widen the view if the skill needs it.
    Frame,
    /// Capture pass: slow motion requested.
    Slow,
    /// Send the cast.
    Cast,
    /// Wait for the accepted edge.
    Edge,
    /// Probe pass: record what the server replicates.
    Probe,
    /// Capture pass: wait for the gate of the next still.
    Gates,
    /// Basic attack sent: wait for its projectile to be in flight.
    Flight,
    /// The switch of a repeater was cast: wait for the replicated rocket mode.
    Mode,
    /// The orb was ordered away: wait for it to arrive, then take the aim still or cast.
    Park(Parked),
    /// The skill key is held: wait for its aim preview.
    Aim(Phase),
    /// Recast offered outside its gate: wait for the hero to be in reach.
    Reach,
    /// Pause requested for a still.
    Pause(Phase),
    /// Screenshot taken: wait for its read-back.
    Read(Phase),
    /// Resume requested after a still.
    Resume,
}

/// What a parked orb is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Parked {
    Aim,
    Cast,
}

/// One sandbox request at a time, resent until the server acknowledges it.
#[derive(Default)]
struct Wire {
    pending: Option<(SandboxRequest, Instant)>,
    last_id: u64,
}
impl Wire {
    fn send(
        &mut self,
        game: &GameStateSnapshot,
        sandbox: &SandboxSnapshot,
        command: SandboxCommand,
        outgoing: &mut MessageWriter<NetworkCommand>,
    ) {
        let request = SandboxRequest {
            server_epoch: game.meta.server_epoch,
            match_id: game.meta.match_id,
            request_id: self.last_id.max(sandbox.last_request_id).saturating_add(1),
            command,
        };
        self.last_id = request.request_id;
        outgoing.write(NetworkCommand::Sandbox(request.clone()));
        self.pending = Some((request, Instant::now()));
    }
    /// `None` while the acknowledgement is outstanding, then its verdict.
    fn poll(
        &mut self,
        sandbox: &SandboxSnapshot,
        outgoing: &mut MessageWriter<NetworkCommand>,
    ) -> Option<Result<(), String>> {
        let Some((request, sent)) = &mut self.pending else {
            return Some(Ok(()));
        };
        if let Some(ack) = sandbox
            .ack
            .as_ref()
            .filter(|ack| ack.request_id == request.request_id)
        {
            let verdict = if ack.accepted {
                Ok(())
            } else {
                Err(ack.message.clone())
            };
            self.pending = None;
            return Some(verdict);
        }
        // The server repeats the first acknowledgement for a duplicate.
        if sent.elapsed() >= Duration::from_millis(300) {
            outgoing.write(NetworkCommand::Sandbox(request.clone()));
            *sent = Instant::now();
        }
        None
    }
}

/// Where the scripted cast of a skill is aimed.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Aim {
    /// At the target, bounded by the cast range.
    Target,
    /// At the caster: the sandbox has no allied hero for a cast that picks one.
    Caster,
    /// At a free point this far up the lane, with no ally in pick reach of it.
    Ahead(f32),
    /// At the allied minion that was walked in behind the hero.
    Ally,
}
impl Aim {
    fn name(self) -> &'static str {
        match self {
            Self::Target => "target",
            Self::Caster => "caster",
            Self::Ahead(_) => "free_point",
            Self::Ally => "allied_minion",
        }
    }
}

/// Where the target stands for one skill and how its cast is staged, derived from the
/// catalog.
#[derive(Clone, Copy)]
struct Staging {
    category: &'static str,
    distance: f32,
    aim: Aim,
    /// The kit's orb is parked beyond the target before the cast, so that its way home
    /// crosses the target.
    park_orb: bool,
    /// Why the staged cast yields no damage receipt; `None` when it has to hit.
    settles: Option<&'static str>,
    /// Radius of the area the cast strikes around its caster; the stills show all of it.
    area: f32,
}
impl Staging {
    /// The basic attack: its target stands inside the attack range, as far as the frame
    /// allows, so that the projectile is seen on its way.
    fn basic(class: HeroClass) -> Self {
        Self {
            category: "basic",
            distance: (shared::basic_attack_for_class(class).range * 0.9)
                .clamp(CLOSE_RANGE, LANE_LIMIT),
            aim: Aim::Target,
            park_orb: false,
            settles: None,
            area: 0.0,
        }
    }

    /// `flight` stands the target of every unit-target ability on the lane.
    fn of(
        class: HeroClass,
        equipped: &shared::loadout::EquippedSkills,
        slot: SkillSlot,
        flight: bool,
    ) -> Self {
        use crate::skill_presentation::evidence;
        let ability = equipped.ability(slot);
        let definition = equipped.skill(slot);
        let effect = definition.map(|skill| skill.effect);
        let technique = match effect {
            Some(SkillEffect::Technique { action, .. }) => Some(action),
            _ => None,
        };
        let radius = match effect {
            Some(SkillEffect::Technique { radius, .. }) => radius,
            _ => 0.0,
        };
        let lane = (ability.cast_range * 0.6).clamp(CLOSE_RANGE, LANE_LIMIT);
        let (category, distance) = match ability.targeting {
            // A charge carries the hero (and the camera) the whole range and
            // strikes around the landing: the target stands inside that area.
            TargetingMode::Direction if technique == Some(Technique::CollisionCharge) => {
                ("charge", ability.cast_range - radius * 0.6)
            }
            TargetingMode::Direction => ("skillshot", lane),
            TargetingMode::Point => ("pick_or_melee", CLOSE_RANGE.min(ability.cast_range)),
            // A legacy kit is melee when its basic attack is.
            TargetingMode::UnitTarget
                if !flight && shared::basic_attack_for_class(class).range <= 5.0 =>
            {
                ("pick_or_melee", CLOSE_RANGE)
            }
            TargetingMode::UnitTarget => ("skillshot", lane),
            TargetingMode::SelfTarget if radius > CLOSE_RANGE => {
                ("self_area", CLOSE_RANGE.min(radius * 0.6))
            }
            TargetingMode::SelfTarget => ("self", CLOSE_RANGE),
        };
        let aim = match technique {
            // Only an allied hero can be guarded, and the sandbox has none but the caster.
            Some(Technique::BallGuard) => Aim::Caster,
            Some(Technique::AllyLeap) => Aim::Ally,
            // A leap to open ground shows the leap; on the caster it would stand still.
            Some(Technique::GuardLeap) => Aim::Ahead(FREE_POINT.min(ability.cast_range)),
            _ => Aim::Target,
        };
        // A cast that can deal damage still yields no receipt when its damage waits for
        // something the stage does not provide.
        let settles = if !evidence::can_damage(ability.id) {
            Some("no_damage")
        } else {
            match effect {
                Some(SkillEffect::RecastZone { .. }) => Some("damage_on_recast_or_expiry"),
                Some(SkillEffect::Technique {
                    action: Technique::SegmentCage,
                    ..
                }) => Some("no_side_touched"),
                Some(SkillEffect::Technique {
                    action: Technique::DetonationMark,
                    ..
                }) => Some("mark_not_detonated"),
                _ => None,
            }
        };
        Self {
            category,
            distance,
            aim,
            park_orb: technique == Some(Technique::BallGuard),
            settles,
            area: definition
                .and_then(|skill| evidence::cast_area_radius(skill.id))
                .unwrap_or(0.0),
        }
    }

    /// What the stills of the cast have to show, as offsets from the hero: the area of
    /// the cast and the hit the probe cast observed, with the reach of its burst.
    fn keep(&self, hit: Option<Vec2>) -> Vec<Vec2> {
        let ring = |centre: Vec2, radius: f32, points: u8| {
            (0..points).map(move |i| {
                centre + Vec2::from_angle(TAU * f32::from(i) / f32::from(points)) * radius
            })
        };
        let mut keep = Vec::new();
        if self.area > 0.0 {
            keep.extend(ring(Vec2::ZERO, self.area, 16));
        }
        if let Some(hit) = hit {
            keep.extend(ring(hit, HIT_REACH, 8));
        }
        keep
    }
}

#[derive(Clone)]
struct Track {
    seen: f64,
    kind: EffectVisualKind,
    armed: bool,
    remaining: f32,
    telegraph: bool,
    flipped: Option<f64>,
    gone: Option<f64>,
}
#[derive(Clone)]
struct Receipt {
    id: u64,
    seen: f64,
    amount: f32,
    target: u64,
    /// Where the hit landed, from the hero as it stood when the receipt arrived.
    offset: Option<Vec2>,
}
/// What the server replicated for one cast, from the accepted edge on.
#[derive(Clone, Default)]
struct CastWatch {
    slot: u8,
    skill: Option<SkillId>,
    action_before: u64,
    event_floor: u64,
    /// Effects that were alive before the cast.
    known: HashSet<u64>,
    /// Simulation time and action sequence of the accepted edge.
    edge: Option<(f64, u64)>,
    tracks: BTreeMap<u64, Track>,
    receipt: Option<Receipt>,
}
impl CastWatch {
    fn observe(
        &mut self,
        game: &GameStateSnapshot,
        action: &crate::net::PlayerCosmeticAction,
        now: f64,
    ) {
        let hero = game.sandbox.as_ref().and_then(|sandbox| {
            sandbox
                .actors
                .iter()
                .find(|actor| actor.actor == SandboxActor::Player)
                .map(|actor| Vec2::from_array(actor.position))
        });
        if self.edge.is_none() && action.sequence > self.action_before && action.slot == self.slot {
            self.edge = Some((now, action.sequence));
        }
        let Some((edge, _)) = self.edge else {
            return;
        };
        if let Some(skill) = self.skill {
            let mut live = HashSet::new();
            for effect in game.skill_effects.iter().filter(|effect| {
                effect.owner_id == game.your_id
                    && effect.skill == skill
                    && !self.known.contains(&effect.id)
            }) {
                live.insert(effect.id);
                let track = self.tracks.entry(effect.id).or_insert_with(|| Track {
                    seen: now,
                    kind: effect.kind,
                    armed: effect.armed,
                    remaining: effect.remaining_secs,
                    telegraph: now - edge <= TELEGRAPH_GRACE
                        && (effect.kind == EffectVisualKind::BeamWarning || !effect.armed),
                    flipped: None,
                    gone: None,
                });
                if track.kind == EffectVisualKind::BeamWarning
                    && effect.kind != EffectVisualKind::BeamWarning
                    && track.flipped.is_none()
                {
                    track.flipped = Some(now);
                }
            }
            for (id, track) in &mut self.tracks {
                if track.gone.is_none() && !live.contains(id) {
                    track.gone = Some(now);
                }
            }
        }
        if self.receipt.is_none() {
            self.receipt = game
                .combat_events
                .iter()
                .filter(|event| {
                    event.id > self.event_floor
                        && event.source.kind == shared::combat::CombatEntityKind::Player
                        && event.source.id == game.your_id
                        && event.action_slot == Some(self.slot)
                        && event.amount.is_finite()
                        && event.amount > 0.0
                })
                .min_by_key(|event| event.id)
                .map(|event| Receipt {
                    id: event.id,
                    seen: now,
                    amount: event.amount,
                    target: event.target.id,
                    offset: hero.map(|hero| Vec2::new(event.x, event.z) - hero),
                });
        }
    }
    /// The first telegraph of the cast: when it was first seen.
    fn telegraph(&self) -> Option<f64> {
        self.tracks
            .values()
            .filter(|track| track.telegraph)
            .map(|track| track.seen)
            .min_by(f64::total_cmp)
    }
    /// The first telegraph that fired: first seen, fired, and how it showed.
    fn fuse(&self) -> Option<(f64, f64, &'static str)> {
        self.tracks
            .values()
            .filter(|track| track.telegraph)
            .filter_map(|track| match (track.flipped, track.gone) {
                (Some(flipped), _) => Some((track.seen, flipped, "kind_flip")),
                (None, Some(gone)) => Some((track.seen, gone, "fuse_gone")),
                (None, None) => None,
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
    fn timeline(&self) -> serde_json::Value {
        let edge = self.edge.map_or(0.0, |(at, _)| at);
        let after = |at: f64| ((at - edge) * 1000.0).round() / 1000.0;
        serde_json::json!({
            "effects": self.tracks.iter().map(|(id, track)| serde_json::json!({
                "id": id,
                "kind": track.kind,
                "armed": track.armed,
                "remaining_secs": track.remaining,
                "telegraph": track.telegraph,
                "seen_after_edge_secs": after(track.seen),
                "kind_flip_after_edge_secs": track.flipped.map(after),
                "gone_after_edge_secs": track.gone.map(after),
            })).collect::<Vec<_>>(),
            "release": self.fuse().map(|(seen, fired, how)| serde_json::json!({
                "how": how,
                "after_edge_secs": after(fired),
                "telegraph_secs": after(fired) - after(seen),
            })),
            "receipt": self.receipt.as_ref().map(|receipt| serde_json::json!({
                "id": receipt.id,
                "amount": receipt.amount,
                "target": receipt.target,
                "after_edge_secs": after(receipt.seen),
                "from_hero": receipt.offset.map(Vec2::length),
            })),
        })
    }
}

/// The most particles and effect parts that were alive at once while a cast was captured.
#[derive(Clone, Copy, Default)]
struct Peaks {
    /// Accent particles of the cast sequence.
    accent: usize,
    /// Impact particles of one receipt.
    impact: usize,
    live_particles: usize,
    visible_parts: usize,
    lights: usize,
}
impl Peaks {
    fn record(&self) -> serde_json::Value {
        serde_json::json!({
            "accent_of_cast": self.accent,
            "impact_of_one_receipt": self.impact,
            "live_particles": self.live_particles,
            "effect_visible_parts": self.visible_parts,
            "effect_lights": self.lights,
        })
    }
}

#[derive(Resource, Default)]
struct Phases {
    step: Step,
    pass: Pass,
    slot: u8,
    entered: Option<Instant>,
    frames: u32,
    wire: Wire,
    config: Option<SandboxConfig>,
    staging: Option<Staging>,
    watch: CastWatch,
    /// The finished probe of the current slot.
    probe: Option<CastWatch>,
    idle_taken: bool,
    windup_taken: bool,
    /// Simulation time of the release still.
    release_taken: Option<f64>,
    third_taken: bool,
    /// Why the still that is being taken was due.
    gate: &'static str,
    /// Paused simulation time and the frames it has not moved.
    frozen: (f64, u32),
    stills: usize,
    skill_stills: Vec<String>,
    skills: Vec<serde_json::Value>,
    /// The basic attack is being captured, after the four skills.
    basic: bool,
    /// The basic attack is being captured a second time, as the rocket of a repeater.
    rockets: bool,
    basic_record: Option<serde_json::Value>,
    aim_taken: bool,
    recast_aim_taken: bool,
    /// The orb was ordered away for the aim still of this slot.
    parked: bool,
    /// The orb was ordered away for the cast of this pass.
    orb_out: bool,
    /// The view of this pass is settled and holds what the skill must show.
    framed: bool,
    /// The idle still of this slot's own view was taken.
    slot_idle_taken: bool,
    /// The idle still the stills of this slot are compared with.
    idle_file: String,
    /// What the stills of this slot have to show, as offsets from the hero.
    keep: Vec<Vec2>,
    /// Pixels per unit of the lane at the hero, and the frames it has not changed.
    span: (f32, u32),
    peaks: Peaks,
    /// Casts of this slot that were staged again after a late windup still.
    retakes: u8,
    /// Why a slot that offered a recast has no still of its aim.
    recast_aim_note: Option<&'static str>,
    aim_stills: Vec<String>,
    /// The key `hold_aim` keeps down and where it points.
    hold: Option<Hold>,
    held: Option<KeyCode>,
    /// The window's cursor before `hold_aim` moved it this frame.
    cursor_before: Option<Option<Vec2>>,
}

/// The skill key of an aim still, the point of the world it aims at and the pixel of
/// that point.
#[derive(Clone, Copy)]
struct Hold {
    slot: u8,
    aim: Vec2,
    cursor: Vec2,
}
impl Phases {
    /// The action slot of the cast that is being captured.
    fn action_slot(&self) -> u8 {
        if self.basic {
            shared::BASIC_ATTACK_ACTION_SLOT
        } else {
            self.slot
        }
    }
    fn enter(&mut self, step: Step) {
        self.step = step;
        self.entered = Some(Instant::now());
        self.frames = 0;
    }
    fn begin_pass(&mut self, pass: Pass) {
        if pass == Pass::Probe {
            self.retakes = 0;
        }
        self.pass = pass;
        self.windup_taken = false;
        self.release_taken = None;
        self.third_taken = false;
        self.aim_taken = false;
        self.recast_aim_taken = false;
        self.parked = false;
        self.orb_out = false;
        self.framed = false;
        self.slot_idle_taken = false;
        self.idle_file = "0-idle.png".into();
        self.keep.clear();
        self.span = (0.0, 0);
        self.peaks = Peaks::default();
        self.recast_aim_note = None;
        self.enter(Step::Arrange);
    }
}

#[derive(SystemParam)]
struct PhaseWorld<'w, 's> {
    game: Res<'w, GameStateSnapshot>,
    mode: Res<'w, crate::sprite::PlayerVisualMode>,
    registry: Res<'w, crate::skill_presentation::SkillPresentation>,
    audio: Res<'w, crate::game_audio::GameAudioDiagnostics>,
    aim: Res<'w, crate::combat::aim_preview::AimPreviewShown>,
    origin: Res<'w, crate::skill_presentation::SkillPresentationOrigin>,
    flights: Res<'w, crate::combat_visuals::CombatVisualRegistry>,
    receipts: Res<'w, crate::combat_feedback::ReceiptLooks>,
    animations: Res<'w, crate::sandbox::AnimationReadout>,
    context: Res<'w, crate::input_context::GameplayInputContext>,
    map: Res<'w, crate::maps::MapLayout>,
    zoom: Res<'w, crate::camera::CameraState>,
    local: Query<
        'w,
        's,
        (
            &'static NetworkHeroClass,
            &'static PlayerProgression,
            &'static PlayerLoadout,
            &'static crate::net::PlayerCosmeticAction,
            Option<&'static crate::net::PlayerSkillCooldowns>,
        ),
        With<Player>,
    >,
    cameras:
        Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<crate::camera::MainCamera>>,
    particles: Query<'w, 's, &'static crate::game_vfx::ParticleSlot>,
    numbers: Query<'w, 's, (), With<crate::combat_feedback::DamageNumber>>,
    vfx: Query<
        'w,
        's,
        (
            Entity,
            &'static Name,
            &'static Visibility,
            &'static crate::skill_presentation::SkillEffectVisual,
            Option<&'static crate::skill_presentation::SkillBodyVisual>,
        ),
    >,
    parts: Query<
        'w,
        's,
        (
            &'static ChildOf,
            &'static Visibility,
            Has<PointLight>,
            Has<SpotLight>,
        ),
    >,
    projectiles: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static crate::net::NetworkProjectile,
            Option<&'static crate::projectile_visuals::ProjectileBodyVisual>,
        ),
    >,
    children: Query<'w, 's, &'static Children>,
    drawn: Query<'w, 's, (&'static InheritedVisibility, Has<Mesh3d>)>,
    hero: Query<'w, 's, Entity, With<Player>>,
    clips: crate::player::HeroClips<'w, 's>,
    states: Query<
        'w,
        's,
        (
            Entity,
            &'static crate::skill_presentation::status::StateVisualShown,
        ),
    >,
    heroes: Query<
        'w,
        's,
        (
            &'static crate::net::NetworkPlayerId,
            &'static Transform,
            &'static NetworkHeroClass,
            &'static PlayerLoadout,
        ),
    >,
    team: Query<'w, 's, &'static Team, With<Player>>,
    minions: Query<'w, 's, (&'static Transform, &'static Team), With<crate::net::NetworkMinion>>,
}
impl PhaseWorld<'_, '_> {
    /// The mesh visual of each hero's state, with the parts that are drawn.
    fn state_visuals(&self) -> Vec<serde_json::Value> {
        self.states
            .iter()
            .map(|(root, shown)| {
                serde_json::json!({
                    "hero": shown.hero,
                    "state": shown.state.id(),
                    "parts": shown.parts,
                    "visible_parts": self.parts.iter()
                        .filter(|(parent, visibility, ..)| {
                            parent.parent() == root && **visibility != Visibility::Hidden
                        })
                        .count(),
                })
            })
            .collect()
    }
    /// What each hero's replicated flags report: its states, highest rank first, and the
    /// recast markers of the slots the server offers it.
    fn hero_states(&self) -> Vec<serde_json::Value> {
        use crate::skill_presentation::status::{self, StateVisual};
        self.heroes
            .iter()
            .filter_map(|(id, pose, class, loadout)| {
                let flags = loadout.0.as_ref()?;
                Some(serde_json::json!({
                    "hero": id.0,
                    "states": StateVisual::of(flags).map(StateVisual::id).collect::<Vec<_>>(),
                    // What the speed read of the particle layer compares.
                    "movement_multiplier": flags.movement_multiplier,
                    "slow_multiplier": flags.slow_multiplier,
                    "recast_markers": status::recast_markers(
                        &self.registry,
                        class.0,
                        flags,
                        id.0,
                        pose.translation.xz(),
                        &self.game.skill_effects,
                    )
                    .into_iter()
                    .map(|(marker, _)| marker.id())
                    .collect::<Vec<_>>(),
                }))
            })
            .collect()
    }
    /// The clip of the hero's animation state: its seek time, its speed and whether it
    /// repeats. Sprites have none.
    fn clip(&self) -> Option<serde_json::Value> {
        let (seek_secs, speed, repeats) = self.clips.of(self.hero.single().ok()?)?;
        Some(serde_json::json!({ "seek_secs": seek_secs, "speed": speed, "repeats": repeats }))
    }
    /// The hero's replicated projectiles and what stands for each of them.
    fn own_projectiles(&self) -> Vec<serde_json::Value> {
        use crate::combat_visuals::FlightBody;
        let hero = self
            .actor(SandboxActor::Player)
            .map(|actor| Vec2::from_array(actor.position));
        self.projectiles
            .iter()
            .filter(|(_, _, projectile, _)| {
                projectile.source_kind == shared::combat::CombatEntityKind::Player
                    && projectile.owner_id == self.game.your_id
            })
            .map(|(entity, pose, projectile, body)| {
                serde_json::json!({
                    "id": projectile.id,
                    "style": projectile.style,
                    "action_slot": projectile.action_slot,
                    "position": pose.translation.to_array(),
                    "from_hero": hero.map(|hero| pose.translation.xz().distance(hero)),
                    // Absent in Sprite2d: the flat backend draws its own shapes.
                    "profile": body.map(|body| body.profile.as_str()),
                    "body": body.map(|body| match body.body {
                        FlightBody::Shape => "shape".to_string(),
                        FlightBody::Form(form, mesh) => format!("{}+{}", form.id(), mesh.id()),
                        FlightBody::Reach => "reach_streak".to_string(),
                    }),
                    "form_parts": body.map(|body| body.parts),
                    "visible_meshes": self.children.iter_descendants(entity)
                        .filter(|part| {
                            self.drawn.get(*part).is_ok_and(|(shown, mesh)| mesh && shown.get())
                        })
                        .count(),
                })
            })
            .collect()
    }
    fn actor(&self, actor: SandboxActor) -> Option<&shared::sandbox::ActorTelemetry> {
        self.game
            .sandbox
            .as_ref()?
            .actors
            .iter()
            .find(|telemetry| telemetry.actor == actor)
    }
    /// The hero's replicated orb.
    fn orb(&self) -> Option<Vec2> {
        let (_, _, loadout, ..) = self.local.single().ok()?;
        loadout.0.as_ref()?.orb_position.map(Vec2::from_array)
    }
    /// Whether the hero is back at rest after a cast that left no effect of its own:
    /// nothing is in flight or on screen and the next action is free.
    fn at_rest(&self) -> bool {
        self.live_particles().is_empty()
            && self.numbers.is_empty()
            && (*self.mode != crate::sprite::PlayerVisualMode::Models3d
                || self.animation().is_none_or(|label| label == "Idle"))
            && self.context.gameplay_allowed()
            && self
                .local
                .single()
                .ok()
                .and_then(|(_, _, _, _, cooldowns)| cooldowns)
                .is_none_or(|cooldowns| cooldowns.recovery_secs <= 0.0)
    }
    /// The pixel a cursor has to stand on to aim at a point of the ground, as the game
    /// maps its cursor to the world.
    fn cursor(&self, aim: Vec2) -> Option<Vec2> {
        let (camera, pose) = self.cameras.single().ok()?;
        let point = Vec3::new(aim.x, 0.0, aim.y);
        let point = if *self.mode == crate::sprite::PlayerVisualMode::Sprite2d {
            crate::world2d::simulation_xz_to_render_xy(point).extend(0.0)
        } else {
            point
        };
        camera.world_to_viewport(pose, point).ok()
    }
    /// The key, the aim point and the cursor of an aim still: the aim of the cast.
    fn hold(
        &self,
        equipped: &shared::loadout::EquippedSkills,
        slot: u8,
        aim: Aim,
    ) -> Result<Hold, &'static str> {
        let aim = self.cast_aim(equipped, slot, aim)?;
        let cursor = self.cursor(aim).ok_or("the aim point is not in view")?;
        Ok(Hold { slot, aim, cursor })
    }
    /// Where the scripted cast of a modular skill is aimed.
    fn cast_aim(
        &self,
        equipped: &shared::loadout::EquippedSkills,
        slot: u8,
        aim: Aim,
    ) -> Result<Vec2, &'static str> {
        let hero = self
            .actor(SandboxActor::Player)
            .ok_or("no hero telemetry")?;
        let origin = Vec2::from_array(hero.position);
        Ok(match aim {
            Aim::Caster => origin,
            Aim::Ahead(distance) => origin + STAGE_LANE * distance,
            Aim::Ally => self.ally(origin).ok_or("no allied minion is staged")?,
            Aim::Target => {
                let enemy = self
                    .actor(SandboxActor::Enemy)
                    .ok_or("no target telemetry")?;
                let ability = equipped.ability(SkillSlot::from_index(slot).ok_or("no such slot")?);
                let delta = Vec2::from_array(enemy.position) - origin;
                origin
                    + delta.normalize_or(STAGE_LANE)
                        * delta.length().min(ability.cast_range.max(0.1))
            }
        })
    }
    /// The allied minion nearest to a point of the ground.
    fn ally(&self, near: Vec2) -> Option<Vec2> {
        let team = self.team.single().ok()?;
        self.minions
            .iter()
            .filter(|(_, other)| *other == team)
            .map(|(pose, _)| pose.translation.xz())
            .min_by(|a, b| a.distance(near).total_cmp(&b.distance(near)))
    }
    /// The pixel of a point of the ground.
    fn ground_pixel(&self, at: Vec2) -> Option<Vec2> {
        let (camera, pose) = self.cameras.single().ok()?;
        let point = Vec3::new(at.x, self.map.terrain_height_3d(at.x, at.y), at.y);
        let point = if *self.mode == crate::sprite::PlayerVisualMode::Sprite2d {
            crate::world2d::simulation_xz_to_render_xy(point).extend(0.0)
        } else {
            point
        };
        camera.world_to_viewport(pose, point).ok()
    }
    /// Pixels one unit of the lane takes at the hero: it changes while the camera moves.
    fn lane_span(&self) -> Option<f32> {
        let hero = Vec2::from_array(self.actor(SandboxActor::Player)?.position);
        Some(
            self.ground_pixel(hero)?
                .distance(self.ground_pixel(hero + STAGE_LANE)?),
        )
    }
    /// How much of the way from the hero to the edge of the stage crop the farthest of
    /// these offsets takes: at most 1 when the crop holds them all.
    fn frame_fit(&self, offsets: &[Vec2]) -> Option<f32> {
        let hero = Vec2::from_array(self.actor(SandboxActor::Player)?.position);
        let centre = self.ground_pixel(hero)?;
        let [left, top, right, bottom] = STAGE_CROP.map(|edge| edge as f32);
        let share = |delta: f32, low: f32, high: f32, centre: f32| {
            if delta >= 0.0 {
                delta / (high - centre)
            } else {
                delta / (low - centre)
            }
        };
        offsets.iter().try_fold(0.0_f32, |most, offset| {
            let delta = self.ground_pixel(hero + *offset)? - centre;
            Some(
                most.max(share(delta.x, left, right, centre.x))
                    .max(share(delta.y, top, bottom, centre.y)),
            )
        })
    }
    /// Live particles and drawn effect parts of this frame, against the cast that is
    /// being captured.
    fn load(&self, cast: Option<u64>) -> Peaks {
        use crate::game_vfx::ParticleSource as Source;
        let mut impacts: HashMap<u64, usize> = HashMap::new();
        let mut peaks = Peaks::default();
        for slot in &self.particles {
            let (Some((id, _)), Some((_, source))) = (slot.sample(), slot.source()) else {
                continue;
            };
            peaks.live_particles += 1;
            match source {
                Source::Accent if Some(id) == cast => peaks.accent += 1,
                Source::Impact => *impacts.entry(id).or_default() += 1,
                _ => {}
            }
        }
        peaks.impact = impacts.into_values().max().unwrap_or(0);
        for (root, _, visibility, ..) in &self.vfx {
            for (parent, part, point, spot) in &self.parts {
                if parent.parent() != root {
                    continue;
                }
                if point || spot {
                    peaks.lights += 1;
                } else if *visibility != Visibility::Hidden && *part != Visibility::Hidden {
                    peaks.visible_parts += 1;
                }
            }
        }
        peaks
    }
    /// The active registry: where it came from, the fingerprint of the packaged file and
    /// the number of its skill rows.
    fn registry_record(&self) -> serde_json::Value {
        use crate::skill_presentation::SkillPresentationOrigin as Origin;
        let (origin, fnv64) = match *self.origin {
            Origin::Embedded => ("embedded", None),
            Origin::Packaged { fnv64 } => ("packaged", Some(format!("{fnv64:016x}"))),
        };
        serde_json::json!({
            "origin": origin,
            "fnv64": fnv64,
            "profiles": self.registry.profile_count(),
        })
    }
    /// Chest height of a simulation position, in pixels of the capture.
    fn pixels(&self, position: [f32; 2]) -> Option<[f32; 2]> {
        let (camera, pose) = self.cameras.single().ok()?;
        let ground = self.map.terrain_height_3d(position[0], position[1]);
        let point = Vec3::new(position[0], ground + 1.0, position[1]);
        let point = if *self.mode == crate::sprite::PlayerVisualMode::Sprite2d {
            crate::world2d::simulation_xz_to_render_xy(point).extend(0.0)
        } else {
            point
        };
        camera
            .world_to_viewport(pose, point)
            .ok()
            .map(|pixel| [pixel.x.round(), pixel.y.round()])
    }
    fn live_particles(&self) -> Vec<(u64, f32)> {
        self.particles
            .iter()
            .filter_map(crate::game_vfx::ParticleSlot::sample)
            .collect()
    }
    /// Seconds since the burst of a receipt was admitted. A recipe may hold some of its
    /// particles back, so its age is that of its oldest particle. The particles of the
    /// built-in burst start together; an accent can carry the same number and is older, so
    /// there the youngest particle with the receipt id counts.
    fn impact_age(&self, receipt: u64) -> Option<f32> {
        use crate::game_vfx::ParticleSource as Source;
        let ages = |source: Source| {
            self.particles
                .iter()
                .filter(move |slot| slot.source().is_some_and(|(_, seen)| seen == source))
                .filter_map(crate::game_vfx::ParticleSlot::sample)
                .filter(move |(id, _)| *id == receipt)
                .map(|(_, age)| age)
        };
        ages(Source::Impact)
            .max_by(f32::total_cmp)
            .or_else(|| ages(Source::Engine).min_by(f32::total_cmp))
    }
    /// Live particles by what they depict. An action sequence and a receipt id can be the
    /// same number, so ids alone do not tell an accent from an impact.
    fn particle_sources(&self) -> serde_json::Value {
        use crate::game_vfx::ParticleSource as Source;
        let count = |source: Source| {
            self.particles
                .iter()
                .filter_map(crate::game_vfx::ParticleSlot::source)
                .filter(|(_, seen)| *seen == source)
                .count()
        };
        serde_json::json!({
            "engine": count(Source::Engine),
            "accent": count(Source::Accent),
            "move": count(Source::Move),
            "link": count(Source::Link),
            "stage": count(Source::Stage),
            "cue": count(Source::Cue),
            "impact": count(Source::Impact),
        })
    }
    fn animation(&self) -> Option<&str> {
        self.animations
            .0
            .get(&self.game.your_id)
            .map(|(label, _)| label.as_str())
    }
    /// Library contact time of the release clip the slot's profile names.
    fn release_contact(&self, slot: u8) -> Option<f32> {
        let (class, _, loadout, ..) = self.local.single().ok()?;
        let profile = self
            .registry
            .action_profile(class.0, loadout.0.as_ref(), slot)?;
        crate::humanoid::SharedHumanoidMotion::embedded()
            .ok()?
            .contact(&profile.release)
    }
    /// What is left of an earlier cast, or `None` on a clean slate.
    fn leftovers(&self, target: Vec2) -> Option<&'static str> {
        let near = |actor: SandboxActor, spot: Vec2| {
            self.actor(actor)
                .is_some_and(|telemetry| Vec2::from_array(telemetry.position).distance(spot) < 0.05)
        };
        if !near(SandboxActor::Player, STAGE_HOME) {
            return Some("the hero is not on its spot");
        }
        if !near(SandboxActor::Enemy, target) {
            return Some("the target is not on its spot");
        }
        // A kit's orb is permanent: a reset puts it back on the hero.
        if self.game.skill_effects.iter().any(|effect| {
            effect.owner_id == self.game.your_id && effect.kind != EffectVisualKind::Orb
        }) {
            return Some("a skill effect of the hero is still replicated");
        }
        if self
            .orb()
            .is_some_and(|orb| orb.distance(STAGE_HOME) > 0.05)
        {
            return Some("the hero's orb has not come home");
        }
        if !self.live_particles().is_empty() {
            return Some("particles are still alive");
        }
        if !self.numbers.is_empty() {
            return Some("a damage number is still shown");
        }
        // Only the 3D rig reports its clip; sprites have no readout to wait for.
        if *self.mode == crate::sprite::PlayerVisualMode::Models3d
            && self.animation().is_some_and(|label| label != "Idle")
        {
            return Some("the hero has not returned to idle");
        }
        if !self.context.gameplay_allowed() {
            return Some("gameplay input is blocked");
        }
        self.local
            .single()
            .ok()
            .and_then(|(_, _, _, _, cooldowns)| cooldowns)
            .is_some_and(|cooldowns| cooldowns.recovery_secs > 0.0)
            .then_some("the hero is still recovering")
    }
}

fn stage_config(qa: &Qa) -> SandboxConfig {
    let class = qa.class;
    let mut config = SandboxConfig {
        player: ActorConfig {
            hero: class,
            recipe: qa.recipe.clone(),
            avatar: Some(qa.avatar.clone()),
            level: 10,
            position: STAGE_HOME.to_array(),
            max_hp: shared::hero_balance::base_hp(class),
            infinite_resource: true,
            god_mode: true,
            ..default()
        },
        ..default()
    };
    config.enemy.enabled = true;
    config.enemy.actor.avatar = Some("agnes".into());
    // Not invulnerable: only a hero that takes the hit yields receipts,
    // control and forced movement. The pool outlasts any single cast.
    config.enemy.actor.max_hp = 1_000_000.0;
    config
}

fn log_sandbox(qa: &mut Qa, config: &SandboxConfig) {
    qa.requests.push(serde_json::json!({
        "command": "sandbox_apply_config",
        "target": config.enemy.actor.position,
        "time_scale": config.environment.time_scale,
        "paused": config.environment.paused,
    }));
}

fn send_cast(
    qa: &mut Qa,
    world: &PhaseWorld,
    equipped: &shared::loadout::EquippedSkills,
    slot: u8,
    aim: Aim,
    outgoing: &mut MessageWriter<NetworkCommand>,
) -> Result<(), &'static str> {
    let hero = world
        .actor(SandboxActor::Player)
        .ok_or("no hero telemetry")?;
    let enemy = world
        .actor(SandboxActor::Enemy)
        .ok_or("no target telemetry")?;
    let ability = equipped.ability(SkillSlot::from_index(slot).ok_or("no such slot")?);
    if equipped.resolved().is_some() {
        let aim = world.cast_aim(equipped, slot, aim)?;
        cast(qa, outgoing, slot, aim, world.game.meta.snapshot_tick);
    } else {
        let target = if ability.targeting == TargetingMode::SelfTarget {
            hero.id
        } else {
            enemy.id
        };
        outgoing.write(NetworkCommand::Cast {
            slot,
            target: shared::wire::TargetId::player(target),
        });
        qa.requests.push(serde_json::json!({"command":"cast","slot":slot,"target":target,"snapshot_tick":world.game.meta.snapshot_tick}));
    }
    Ok(())
}

/// A shape of the geometry table.
fn shape_record(shape: &crate::skill_presentation::geometry::GeoShape) -> serde_json::Value {
    use crate::skill_presentation::geometry::GeoShape;
    let point = |point: Vec2| point.to_array();
    match *shape {
        GeoShape::Ring { center, radius } => {
            serde_json::json!({"shape": "ring", "center": point(center), "radius": radius})
        }
        GeoShape::Capsule { from, to, radius } => serde_json::json!({
            "shape": "capsule", "from": point(from), "to": point(to), "radius": radius,
        }),
        GeoShape::Lane {
            from,
            to,
            half_width,
        } => serde_json::json!({
            "shape": "lane", "from": point(from), "to": point(to), "half_width": half_width,
        }),
        GeoShape::Sector {
            apex,
            axis,
            radius,
            half_angle,
        } => serde_json::json!({
            "shape": "sector", "apex": point(apex), "axis": point(axis),
            "radius": radius, "half_angle": half_angle,
        }),
        GeoShape::Pentagon { center, radius } => {
            serde_json::json!({"shape": "pentagon", "center": point(center), "radius": radius})
        }
        GeoShape::Segment { from, to } => {
            serde_json::json!({"shape": "segment", "from": point(from), "to": point(to)})
        }
        GeoShape::None => serde_json::json!({"shape": "none"}),
    }
}

/// The archetype, the boundary the engine drew and the part counts of a staged body.
fn body_record(body: &crate::skill_presentation::SkillBodyVisual) -> serde_json::Value {
    serde_json::json!({
        "archetype": body.archetype.id(),
        "boundary": shape_record(&body.boundary),
        "engine_parts": body.engine,
        "authored_parts": body.authored,
        "trail_parts": body.trail,
        "budget_hidden": body.budget_hidden,
    })
}

/// The state of one still, read in the frame its screenshot is taken.
fn still_record(
    world: &PhaseWorld,
    run: &Phases,
    phase: Phase,
    sandbox: &SandboxSnapshot,
    skill: &str,
) -> serde_json::Value {
    use crate::skill_presentation::evidence;
    let me = world.game.your_id;
    let particles = world.live_particles();
    let edge = run.watch.edge.filter(|_| phase != Phase::Idle);
    let receipt = run.watch.receipt.as_ref().filter(|_| phase != Phase::Idle);
    let of_event = |id: u64| particles.iter().filter(move |(event, _)| *event == id);
    let roots: Vec<_> = world
        .vfx
        .iter()
        .map(|(root, name, visibility, visual, body)| {
            let parts: Vec<_> = world
                .parts
                .iter()
                .filter(|(parent, ..)| parent.parent() == root)
                .collect();
            let shown = *visibility != Visibility::Hidden;
            let lights = parts
                .iter()
                .filter(|(_, _, point, spot)| *point || *spot)
                .count();
            serde_json::json!({
                "name": name.as_str(),
                "effect_id": visual.id,
                "visible": shown,
                "model_ready": visual.model_ready,
                // Present when the effect is drawn through the `body` of its row.
                "body": body.map(body_record),
                "parts": parts.len(),
                "mesh_parts": parts.len() - lights,
                "visible_parts": parts.iter()
                    .filter(|(_, visibility, ..)| shown && **visibility != Visibility::Hidden)
                    .count(),
                "lights": lights,
            })
        })
        .collect();
    // The hero's replicated effects: the block of the row that draws each of them and the
    // most mesh parts its body may have.
    let effect_rows: Vec<_> = world
        .game
        .skill_effects
        .iter()
        .filter(|effect| effect.owner_id == me)
        .map(|effect| {
            let (kind, block) = evidence::binding(effect);
            serde_json::json!({
                "id": effect.id,
                "skill": effect.skill.id(),
                "kind": kind,
                "block": block,
                "part_budget": evidence::part_budget(effect.skill),
            })
        })
        .collect();
    let total = |field: &str| {
        roots
            .iter()
            .filter_map(|root| root[field].as_u64())
            .sum::<u64>()
    };
    let profile = world
        .local
        .single()
        .ok()
        .filter(|_| phase != Phase::Idle)
        .and_then(|(class, _, loadout, ..)| {
            world
                .registry
                .action_profile(class.0, loadout.0.as_ref(), run.action_slot())
        });
    serde_json::json!({
        "phase": phase.name(),
        "gate": run.gate,
        "slot": (phase != Phase::Idle).then_some(run.action_slot()),
        "skill": (phase != Phase::Idle).then_some(skill),
        "snapshot_tick": world.game.meta.snapshot_tick,
        "simulation_secs": sandbox.simulation_secs,
        "time_scale": sandbox.config.environment.time_scale,
        "paused": sandbox.config.environment.paused,
        // The camera distance of the still and the idle still of the same view.
        "zoom": world.zoom.zoom,
        "idle_file": (!matches!(phase, Phase::Idle | Phase::SlotIdle))
            .then_some(run.idle_file.as_str()),
        "since_edge_secs": edge.map(|(at, _)| sandbox.simulation_secs - at),
        "animation": world.animation(),
        "clip": world.clip(),
        // The hero's latest accepted action: another one than the cast when an
        // action was accepted after it.
        "latest_action": world.local.single().ok().map(|(.., action, _)| serde_json::json!({
            "sequence": action.sequence,
            "slot": action.slot,
        })),
        "profile": profile.map(|profile| serde_json::json!({
            "release": profile.release,
            "windup": profile.windup,
            "release_contact_secs": world.release_contact(run.slot)
                .map(|secs| (f64::from(secs) * 1e4).round() / 1e4),
        })),
        "registry_profiles": world.registry.profile_count(),
        // Cast accents carry the action sequence and impact bursts the receipt
        // id in the same field: when the two numbers are equal the counts are
        // of the same particles.
        "particles": {
            "live": particles.len(),
            "with_cast_sequence": edge.map(|(_, sequence)| of_event(sequence).count()),
            "with_receipt_id": receipt.map(|receipt| of_event(receipt.id).count()),
            "ids_collide": edge.zip(receipt).map(|((_, sequence), receipt)| sequence == receipt.id),
            "impact_age_secs": receipt.and_then(|receipt| world.impact_age(receipt.id)),
            "by_source": world.particle_sources(),
        },
        "receipt": receipt.map(|receipt| serde_json::json!({
            "id": receipt.id,
            "amount": receipt.amount,
            "target": receipt.target,
            "since_seen_secs": sandbox.simulation_secs - receipt.seen,
        })),
        // What drew the burst of each of the newest accepted receipts, oldest first.
        "receipt_looks": world.receipts.0.iter().map(|look| serde_json::json!({
            "receipt": look.receipt,
            "slot": look.slot,
            "impact": look.impact,
            "particles": look.particles,
        })).collect::<Vec<_>>(),
        "damage_numbers": world.numbers.iter().count(),
        "effects": world.game.skill_effects.iter()
            .filter(|effect| effect.owner_id == me)
            .collect::<Vec<&SkillEffectState>>(),
        "effect_rows": effect_rows,
        "skill_vfx": roots,
        "projectiles": world.own_projectiles(),
        "state_visuals": world.state_visuals(),
        "hero_states": world.hero_states(),
        // The action sequence of the cast, and the newest voices the rows of the
        // registry asked the audio layer for, oldest first. A voice is recorded when
        // it is asked for, whether or not the mixer lets it sound.
        "audio": {
            "cast_sequence": edge.map(|(_, sequence)| sequence),
            "row_voices": world.audio.row_voices,
        },
        "effect_parts": total("parts"),
        "effect_visible_parts": total("visible_parts"),
        "effect_lights": total("lights"),
        "hero_pixels": world.actor(SandboxActor::Player)
            .and_then(|actor| world.pixels(actor.position)),
        // The camera follows the hero: a still taken away from the stage shows other ground
        // than the idle still it is compared with.
        "hero_from_home": world.actor(SandboxActor::Player)
            .map(|actor| Vec2::from_array(actor.position).distance(STAGE_HOME)),
        // Where the server has the hero, to read next to the positions of its effects.
        "hero_position": world.actor(SandboxActor::Player).map(|actor| actor.position),
        "target_pixels": world.actor(SandboxActor::Enemy)
            .and_then(|actor| world.pixels(actor.position)),
    })
}

/// The state of an aim still: the full record of the frame and the preview the game drew
/// for the held key in it.
fn aim_record(
    world: &PhaseWorld,
    run: &Phases,
    phase: Phase,
    sandbox: &SandboxSnapshot,
    skill: &str,
) -> serde_json::Value {
    use crate::skill_presentation::geometry::PreviewMark;
    let point = |point: Vec2| point.to_array();
    let mut record = still_record(world, run, phase, sandbox, skill);
    let flags = world
        .local
        .single()
        .ok()
        .and_then(|(_, _, loadout, ..)| loadout.0.as_ref());
    record["aim"] = serde_json::json!({
        "held_key": crate::input_bindings::SKILL_SLOT_KEY_LABELS[usize::from(run.slot)],
        "aim_point": run.hold.map(|hold| point(hold.aim)),
        "cursor": run.hold.map(|hold| point(hold.cursor)),
        "can_recast": flags.map(|flags| flags.slots[usize::from(run.slot)].can_recast),
        "orb_position": flags.and_then(|flags| flags.orb_position),
        "preview": world.aim.0.as_ref().map(|(slot, preview)| serde_json::json!({
            "slot": slot,
            "shape": preview.shape.id(),
            "areas": preview.areas.iter().map(shape_record).collect::<Vec<_>>(),
            "marks": preview.marks.iter().map(|mark| match *mark {
                PreviewMark::Path { from, to } => {
                    serde_json::json!({"mark": "path", "from": point(from), "to": point(to)})
                }
                PreviewMark::Landing(at) => serde_json::json!({"mark": "landing", "at": point(at)}),
                PreviewMark::Picked { at, radius } => {
                    serde_json::json!({"mark": "picked", "at": point(at), "radius": radius})
                }
                PreviewMark::Push { from, to } => {
                    serde_json::json!({"mark": "push", "from": point(from), "to": point(to)})
                }
            }).collect::<Vec<_>>(),
            "pick": preview.pick,
            "refused": preview.refused,
        })),
    });
    record
}

/// Aim stills: keeps the skill key of `Phases::hold` down and the cursor on its aim
/// point, as a player who holds the key does. When the hold ends the key is let go
/// without a release, so nothing is cast.
fn hold_aim(
    mut run: ResMut<Phases>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
) {
    let run = &mut *run;
    let Some(hold) = run.hold else {
        if let Some(key) = run.held.take() {
            keyboard.reset(key);
        }
        return;
    };
    let key = crate::input_bindings::SKILL_CAST_KEYS[usize::from(hold.slot)];
    if let Ok(mut window) = windows.single_mut() {
        run.cursor_before = Some(window.physical_cursor_position());
        window.set_cursor_position(Some(hold.cursor));
    }
    keyboard.press(key);
    run.held = Some(key);
}
/// Puts the window's cursor position back after the frame of a held aim, so the window
/// system has no pointer move to perform on the desktop.
fn restore_cursor(mut run: ResMut<Phases>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    if let Some(before) = run.cursor_before.take()
        && let Ok(mut window) = windows.single_mut()
    {
        window.set_physical_cursor_position(before.map(|position| position.as_dvec2()));
    }
}

fn drive_phases(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    mut run: ResMut<Phases>,
    world: PhaseWorld,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut outgoing: MessageWriter<NetworkCommand>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage != PHASE_STAGE {
        return;
    }
    let (qa, run) = (&mut *qa, &mut *run);
    let step = run.step;
    let stop = |qa: &mut Qa, run: &Phases, exit: &mut MessageWriter<AppExit>, reason: String| {
        let detail = serde_json::json!({
            "step": format!("{step:?}"),
            "pass": format!("{:?}", run.pass),
            "slot": run.slot,
            "watch": run.watch.timeline(),
            "probe": run.probe.as_ref().map(CastWatch::timeline),
            "skills": run.skills,
        });
        fail(qa, exit, &reason, detail);
    };
    if let Some(file) = qa.black.take() {
        stop(
            qa,
            run,
            &mut exit,
            format!("Captured frame {file} is black"),
        );
        return;
    }
    if qa.started.elapsed() > PHASE_DEADLINE {
        stop(
            qa,
            run,
            &mut exit,
            "The phase run exceeded its time limit".into(),
        );
        return;
    }
    run.frames += 1;
    if run.entered.get_or_insert_with(Instant::now).elapsed() > STEP_STALL {
        let waiting = match step {
            Step::Settle => world
                .leftovers(
                    STAGE_HOME + STAGE_LANE * run.staging.map_or(0.0, |staging| staging.distance),
                )
                .unwrap_or("the reset acknowledgement"),
            Step::March => "the allied wave to reach the hero",
            Step::Frame => "the camera to rest on a view that holds the skill",
            Step::Edge => "the server to accept the cast",
            Step::Gates => "the gate of the next still",
            Step::Flight => "the projectile of the basic attack",
            Step::Mode => "the replicated rocket mode",
            Step::Park(_) => "the orb to be parked",
            Step::Aim(_) => "the aim preview of the held key",
            Step::Reach => "the hero to be in reach of its recast",
            Step::Read(_) => "the screenshot read-back",
            _ => "the sandbox acknowledgement",
        };
        stop(qa, run, &mut exit, format!("Stalled waiting for {waiting}"));
        return;
    }
    let Some(sandbox) = world.game.sandbox.as_ref() else {
        return;
    };
    let Ok((class, progression, loadout, action, _)) = world.local.single() else {
        return;
    };
    let Some(equipped) = crate::equipped_skills::resolve(class.0, Some(loadout)) else {
        stop(qa, run, &mut exit, "The hero's kit does not resolve".into());
        return;
    };
    let now = sandbox.simulation_secs;
    let slot = SkillSlot::ALL[usize::from(run.slot)];
    let skill = if run.basic {
        "basic"
    } else {
        equipped.ability(slot).id
    };
    // The run ends after the last skill, or after the basic attack of a flight look.
    let mut finished = false;
    let acknowledged = match run.wire.poll(sandbox, &mut outgoing) {
        None => false,
        Some(Ok(())) => true,
        Some(Err(message)) => {
            stop(
                qa,
                run,
                &mut exit,
                format!("The sandbox rejected a request: {message}"),
            );
            return;
        }
    };
    if matches!(
        step,
        Step::Edge
            | Step::Probe
            | Step::Gates
            | Step::Flight
            | Step::Pause(_)
            | Step::Read(_)
            | Step::Resume
            | Step::Reach
    ) || step == Step::Aim(Phase::RecastAim)
    {
        run.watch.observe(&world.game, action, now);
        // What is alive while the cast is captured, also between its stills.
        if run.pass == Pass::Capture
            && let Some((_, sequence)) = run.watch.edge
        {
            let load = world.load(Some(sequence));
            let peaks = &mut run.peaks;
            peaks.accent = peaks.accent.max(load.accent);
            peaks.impact = peaks.impact.max(load.impact);
            peaks.live_particles = peaks.live_particles.max(load.live_particles);
            peaks.visible_parts = peaks.visible_parts.max(load.visible_parts);
            peaks.lights = peaks.lights.max(load.lights);
        }
    }
    // An aim still needs a modular skill: a legacy kit casts at a selected unit.
    let previewed = qa.aim && !run.basic && equipped.skill(slot).is_some();
    match step {
        Step::Arrange => {
            let staging = if run.basic {
                Staging::basic(class.0)
            } else {
                Staging::of(class.0, &equipped, slot, qa.flight)
            };
            let mut config = run.config.take().unwrap_or_else(|| stage_config(qa));
            config.enemy.actor.position = (STAGE_HOME + STAGE_LANE * staging.distance).to_array();
            // A cast that needs an ally gets a wave: it marches in from its base.
            let marches = staging.aim == Aim::Ally;
            config.environment.minions = marches;
            config.environment.minions_paused = false;
            config.environment.time_scale = if marches { MARCH_SCALE } else { 1.0 };
            config.environment.paused = false;
            log_sandbox(qa, &config);
            run.wire.send(
                &world.game,
                sandbox,
                SandboxCommand::ApplyConfig {
                    config: config.clone(),
                },
                &mut outgoing,
            );
            run.config = Some(config);
            run.staging = Some(staging);
            run.enter(Step::Reset);
        }
        Step::Reset if acknowledged => {
            qa.requests
                .push(serde_json::json!({"command":"sandbox_reset_duel","slot":run.slot}));
            run.wire.send(
                &world.game,
                sandbox,
                SandboxCommand::ResetDuel,
                &mut outgoing,
            );
            let marches = run.staging.is_some_and(|staging| staging.aim == Aim::Ally);
            run.enter(if marches { Step::Wave } else { Step::Settle });
        }
        // The reset removed every minion: the wave is ordered after it.
        Step::Wave if acknowledged => {
            qa.requests
                .push(serde_json::json!({"command":"sandbox_spawn_wave","slot":run.slot}));
            run.wire.send(
                &world.game,
                sandbox,
                SandboxCommand::SpawnWave,
                &mut outgoing,
            );
            run.enter(Step::March);
        }
        Step::March
            if acknowledged
                && world
                    .ally(STAGE_HOME)
                    .is_some_and(|ally| ally.distance(STAGE_HOME) <= ALLY_STOP) =>
        {
            let Some(config) = run.config.as_mut() else {
                return;
            };
            config.environment.minions_paused = true;
            config.environment.time_scale = 1.0;
            log_sandbox(qa, config);
            let command = SandboxCommand::ApplyConfig {
                config: config.clone(),
            };
            run.wire.send(&world.game, sandbox, command, &mut outgoing);
            run.enter(Step::Settle);
        }
        Step::Settle if acknowledged && run.frames >= 20 => {
            let Some(staging) = run.staging else {
                return;
            };
            if class.0 != qa.class
                || progression.level != 10
                || world
                    .leftovers(STAGE_HOME + STAGE_LANE * staging.distance)
                    .is_some()
            {
                return;
            }
            // The orb of a kit is ordered away by the skill of the kit that moves it.
            let orb_order = SkillSlot::ALL
                .iter()
                .position(|slot| {
                    matches!(
                        equipped.skill(*slot).map(|definition| definition.effect),
                        Some(SkillEffect::Technique {
                            action: Technique::BallMove,
                            ..
                        })
                    )
                })
                .filter(|_| world.orb().is_some());
            // A cast that calls the orb home is staged with the orb beyond the target.
            let orb_out = orb_order.filter(|_| staging.park_orb && !run.orb_out);
            if run.pass == Pass::Probe {
                if let Some(order) = orb_out {
                    run.orb_out = true;
                    park_orb(qa, run, &world, order, Parked::Cast, &mut outgoing);
                } else {
                    run.enter(Step::Cast);
                }
            } else if !run.idle_taken {
                run.gate = "clean_slate";
                let record = still_record(&world, run, Phase::Idle, sandbox, skill);
                let index = PHASE_SHOT + run.stills;
                shoot(&mut commands, qa, index, "0-idle.png".into(), record);
                run.enter(Step::Read(Phase::Idle));
            } else if !run.framed {
                // What the probe cast hit is kept in view with the area of the cast.
                let hit = run
                    .probe
                    .as_ref()
                    .and_then(|probe| probe.receipt.as_ref())
                    .and_then(|receipt| receipt.offset);
                run.keep = staging.keep(hit);
                run.span = (0.0, 0);
                run.enter(Step::Frame);
            } else if qa.zoom > crate::camera::CAMERA_MIN_ZOOM && !run.slot_idle_taken {
                run.gate = "clean_slate";
                let record = still_record(&world, run, Phase::SlotIdle, sandbox, skill);
                let file = format!(
                    "{}-{}-0-idle.png",
                    run.slot + 1,
                    ["q", "w", "e", "r"][usize::from(run.slot)]
                );
                run.idle_file = file.clone();
                let index = PHASE_SHOT + run.stills;
                shoot(&mut commands, qa, index, file, record);
                run.enter(Step::Read(Phase::SlotIdle));
            } else if previewed && !run.aim_taken {
                // A kit with an orb draws its previews from the orb: it is ordered away
                // from the hero first.
                if let Some(order) = orb_order.filter(|_| !run.parked) {
                    run.parked = true;
                    park_orb(qa, run, &world, order, Parked::Aim, &mut outgoing);
                    return;
                }
                match world.hold(&equipped, run.slot, staging.aim) {
                    Ok(hold) => {
                        run.hold = Some(hold);
                        run.enter(Step::Aim(Phase::Aim));
                    }
                    Err(reason) => stop(qa, run, &mut exit, format!("{skill}: {reason}")),
                }
            } else if let Some(order) = orb_out {
                run.orb_out = true;
                park_orb(qa, run, &world, order, Parked::Cast, &mut outgoing);
            } else {
                request_speed(qa, run, &world, sandbox, SLOW_MOTION, false, &mut outgoing);
                run.enter(Step::Slow);
            }
        }
        // The camera follows a change of its distance over some frames: the view is
        // measured once it rests, and widened until it holds what the skill must show.
        Step::Frame => {
            let Some(span) = world.lane_span() else {
                return;
            };
            if (span - run.span.0).abs() <= 0.02 {
                run.span.1 += 1;
            } else {
                run.span = (span, 0);
            }
            if run.span.1 < FRAME_REST {
                return;
            }
            match world.frame_fit(&run.keep) {
                None => stop(
                    qa,
                    run,
                    &mut exit,
                    format!("{skill}: the stage is not in view"),
                ),
                Some(fit)
                    if fit > FRAME_FILL + 0.01 && qa.zoom < crate::camera::CAMERA_MAX_ZOOM =>
                {
                    qa.zoom = (qa.zoom * fit / FRAME_FILL).min(crate::camera::CAMERA_MAX_ZOOM);
                    run.span = (0.0, 0);
                }
                Some(_) => {
                    run.framed = true;
                    run.enter(Step::Settle);
                }
            }
        }
        Step::Park(parked)
            if run.frames >= 20
                && world
                    .orb()
                    .is_some_and(|orb| orb.distance(STAGE_HOME + STAGE_LANE * ORB_PARK) < 0.05)
                && world.at_rest() =>
        {
            let aim = run.staging.map_or(Aim::Target, |staging| staging.aim);
            match parked {
                Parked::Aim => match world.hold(&equipped, run.slot, aim) {
                    Ok(hold) => {
                        run.hold = Some(hold);
                        run.enter(Step::Aim(Phase::Aim));
                    }
                    Err(reason) => stop(qa, run, &mut exit, format!("{skill}: {reason}")),
                },
                Parked::Cast if run.pass == Pass::Probe => run.enter(Step::Cast),
                Parked::Cast => {
                    request_speed(qa, run, &world, sandbox, SLOW_MOTION, false, &mut outgoing);
                    run.enter(Step::Slow);
                }
            }
        }
        // The key has been down for a few frames and the game drew its preview.
        Step::Aim(phase)
            if run.frames >= 6
                && world
                    .aim
                    .0
                    .as_ref()
                    .is_some_and(|(shown, _)| *shown == usize::from(run.slot)) =>
        {
            run.gate = "key_held";
            let record = aim_record(&world, run, phase, sandbox, skill);
            let file = format!(
                "{}-{}-{}-{}.png",
                run.slot + 1,
                ["q", "w", "e", "r"][usize::from(run.slot)],
                phase.order(),
                phase.name().replace('_', "-")
            );
            run.aim_stills.push(file.clone());
            let index = PHASE_SHOT + run.stills;
            shoot(&mut commands, qa, index, file, record);
            run.enter(Step::Read(phase));
        }
        Step::Reach if acknowledged && !sandbox.config.environment.paused => {
            let hero = world
                .actor(SandboxActor::Player)
                .map(|actor| Vec2::from_array(actor.position));
            let offered = loadout
                .0
                .as_ref()
                .is_some_and(|flags| flags.slots[usize::from(run.slot)].can_recast);
            let own = equipped.skill(slot).map(|definition| definition.id);
            let alive =
                world.game.skill_effects.iter().any(|effect| {
                    effect.owner_id == world.game.your_id && Some(effect.skill) == own
                });
            let reach = hero.zip(own).is_some_and(|(hero, own)| {
                crate::skill_presentation::status::recast_in_reach(
                    own,
                    world.game.your_id,
                    hero,
                    &world.game.skill_effects,
                )
            });
            if reach {
                run.frozen = (now, 0);
                request_speed(qa, run, &world, sandbox, SLOW_MOTION, true, &mut outgoing);
                run.enter(Step::Pause(Phase::RecastAim));
            } else if !offered || !alive {
                // The window closed before the hero was in reach: there is no press to
                // preview, and none is staged.
                run.recast_aim_taken = true;
                run.recast_aim_note = Some("recast_never_in_reach");
                finish_skill(qa, run, &world, &equipped, slot, skill);
                finished = advance_slot(qa, run);
            }
        }
        Step::Slow if acknowledged && sandbox.config.environment.time_scale == SLOW_MOTION => {
            run.enter(Step::Cast);
        }
        Step::Cast => {
            let Some(staging) = run.staging else {
                return;
            };
            run.watch = CastWatch {
                slot: run.action_slot(),
                skill: equipped
                    .skill(slot)
                    .filter(|_| !run.basic)
                    .map(|definition| definition.id),
                action_before: action.sequence,
                event_floor: world
                    .game
                    .combat_events
                    .iter()
                    .map(|event| event.id)
                    .max()
                    .unwrap_or(0),
                known: world
                    .game
                    .skill_effects
                    .iter()
                    .map(|effect| effect.id)
                    .collect(),
                ..default()
            };
            if run.basic {
                // The rocket round: the kit's own switch is cast first, and the attack
                // waits for the mode the server replicates.
                let in_rocket_mode = loadout
                    .0
                    .as_ref()
                    .is_some_and(|flags| flags.weapon_mode == WeaponMode::Rockets);
                if run.rockets && !in_rocket_mode {
                    let Some(switch) = weapon_switch(&equipped) else {
                        stop(qa, run, &mut exit, "the kit has no weapon switch".into());
                        return;
                    };
                    if let Err(reason) =
                        send_cast(qa, &world, &equipped, switch, Aim::Target, &mut outgoing)
                    {
                        stop(qa, run, &mut exit, reason.into());
                        return;
                    }
                    run.enter(Step::Mode);
                    return;
                }
                let Some(enemy) = world.actor(SandboxActor::Enemy) else {
                    stop(qa, run, &mut exit, "no target telemetry".into());
                    return;
                };
                outgoing.write(NetworkCommand::BasicAttack {
                    target: shared::wire::TargetId::player(enemy.id),
                });
                qa.requests.push(serde_json::json!({"command":"basic_attack","target":enemy.id,"rockets":run.rockets,"snapshot_tick":world.game.meta.snapshot_tick}));
                // A melee core resolves its attack at once: its hit is the only still.
                run.enter(if throws(&equipped) {
                    Step::Flight
                } else {
                    Step::Gates
                });
                return;
            }
            if let Err(reason) =
                send_cast(qa, &world, &equipped, run.slot, staging.aim, &mut outgoing)
            {
                stop(qa, run, &mut exit, reason.into());
                return;
            }
            run.enter(Step::Edge);
        }
        // The mode is replicated and the switch has run out: the attack starts from rest.
        Step::Mode
            if loadout
                .0
                .as_ref()
                .is_some_and(|flags| flags.weapon_mode == WeaponMode::Rockets)
                && world.at_rest() =>
        {
            run.enter(Step::Cast);
        }
        Step::Flight => {
            let Some(staging) = run.staging else {
                return;
            };
            // Gated on the replicated projectile alone; a hit is never staged.
            let flying = world.own_projectiles().iter().any(|projectile| {
                projectile["action_slot"].as_u64()
                    == Some(u64::from(shared::BASIC_ATTACK_ACTION_SLOT))
                    && projectile["from_hero"]
                        .as_f64()
                        .is_some_and(|out| out >= f64::from(staging.distance) * FLIGHT_SHARE)
            });
            if flying {
                run.gate = "own_projectile";
                run.frozen = (now, 0);
                request_speed(qa, run, &world, sandbox, SLOW_MOTION, true, &mut outgoing);
                run.enter(Step::Pause(Phase::Flight));
            } else if run.watch.receipt.is_some() {
                stop(
                    qa,
                    run,
                    &mut exit,
                    "The basic attack hit before its projectile was seen in flight".into(),
                );
            }
        }
        Step::Edge if run.watch.edge.is_some() => {
            let next = if run.pass == Pass::Probe {
                Step::Probe
            } else {
                Step::Gates
            };
            // The telegraph the probe saw is running now: an attack ordered here is
            // accepted during it, when the target is within the hero's reach.
            let telegraphed = run.probe.as_ref().is_some_and(|p| p.fuse().is_some());
            if qa.interleave
                && run.pass == Pass::Capture
                && telegraphed
                && let Some(enemy) = world.actor(SandboxActor::Enemy)
            {
                outgoing.write(NetworkCommand::BasicAttack {
                    target: shared::wire::TargetId::player(enemy.id),
                });
                qa.requests.push(serde_json::json!({"command":"basic_attack","target":enemy.id,"during":skill,"snapshot_tick":world.game.meta.snapshot_tick}));
            }
            // The telegraph is running: the sandbox moves its caster now. The probe is
            // moved too, so that it sees the receipt the capture will see.
            let follows = equipped.skill(slot).is_some_and(|definition| {
                matches!(
                    definition.effect,
                    SkillEffect::Technique {
                        action: Technique::ConeBrittle,
                        ..
                    }
                )
            });
            if qa.displace
                && follows
                && let Some(actor) = world.actor(SandboxActor::Player)
            {
                let to = (Vec2::from_array(actor.position) + STAGE_LANE.perp() * DISPLACE_UNITS)
                    .to_array();
                qa.requests.push(serde_json::json!({"command":"sandbox_teleport","to":to,"during":skill,"snapshot_tick":world.game.meta.snapshot_tick}));
                let command = SandboxCommand::Teleport {
                    actor: SandboxActor::Player,
                    position: to,
                };
                run.wire.send(&world.game, sandbox, command, &mut outgoing);
            }
            // The wall is up: the target throws its first skill at it, in the probe and
            // in the capture. Nothing is staged beyond that order; whether the wall
            // stops the projectile is the server's business.
            let walls = equipped.skill(slot).is_some_and(|definition| {
                matches!(
                    definition.effect,
                    SkillEffect::Technique {
                        action: Technique::InterceptShield,
                        ..
                    }
                )
            });
            if qa.block && walls {
                qa.requests.push(serde_json::json!({"command":"sandbox_force_cast","actor":"enemy","slot":0,"target":world.game.your_id,"during":skill,"snapshot_tick":world.game.meta.snapshot_tick}));
                let command = SandboxCommand::ForceCast {
                    actor: SandboxActor::Enemy,
                    slot: 0,
                    target_id: Some(world.game.your_id),
                };
                run.wire.send(&world.game, sandbox, command, &mut outgoing);
            }
            run.enter(next);
        }
        Step::Probe
            if run
                .watch
                .edge
                .is_some_and(|(edge, _)| now - edge >= PROBE_SECS) =>
        {
            run.probe = Some(std::mem::take(&mut run.watch));
            run.begin_pass(Pass::Capture);
        }
        Step::Gates => {
            if run.basic {
                // The hit of the basic attack: gated on its own receipt, never staged.
                let landed = run
                    .watch
                    .receipt
                    .as_ref()
                    .is_some_and(|receipt| now - receipt.seen >= f64::from(IMPACT_AGE.0));
                if landed {
                    run.gate = "receipt";
                    run.frozen = (now, 0);
                    request_speed(qa, run, &world, sandbox, SLOW_MOTION, true, &mut outgoing);
                    run.enter(Step::Pause(Phase::Impact));
                }
                return;
            }
            let (Some(staging), Some(probe), Some((edge, _))) =
                (run.staging, run.probe.as_ref(), run.watch.edge)
            else {
                return;
            };
            let fused = probe.fuse();
            let release_window = qa.release_at.window(world.release_contact(run.slot));
            let due = if !run.windup_taken {
                match fused {
                    None => Some((Phase::Windup, "edge")),
                    Some(_) if run.watch.fuse().is_some() => {
                        stop(
                            qa,
                            run,
                            &mut exit,
                            format!("{skill}: the telegraph fired before its windup still"),
                        );
                        return;
                    }
                    Some((seen, fired, _)) => run
                        .watch
                        .telegraph()
                        .filter(|first| now - first >= TELEGRAPH_SHARE * (fired - seen))
                        .map(|_| (Phase::Windup, "telegraph_share")),
                }
            } else if !run.third_taken
                && run
                    .watch
                    .receipt
                    .as_ref()
                    .is_some_and(|receipt| now - receipt.seen >= f64::from(IMPACT_AGE.0))
            {
                Some((Phase::Impact, "receipt"))
            } else if run.release_taken.is_none() {
                match fused {
                    None => (now - edge >= release_window.0)
                        .then_some((Phase::Release, "release_window")),
                    Some(_) => run.watch.fuse().map(|(_, _, how)| (Phase::Release, how)),
                }
            } else if !run.third_taken {
                // No receipt yet: wait for the one the probe saw, never invent one.
                let expected = probe
                    .receipt
                    .as_ref()
                    .zip(probe.edge)
                    .map(|(receipt, (probe_edge, _))| receipt.seen - probe_edge);
                let waited = expected.is_none_or(|after| now - edge >= after + RECEIPT_GRACE);
                let settled = run
                    .release_taken
                    .is_some_and(|release| now - release >= SETTLED_AFTER_RELEASE);
                (waited && settled).then_some((
                    Phase::Settled,
                    // The reason the staging gives; otherwise a cast that had to hit missed.
                    staging.settles.unwrap_or(if expected.is_none() {
                        "no_receipt_in_probe"
                    } else {
                        "no_receipt_observed"
                    }),
                ))
            } else {
                None
            };
            if let Some((phase, gate)) = due {
                run.gate = gate;
                run.frozen = (now, 0);
                request_speed(qa, run, &world, sandbox, SLOW_MOTION, true, &mut outgoing);
                run.enter(Step::Pause(phase));
            }
        }
        Step::Pause(phase) if acknowledged && sandbox.config.environment.paused => {
            // The presentation clock reads the previous snapshot: let it stop.
            if run.frozen.0 == now {
                run.frozen.1 += 1;
            } else {
                run.frozen = (now, 0);
            }
            if run.frozen.1 < 3 {
                return;
            }
            if phase == Phase::RecastAim {
                let aim = run.staging.map_or(Aim::Target, |staging| staging.aim);
                match world.hold(&equipped, run.slot, aim) {
                    Ok(hold) => {
                        run.hold = Some(hold);
                        run.enter(Step::Aim(phase));
                    }
                    Err(reason) => stop(qa, run, &mut exit, format!("{skill}: {reason}")),
                }
                return;
            }
            let record = still_record(&world, run, phase, sandbox, skill);
            if phase == Phase::Release
                && run.gate == "release_window"
                && record["since_edge_secs"].as_f64().is_none_or(|after| {
                    after > qa.release_at.window(world.release_contact(run.slot)).1
                })
            {
                stop(
                    qa,
                    run,
                    &mut exit,
                    format!(
                        "{skill}: the release still missed its window ({})",
                        record["since_edge_secs"]
                    ),
                );
                return;
            }
            if phase == Phase::Windup
                && run.gate == "edge"
                && record["since_edge_secs"]
                    .as_f64()
                    .is_none_or(|after| after > EDGE_WINDOW)
            {
                // A stalled frame: the still would show the cast after its windup. It
                // is not taken; the cast is staged again from a clean slate.
                if run.retakes < RETAKES {
                    run.retakes += 1;
                    qa.requests.push(serde_json::json!({
                        "command": "retake",
                        "skill": skill,
                        "late_windup_secs": record["since_edge_secs"],
                    }));
                    run.orb_out = false;
                    run.peaks = Peaks::default();
                    run.enter(Step::Arrange);
                    return;
                }
                stop(
                    qa,
                    run,
                    &mut exit,
                    format!(
                        "{skill}: the windup still was taken too long after the accepted cast ({})",
                        record["since_edge_secs"]
                    ),
                );
                return;
            }
            if phase == Phase::Impact
                && record["particles"]["impact_age_secs"]
                    .as_f64()
                    .is_none_or(|age| {
                        age < f64::from(IMPACT_AGE.0) - 1e-3 || age > f64::from(IMPACT_AGE.1)
                    })
            {
                stop(
                    qa,
                    run,
                    &mut exit,
                    format!(
                        "{skill}: the impact still has no burst of the receipt in its age window ({})",
                        record["particles"]
                    ),
                );
                return;
            }
            let basic_in_flight = record["projectiles"].as_array().is_some_and(|all| {
                all.iter().any(|projectile| {
                    projectile["action_slot"].as_u64()
                        == Some(u64::from(shared::BASIC_ATTACK_ACTION_SLOT))
                })
            });
            if phase == Phase::Flight && !basic_in_flight {
                stop(
                    qa,
                    run,
                    &mut exit,
                    "The projectile of the basic attack was gone before its still".into(),
                );
                return;
            }
            let file = if run.basic {
                let round = if run.rockets { "6-rockets" } else { "5-basic" };
                format!("{round}-{}.png", phase.name())
            } else {
                format!(
                    "{}-{}-{}-{}.png",
                    run.slot + 1,
                    ["q", "w", "e", "r"][usize::from(run.slot)],
                    phase.order(),
                    phase.name()
                )
            };
            run.skill_stills.push(file.clone());
            let index = PHASE_SHOT + run.stills;
            shoot(&mut commands, qa, index, file, record);
            run.enter(Step::Read(phase));
        }
        Step::Read(phase) if qa.readbacks.contains(&(PHASE_SHOT + run.stills)) => {
            run.stills += 1;
            match phase {
                Phase::Idle => run.idle_taken = true,
                Phase::SlotIdle => run.slot_idle_taken = true,
                Phase::Windup => run.windup_taken = true,
                Phase::Release => run.release_taken = Some(now),
                Phase::Impact | Phase::Settled => run.third_taken = true,
                Phase::Flight => {}
                Phase::Aim => run.aim_taken = true,
                Phase::RecastAim => run.recast_aim_taken = true,
            }
            // The key of an aim still is let go; `hold_aim` sends no release.
            run.hold = None;
            let offered = loadout
                .0
                .as_ref()
                .is_some_and(|flags| flags.slots[usize::from(run.slot)].can_recast);
            if run.basic && phase == Phase::Flight {
                // The shot flies on: its hit is the second still of the round.
                request_speed(qa, run, &world, sandbox, SLOW_MOTION, false, &mut outgoing);
                run.enter(Step::Resume);
            } else if run.basic && phase == Phase::Impact {
                let staging = run.staging;
                let round = serde_json::json!({
                    "slot": shared::BASIC_ATTACK_ACTION_SLOT,
                    // The replicated mode of the hero when the round was captured.
                    "weapon_mode": loadout.0.as_ref().map(|flags| format!("{:?}", flags.weapon_mode)),
                    "staging": staging.map(|staging| serde_json::json!({
                        "category": staging.category,
                        "target_distance": staging.distance,
                    })),
                    "capture": run.watch.timeline(),
                    "stills": std::mem::take(&mut run.skill_stills),
                });
                match run.basic_record.as_mut().filter(|_| run.rockets) {
                    Some(record) => record["rockets"] = round,
                    None => run.basic_record = Some(round),
                }
                // A repeater has a second round: it is captured after its switch.
                if !run.rockets && weapon_switch(&equipped).is_some() {
                    run.rockets = true;
                    run.begin_pass(Pass::Capture);
                } else {
                    finished = true;
                }
            } else if matches!(phase, Phase::Idle | Phase::SlotIdle) {
                // The slate is still clean: what comes before the cast is decided there.
                run.enter(Step::Settle);
            } else if phase == Phase::Aim && run.parked {
                // The orb is away from the hero: the cast starts from a clean slate again.
                qa.requests
                    .push(serde_json::json!({"command":"sandbox_reset_duel","slot":run.slot}));
                run.wire.send(
                    &world.game,
                    sandbox,
                    SandboxCommand::ResetDuel,
                    &mut outgoing,
                );
                run.enter(Step::Settle);
            } else if phase == Phase::Aim {
                // Only a key was held: the slate is still clean.
                run.enter(Step::Settle);
            } else if !(run.windup_taken && run.release_taken.is_some() && run.third_taken) {
                request_speed(qa, run, &world, sandbox, SLOW_MOTION, false, &mut outgoing);
                run.enter(Step::Resume);
            } else if previewed && !run.recast_aim_taken && offered {
                // The simulation is paused on the third still and the slot offers a
                // recast: its key is held for one more still. A recast with a gate is
                // previewed where the server accepts it, so the cast runs on until the
                // hero is in reach.
                let in_reach = world
                    .actor(SandboxActor::Player)
                    .zip(equipped.skill(slot))
                    .is_some_and(|(hero, definition)| {
                        crate::skill_presentation::status::recast_in_reach(
                            definition.id,
                            world.game.your_id,
                            Vec2::from_array(hero.position),
                            &world.game.skill_effects,
                        )
                    });
                if in_reach {
                    let aim = run.staging.map_or(Aim::Target, |staging| staging.aim);
                    match world.hold(&equipped, run.slot, aim) {
                        Ok(hold) => {
                            run.hold = Some(hold);
                            run.enter(Step::Aim(Phase::RecastAim));
                        }
                        Err(reason) => stop(qa, run, &mut exit, format!("{skill}: {reason}")),
                    }
                } else {
                    request_speed(qa, run, &world, sandbox, SLOW_MOTION, false, &mut outgoing);
                    run.enter(Step::Reach);
                }
            } else {
                finish_skill(qa, run, &world, &equipped, slot, skill);
                finished = advance_slot(qa, run);
            }
        }
        Step::Resume if acknowledged && !sandbox.config.environment.paused => {
            run.enter(Step::Gates);
        }
        _ => {}
    }
    if finished {
        let summary = serde_json::json!({
            "pass": true,
            "scenario": "skill_phases",
            "class": qa.class.id(),
            "locale": "en",
            "pixels": [qa.pixels().0, qa.pixels().1],
            "offscreen": qa.offscreen,
            "avatar": qa.avatar,
            "release_at": format!("{:?}", qa.release_at),
            "visual_mode": format!("{:?}", *world.mode),
            "flight": qa.flight,
            "interleave": qa.interleave,
            "displace": qa.displace,
            "block": qa.block,
            "aim": qa.aim,
            "manual_interaction_verified": false,
            "physical_device_verified": false,
            "setup": "live sandbox; level 10; rank 1; infinite resource, normal cooldowns; ResetDuel before every cast; one probe cast at 1x, then one cast at 0.25x paused for each still; stationary enemy hero with 1,000,000 HP that takes hits; a cast that picks an allied hero targets the caster; a cast that picks any ally gets a minion wave walked in behind the hero",
            "registry_profiles": world.registry.profile_count(),
            "registry": world.registry_record(),
            // The authored kit of the hero; absent for the preset of its class.
            "recipe": qa.recipe.as_ref().map(|recipe| recipe.skills.map(SkillId::id)),
            "skills": run.skills,
            // The basic attack of a flight look: its projectile in flight (a melee core
            // throws none) and its hit, and for a repeater the same of its rocket round.
            "basic": run.basic_record,
            "requests": qa.requests,
            "captures": qa.captures,
        });
        if std::fs::write(
            qa.directory.join("qa-summary.json"),
            serde_json::to_vec_pretty(&summary).unwrap(),
        )
        .is_err()
        {
            stop(qa, run, &mut exit, "Cannot write phase evidence".into());
            return;
        }
        qa.stage = 255;
        if let Ok(window) = windows.single() {
            commands.entity(window).despawn();
        }
        exit.write(AppExit::Success);
    }
}

/// Records the finished skill of the current slot: its staging, what the probe and the
/// captured cast replicated, and its stills.
fn finish_skill(
    qa: &Qa,
    run: &mut Phases,
    world: &PhaseWorld,
    equipped: &shared::loadout::EquippedSkills,
    slot: SkillSlot,
    skill: &str,
) {
    use crate::skill_presentation::evidence;
    let staging = run.staging;
    let probe = run.probe.take();
    let entry = serde_json::json!({
        "slot": run.slot,
        "skill": skill,
        // The class whose kit owns the skill, and the identity the registry gives it.
        "home": evidence::home(skill),
        "identity": evidence::identity(&world.registry, &world.flights, skill),
        "modular": equipped.skill(slot).is_some(),
        "targeting": format!("{:?}", equipped.ability(slot).targeting),
        "cast_range": equipped.ability(slot).cast_range,
        "staging": staging.map(|staging| serde_json::json!({
            "category": staging.category,
            "target_distance": staging.distance,
            "aim": staging.aim.name(),
            "cast_on_caster": staging.aim == Aim::Caster,
            "orb_parked": staging.park_orb,
            "can_damage": evidence::can_damage(skill),
            // Why the staged cast yields no receipt; absent when it has to hit.
            "settles": staging.settles,
            "area_radius": (staging.area > 0.0).then_some(staging.area),
        })),
        // The view of the stills: the closest one unless the skill needs more room.
        "framing": {
            "zoom": qa.zoom,
            "widened": qa.zoom > crate::camera::CAMERA_MIN_ZOOM,
            "idle_file": run.idle_file,
        },
        "peaks": run.peaks.record(),
        // Casts that were staged again because a stalled frame made the windup still late.
        "retakes": run.retakes,
        "probe": probe.as_ref().map(CastWatch::timeline),
        "capture": run.watch.timeline(),
        "stills": std::mem::take(&mut run.skill_stills),
        // The stills of the aim previews, and why an offered recast has none.
        "aim_stills": std::mem::take(&mut run.aim_stills),
        "recast_aim_note": run.recast_aim_note,
    });
    run.skills.push(entry);
}

/// Moves on to the next skill, or to the basic attack of a flight look. Returns whether
/// the run is over.
fn advance_slot(qa: &mut Qa, run: &mut Phases) -> bool {
    // The next cast starts in the closest view again.
    qa.zoom = crate::camera::CAMERA_MIN_ZOOM;
    if run.slot < 3 {
        run.slot += 1;
        run.begin_pass(Pass::Probe);
        return false;
    }
    if qa.flight {
        run.basic = true;
        run.begin_pass(Pass::Capture);
        return false;
    }
    true
}

/// Whether the basic attack of the kit throws a projectile. A melee core resolves its
/// attack at once and throws nothing.
fn throws(equipped: &shared::loadout::EquippedSkills) -> bool {
    equipped
        .resolved()
        .is_none_or(|kit| kit.attack_profile() != shared::loadout::AttackProfileId::Melee)
}

/// The slot of the skill that switches the weapon of a repeater.
fn weapon_switch(equipped: &shared::loadout::EquippedSkills) -> Option<u8> {
    SkillSlot::ALL
        .iter()
        .position(|slot| {
            matches!(
                equipped.skill(*slot).map(|definition| definition.effect),
                Some(SkillEffect::WeaponToggle { .. })
            )
        })
        .map(|slot| slot as u8)
}

/// Orders the orb of the kit to its parking spot up the lane, with the skill in `order`.
fn park_orb(
    qa: &mut Qa,
    run: &mut Phases,
    world: &PhaseWorld,
    order: usize,
    parked: Parked,
    outgoing: &mut MessageWriter<NetworkCommand>,
) {
    let spot = STAGE_HOME + STAGE_LANE * ORB_PARK;
    qa.requests.push(serde_json::json!({
        "command": "cast_skill",
        "purpose": match parked {
            Parked::Aim => "park_orb_for_aim",
            Parked::Cast => "park_orb_for_cast",
        },
        "slot": order,
        "aim": spot.to_array(),
        "snapshot_tick": world.game.meta.snapshot_tick,
    }));
    outgoing.write(NetworkCommand::CastSkill {
        slot: order as u8,
        aim: spot,
    });
    run.enter(Step::Park(parked));
}

/// Change the simulation pace; players and target stay as last configured.
fn request_speed(
    qa: &mut Qa,
    run: &mut Phases,
    world: &PhaseWorld,
    sandbox: &SandboxSnapshot,
    time_scale: f32,
    paused: bool,
    outgoing: &mut MessageWriter<NetworkCommand>,
) {
    let Some(config) = run.config.as_mut() else {
        return;
    };
    config.environment.time_scale = time_scale;
    config.environment.paused = paused;
    log_sandbox(qa, config);
    let command = SandboxCommand::ApplyConfig {
        config: config.clone(),
    };
    run.wire.send(&world.game, sandbox, command, outgoing);
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::loadout::EquippedSkills;

    /// The staging of a skill of a class preset, at its home button.
    fn staged(class: HeroClass, id: &str) -> Staging {
        let kit = EquippedSkills::resolve(class, None).unwrap();
        let slot = SkillSlot::ALL
            .into_iter()
            .find(|slot| kit.ability(*slot).id == id)
            .unwrap_or_else(|| panic!("{id} is not in the kit of {}", class.id()));
        Staging::of(class, &kit, slot, false)
    }

    #[test]
    fn staging_follows_the_catalog() {
        // A plain skillshot: aimed at the target on the lane, and it has to hit.
        let bind = staged(HeroClass::Dawnweaver, "dawn_bind");
        assert_eq!(
            (bind.category, bind.aim, bind.park_orb, bind.settles),
            ("skillshot", Aim::Target, false, None)
        );
        assert_eq!((bind.distance, bind.area), (LANE_LIMIT, 0.0));
        // A leap to open ground is aimed at a free point, not at the caster.
        let step = staged(HeroClass::Stormfist, "anchor_step");
        assert_eq!(
            (step.aim, step.settles),
            (Aim::Ahead(FREE_POINT), Some("no_damage"))
        );
        // A leap to an ally needs one: a wave is walked in.
        let leap = staged(HeroClass::Frostguard, "sheltering_leap");
        assert_eq!((leap.aim, leap.settles), (Aim::Ally, Some("no_damage")));
        // Only a hero can be guarded: the caster. The orb is parked so that it hits on its
        // way home.
        let guard = staged(HeroClass::Orbitwright, "orbital_guard");
        assert_eq!(
            (guard.aim, guard.park_orb, guard.settles),
            (Aim::Caster, true, None)
        );
        assert!(ORB_PARK > guard.distance);
        // Damage that waits for something the stage does not provide.
        for (class, id, reason) in [
            (
                HeroClass::Dawnweaver,
                "dawn_field",
                "damage_on_recast_or_expiry",
            ),
            (HeroClass::Chainkeeper, "iron_boundary", "no_side_touched"),
            (HeroClass::Riftshot, "rift_seal", "mark_not_detonated"),
            (HeroClass::Edgeweaver, "twin_tempo", "no_damage"),
            (HeroClass::Warrior, "battle_rally", "no_damage"),
        ] {
            assert_eq!(staged(class, id).settles, Some(reason), "{id}");
        }
        assert_eq!(staged(HeroClass::Warrior, "shield_bash").settles, None);
        // The area a cast strikes around its caster is what its stills have to show.
        let pulse = staged(HeroClass::Stormfist, "thunder_pulse");
        assert_eq!((pulse.category, pulse.area), ("self_area", 5.0));
        assert_eq!(staged(HeroClass::Chainkeeper, "chain_sweep").area, 6.0);
        assert_eq!(staged(HeroClass::Cinderforge, "anvil_charge").area, 3.0);
        // A cone is not an area around the caster.
        assert_eq!(staged(HeroClass::Veilstalker, "nightfall").area, 0.0);
        assert_eq!(Staging::basic(HeroClass::Mage).settles, None);
    }

    #[test]
    fn the_view_keeps_the_area_of_the_cast_and_the_burst_of_its_hit() {
        let pulse = staged(HeroClass::Stormfist, "thunder_pulse");
        let area = pulse.keep(None);
        assert_eq!(area.len(), 16);
        assert!(area.iter().all(|point| (point.length() - 5.0).abs() < 1e-4));
        // A hit far up the lane, as a kick leaves it.
        let hit = STAGE_LANE * 12.5;
        let kick = staged(HeroClass::Stormfist, "thunder_kick").keep(Some(hit));
        assert_eq!(kick.len(), 8);
        assert!(
            kick.iter()
                .all(|point| (point.distance(hit) - HIT_REACH).abs() < 1e-4)
        );
        assert_eq!(pulse.keep(Some(hit)).len(), 24);
        assert!(
            staged(HeroClass::Dawnweaver, "dawn_bind")
                .keep(None)
                .is_empty()
        );
    }

    #[test]
    fn a_still_is_measured_on_its_stage_crop() {
        let [left, top, right, bottom] = STAGE_CROP;
        let (width, height) = (1280_u32, 720_u32);
        let frame = |paint: &dyn Fn(u32, u32) -> [u8; 3]| -> Vec<u8> {
            (0..height)
                .flat_map(|y| (0..width).flat_map(move |x| paint(x, y)))
                .collect()
        };
        let idle = frame(&|_, _| [90, 90, 90]);
        // A box that reaches over the right edge of the crop and one outside it.
        let cast = frame(&|x, y| {
            if (right - 10..right + 30).contains(&x) && (top..top + 20).contains(&y) {
                [90, 90 + CHANGED_LEVEL + 1, 90]
            } else if x < left && y > bottom {
                [255, 255, 255]
            } else if (left..left + 5).contains(&x) && (bottom - 5..bottom).contains(&y) {
                // At the level, not over it.
                [90 + CHANGED_LEVEL; 3]
            } else {
                [90, 90, 90]
            }
        });
        let crop = stage_crop(&cast, width, height).unwrap();
        assert_eq!(crop.len(), ((right - left) * (bottom - top) * 3) as usize);
        let rest = stage_crop(&idle, width, height).unwrap();
        assert_eq!(changed_pixels(&crop, &rest), 10 * 20);
        assert_eq!(changed_pixels(&rest, &rest), 0);
        // A frame of another size holds no stage.
        assert!(stage_crop(&idle, width, height - 300).is_none());
        assert!(stage_crop(&idle[..idle.len() - 3], width, height).is_none());
        assert_eq!(mean_pixel(&[10, 20, 60]), Some(30.0));
        assert_eq!(mean_pixel(&[]), None);
    }

    #[test]
    fn an_authored_kit_is_what_the_server_admits() {
        let kit = authored_kit(
            HeroClass::Stormfist,
            "dawn_ray,winter_shard,furnace_breath,wandering_ember",
        )
        .unwrap();
        assert_eq!(kit.core.class(), HeroClass::Stormfist);
        assert_eq!(
            kit.skills.map(SkillId::id),
            [
                "dawn_ray",
                "winter_shard",
                "furnace_breath",
                "wandering_ember"
            ]
        );
        // The passive stays that of the core.
        assert_eq!(
            kit.passive,
            shared::loadout::preset_for_class(HeroClass::Stormfist)
                .unwrap()
                .passive()
        );
        for (class, ids) in [
            // A legacy class has no core to author a kit on.
            (
                HeroClass::Warrior,
                "dawn_ray,winter_shard,furnace_breath,wandering_ember",
            ),
            (HeroClass::Stormfist, "dawn_ray,winter_shard,furnace_breath"),
            (
                HeroClass::Stormfist,
                "dawn_ray,dawn_ray,furnace_breath,wandering_ember",
            ),
            (
                HeroClass::Stormfist,
                "dawn_ray,shield_bash,furnace_breath,wandering_ember",
            ),
            // The field needs a skill that moves the orb.
            (
                HeroClass::Stormfist,
                "dawn_ray,orbital_field,furnace_breath,wandering_ember",
            ),
        ] {
            assert!(authored_kit(class, ids).is_err(), "{ids}");
        }
    }

    #[test]
    fn a_telegraph_fires_by_its_kind_flip_or_by_leaving() {
        let track = |seen: f64, telegraph: bool, flipped: Option<f64>, gone: Option<f64>| Track {
            seen,
            kind: EffectVisualKind::BeamWarning,
            armed: false,
            remaining: 1.0,
            telegraph,
            flipped,
            gone,
        };
        let mut watch = CastWatch::default();
        assert_eq!(watch.telegraph(), None);
        assert!(watch.fuse().is_none());
        // A warning that became its beam, and a later effect that is no telegraph.
        watch
            .tracks
            .insert(1, track(10.1, true, Some(10.9), Some(11.05)));
        watch.tracks.insert(2, track(10.0, false, None, Some(10.4)));
        assert_eq!(watch.telegraph(), Some(10.1));
        assert_eq!(watch.fuse(), Some((10.1, 10.9, "kind_flip")));
        // A fuse fires when its effect leaves the snapshot; the first one to fire counts.
        watch.tracks.insert(3, track(10.2, true, None, Some(10.7)));
        assert_eq!(watch.fuse(), Some((10.2, 10.7, "fuse_gone")));
        // A telegraph that is still burning has not fired.
        let mut burning = CastWatch::default();
        burning.tracks.insert(1, track(4.0, true, None, None));
        assert_eq!(burning.telegraph(), Some(4.0));
        assert!(burning.fuse().is_none());
        // The windup still of a skill without a telegraph is taken on the fresh edge.
        assert!(EDGE_WINDOW < RELEASE_WINDOW.0);
    }
}
