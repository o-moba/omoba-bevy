use super::animation::{
    AvatarKey, CharacterAnimationSet, HeroAnimationPlayback, HeroAnimationState,
    bind_player_animation_players, sandbox_available_animations, sandbox_preview_state,
    sandbox_seek_time, start_hero_animation, sync_player_animation_state,
};
use super::motion::SandboxVisualClock;
use super::respawn_ui::RespawnCountdownText;
use super::*;
use crate::combat::CombatStats;
use crate::debug::DebugConsole;
use crate::net::{
    GameState, GameStateSnapshot, NetworkAvatar, NetworkCharacterChoice, PlayerCosmeticAction,
    RemotePlayer,
};
use crate::team::{CharacterChoice, Team};
use shared::PlayerActionKind;

fn action(sequence: u64, kind: PlayerActionKind) -> PlayerCosmeticAction {
    PlayerCosmeticAction {
        sequence,
        kind,
        slot: 0,
    }
}

#[test]
fn skill_motion_waits_for_phase_deduplicates_and_respects_cancel_death_respawn() {
    use shared::loadout::{CoreId, EffectVisualKind, LoadoutState, SkillEffectState, SkillId};
    let mut app = App::new();
    let mut set = animation_set();
    set.motion_nodes
        .push(("spell_prepare".into(), set.attack_node.unwrap()));
    let mut library = PlayerAnimationLibrary::default();
    library
        .sets
        .insert(AvatarKey::Roster("agnes".into()), set.clone());
    let profiles: crate::skill_presentation::SkillPresentation =
        serde_json::from_str(include_str!("../../assets/config/skills.skillfx")).unwrap();
    app.insert_resource(Time::<()>::default())
        .insert_resource(library)
        .insert_resource(profiles)
        .init_resource::<GameStateSnapshot>()
        .add_systems(
            Update,
            (bind_player_animation_players, sync_player_animation_state).chain(),
        );
    // Deliberately move R into Q in an already accepted recipe: presentation is skill-owned.
    let mut recipe = CoreId::Wildspark.preset();
    recipe.skills[0] = SkillId::DawnRay;
    let owner = app
        .world_mut()
        .spawn((
            Player,
            Transform::default(),
            CombatStats::default(),
            NetworkCharacterChoice(CharacterChoice::Cube),
            NetworkAvatar(Some("agnes".into())),
            crate::net::NetworkPlayerId(7),
            crate::net::NetworkHeroClass(shared::HeroClass::Wildspark),
            PlayerCosmeticAction::default(),
            crate::net::PlayerLoadout(Some(LoadoutState {
                recipe: Some(recipe),
                ..default()
            })),
        ))
        .id();
    let child = app
        .world_mut()
        .spawn((AnimationPlayer::default(), ChildOf(owner)))
        .id();
    let state = |app: &App| {
        app.world()
            .get::<PlayerAnimationBinding>(child)
            .unwrap()
            .playback
            .state
    };
    app.update();
    app.world_mut()
        .entity_mut(owner)
        .insert(action(1, PlayerActionKind::Attack));
    app.update();
    assert_eq!(
        state(&app),
        HeroAnimationState::Idle,
        "no guessed release before warning arrives"
    );
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .skill_effects
        .push(SkillEffectState {
            id: 9,
            owner_id: 7,
            owner_team: shared::map::Team::Green,
            skill: SkillId::DawnRay,
            kind: EffectVisualKind::BeamWarning,
            position: [0.0; 2],
            end: [10.0, 0.0],
            radius: 0.8,
            remaining_secs: 0.8,
            armed: false,
            consumed_segments: 0,
        });
    app.update();
    assert_eq!(state(&app), HeroAnimationState::Motion(0));
    app.world_mut()
        .get_mut::<AnimationPlayer>(child)
        .unwrap()
        .animation_mut(set.attack_node.unwrap())
        .unwrap()
        .seek_to(0.4);
    app.update();
    assert_eq!(
        app.world()
            .get::<AnimationPlayer>(child)
            .unwrap()
            .animation(set.attack_node.unwrap())
            .unwrap()
            .seek_time(),
        0.4
    );
    let mut beam = app
        .world_mut()
        .resource_mut::<GameStateSnapshot>()
        .skill_effects
        .remove(0);
    app.update();
    assert_eq!(
        state(&app),
        HeroAnimationState::Idle,
        "cancelled preparation returns to locomotion"
    );
    beam.kind = EffectVisualKind::Beam;
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .skill_effects
        .push(beam);
    app.update();
    assert_eq!(
        state(&app),
        HeroAnimationState::Cast,
        "release requires a server beam"
    );
    app.world_mut().get_mut::<CombatStats>(owner).unwrap().hp = 0.0;
    app.world_mut()
        .entity_mut(owner)
        .insert(action(2, PlayerActionKind::Attack));
    app.update();
    assert_eq!(state(&app), HeroAnimationState::Death);
    app.world_mut().get_mut::<CombatStats>(owner).unwrap().hp = 100.0;
    app.world_mut()
        .entity_mut(owner)
        .insert(action(3, PlayerActionKind::Attack));
    app.update();
    app.update();
    assert_eq!(
        state(&app),
        HeroAnimationState::Idle,
        "stale phase must not replay after respawn"
    );
}

#[test]
fn combat_animation_transitions_deduplicate_restart_and_hold_death() {
    let mut playback = HeroAnimationPlayback::new(0);
    let attack = action(1, PlayerActionKind::Attack);
    assert!(playback.advance(true, true, attack, false, |_| true));
    assert_eq!(playback.state, HeroAnimationState::Attack);
    assert!(!playback.advance(true, true, attack, false, |_| true));
    assert!(playback.advance(
        true,
        true,
        action(2, PlayerActionKind::Attack),
        false,
        |_| true
    ));
    assert_eq!(playback.state, HeroAnimationState::Attack);
    assert!(
        playback.advance(true, true, action(3, PlayerActionKind::Cast), false, |_| {
            true
        })
    );
    assert_eq!(playback.state, HeroAnimationState::Cast);
    assert!(
        playback.advance(true, true, action(3, PlayerActionKind::Cast), true, |_| {
            true
        })
    );
    assert_eq!(playback.state, HeroAnimationState::Run);
    assert!(playback.advance(false, false, attack, true, |_| true));
    assert_eq!(playback.state, HeroAnimationState::Death);
    assert!(
        !playback.advance(false, true, action(4, PlayerActionKind::Cast), true, |_| {
            true
        })
    );
    assert_eq!(playback.state, HeroAnimationState::Death);
    assert!(
        playback.advance(true, false, action(4, PlayerActionKind::Cast), true, |_| {
            true
        })
    );
    assert_eq!(playback.state, HeroAnimationState::Idle);
}

#[test]
fn new_round_action_rearms_even_when_initial_zero_snapshot_was_lost() {
    let mut playback = HeroAnimationPlayback::new(42);
    assert!(!playback.observe_round((100, 1)));
    playback.state = HeroAnimationState::Death;
    playback.alive = false;
    assert!(playback.observe_round((100, 2)));
    assert!(playback.advance(
        true,
        false,
        action(1, PlayerActionKind::Cast),
        false,
        |_| true
    ));
    assert_eq!(playback.state, HeroAnimationState::Cast);
    assert!(!playback.observe_round((100, 2)));
    assert!(!playback.advance(
        true,
        false,
        action(1, PlayerActionKind::Cast),
        false,
        |_| true
    ));
    assert!(playback.observe_round((200, 1)));
    assert!(playback.advance(
        true,
        false,
        action(1, PlayerActionKind::Attack),
        false,
        |_| true
    ));
    assert_eq!(playback.state, HeroAnimationState::Attack);
}

