use super::animation::{
    AvatarKey, CharacterAnimationSet, HeroAnimationPlayback, HeroAnimationState,
    bind_player_animation_players, sandbox_available_animations, sandbox_preview_state,
    sandbox_seek_time, start_hero_animation, sync_player_animation_state,
};
use super::motion::SandboxVisualClock;
use super::respawn_ui::RespawnCountdownText;
use super::*;
use crate::combat::CombatStats;
use crate::debug_console::DebugConsole;
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
