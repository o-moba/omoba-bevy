//! Focused, opt-in native captures using live sandbox setup and normal skill commands.
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
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use shared::{
    HeroClass,
    loadout::{EffectVisualKind, WeaponMode},
    sandbox::{ActorConfig, SandboxCommand, SandboxConfig, SandboxRequest},
};
use std::{
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
        app.insert_resource(Qa {
            directory,
            class,
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
            (
                super::combat_qa::focus_capture_window,
                prepare
                    .after(bevy::ui::UiSystems::Focus)
                    .after(bevy::input::InputSystems),
            ),
        )
        .add_systems(
            PostUpdate,
            observe
                .after(bevy::ui::UiSystems::Layout)
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}
#[derive(Resource)]
struct Qa {
    directory: PathBuf,
    class: HeroClass,
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
fn label(mut commands: Commands, qa: Res<Qa>) {
    commands.spawn((
        Text::new(if qa.hud {
            "QA · live practice · authoritative bots and kill notices"
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
    if qa.weapons {
        // Use the closest supported gameplay view for grip inspection.
        *camera = crate::camera::CameraState {
            zoom: crate::camera::CAMERA_MIN_ZOOM,
            ..default()
        };
        camera_settings.zoom = crate::camera::CAMERA_MIN_ZOOM;
    }
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        let (width, height) = if qa.ux { (1180, 820) } else { (1280, 720) };
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
        selection.avatar = Some("agnes".into());
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
    if qa.stage == 255 {
        return;
    }
    if qa.started.elapsed() > Duration::from_secs(120) {
        fail(
            &mut qa,
            &mut exit,
            "Timed out waiting for live server state or screenshot readback",
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
                    fail(&mut qa, &mut exit, "Cannot write roster evidence");
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
                fail(&mut qa, &mut exit, "Cannot write evidence summary");
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
struct Shot(usize);
fn capture(commands: &mut Commands, qa: &mut Qa, index: usize, mut frame: serde_json::Value) {
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
    frame["file"] = file.into();
    qa.captures.push(frame);
    commands
        .spawn((Screenshot::primary_window(), Shot(index)))
        .observe(save_to_disk(qa.directory.join(file)))
        .observe(readback);
}
fn readback(event: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<Qa>) {
    if let Ok(shot) = shots.get(event.entity)
        && event.image.width() == if qa.ux { 1180 } else { 1280 }
        && event.image.height() == if qa.ux { 820 } else { 720 }
        && !qa.readbacks.contains(&shot.0)
    {
        qa.readbacks.push(shot.0);
    }
}
fn fail(qa: &mut Qa, exit: &mut MessageWriter<AppExit>, reason: &str) {
    let _=std::fs::write(qa.directory.join("qa-failure.json"),serde_json::to_vec_pretty(&serde_json::json!({"stage":qa.stage,"reason":reason,"captures":qa.captures,"requests":qa.requests})).unwrap());
    error!("STANDARD_KITS_QA stage={}: {reason}", qa.stage);
    qa.stage = 255;
    exit.write(AppExit::error());
}