#[test]
fn absent_action_clip_falls_back_and_round_zero_rearms_sequences() {
    let mut playback = HeroAnimationPlayback::new(10);
    assert!(!playback.advance(
        true,
        false,
        action(11, PlayerActionKind::Cast),
        false,
        |_| false
    ));
    assert_eq!(playback.state, HeroAnimationState::Idle);
    assert!(!playback.advance(
        true,
        false,
        action(9, PlayerActionKind::Attack),
        false,
        |_| true
    ));
    playback.advance(true, false, PlayerCosmeticAction::default(), true, |_| true);
    assert!(playback.advance(
        true,
        false,
        action(1, PlayerActionKind::Attack),
        false,
        |_| true
    ));
    assert_eq!(playback.state, HeroAnimationState::Attack);
    playback.advance(
        false,
        false,
        action(1, PlayerActionKind::Attack),
        true,
        |_| false,
    );
    assert_eq!(playback.state, HeroAnimationState::Death);
}

fn animation_set() -> CharacterAnimationSet {
    let (_, nodes) = AnimationGraph::from_clips([
        Handle::<AnimationClip>::default(),
        Handle::default(),
        Handle::default(),
        Handle::default(),
        Handle::default(),
    ]);
    CharacterAnimationSet {
        graph: Handle::default(),
        idle_node: nodes[0],
        run_node: nodes[1],
        walk_node: None,
        runtime: false,
        attack_node: Some(nodes[2]),
        cast_node: Some(nodes[3]),
        death_node: Some(nodes[4]),
        motion_nodes: Vec::new(),
    }
}

#[test]
fn local_and_remote_scene_players_play_one_shots_once_and_respawn() {
    let mut app = App::new();
    let set = animation_set();
    let mut library = PlayerAnimationLibrary::default();
    library
        .sets
        .insert(AvatarKey::Roster("agnes".to_owned()), set.clone());
    app.insert_resource(Time::<()>::default())
        .insert_resource(library)
        .add_systems(
            Update,
            (bind_player_animation_players, sync_player_animation_state).chain(),
        );
    let mut entities = Vec::new();
    for local in [true, false] {
        let owner = app
            .world_mut()
            .spawn((
                Transform::default(),
                CombatStats::default(),
                NetworkCharacterChoice(CharacterChoice::Cube),
                NetworkAvatar(Some("agnes".to_owned())),
                PlayerCosmeticAction::default(),
            ))
            .id();
        if local {
            app.world_mut().entity_mut(owner).insert(Player);
        } else {
            app.world_mut().entity_mut(owner).insert(RemotePlayer);
        }
        let child = app
            .world_mut()
            .spawn((AnimationPlayer::default(), ChildOf(owner)))
            .id();
        entities.push((owner, child));
    }
    app.update();
    for (owner, _) in &entities {
        app.world_mut()
            .entity_mut(*owner)
            .insert(action(1, PlayerActionKind::Attack));
    }
    app.update();
    for (_, child) in &entities {
        let player = app.world().get::<AnimationPlayer>(*child).unwrap();
        let active = player.animation(set.attack_node.unwrap()).unwrap();
        assert_eq!(
            active.repeat_mode(),
            bevy::animation::RepeatAnimation::Never
        );
        app.world_mut()
            .get_mut::<AnimationPlayer>(*child)
            .unwrap()
            .animation_mut(set.attack_node.unwrap())
            .unwrap()
            .seek_to(0.3);
    }
    app.update();
    for (owner, child) in &entities {
        assert_eq!(
            app.world()
                .get::<AnimationPlayer>(*child)
                .unwrap()
                .animation(set.attack_node.unwrap())
                .unwrap()
                .seek_time(),
            0.3
        );
        app.world_mut()
            .entity_mut(*owner)
            .insert(action(2, PlayerActionKind::Cast));
    }
    app.update();
    for (owner, child) in &entities {
        assert!(
            app.world()
                .get::<AnimationPlayer>(*child)
                .unwrap()
                .is_playing_animation(set.cast_node.unwrap())
        );
        app.world_mut().get_mut::<CombatStats>(*owner).unwrap().hp = 0.0;
    }
    app.update();
    for (_, child) in &entities {
        let mut player = app.world_mut().get_mut::<AnimationPlayer>(*child).unwrap();
        let active = player.animation_mut(set.death_node.unwrap()).unwrap();
        assert_eq!(
            active.repeat_mode(),
            bevy::animation::RepeatAnimation::Never
        );
        active.seek_to(0.9);
    }
    app.update();
    for (owner, child) in &entities {
        assert_eq!(
            app.world()
                .get::<AnimationPlayer>(*child)
                .unwrap()
                .animation(set.death_node.unwrap())
                .unwrap()
                .seek_time(),
            0.9
        );
        app.world_mut().get_mut::<CombatStats>(*owner).unwrap().hp = 100.0;
    }
    app.update();
    for (_, child) in &entities {
        assert!(
            app.world()
                .get::<AnimationPlayer>(*child)
                .unwrap()
                .is_playing_animation(set.idle_node)
        );
    }
}

#[test]
fn both_movement_paths_run_and_remote_pauses_keep_locomotion_stable() {
    let mut app = App::new();
    let set = animation_set();
    let mut library = PlayerAnimationLibrary::default();
    library
        .sets
        .insert(AvatarKey::Roster("agnes".into()), set.clone());
    app.insert_resource(Time::<()>::default())
        .insert_resource(library)
        .add_systems(
            Update,
            (bind_player_animation_players, sync_player_animation_state).chain(),
        );
    let mut actors = Vec::new();
    for local in [true, false] {
        let owner = app
            .world_mut()
            .spawn((
                Transform::default(),
                CombatStats::default(),
                NetworkCharacterChoice(CharacterChoice::Cube),
                NetworkAvatar(Some("agnes".into())),
                PlayerCosmeticAction::default(),
            ))
            .id();
        if local {
            app.world_mut().entity_mut(owner).insert(Player);
        } else {
            app.world_mut().entity_mut(owner).insert(RemotePlayer);
        }
        let child = app
            .world_mut()
            .spawn((AnimationPlayer::default(), ChildOf(owner)))
            .id();
        actors.push((owner, child));
    }
    app.update();
    app.world_mut()
        .entity_mut(actors[0].0)
        .insert(MovementTarget { target: Vec3::X });
    app.world_mut()
        .get_mut::<Transform>(actors[1].0)
        .unwrap()
        .translation
        .x = 0.1;
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs_f32(0.1));
    app.update();
    for (_, child) in &actors {
        assert!(
            app.world()
                .get::<PlayerAnimationBinding>(*child)
                .unwrap()
                .is_running()
        );
        assert!(
            app.world()
                .get::<AnimationPlayer>(*child)
                .unwrap()
                .is_playing_animation(set.run_node)
        );
    }
    // A still interpolation frame must not flash to idle.
    app.update();
    assert!(
        app.world()
            .get::<PlayerAnimationBinding>(actors[1].1)
            .unwrap()
            .is_running()
    );
    app.world_mut()
        .entity_mut(actors[0].0)
        .remove::<MovementTarget>();
    for _ in 0..4 {
        app.update();
    }
    for (_, child) in &actors {
        assert_eq!(
            app.world()
                .get::<PlayerAnimationBinding>(*child)
                .unwrap()
                .playback
                .state,
            HeroAnimationState::Idle
        );
    }
}

#[test]
fn unavailable_death_uses_a_paused_safe_pose() {
    let mut set = animation_set();
    set.death_node = None;
    let mut player = AnimationPlayer::default();
    start_hero_animation(&mut player, &set, HeroAnimationState::Death);
    assert!(player.animation(set.idle_node).unwrap().is_paused());
}

