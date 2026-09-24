//! Opt-in Combat Test client: real server commands, stable UI and durable presets.
mod presets;
mod ui;

use crate::net::{GameStateSnapshot, NetworkCommand};
use bevy::prelude::*;
use shared::sandbox::*;
use std::{
    collections::{HashMap, VecDeque},
    sync::OnceLock,
    time::{Duration, Instant},
};

#[derive(Debug, Default)]
pub(crate) struct Launch {
    pub enabled: bool,
    pub hero: Option<shared::HeroClass>,
    pub avatar: Option<String>,
    pub preset: Option<String>,
}
fn parse_launch(args: impl IntoIterator<Item = String>, enabled: bool) -> Result<Launch, String> {
    let mut result = Launch {
        enabled,
        ..Default::default()
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--combat-test" => result.enabled = true,
            "--hero" => {
                let value = args
                    .next()
                    .ok_or("--hero needs warrior, mage, ranger or cleric")?;
                result.hero = Some(
                    shared::HeroClass::from_id(&value)
                        .ok_or_else(|| format!("Unknown hero: {value}"))?,
                );
            }
            "--avatar" => result.avatar = Some(args.next().ok_or("--avatar needs a shipped slug")?),
            "--sandbox-preset" => {
                result.preset = Some(args.next().ok_or("--sandbox-preset needs a name or file")?)
            }
            _ => {}
        }
    }
    if !result.enabled
        && (result.hero.is_some() || result.avatar.is_some() || result.preset.is_some())
    {
        return Err("Sandbox options require --combat-test".into());
    }
    if let Some(slug) = &result.avatar {
        if !shared::avatar_definition(slug).is_some_and(|a| a.passport.is_none()) {
            return Err(format!(
                "Unknown or paid avatar '{slug}'; use a shipped free avatar"
            ));
        }
    }
    Ok(result)
}
fn launch_result() -> &'static Result<Launch, String> {
    static OPTIONS: OnceLock<Result<Launch, String>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        parse_launch(
            std::env::args().skip(1),
            std::env::var("OMOBA_COMBAT_SANDBOX").is_ok_and(|v| v == "1"),
        )
    })
}
pub(crate) fn launch() -> &'static Launch {
    launch_result()
        .as_ref()
        .expect("validated Combat Test launch options")
}
pub(crate) fn validate_launch() -> Result<(), String> {
    launch_result().as_ref().map(|_| ()).map_err(Clone::clone)
}
pub(crate) fn requested() -> bool {
    launch_result().as_ref().is_ok_and(|v| v.enabled)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewKind {
    Idle,
    Run,
    Walk,
    Attack,
    Cast,
    Hit,
    Death,
}
#[derive(Clone, Debug)]
pub(crate) struct Preview {
    pub id: u64,
    pub kind: PreviewKind,
    pub sequence: u64,
}
#[derive(Resource, Default)]
pub(crate) struct AnimationReadout(pub HashMap<u64, (String, Vec<String>)>);

#[derive(Resource)]
pub(crate) struct SandboxClient {
    pub enabled: bool,
    pub open: bool,
    pub overlay: bool,
    pub geometry: bool,
    pub config: SandboxConfig,
    pub actor: SandboxActor,
    tab: ui::Tab,
    pub status: String,
    pub preset_name: String,
    pub preview: Option<Preview>,
    pub preview_sequence: u64,
    pub teleport: bool,
    edit: Option<ui::Edit>,
    pub rebuild: bool,
    pub last_action: Option<SandboxCommand>,
    pub last_cast: Option<(crate::net::TargetId, u8)>,
    identity: Option<(u64, u64, u64)>,
    next_id: u64,
    pending: Option<(SandboxRequest, Instant, Instant)>,
    queue: VecDeque<SandboxCommand>,
    initial_preset: bool,
}
impl Default for SandboxClient {
    fn default() -> Self {
        Self {
            enabled: requested(),
            open: requested(),
            overlay: true,
            geometry: false,
            config: Default::default(),
            actor: SandboxActor::Player,
            tab: ui::Tab::Player,
            status: "Waiting for Combat Test server…".into(),
            preset_name: "my-test".into(),
            preview: None,
            preview_sequence: 0,
            teleport: false,
            edit: None,
            rebuild: true,
            last_action: None,
            last_cast: None,
            identity: None,
            next_id: 1,
            pending: None,
            queue: VecDeque::new(),
            initial_preset: false,
        }
    }
}
impl SandboxClient {
    pub fn blocks_world(&self) -> bool {
        self.enabled && (self.open || self.teleport || self.edit.is_some())
    }
    pub fn submit(&mut self, command: SandboxCommand) {
        if self.queue.len() >= 32 {
            self.status = "Wait for outstanding changes before adding more".into();
            return;
        }
        if matches!(
            command,
            SandboxCommand::ResetDuel | SandboxCommand::ResetActor { .. }
        ) {
            self.preview = None;
            self.last_cast = None;
            self.last_action = None;
        }
        if matches!(command, SandboxCommand::ForceCast { .. }) {
            self.last_action = Some(command.clone());
            self.last_cast = None;
        }
        self.queue.push_back(command);
    }
    pub fn apply(&mut self) {
        self.submit(SandboxCommand::ApplyConfig {
            config: self.config.clone(),
        });
    }
    pub fn actor_config_mut(&mut self) -> &mut ActorConfig {
        if self.actor == SandboxActor::Enemy {
            &mut self.config.enemy.actor
        } else {
            &mut self.config.player
        }
    }
    pub fn actor_config(&self) -> &ActorConfig {
        if self.actor == SandboxActor::Enemy {
            &self.config.enemy.actor
        } else {
            &self.config.player
        }
    }
}

pub(crate) struct SandboxPlugin;
impl Plugin for SandboxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SandboxClient>()
            .init_resource::<AnimationReadout>()
            .add_systems(
                Update,
                (autojoin, sync_state, sync_progression, remember_cast)
                    .chain()
                    .after(crate::net::ClientNetPipeline::ApplySnapshot)
                    .before(crate::net::ClientNetPipeline::SendCommands)
                    .before(crate::input_context::InputContextSet::Modal),
            );
        ui::install(app);
    }
}
fn autojoin(
    mut done: Local<bool>,
    mut selection: ResMut<crate::team::TeamSelection>,
    mut commands: MessageWriter<NetworkCommand>,
) {
    if *done || !requested() {
        return;
    }
    *done = true;
    if launch().avatar.is_some() {
        selection.avatar.clone_from(&launch().avatar);
    }
    let Some(hero) = launch().hero else {
        return;
    };
    selection.hero_class = hero;
    selection.team = Some(crate::team::Team::Green);
    commands.write(NetworkCommand::Join {
        team: crate::team::Team::Green,
        character: selection.character,
        hero_class: hero,
        avatar: selection.avatar.clone(),
        sprite_character: Some(selection.sprite_character.clone()),
    });
}
fn sync_state(
    mut state: ResMut<SandboxClient>,
    game: Res<GameStateSnapshot>,
    session: Res<crate::net::ClientSession>,
    mut out: MessageWriter<NetworkCommand>,
) {
    if !state.enabled {
        return;
    }
    let Some(snapshot) = game.sandbox.as_ref() else {
        if session.join_confirmed() {
            state.status="This server is not a Combat Test host. Launch scripts/combat_test.py, then reconnect.".into();
        }
        return;
    };
    let identity = (game.meta.server_epoch, game.meta.match_id, game.your_id);
    if state.identity != Some(identity) {
        state.identity = Some(identity);
        state.pending = None;
        state.queue.clear();
        state.next_id = snapshot.last_request_id.saturating_add(1);
        state.config = snapshot.config.clone();
        state.preview = None;
        state.status = "Combat Test connected · no ranked progress".into();
        state.rebuild = true;
    }
    let now = Instant::now();
    if let Some((request, started, _)) = &state.pending {
        if let Some(ack) = snapshot
            .ack
            .as_ref()
            .filter(|a| a.request_id == request.request_id)
        {
            state.status = if ack.accepted {
                format!("Applied · {}", ack.message)
            } else {
                format!("Rejected · {}", ack.message)
            };
            if !ack.accepted {
                state.queue.clear();
            }
            state.pending = None;
            if state.queue.is_empty() {
                state.config = snapshot.config.clone();
            }
        } else if now.duration_since(*started) > Duration::from_secs(8) {
            state.status = "No server acknowledgement. Reconnect or retry the change.".into();
            state.pending = None;
            state.queue.clear();
            state.config = snapshot.config.clone();
        }
    }
    if state.pending.is_none() && state.queue.is_empty() && state.edit.is_none() {
        state.config = snapshot.config.clone();
    }
    if !state.initial_preset {
        state.initial_preset = true;
        if let Some(name) = &launch().preset {
            match presets::load(name) {
                Ok(config) => {
                    state.config = config;
                    state.apply();
                    state.submit(SandboxCommand::ResetDuel);
                }
                Err(e) => state.status = e,
            }
        }
    }
    if state.pending.is_none() {
        if let Some(command) = state.queue.pop_front() {
            let request = SandboxRequest {
                server_epoch: identity.0,
                match_id: identity.1,
                request_id: state.next_id,
                command,
            };
            state.next_id += 1;
            out.write(NetworkCommand::Sandbox(request.clone()));
            state.pending = Some((request, now, now));
        }
    } else if let Some((request, _, sent)) = &mut state.pending {
        if now.duration_since(*sent) >= Duration::from_millis(300) {
            out.write(NetworkCommand::Sandbox(request.clone()));
            *sent = now;
        }
    }
}
/// Simulation pacing is explicit: network/UI clocks always continue in real time.
pub(crate) fn time_scale(game: Option<&GameStateSnapshot>) -> f32 {
    game.and_then(|g| g.sandbox.as_ref()).map_or(1.0, |s| {
        if s.config.environment.paused {
            0.0
        } else {
            s.config.environment.time_scale
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launch_requires_explicit_mode_and_valid_hero() {
        assert!(!parse_launch(Vec::new(), false).unwrap().enabled);
        assert!(parse_launch(vec!["--hero".into(), "mage".into()], false).is_err());
        assert!(
            parse_launch(
                vec!["--combat-test".into(), "--hero".into(), "wrong".into()],
                false
            )
            .is_err()
        );
        let launch = parse_launch(
            vec!["--combat-test".into(), "--hero".into(), "mage".into()],
            false,
        )
        .unwrap();
        assert_eq!(launch.hero, Some(shared::HeroClass::Mage));
    }
}

fn sync_progression(
    game: Res<GameStateSnapshot>,
    mut players: Query<(
        &crate::net::NetworkPlayerId,
        &mut crate::net::PlayerProgression,
    )>,
) {
    for (id, mut p) in &mut players {
        p.sandbox_unlocked = game
            .sandbox
            .as_ref()
            .and_then(|s| s.actors.iter().find(|a| a.id == id.0))
            .map(|a| a.unlocked);
    }
}
/// The server replicates effective speed including equipment; utility haste is
/// applied by the existing movement code after this baseline.
pub(crate) fn movement_speed(game: Option<&GameStateSnapshot>, ordinary: f32) -> f32 {
    game.and_then(|g| g.sandbox.as_ref())
        .and_then(|s| s.actors.iter().find(|a| a.actor == SandboxActor::Player))
        .map_or(ordinary, |a| a.move_speed)
}

fn remember_cast(mut messages: MessageReader<NetworkCommand>, mut state: ResMut<SandboxClient>) {
    for command in messages.read() {
        if state.enabled {
            if let NetworkCommand::Cast { target, slot } = command {
                state.last_cast = Some((*target, *slot));
                state.last_action = None;
                state.preview = None;
            }
        }
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;
    fn app() -> App {
        let mut app = App::new();
        app.insert_resource(SandboxClient {
            enabled: true,
            open: true,
            ..default()
        })
        .insert_resource(crate::net::ClientSession::admitted_for_test())
        .insert_resource(GameStateSnapshot {
            meta: shared::protocol::SnapshotMeta::new(1, 1, 1),
            sandbox: Some(SandboxSnapshot {
                config: Default::default(),
                ack: None,
                last_request_id: 0,
                actors: vec![],
                analytics: Default::default(),
                simulation_secs: 0.0,
                frame: 1,
            }),
            ..default()
        })
        .add_message::<NetworkCommand>()
        .add_systems(Update, sync_state);
        app.update();
        app
    }
    #[test]
    fn commands_wait_for_ack_and_rejection_cancels_dependent_reset() {
        let mut app = app();
        {
            let mut s = app.world_mut().resource_mut::<SandboxClient>();
            s.config.player.max_hp = -1.0;
            s.apply();
            s.submit(SandboxCommand::ResetDuel);
        }
        app.update();
        {
            let s = app.world().resource::<SandboxClient>();
            assert!(s.pending.is_some());
            assert_eq!(s.queue.len(), 1);
        }
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .sandbox
            .as_mut()
            .unwrap()
            .ack = Some(SandboxAck {
            request_id: 1,
            accepted: false,
            message: "Invalid HP".into(),
        });
        app.update();
        let s = app.world().resource::<SandboxClient>();
        assert!(s.pending.is_none());
        assert!(s.queue.is_empty());
        assert_eq!(s.config.player.max_hp, 100.0);
        assert!(s.status.contains("Rejected"));
    }
    #[test]
    fn new_match_drops_old_commands_and_restores_authoritative_baseline() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<SandboxClient>()
            .submit(SandboxCommand::SpawnWave);
        app.update();
        {
            let mut g = app.world_mut().resource_mut::<GameStateSnapshot>();
            g.meta = shared::protocol::SnapshotMeta::new(2, 1, 1);
            g.sandbox.as_mut().unwrap().config.player.hero = shared::HeroClass::Ranger;
        }
        app.update();
        let s = app.world().resource::<SandboxClient>();
        assert!(s.pending.is_none());
        assert_eq!(s.next_id, 1);
        assert_eq!(s.config.player.hero, shared::HeroClass::Ranger);
    }
    #[test]
    fn reclaimed_actor_continues_above_server_request_high_water() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .sandbox
            .as_mut()
            .unwrap()
            .ack = Some(SandboxAck {
            request_id: 42,
            accepted: true,
            message: "Applied".into(),
        });
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .sandbox
            .as_mut()
            .unwrap()
            .last_request_id = 42;
        app.insert_resource(SandboxClient {
            enabled: true,
            ..default()
        });
        app.update();
        app.world_mut()
            .resource_mut::<SandboxClient>()
            .submit(SandboxCommand::SpawnWave);
        app.update();
        let pending = app
            .world()
            .resource::<SandboxClient>()
            .pending
            .as_ref()
            .unwrap();
        assert_eq!(pending.0.request_id, 43);
        assert_eq!(pending.0.server_epoch, 1);
    }

    #[test]
    fn sandbox_pause_does_not_change_ordinary_match_pacing() {
        assert_eq!(time_scale(None), 1.0);
        let mut g = app().world().resource::<GameStateSnapshot>().clone();
        g.sandbox.as_mut().unwrap().config.environment.paused = true;
        assert_eq!(time_scale(Some(&g)), 0.0);
        g.sandbox = None;
        assert_eq!(time_scale(Some(&g)), 1.0);
    }
}
