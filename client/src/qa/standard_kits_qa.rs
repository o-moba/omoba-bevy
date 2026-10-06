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
    collections::{BTreeMap, HashSet},
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
            zoom: crate::camera::CAMERA_MIN_ZOOM,
            ..default()
        };
        camera_settings.zoom = crate::camera::CAMERA_MIN_ZOOM;
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
    let mean = mean_pixel(&event.image);
    if let Some(capture) = qa
        .captures
        .iter_mut()
        .find(|capture| capture["file"] == shot.file.as_str())
    {
        capture["mean_pixel"] = mean.into();
    }
    // A locked or hidden desktop presents nothing: such a frame is not evidence.
    if mean == Some(0.0) {
        qa.black = Some(shot.file.clone());
    } else {
        qa.readbacks.push(shot.index);
    }
}
/// Mean of the colour channels (0 to 255), or `None` for a format the PNG
/// writer cannot convert either.
fn mean_pixel(image: &Image) -> Option<f64> {
    let rgb = image.clone().try_into_dynamic().ok()?.to_rgb8();
    let bytes = rgb.as_raw();
    (!bytes.is_empty())
        .then(|| bytes.iter().map(|&value| f64::from(value)).sum::<f64>() / bytes.len() as f64)
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
}
impl Phase {
    fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Windup => "windup",
            Self::Release => "release",
            Self::Impact => "impact",
            Self::Settled => "settled",
        }
    }
    /// Position of the still in the skill's row of three.
    fn order(self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Windup => 1,
            Self::Release => 2,
            Self::Impact | Self::Settled => 3,
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
    /// Reset acknowledged: wait until nothing of an earlier cast is left.
    Settle,
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
    /// Pause requested for a still.
    Pause(Phase),
    /// Screenshot taken: wait for its read-back.
    Read(Phase),
    /// Resume requested after a still.
    Resume,
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

/// Where the target stands for one skill, derived from the catalog.
#[derive(Clone, Copy)]
struct Staging {
    category: &'static str,
    distance: f32,
    /// Ally-only casts are aimed at the caster.
    on_caster: bool,
    can_damage: bool,
}
impl Staging {
    fn of(class: HeroClass, equipped: &shared::loadout::EquippedSkills, slot: SkillSlot) -> Self {
        let ability = equipped.ability(slot);
        let effect = equipped.skill(slot).map(|skill| skill.effect);
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
            TargetingMode::UnitTarget if shared::basic_attack_for_class(class).range <= 5.0 => {
                ("pick_or_melee", CLOSE_RANGE)
            }
            TargetingMode::UnitTarget => ("skillshot", lane),
            TargetingMode::SelfTarget if radius > CLOSE_RANGE => {
                ("self_area", CLOSE_RANGE.min(radius * 0.6))
            }
            TargetingMode::SelfTarget => ("self", CLOSE_RANGE),
        };
        Self {
            category,
            distance,
            on_caster: matches!(technique, Some(Technique::AllyLeap | Technique::BallGuard)),
            can_damage: match effect {
                Some(effect) => effect.damage().is_some_and(|damage| damage > 0.0),
                None => ability.projectile_damage.is_some(),
            },
        }
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
            })),
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
}
impl Phases {
    fn enter(&mut self, step: Step) {
        self.step = step;
        self.entered = Some(Instant::now());
        self.frames = 0;
    }
    fn begin_pass(&mut self, pass: Pass) {
        self.pass = pass;
        self.windup_taken = false;
        self.release_taken = None;
        self.third_taken = false;
        self.enter(Step::Arrange);
    }
}