fn sandbox_animation_app() -> (App, CharacterAnimationSet, Vec<(Entity, Entity)>) {
    let mut clips = Assets::<AnimationClip>::default();
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let mut clip = AnimationClip::default();
            clip.set_duration(1.0);
            clips.add(clip)
        })
        .collect();
    let (graph, nodes) = AnimationGraph::from_clips(handles);
    let mut graphs = Assets::<AnimationGraph>::default();
    let set = CharacterAnimationSet {
        graph: graphs.add(graph),
        idle_node: nodes[0],
        run_node: nodes[1],
        walk_node: Some(nodes[2]),
        runtime: false,
        attack_node: Some(nodes[3]),
        cast_node: Some(nodes[4]),
        death_node: Some(nodes[5]),
        motion_nodes: Vec::new(),
    };
    let mut library = PlayerAnimationLibrary::default();
    library
        .sets
        .insert(AvatarKey::Roster("agnes".into()), set.clone());
    let game = GameStateSnapshot {
        meta: shared::protocol::SnapshotMeta::new(77, 1, 1),
        sandbox: Some(shared::sandbox::SandboxSnapshot {
            config: Default::default(),
            ack: None,
            last_request_id: 0,
            actors: Vec::new(),
            analytics: Default::default(),
            simulation_secs: 0.0,
            frame: 0,
        }),
        ..Default::default()
    };
    let mut app = App::new();
    app.insert_resource(Time::<()>::default())
        .insert_resource(library)
        .insert_resource(graphs)
        .insert_resource(clips)
        .insert_resource(game)
        .init_resource::<crate::sandbox::SandboxClient>()
        .init_resource::<crate::sandbox::AnimationReadout>()
        .add_systems(
            Update,
            (bind_player_animation_players, sync_player_animation_state).chain(),
        );
    let mut entities = Vec::new();
    for id in 1..=2 {
        let owner = app
            .world_mut()
            .spawn((
                Transform::default(),
                CombatStats::default(),
                NetworkCharacterChoice(CharacterChoice::Cube),
                NetworkAvatar(Some("agnes".into())),
                crate::net::NetworkPlayerId(id),
                PlayerCosmeticAction::default(),
            ))
            .id();
        if id == 1 {
            app.world_mut().entity_mut(owner).insert(Player);
        } else {
            app.world_mut().entity_mut(owner).insert(RemotePlayer);
        }
        let child = app
            .world_mut()
            .spawn((AnimationPlayer::default(), ChildOf(owner)))
            .id();
        entities.push((owner, child));
    }
    app.update();
    (app, set, entities)
}
fn preview(app: &mut App, id: u64, kind: crate::sandbox::PreviewKind, sequence: u64) {
    app.world_mut()
        .resource_mut::<crate::sandbox::SandboxClient>()
        .preview = Some(crate::sandbox::Preview { id, kind, sequence });
}
#[test]
fn sandbox_previews_target_only_selected_actor_and_repeated_action_restarts() {
    use crate::sandbox::PreviewKind;
    let (mut app, set, actors) = sandbox_animation_app();
    for (index, kind) in [
        PreviewKind::Idle,
        PreviewKind::Run,
        PreviewKind::Walk,
        PreviewKind::Attack,
        PreviewKind::Cast,
        PreviewKind::Hit,
        PreviewKind::Death,
    ]
    .into_iter()
    .enumerate()
    {
        preview(&mut app, 1, kind, index as u64 + 1);
        app.update();
        let (expected, _) = sandbox_preview_state(kind, &set);
        assert!(
            app.world()
                .get::<AnimationPlayer>(actors[0].1)
                .unwrap()
                .is_playing_animation(set.node(expected))
        );
        assert!(
            app.world()
                .get::<AnimationPlayer>(actors[1].1)
                .unwrap()
                .is_playing_animation(set.idle_node)
        );
        let readout = app.world().resource::<crate::sandbox::AnimationReadout>();
        assert_eq!(readout.0[&1].1.len(), 6);
        assert!(readout.0[&1].1.iter().all(|s| !s.contains("Hit")));
        if kind == PreviewKind::Hit {
            assert!(readout.0[&1].0.contains("Hit → Attack"));
        }
    }
    preview(&mut app, 2, PreviewKind::Attack, 20);
    app.update();
    app.world_mut()
        .get_mut::<AnimationPlayer>(actors[1].1)
        .unwrap()
        .animation_mut(set.attack_node.unwrap())
        .unwrap()
        .seek_to(0.7);
    app.update();
    assert_eq!(
        app.world()
            .get::<AnimationPlayer>(actors[1].1)
            .unwrap()
            .animation(set.attack_node.unwrap())
            .unwrap()
            .seek_time(),
        0.7
    );
    preview(&mut app, 2, PreviewKind::Attack, 21);
    app.update();
    assert_eq!(
        app.world()
            .get::<AnimationPlayer>(actors[1].1)
            .unwrap()
            .animation(set.attack_node.unwrap())
            .unwrap()
            .seek_time(),
        0.0
    );
    assert!(
        app.world()
            .get::<AnimationPlayer>(actors[0].1)
            .unwrap()
            .is_playing_animation(set.idle_node)
    );
}
#[test]
fn sandbox_pause_steps_real_graph_once_and_preserves_one_shot_end_pose() {
    use crate::sandbox::PreviewKind;
    let (mut app, set, actors) = sandbox_animation_app();
    preview(&mut app, 1, PreviewKind::Attack, 1);
    app.update();
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .sandbox
        .as_mut()
        .unwrap()
        .config
        .environment
        .paused = true;
    app.update();
    assert!(
        app.world()
            .get::<AnimationPlayer>(actors[0].1)
            .unwrap()
            .all_paused()
    );
    assert!(
        app.world()
            .get::<AnimationPlayer>(actors[1].1)
            .unwrap()
            .all_paused()
    );
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .sandbox
        .as_mut()
        .unwrap()
        .simulation_secs += 1.0 / 60.0;
    app.update();
    let seek = app
        .world()
        .get::<AnimationPlayer>(actors[0].1)
        .unwrap()
        .animation(set.attack_node.unwrap())
        .unwrap()
        .seek_time();
    assert!((seek - 1.0 / 60.0).abs() < 0.00001);
    app.update();
    assert_eq!(
        app.world()
            .get::<AnimationPlayer>(actors[0].1)
            .unwrap()
            .animation(set.attack_node.unwrap())
            .unwrap()
            .seek_time(),
        seek
    );
    preview(&mut app, 1, PreviewKind::Death, 2);
    app.update();
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .sandbox
        .as_mut()
        .unwrap()
        .simulation_secs += 2.0;
    app.update();
    assert_eq!(
        app.world()
            .get::<AnimationPlayer>(actors[0].1)
            .unwrap()
            .animation(set.death_node.unwrap())
            .unwrap()
            .seek_time(),
        1.0
    );
    app.update();
    assert!(
        app.world()
            .get::<AnimationPlayer>(actors[0].1)
            .unwrap()
            .is_playing_animation(set.death_node.unwrap())
    );
    app.world_mut()
        .resource_mut::<crate::sandbox::SandboxClient>()
        .preview = None;
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .sandbox
        .as_mut()
        .unwrap()
        .config
        .environment
        .paused = false;
    app.update();
    assert!(
        app.world()
            .get::<AnimationPlayer>(actors[0].1)
            .unwrap()
            .is_playing_animation(set.idle_node)
    );
    assert!(
        !app.world()
            .get::<AnimationPlayer>(actors[0].1)
            .unwrap()
            .all_paused()
    );
}
#[test]
fn sandbox_speed_applies_to_ordinary_and_forced_animations_and_resets_after_exit() {
    let (mut app, set, actors) = sandbox_animation_app();
    for scale in shared::sandbox::TIME_SCALES {
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .sandbox
            .as_mut()
            .unwrap()
            .config
            .environment
            .time_scale = scale;
        preview(&mut app, 1, crate::sandbox::PreviewKind::Run, 1);
        app.update();
        for (_, child) in &actors {
            let player = app.world().get::<AnimationPlayer>(*child).unwrap();
            assert!(
                player
                    .playing_animations()
                    .all(|(_, active)| active.speed() == scale)
            );
        }
    }
    app.world_mut().resource_mut::<GameStateSnapshot>().sandbox = None;
    app.update();
    for (_, child) in &actors {
        let player = app.world().get::<AnimationPlayer>(*child).unwrap();
        assert_eq!(player.animation(set.idle_node).unwrap().speed(), 1.0);
        assert!(!player.all_paused());
    }
}
#[test]
fn sandbox_missing_clips_are_reported_and_frame_seek_wraps_only_loops() {
    use crate::sandbox::PreviewKind;
    let mut set = animation_set();
    set.attack_node = None;
    set.cast_node = None;
    set.death_node = None;
    let available = sandbox_available_animations(&set);
    assert_eq!(available, vec!["Idle", "Run"]);
    for kind in [
        PreviewKind::Walk,
        PreviewKind::Attack,
        PreviewKind::Cast,
        PreviewKind::Hit,
        PreviewKind::Death,
    ] {
        let (state, label) = sandbox_preview_state(kind, &set);
        assert!(label.contains("unavailable"));
        assert_eq!(set.node(state), set.idle_node);
    }
    assert!((sandbox_seek_time(0.99, 0.02, Some(1.0), true) - 0.01).abs() < 0.00001);
    assert_eq!(sandbox_seek_time(0.99, 0.02, Some(1.0), false), 1.0);
}
#[test]
fn sandbox_visual_clock_freezes_steps_and_returns_to_wall_delta() {
    let (mut app, _, _) = sandbox_animation_app();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(100));
    let mut clock = SandboxVisualClock::default();
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .sandbox
        .as_mut()
        .unwrap()
        .config
        .environment
        .time_scale = 0.25;
    let delta = clock.delta(
        app.world().resource::<Time>(),
        Some(app.world().resource::<GameStateSnapshot>()),
    );
    assert!((delta - 0.025).abs() < 0.00001);
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .sandbox
        .as_mut()
        .unwrap()
        .config
        .environment
        .paused = true;
    assert_eq!(
        clock.delta(
            app.world().resource::<Time>(),
            Some(app.world().resource::<GameStateSnapshot>())
        ),
        0.0
    );
    app.world_mut()
        .resource_mut::<GameStateSnapshot>()
        .sandbox
        .as_mut()
        .unwrap()
        .simulation_secs += 1.0 / 60.0;
    let delta = clock.delta(
        app.world().resource::<Time>(),
        Some(app.world().resource::<GameStateSnapshot>()),
    );
    assert!((delta - 1.0 / 60.0).abs() < 0.00001);
    assert_eq!(
        clock.delta(
            app.world().resource::<Time>(),
            Some(app.world().resource::<GameStateSnapshot>())
        ),
        0.0
    );
    assert!((clock.delta(app.world().resource::<Time>(), None) - 0.1).abs() < 0.00001);
}
#[test]
fn sandbox_refill_after_death_preserves_authoritatively_reconciled_position() {
    let (mut app, _, actors) = sandbox_animation_app();
    app.init_resource::<MapLayout>()
        .init_resource::<DebugConsole>();
    app.insert_resource(RespawnCountdown {
        end_time: Some(5.0),
        last_shown: 1,
        last_hp: 0.0,
    });
    app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
    app.world_mut().entity_mut(actors[0].0).insert((
        Team::Green,
        VerticalVelocity::default(),
        Transform::from_xyz(3.0, 0.5, 2.0),
    ));
    app.world_mut()
        .spawn((Text::new("1"), Visibility::Visible, RespawnCountdownText));
    app.add_systems(Update, respawn_countdown_system);
    app.update();
    assert_eq!(
        app.world()
            .get::<Transform>(actors[0].0)
            .unwrap()
            .translation,
        Vec3::new(3.0, 0.5, 2.0)
    );
    assert!(
        app.world()
            .resource::<RespawnCountdown>()
            .end_time
            .is_none()
    );
}

/// A local hero (id 7) on a rig whose clips are one second long, with the registry its
/// motions are read from and the messages of the cast observer and the stage tracker.
struct Rig {
    app: App,
    owner: Entity,
    child: Entity,
    set: CharacterAnimationSet,
}

impl Rig {
    /// A rig with every named clip of the motion library.
    fn new(
        registry: crate::skill_presentation::SkillPresentation,
        class: shared::HeroClass,
        loadout: shared::loadout::LoadoutState,
    ) -> Self {
        Self::with_clips(registry, class, loadout, true)
    }

    /// A rig with the six base clips, and with the named ones when `named` is set.
    fn with_clips(
        registry: crate::skill_presentation::SkillPresentation,
        class: shared::HeroClass,
        loadout: shared::loadout::LoadoutState,
        named: bool,
    ) -> Self {
        let library = crate::humanoid::SharedHumanoidMotion::embedded().unwrap();
        let named: Vec<&str> = library
            .clips
            .keys()
            .map(String::as_str)
            .filter(|name| !matches!(*name, "idle" | "walk" | "run" | "attack" | "cast" | "death"))
            .filter(|_| named)
            .collect();
        let mut clips = Assets::<AnimationClip>::default();
        let handles: Vec<_> = (0..5 + named.len())
            .map(|_| {
                let mut clip = AnimationClip::default();
                clip.set_duration(1.0);
                clips.add(clip)
            })
            .collect();
        let (graph, nodes) = AnimationGraph::from_clips(handles);
        let mut graphs = Assets::<AnimationGraph>::default();
        let set = CharacterAnimationSet {
            graph: graphs.add(graph),
            idle_node: nodes[0],
            run_node: nodes[1],
            walk_node: None,
            runtime: false,
            attack_node: Some(nodes[2]),
            cast_node: Some(nodes[3]),
            death_node: Some(nodes[4]),
            motion_nodes: named
                .iter()
                .zip(&nodes[5..])
                .map(|(name, node)| ((*name).to_owned(), *node))
                .collect(),
        };
        let mut sets = PlayerAnimationLibrary::default();
        sets.sets
            .insert(AvatarKey::Roster("agnes".into()), set.clone());
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .insert_resource(sets)
            .insert_resource(graphs)
            .insert_resource(clips)
            .insert_resource(registry)
            .init_resource::<GameStateSnapshot>()
            .add_message::<crate::skill_presentation::stage::StageEvent>()
            .add_message::<crate::skill_presentation::cast::SkillCastObserved>()
            .add_systems(
                Update,
                (bind_player_animation_players, sync_player_animation_state).chain(),
            );
        let owner = app
            .world_mut()
            .spawn((
                Player,
                Transform::default(),
                CombatStats::default(),
                NetworkCharacterChoice(CharacterChoice::Cube),
                NetworkAvatar(Some("agnes".into())),
                crate::net::NetworkPlayerId(7),
                crate::net::NetworkHeroClass(class),
                PlayerCosmeticAction::default(),
                crate::net::PlayerLoadout(Some(loadout)),
            ))
            .id();
        let child = app
            .world_mut()
            .spawn((AnimationPlayer::default(), ChildOf(owner)))
            .id();
        app.update();
        Self {
            app,
            owner,
            child,
            set,
        }
    }

    /// One frame in which the hero's latest accepted action is this one.
    fn act(&mut self, sequence: u64, slot: u8, kind: PlayerActionKind) {
        self.app
            .world_mut()
            .entity_mut(self.owner)
            .insert(PlayerCosmeticAction {
                sequence,
                slot,
                kind,
            });
        self.app.update();
    }