#[derive(SystemParam)]
struct PhaseWorld<'w, 's> {
    game: Res<'w, GameStateSnapshot>,
    mode: Res<'w, crate::sprite::PlayerVisualMode>,
    registry: Res<'w, crate::skill_presentation::SkillPresentation>,
    animations: Res<'w, crate::sandbox::AnimationReadout>,
    context: Res<'w, crate::input_context::GameplayInputContext>,
    map: Res<'w, crate::maps::MapLayout>,
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
}
impl PhaseWorld<'_, '_> {
    fn actor(&self, actor: SandboxActor) -> Option<&shared::sandbox::ActorTelemetry> {
        self.game
            .sandbox
            .as_ref()?
            .actors
            .iter()
            .find(|telemetry| telemetry.actor == actor)
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

fn stage_config(class: HeroClass, avatar: &str) -> SandboxConfig {
    let mut config = SandboxConfig {
        player: ActorConfig {
            hero: class,
            avatar: Some(avatar.into()),
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
    on_caster: bool,
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
        let origin = Vec2::from_array(hero.position);
        let delta = Vec2::from_array(enemy.position) - origin;
        let aim = if on_caster {
            origin
        } else {
            origin
                + delta.normalize_or(STAGE_LANE) * delta.length().min(ability.cast_range.max(0.1))
        };
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

/// The state of one still, read in the frame its screenshot is taken.
fn still_record(
    world: &PhaseWorld,
    run: &Phases,
    phase: Phase,
    sandbox: &SandboxSnapshot,
    skill: &str,
) -> serde_json::Value {
    let me = world.game.your_id;
    let particles = world.live_particles();
    let edge = run.watch.edge.filter(|_| phase != Phase::Idle);
    let receipt = run.watch.receipt.as_ref().filter(|_| phase != Phase::Idle);
    let of_event = |id: u64| particles.iter().filter(move |(event, _)| *event == id);
    let youngest = |id: u64| of_event(id).map(|(_, age)| *age).min_by(f32::total_cmp);
    let roots: Vec<_> = world
        .vfx
        .iter()
        .map(|(root, name, visibility, visual)| {
            let parts: Vec<_> = world
                .parts
                .iter()
                .filter(|(parent, ..)| parent.parent() == root)
                .collect();
            let shown = *visibility != Visibility::Hidden;
            serde_json::json!({
                "name": name.as_str(),
                "effect_id": visual.id,
                "visible": shown,
                "model_ready": visual.model_ready,
                "parts": parts.len(),
                "visible_parts": parts.iter()
                    .filter(|(_, visibility, ..)| shown && **visibility != Visibility::Hidden)
                    .count(),
                "lights": parts.iter().filter(|(_, _, point, spot)| *point || *spot).count(),
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
                .action_profile(class.0, loadout.0.as_ref(), run.slot)
        });
    serde_json::json!({
        "phase": phase.name(),
        "gate": run.gate,
        "slot": (phase != Phase::Idle).then_some(run.slot),
        "skill": (phase != Phase::Idle).then_some(skill),
        "snapshot_tick": world.game.meta.snapshot_tick,
        "simulation_secs": sandbox.simulation_secs,
        "time_scale": sandbox.config.environment.time_scale,
        "paused": sandbox.config.environment.paused,
        "since_edge_secs": edge.map(|(at, _)| sandbox.simulation_secs - at),
        "animation": world.animation(),
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
            "impact_age_secs": receipt.and_then(|receipt| youngest(receipt.id)),
        },
        "receipt": receipt.map(|receipt| serde_json::json!({
            "id": receipt.id,
            "amount": receipt.amount,
            "target": receipt.target,
            "since_seen_secs": sandbox.simulation_secs - receipt.seen,
        })),
        "damage_numbers": world.numbers.iter().count(),
        "effects": world.game.skill_effects.iter()
            .filter(|effect| effect.owner_id == me)
            .collect::<Vec<&SkillEffectState>>(),
        "skill_vfx": roots,
        "effect_parts": total("parts"),
        "effect_visible_parts": total("visible_parts"),
        "effect_lights": total("lights"),
        "hero_pixels": world.actor(SandboxActor::Player)
            .and_then(|actor| world.pixels(actor.position)),
        "target_pixels": world.actor(SandboxActor::Enemy)
            .and_then(|actor| world.pixels(actor.position)),
    })
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
            Step::Edge => "the server to accept the cast",
            Step::Gates => "the gate of the next still",
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
    let skill = equipped.ability(slot).id;
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
        Step::Edge | Step::Probe | Step::Gates | Step::Pause(_) | Step::Read(_) | Step::Resume
    ) {
        run.watch.observe(&world.game, action, now);
    }
    match step {
        Step::Arrange => {
            let staging = Staging::of(class.0, &equipped, slot);
            let mut config = run
                .config
                .take()
                .unwrap_or_else(|| stage_config(qa.class, &qa.avatar));
            config.enemy.actor.position = (STAGE_HOME + STAGE_LANE * staging.distance).to_array();
            config.environment.time_scale = 1.0;
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
            if run.pass == Pass::Probe {
                run.enter(Step::Cast);
            } else if !run.idle_taken {
                run.gate = "clean_slate";
                let record = still_record(&world, run, Phase::Idle, sandbox, skill);
                let index = PHASE_SHOT + run.stills;
                shoot(&mut commands, qa, index, "0-idle.png".into(), record);
                run.enter(Step::Read(Phase::Idle));
            } else {
                request_speed(qa, run, &world, sandbox, SLOW_MOTION, false, &mut outgoing);
                run.enter(Step::Slow);
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
                slot: run.slot,
                skill: equipped.skill(slot).map(|definition| definition.id),
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
            if let Err(reason) = send_cast(
                qa,
                &world,
                &equipped,
                run.slot,
                staging.on_caster,
                &mut outgoing,
            ) {
                stop(qa, run, &mut exit, reason.into());
                return;
            }
            run.enter(Step::Edge);
        }
        Step::Edge if run.watch.edge.is_some() => {
            let next = if run.pass == Pass::Probe {
                Step::Probe
            } else {
                Step::Gates
            };
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
                    if !staging.can_damage {
                        "no_damage"
                    } else if expected.is_none() {
                        "no_receipt_in_probe"
                    } else {
                        "no_receipt_observed"
                    },
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
            let file = format!(
                "{}-{}-{}-{}.png",
                run.slot + 1,
                ["q", "w", "e", "r"][usize::from(run.slot)],
                phase.order(),
                phase.name()
            );
            run.skill_stills.push(file.clone());
            let index = PHASE_SHOT + run.stills;
            shoot(&mut commands, qa, index, file, record);
            run.enter(Step::Read(phase));
        }
        Step::Read(phase) if qa.readbacks.contains(&(PHASE_SHOT + run.stills)) => {
            run.stills += 1;
            match phase {
                Phase::Idle => run.idle_taken = true,
                Phase::Windup => run.windup_taken = true,
                Phase::Release => run.release_taken = Some(now),
                Phase::Impact | Phase::Settled => run.third_taken = true,
            }
            if phase == Phase::Idle {
                request_speed(qa, run, &world, sandbox, SLOW_MOTION, false, &mut outgoing);
                run.enter(Step::Slow);
            } else if !(run.windup_taken && run.release_taken.is_some() && run.third_taken) {
                request_speed(qa, run, &world, sandbox, SLOW_MOTION, false, &mut outgoing);
                run.enter(Step::Resume);
            } else {
                let staging = run.staging;
                let probe = run.probe.take();
                let stills = std::mem::take(&mut run.skill_stills);
                let watch = run.watch.timeline();
                let entry = serde_json::json!({
                    "slot": run.slot,
                    "skill": skill,
                    "targeting": format!("{:?}", equipped.ability(slot).targeting),
                    "cast_range": equipped.ability(slot).cast_range,
                    "staging": staging.map(|staging| serde_json::json!({
                        "category": staging.category,
                        "target_distance": staging.distance,
                        "cast_on_caster": staging.on_caster,
                        "can_damage": staging.can_damage,
                    })),
                    "probe": probe.as_ref().map(CastWatch::timeline),
                    "capture": watch,
                    "stills": stills,
                });
                run.skills.push(entry);
                if run.slot < 3 {
                    run.slot += 1;
                    run.begin_pass(Pass::Probe);
                    return;
                }
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
                    "manual_interaction_verified": false,
                    "physical_device_verified": false,
                    "setup": "live sandbox; level 10; rank 1; infinite resource, normal cooldowns; ResetDuel before every cast; one probe cast at 1x, then one cast at 0.25x paused for each still; stationary enemy hero with 1,000,000 HP that takes hits; ally-only casts target the caster",
                    "registry_profiles": world.registry.profile_count(),
                    "skills": run.skills,
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
        Step::Resume if acknowledged && !sandbox.config.environment.paused => {
            run.enter(Step::Gates);
        }
        _ => {}
    }
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