    /// One frame that lasts `secs`.
    fn wait(&mut self, secs: f32) {
        let mut time = self.app.world_mut().resource_mut::<Time>();
        time.advance_by(std::time::Duration::from_secs_f32(secs));
        self.app.update();
        // The frames after it take no time again.
        let mut time = self.app.world_mut().resource_mut::<Time>();
        time.advance_by(std::time::Duration::ZERO);
    }

    /// What the stage tracker writes when a telegraph of the hero fires by leaving the
    /// snapshot.
    fn report_release(&mut self, effect: shared::loadout::SkillEffectState) {
        use crate::skill_presentation::stage::{EndKind, StageChange, StageEvent};
        self.app.world_mut().write_message(StageEvent {
            effect,
            change: StageChange::Ended(EndKind::Released),
            owner: None,
        });
    }

    /// What the cast observer writes for an accepted action of a hero.
    fn report_cast(&mut self, actor_id: u64, sequence: u64, slot: u8, recast: bool) {
        use crate::skill_presentation::cast::{CastKey, SkillCastObserved};
        let (class, loadout) = {
            let hero = self.app.world().entity(self.owner);
            (
                hero.get::<crate::net::NetworkHeroClass>().unwrap().0,
                hero.get::<crate::net::PlayerLoadout>().unwrap().0.clone(),
            )
        };
        self.app.world_mut().write_message(SkillCastObserved {
            actor_id,
            key: CastKey::of(class, loadout.as_ref(), slot).unwrap(),
            slot,
            sequence,
            recast,
            origin: Vec3::ZERO,
            position: Vec3::ZERO,
            yaw: None,
            forward: Vec3::NEG_Z,
            local: true,
        });
    }

    fn loadout(&mut self) -> Mut<'_, shared::loadout::LoadoutState> {
        self.app
            .world_mut()
            .get_mut::<crate::net::PlayerLoadout>(self.owner)
            .unwrap()
            .map_unchanged(|loadout| loadout.0.as_mut().unwrap())
    }

    fn set_hp(&mut self, hp: f32) {
        self.app
            .world_mut()
            .get_mut::<CombatStats>(self.owner)
            .unwrap()
            .hp = hp;
        self.app.update();
    }

    fn effects(&mut self) -> Mut<'_, Vec<shared::loadout::SkillEffectState>> {
        self.app
            .world_mut()
            .resource_mut::<GameStateSnapshot>()
            .map_unchanged(|game| &mut game.skill_effects)
    }

    fn state(&self) -> HeroAnimationState {
        self.app
            .world()
            .get::<PlayerAnimationBinding>(self.child)
            .unwrap()
            .playback
            .state
    }

    /// The clip of the current state, as the player holds it.
    fn clip(&mut self) -> Mut<'_, bevy::animation::ActiveAnimation> {
        let node = self.set.node(self.state());
        self.app
            .world_mut()
            .get_mut::<AnimationPlayer>(self.child)
            .unwrap()
            .map_unchanged(|player| player.animation_mut(node).unwrap())
    }

    /// Marks the playing clip, so a later restart shows as a seek time other than this.
    fn mark(&mut self) {
        self.clip().seek_to(0.4);
    }

    fn marked(&mut self) -> bool {
        self.clip().seek_time() == 0.4
    }
}

fn own_effect(
    id: u64,
    skill: shared::loadout::SkillId,
    kind: shared::loadout::EffectVisualKind,
) -> shared::loadout::SkillEffectState {
    shared::loadout::SkillEffectState {
        id,
        owner_id: 7,
        owner_team: shared::map::Team::Green,
        skill,
        kind,
        position: [0.0; 2],
        end: [10.0, 0.0],
        radius: 0.8,
        remaining_secs: 0.8,
        armed: false,
        consumed_segments: 0,
    }
}

/// The triage cases of the Dawn Ray windup: a basic attack accepted during the warning, a
/// recast accepted during it, and a cast edge that is never observed because the snapshot
/// that brings the warning already carries a later action. Each runs on the unmigrated rows
/// and on the final ones.
#[test]
fn windup_survives_interleaved_basic_and_recast_and_releases_once() {
    use crate::skill_presentation::SkillPresentation;
    use shared::loadout::{CoreId, EffectVisualKind, LoadoutState, SkillId};
    use shared::{BASIC_ATTACK_ACTION_SLOT, HeroClass};
    const RAY: u8 = 3;
    const FIELD: u8 = 2;
    // (case, the ray's own edge is observed, the action accepted during the warning)
    let cases = [
        (
            "basic attack",
            true,
            BASIC_ATTACK_ACTION_SLOT,
            PlayerActionKind::Attack,
        ),
        ("recast", true, FIELD, PlayerActionKind::Cast),
        (
            "edge never observed",
            false,
            BASIC_ATTACK_ACTION_SLOT,
            PlayerActionKind::Attack,
        ),
    ];
    for registry in [SkillPresentation::unmigrated, SkillPresentation::target] {
        for (case, own_edge, slot, kind) in cases {
            let ray = registry().profile(SkillId::DawnRay).unwrap().clone();
            let mut rig = Rig::new(
                registry(),
                HeroClass::Dawnweaver,
                LoadoutState {
                    recipe: Some(CoreId::Dawnweaver.preset()),
                    ..default()
                },
            );
            let windup = rig.set.motion(ray.windup.as_deref().unwrap());
            let release = rig.set.motion(&ray.release);
            assert_ne!(windup, release, "{case}");

            if own_edge {
                rig.act(1, RAY, PlayerActionKind::Cast);
                assert_eq!(
                    rig.state(),
                    HeroAnimationState::Idle,
                    "{case}: no guessed pose"
                );
                rig.effects().push(own_effect(
                    9,
                    SkillId::DawnRay,
                    EffectVisualKind::BeamWarning,
                ));
                rig.app.update();
            } else {
                rig.effects().push(own_effect(
                    9,
                    SkillId::DawnRay,
                    EffectVisualKind::BeamWarning,
                ));
                rig.act(2, slot, kind);
            }
            assert_eq!(rig.state(), windup, "{case}: the warning starts the windup");
            rig.mark();

            // The action accepted during the warning does not take the body, however often
            // it is repeated, and does not restart the held pose.
            for sequence in [2, 3] {
                rig.act(sequence, slot, kind);
                assert_eq!(rig.state(), windup, "{case}: sequence {sequence}");
                assert!(rig.marked(), "{case}: the held windup restarted");
            }

            // The same effect as a beam: the release, once.
            rig.effects()[0].kind = EffectVisualKind::Beam;
            rig.app.update();
            assert_eq!(rig.state(), release, "{case}: the flip releases");
            assert_eq!(rig.clip().seek_time(), 0.0, "{case}: the release starts");
            rig.mark();
            for _ in 0..3 {
                rig.app.update();
                assert_eq!(rig.state(), release, "{case}");
                assert!(rig.marked(), "{case}: the release replayed");
            }
            // The beam leaves the snapshot: nothing more is played for it.
            rig.effects().clear();
            rig.app.update();
            assert!(
                rig.marked(),
                "{case}: the release replayed when the beam left"
            );

            // The body is free again: the next accepted action takes it.
            rig.act(4, BASIC_ATTACK_ACTION_SLOT, PlayerActionKind::Attack);
            assert!(!rig.marked(), "{case}: the next action plays");
            assert_ne!(rig.state(), windup, "{case}");
        }

        // A warning that vanishes fired nothing, also with an action accepted during it.
        let ray = registry().profile(SkillId::DawnRay).unwrap().clone();
        let mut rig = Rig::new(
            registry(),
            HeroClass::Dawnweaver,
            LoadoutState {
                recipe: Some(CoreId::Dawnweaver.preset()),
                ..default()
            },
        );
        rig.effects().push(own_effect(
            9,
            SkillId::DawnRay,
            EffectVisualKind::BeamWarning,
        ));
        rig.act(1, RAY, PlayerActionKind::Cast);
        rig.act(2, BASIC_ATTACK_ACTION_SLOT, PlayerActionKind::Attack);
        assert_eq!(rig.state(), rig.set.motion(ray.windup.as_deref().unwrap()));
        rig.effects().clear();
        for _ in 0..3 {
            rig.app.update();
            assert_eq!(
                rig.state(),
                HeroAnimationState::Idle,
                "a vanished warning releases nothing"
            );
        }
    }
}

/// The four cases of a hero's own telegraph, and death.
#[test]
fn a_telegraph_starts_its_windup_and_its_release_once_each() {
    use super::animation::{Telegraph, follow_telegraph};
    use crate::skill_presentation::MotionCue;
    let cue = |motion: &str, hold| MotionCue {
        motion: motion.into(),
        hold,
        rate: 1.0,
        start: 0.0,
        looping: false,
    };
    let (windup, release) = (cue("spell_prepare", true), cue("cast", false));
    let warns = |id| Some((id, windup.clone()));
    let fired = |id| Some((id, release.clone()));
    let mut followed = None;
    // No fuse is reported as fired here: every telegraph of these cases is a warning.
    let mut step = |own, live| follow_telegraph(&mut followed, own, live, |_| None);

    // (a) A warning starts the windup when it is first seen and asks nothing while it lasts.
    assert_eq!(step(warns(9), true), Telegraph::Start(windup.clone()));
    assert_eq!(step(warns(9), true), Telegraph::Unchanged);
    // (b) The same effect fires: the release, (c) once.
    assert_eq!(step(fired(9), true), Telegraph::Start(release.clone()));
    assert_eq!(step(fired(9), true), Telegraph::Unchanged);
    assert_eq!(step(None, true), Telegraph::Unchanged);
    // (d) A warning that vanishes cancels the windup and releases nothing, also later.
    assert_eq!(step(warns(10), true), Telegraph::Start(windup.clone()));
    assert_eq!(step(None, true), Telegraph::Cancel);
    assert_eq!(step(None, true), Telegraph::Unchanged);
    // (b) An effect first seen after it fired still releases once.
    assert_eq!(step(fired(10), true), Telegraph::Start(release.clone()));
    assert_eq!(step(fired(10), true), Telegraph::Unchanged);
    // A second cast is another effect: both stages play again.
    assert_eq!(step(warns(11), true), Telegraph::Start(windup.clone()));
    assert_eq!(step(warns(12), true), Telegraph::Start(windup.clone()));
    assert_eq!(step(fired(12), true), Telegraph::Start(release.clone()));
    // A dead hero follows nothing, and what is replicated when it is back is history.
    assert_eq!(step(warns(13), true), Telegraph::Start(windup.clone()));
    assert_eq!(step(warns(13), false), Telegraph::Unchanged);
    assert_eq!(step(fired(13), false), Telegraph::Unchanged);
    assert_eq!(step(fired(13), true), Telegraph::Unchanged);
    assert_eq!(step(warns(14), false), Telegraph::Unchanged);
    assert_eq!(step(warns(14), true), Telegraph::Unchanged);
    assert_eq!(step(fired(14), true), Telegraph::Unchanged);

    // (d) A fuse fires by leaving the snapshot. Only the effect whose windup was held is
    // asked about, and only once.
    let mut followed = None;
    let mut asked = Vec::new();
    let mut step = |own, live, fires: bool| {
        follow_telegraph(&mut followed, own, live, |effect| {
            asked.push(effect);
            fires.then(|| release.clone())
        })
    };
    assert_eq!(
        step(warns(20), true, true),
        Telegraph::Start(windup.clone())
    );
    assert_eq!(step(warns(20), true, true), Telegraph::Unchanged);
    assert_eq!(step(None, true, true), Telegraph::Start(release.clone()));
    assert_eq!(step(None, true, true), Telegraph::Unchanged);
    assert_eq!(
        step(warns(21), true, false),
        Telegraph::Start(windup.clone())
    );
    assert_eq!(step(None, true, false), Telegraph::Cancel);
    // The owner died: the telegraph leaves with it and nothing is asked or played.
    assert_eq!(
        step(warns(22), true, true),
        Telegraph::Start(windup.clone())
    );
    assert_eq!(step(None, false, true), Telegraph::Unchanged);
    assert_eq!(step(None, true, true), Telegraph::Unchanged);
    assert_eq!(asked, [20, 21]);
}

/// The kit of a core, and the slot one of its skills sits in.
fn kit_with(
    core: shared::loadout::CoreId,
    skill: shared::loadout::SkillId,
) -> (shared::HeroClass, shared::loadout::LoadoutState, u8) {
    let recipe = core.preset();
    let slot = recipe.skills.iter().position(|held| *held == skill);
    (
        recipe.core.class(),
        shared::loadout::LoadoutState {
            recipe: Some(recipe),
            ..default()
        },
        slot.unwrap() as u8,
    )
}

/// The three skills whose telegraph fires by leaving the snapshot, on the final rows: the
/// windup starts on the accepted cast, is held while the telegraph is replicated and
/// through an action accepted during it, and the release plays once when the stage tracker
/// reports that the telegraph fired. Nothing is released for a telegraph that was never
/// seen, that vanished, or whose owner died.
#[test]
fn fuse_and_parry_windups_start_on_the_cast_hold_and_release_when_the_telegraph_fires() {
    use super::animation::UNSEEN_HOLD_SECS;
    use crate::skill_presentation::SkillPresentation;
    use bevy::animation::RepeatAnimation;
    use shared::BASIC_ATTACK_ACTION_SLOT;
    use shared::loadout::{CoreId, EffectVisualKind, SkillId};
    for (core, skill, kind) in [
        (
            CoreId::Cinderforge,
            SkillId::FurnaceBreath,
            EffectVisualKind::BeamWarning,
        ),
        (
            CoreId::Orbitwright,
            SkillId::OrbitalCollapse,
            EffectVisualKind::BeamWarning,
        ),
        (
            CoreId::Edgeweaver,
            SkillId::MirrorGuard,
            EffectVisualKind::Barrier,
        ),
    ] {
        let name = skill.id();
        let parry = skill == SkillId::MirrorGuard;
        let (class, mut kit, slot) = kit_with(core, skill);
        // The accepted cast of a parry comes with the replicated stance.
        kit.parrying = parry;
        let row = SkillPresentation::target().profile(skill).unwrap().clone();
        let telegraph = own_effect(9, skill, kind);
        let fresh = || Rig::new(SkillPresentation::target(), class, kit.clone());
        let windup = |rig: &Rig| rig.set.motion(row.windup.as_deref().unwrap());
        let release = |rig: &Rig| rig.set.motion(&row.release);

        // The telegraph is seen, a basic attack is accepted during it, and it fires.
        let mut rig = fresh();
        assert_ne!(windup(&rig), release(&rig), "{name}");
        rig.act(1, slot, PlayerActionKind::Cast);
        assert_eq!(
            rig.state(),
            windup(&rig),
            "{name}: the cast starts the windup"
        );
        assert_eq!(rig.clip().repeat_mode(), RepeatAnimation::Forever, "{name}");
        assert_eq!(rig.clip().speed(), 1.0, "{name}");
        rig.mark();
        rig.effects().push(telegraph.clone());
        rig.app.update();
        rig.act(2, BASIC_ATTACK_ACTION_SLOT, PlayerActionKind::Attack);
        // Replicated, it holds the pose for as long as it lasts.
        rig.wait(UNSEEN_HOLD_SECS * 2.0);
        assert_eq!(
            rig.state(),
            windup(&rig),
            "{name}: held through the basic attack"
        );
        assert!(rig.marked(), "{name}: the held windup restarted");
        rig.effects().clear();
        rig.loadout().parrying = false;
        rig.report_release(telegraph.clone());
        rig.app.update();
        assert_eq!(
            rig.state(),
            release(&rig),
            "{name}: the fired telegraph releases"
        );
        assert_eq!(rig.clip().seek_time(), row.motion.start, "{name}");
        assert_eq!(rig.clip().speed(), row.motion.rate, "{name}");
        assert_eq!(rig.clip().repeat_mode(), RepeatAnimation::Never, "{name}");
        rig.mark();
        for _ in 0..3 {
            rig.app.update();
            assert_eq!(rig.state(), release(&rig), "{name}");
            assert!(rig.marked(), "{name}: the release replayed");
        }

        // The telegraph never reaches the client: a bounded hold and no release, also when
        // a release of that effect is reported afterwards.
        let mut rig = fresh();
        rig.act(1, slot, PlayerActionKind::Cast);
        rig.wait(UNSEEN_HOLD_SECS - 0.1);
        rig.app.update();
        assert_eq!(
            rig.state(),
            windup(&rig),
            "{name}: held without its telegraph"
        );
        rig.wait(0.2);
        rig.app.update();
        assert_eq!(
            rig.state(),
            HeroAnimationState::Idle,
            "{name}: the hold is bounded"
        );
        rig.report_release(telegraph.clone());
        rig.app.update();
        assert_eq!(
            rig.state(),
            HeroAnimationState::Idle,
            "{name}: nothing was held"
        );

        // The telegraph vanishes and nothing reports that it fired: no release. A release
        // reported for another effect or another hero's is not this one's.
        let mut rig = fresh();
        rig.effects().push(telegraph.clone());
        rig.act(1, slot, PlayerActionKind::Cast);
        assert_eq!(rig.state(), windup(&rig), "{name}");
        rig.effects().clear();
        rig.loadout().parrying = false;
        rig.report_release(own_effect(10, skill, kind));
        rig.report_release(shared::loadout::SkillEffectState {
            owner_id: 8,
            ..telegraph.clone()
        });
        for _ in 0..3 {
            rig.app.update();
            assert_eq!(rig.state(), HeroAnimationState::Idle, "{name}: cancelled");
        }

        // The owner dies during the windup: the telegraph leaves with it, and nothing is
        // played for it when the hero is back.
        let mut rig = fresh();
        rig.effects().push(telegraph.clone());
        rig.act(1, slot, PlayerActionKind::Cast);
        assert_eq!(rig.state(), windup(&rig), "{name}");
        rig.set_hp(0.0);
        assert_eq!(rig.state(), HeroAnimationState::Death, "{name}");
        // What is still replicated when the hero is back is history: neither the windup
        // nor an action accepted beside it starts anything.
        rig.set_hp(100.0);
        rig.act(2, BASIC_ATTACK_ACTION_SLOT, PlayerActionKind::Attack);
        assert_eq!(
            rig.state(),
            HeroAnimationState::Idle,
            "{name}: a stale windup"
        );
        rig.effects().clear();
        rig.loadout().parrying = false;
        rig.report_release(telegraph.clone());
        for _ in 0..3 {
            rig.app.update();
            assert_eq!(
                rig.state(),
                HeroAnimationState::Idle,
                "{name}: no stale release"
            );
        }
    }

    // A windup that is not a loop is held on its last key, from the cast through the
    // arrival of its telegraph to the release.
    let (class, kit, slot) = kit_with(CoreId::Cinderforge, SkillId::FurnaceBreath);
    let registry = || {
        SkillPresentation::target_with(|config| {
            config["skills"]["furnace_breath"]["windup"] = "spell_prepare".into();
        })
    };
    let row = registry().profile(SkillId::FurnaceBreath).unwrap().clone();
    let mut rig = Rig::new(registry(), class, kit);
    let telegraph = own_effect(9, SkillId::FurnaceBreath, EffectVisualKind::BeamWarning);
    rig.act(1, slot, PlayerActionKind::Cast);
    assert_eq!(rig.state(), rig.set.motion("spell_prepare"));
    assert_eq!(rig.clip().repeat_mode(), RepeatAnimation::Never);
    rig.clip().seek_to(1.0);
    rig.app.update();
    rig.effects().push(telegraph.clone());
    for _ in 0..3 {
        rig.app.update();
        assert_eq!(rig.state(), rig.set.motion("spell_prepare"));
        assert_eq!(rig.clip().seek_time(), 1.0, "the pose restarted");
    }
    rig.effects().clear();
    rig.report_release(telegraph);
    rig.app.update();
    assert_eq!(rig.state(), rig.set.motion(&row.release));

    // A parry is held by the stance, not by the cast alone: an accepted cast without the
    // replicated stance holds nothing, and a stance that ends without a report ends the hold.
    let (class, kit, slot) = kit_with(CoreId::Edgeweaver, SkillId::MirrorGuard);
    let mut rig = Rig::new(SkillPresentation::target(), class, kit);
    rig.act(1, slot, PlayerActionKind::Cast);
    assert_eq!(rig.state(), HeroAnimationState::Idle);
    rig.loadout().parrying = true;
    rig.act(2, slot, PlayerActionKind::Cast);
    assert_ne!(rig.state(), HeroAnimationState::Idle);
    rig.loadout().parrying = false;
    rig.app.update();
    assert_eq!(rig.state(), HeroAnimationState::Idle);
}

/// `rate`, `start`, the recast clip, the alternating basic attack and the fitted windup of
/// the final rows, as the animation player holds them. An unmigrated row, which has none of
/// these, plays as before.
#[test]
fn action_clips_enter_at_their_start_and_play_at_their_rate() {
    use crate::skill_presentation::SkillPresentation;
    use bevy::animation::RepeatAnimation;
    use shared::BASIC_ATTACK_ACTION_SLOT;
    use shared::loadout::{CoreId, EffectVisualKind, SkillId};
    let (class, kit, field) = kit_with(CoreId::Dawnweaver, SkillId::DawnField);
    let target = SkillPresentation::target();
    let row = target.profile(SkillId::DawnField).unwrap().clone();
    let basic = target.basic(class).unwrap().clone();
    assert!(row.motion.start > 0.0 && row.motion.rate != 1.0 && basic.start > 0.0);
    let mut rig = Rig::new(SkillPresentation::target(), class, kit.clone());

    // The first cast: the release, entered at its start and played at its rate, once.
    rig.report_cast(7, 1, field, false);
    rig.act(1, field, PlayerActionKind::Cast);
    assert_eq!(rig.state(), rig.set.motion(&row.release));
    assert_eq!(rig.clip().seek_time(), row.motion.start);
    assert_eq!(rig.clip().speed(), row.motion.rate);
    assert_eq!(rig.clip().repeat_mode(), RepeatAnimation::Never);
    // The rate belongs to that clip alone: the idle clip fading out keeps its speed.
    let idle = rig.set.idle_node;
    let player = rig.app.world().get::<AnimationPlayer>(rig.child).unwrap();
    assert_eq!(player.animation(idle).unwrap().speed(), 1.0);

    // A recast of the slot: the recast clip at the recast rate, from its first key.
    let recast = row.motion.recast.as_deref().unwrap();
    rig.report_cast(7, 2, field, true);
    rig.act(2, field, PlayerActionKind::Cast);
    assert_eq!(rig.state(), rig.set.motion(recast));
    assert_ne!(rig.state(), rig.set.motion(&row.release));
    assert_eq!(rig.clip().seek_time(), 0.0);
    assert_eq!(rig.clip().speed(), row.motion.recast_rate);
    // A recast of another hero, of another slot or of an earlier action is not this one.
    rig.report_cast(8, 3, field, true);
    rig.report_cast(7, 3, field + 1, true);
    rig.report_cast(7, 2, field, true);
    rig.act(3, field, PlayerActionKind::Cast);
    assert_eq!(rig.state(), rig.set.motion(&row.release));

    // The basic attack alternates its two motions, the first on odd sequences, and a new
    // edge during the clip starts it again from its start (rule E-13).
    let [odd, even] = [0, 1].map(|turn| rig.set.motion(&basic.motions[turn]));
    assert_ne!(odd, even);
    for (sequence, turn) in [(5, odd), (6, even), (8, even), (9, odd)] {
        rig.act(sequence, BASIC_ATTACK_ACTION_SLOT, PlayerActionKind::Attack);
        assert_eq!(rig.state(), turn, "sequence {sequence}");
        assert_eq!(rig.clip().seek_time(), basic.start, "sequence {sequence}");
        assert_eq!(rig.clip().speed(), basic.rate, "sequence {sequence}");
        rig.mark();
    }

    // The sandbox speed multiplies the rate; a clip that is not the cue's keeps the speed.
    rig.app
        .world_mut()
        .resource_mut::<GameStateSnapshot>()
        .sandbox = Some(shared::sandbox::SandboxSnapshot {
        config: Default::default(),
        ack: None,
        last_request_id: 0,
        actors: Vec::new(),
        analytics: Default::default(),
        simulation_secs: 0.0,
        frame: 0,
    });
    for scale in shared::sandbox::TIME_SCALES {
        rig.app
            .world_mut()
            .resource_mut::<GameStateSnapshot>()
            .sandbox
            .as_mut()
            .unwrap()
            .config
            .environment
            .time_scale = scale;
        rig.app.update();
        assert_eq!(rig.clip().speed(), scale * basic.rate, "scale {scale}");
    }
    // A paused sandbox steps the clip by the simulated time at the same rate.
    let step = |rig: &mut Rig, paused: Option<bool>, secs: f64| {
        let mut game = rig.app.world_mut().resource_mut::<GameStateSnapshot>();
        let sandbox = game.sandbox.as_mut().unwrap();
        if let Some(paused) = paused {
            sandbox.config.environment.paused = paused;
        }
        sandbox.config.environment.time_scale = 1.0;
        sandbox.simulation_secs += secs;
        rig.app.update();
    };
    step(&mut rig, Some(true), 0.0);
    let before = rig.clip().seek_time();
    step(&mut rig, None, 0.25);
    assert!((rig.clip().seek_time() - (before + 0.25 * basic.rate)).abs() < 1e-5);
    step(&mut rig, Some(false), 0.0);
    rig.app
        .world_mut()
        .resource_mut::<GameStateSnapshot>()
        .sandbox = None;
    // When the clip has run out the body idles at the plain speed again.
    rig.clip().seek_to(1.0);
    rig.app.update();
    assert_eq!(rig.state(), HeroAnimationState::Idle);
    assert_eq!(rig.clip().speed(), 1.0);

    // A sandbox preview plays its clip as it is, also when the last skill asked for that
    // very clip at another rate.
    let cleric = shared::HeroClass::Cleric;
    let cast = SkillPresentation::target().basic(cleric).unwrap().clone();
    assert_eq!((cast.motions[0].as_str(), cast.rate != 1.0), ("cast", true));
    let mut rig = Rig::new(SkillPresentation::target(), cleric, default());
    rig.app.init_resource::<crate::sandbox::SandboxClient>();
    rig.app
        .world_mut()
        .resource_mut::<GameStateSnapshot>()
        .sandbox = Some(shared::sandbox::SandboxSnapshot {
        config: Default::default(),
        ack: None,
        last_request_id: 0,
        actors: Vec::new(),
        analytics: Default::default(),
        simulation_secs: 0.0,
        frame: 0,
    });
    rig.act(1, BASIC_ATTACK_ACTION_SLOT, PlayerActionKind::Attack);
    assert_eq!(rig.state(), HeroAnimationState::Cast);
    assert_eq!(rig.clip().speed(), cast.rate);
    preview(&mut rig.app, 7, crate::sandbox::PreviewKind::Cast, 1);
    rig.app.update();
    assert_eq!(rig.state(), HeroAnimationState::Cast);
    assert_eq!(rig.clip().speed(), 1.0);

    // A windup fitted to its telegraph plays once at the fitted rate; a loop repeats at
    // its own speed.
    let (class, ray_kit, ray) = kit_with(CoreId::Dawnweaver, SkillId::DawnRay);
    let mut rig = Rig::new(SkillPresentation::target(), class, ray_kit);
    rig.effects().push(own_effect(
        9,
        SkillId::DawnRay,
        EffectVisualKind::BeamWarning,
    ));
    rig.act(1, ray, PlayerActionKind::Cast);
    assert_eq!(rig.state(), rig.set.motion("spell_prepare"));
    assert_eq!(rig.clip().speed(), 0.625);
    assert_eq!(rig.clip().repeat_mode(), RepeatAnimation::Never);
    let (class, wave_kit, wave) = kit_with(CoreId::Riftshot, SkillId::HorizonWave);
    let mut rig = Rig::new(SkillPresentation::target(), class, wave_kit);
    rig.effects().push(own_effect(
        9,
        SkillId::HorizonWave,
        EffectVisualKind::BeamWarning,
    ));
    rig.act(1, wave, PlayerActionKind::Cast);
    assert_eq!(rig.state(), rig.set.motion("aim_hold_loop"));
    assert_eq!(rig.clip().speed(), 1.0);
    assert_eq!(rig.clip().repeat_mode(), RepeatAnimation::Forever);

    // An unmigrated row has no playback values: its clip plays from its first key at speed 1.
    let unmigrated = SkillPresentation::unmigrated();
    let shipped = unmigrated.profile(SkillId::DawnField).unwrap().clone();
    let mut rig = Rig::new(unmigrated, class_of(&kit), kit.clone());
    rig.report_cast(7, 1, field, true);
    rig.act(1, field, PlayerActionKind::Cast);
    assert_eq!(rig.state(), rig.set.motion(&shipped.release));
    assert_eq!((rig.clip().seek_time(), rig.clip().speed()), (0.0, 1.0));

    // A rig without the named clip plays its cast clip in that place, as that clip is:
    // the values written for the named motion are not applied to it, and it is not looped.
    let mut rig = Rig::with_clips(SkillPresentation::target(), class_of(&kit), kit, false);
    rig.act(1, field, PlayerActionKind::Cast);
    assert_eq!(rig.state(), HeroAnimationState::Cast);
    assert_eq!((rig.clip().seek_time(), rig.clip().speed()), (0.0, 1.0));
    let (class, mut furnace_kit, furnace) = kit_with(CoreId::Cinderforge, SkillId::FurnaceBreath);
    furnace_kit.parrying = false;
    let mut rig = Rig::with_clips(SkillPresentation::target(), class, furnace_kit, false);
    rig.effects().push(own_effect(
        9,
        SkillId::FurnaceBreath,
        EffectVisualKind::BeamWarning,
    ));
    rig.act(1, furnace, PlayerActionKind::Cast);
    assert_eq!(rig.state(), HeroAnimationState::Cast);
    assert_eq!(rig.clip().repeat_mode(), RepeatAnimation::Never);
    // It is still held while the telegraph lasts.
    rig.clip().seek_to(1.0);
    rig.app.update();
    assert_eq!(rig.state(), HeroAnimationState::Cast);
}

fn class_of(kit: &shared::loadout::LoadoutState) -> shared::HeroClass {
    kit.recipe.as_ref().unwrap().core.class()
}
