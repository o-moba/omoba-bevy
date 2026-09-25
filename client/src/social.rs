//! Ephemeral server-confirmed match communication and exclusive gesture ownership.
use crate::{
    camera::MainCamera,
    combat::{CombatStats, PendingCast},
    input_context::InputContextSet,
    mobile_controls::{MobileControls, MobileControlsSet},
    net::{
        ClientSession, GameState, GameStateSnapshot, NetworkBot, NetworkCommand, NetworkPlayerId,
        SessionEvent, SessionReactions,
    },
    player::{MovementRoute, MovementTarget, Player},
    reaction_visuals::{self, ReactionVisuals},
    sprite::PlayerVisualMode,
    targeting::BasicAttackState,
    ui::{
        Activated, TestId, UiAction, UiActionAppExt, UiSet, theme as ui, theme::ButtonKind,
        widgets::ButtonStyle,
    },
};
use bevy::{
    input::{
        keyboard::{Key, KeyboardInput},
        mouse::{MouseScrollUnit, MouseWheel},
        touch::{TouchInput, TouchPhase},
    },
    prelude::*,
    ui::FocusPolicy,
    window::PrimaryWindow,
};
use shared::social::{
    SocialChannel, SocialCommand, SocialEvent, SocialEventKind, SocialView, normalize_chat_text,
};
use std::{
    collections::{HashSet, VecDeque},
    time::{Duration, Instant},
};

const HOLD_SECONDS: f32 = 0.45;
const MOVE_SLOP: f32 = 12.0;
const REACTION_SECONDS: f32 = 2.8;
const LOG_LIMIT: usize = 40;
fn chat_allowed(joined: bool, state: &GameState) -> bool {
    joined && matches!(state, GameState::Running | GameState::Victory { .. })
}

#[derive(Clone, Debug, PartialEq)]
struct Hold {
    id: u64,
    origin: Vec2,
    elapsed: f32,
}
#[derive(Clone, Debug, Default, PartialEq)]
struct Wheel {
    pending: Option<Hold>,
    owner: Option<u64>,
    center: Option<Vec2>,
    selected: Option<usize>,
}
impl Wheel {
    fn cancel(&mut self) {
        *self = Self::default();
    }
    fn open(&mut self, origin: Vec2, viewport: Vec2, scale: f32) {
        let margin = 112.0 * scale;
        self.center = Some(origin.clamp(
            Vec2::splat(margin),
            (viewport - Vec2::splat(margin)).max(Vec2::splat(margin)),
        ));
        self.selected = None;
    }
    fn event(
        &mut self,
        id: u64,
        phase: TouchPhase,
        position: Vec2,
        start_allowed: bool,
        scale: f32,
    ) -> Option<usize> {
        if !position.is_finite() {
            if self.owner == Some(id) || self.pending.as_ref().is_some_and(|h| h.id == id) {
                self.cancel();
            }
            return None;
        }
        match phase {
            TouchPhase::Started
                if self.owner.is_none()
                    && self.pending.is_none()
                    && self.center.is_none()
                    && start_allowed =>
            {
                self.pending = Some(Hold {
                    id,
                    origin: position,
                    elapsed: 0.0,
                });
            }
            TouchPhase::Moved => {
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|h| h.id == id && h.origin.distance(position) > MOVE_SLOP * scale)
                {
                    self.pending = None;
                }
                if self.owner == Some(id) {
                    self.selected = self
                        .center
                        .and_then(|center| wheel_choice(center, position, scale));
                }
            }
            TouchPhase::Ended | TouchPhase::Canceled => {
                if self.owner == Some(id) {
                    let selected = if phase == TouchPhase::Ended {
                        self.center
                            .and_then(|center| wheel_choice(center, position, scale))
                    } else {
                        None
                    };
                    self.cancel();
                    return selected;
                }
                if self.pending.as_ref().is_some_and(|h| h.id == id) {
                    self.pending = None;
                }
            }
            _ => {}
        }
        None
    }
    fn advance(&mut self, delta: f32, viewport: Vec2, scale: f32) {
        if let Some(hold) = self.pending.as_mut() {
            hold.elapsed += delta.max(0.0);
            if hold.elapsed >= HOLD_SECONDS {
                let (id, origin) = (hold.id, hold.origin);
                self.pending = None;
                self.owner = Some(id);
                self.open(origin, viewport, scale);
            }
        }
    }
}
pub(crate) fn choice_offset(index: usize, scale: f32) -> Vec2 {
    [Vec2::NEG_Y, Vec2::X, Vec2::Y, Vec2::NEG_X][index] * 74.0 * scale
}
fn wheel_choice(center: Vec2, pointer: Vec2, scale: f32) -> Option<usize> {
    (0..4).find(|&i| (center + choice_offset(i, scale)).distance(pointer) <= 30.0 * scale)
}
#[derive(Clone)]
struct PendingSend {
    id: u64,
    command: SocialCommand,
    started: Instant,
    next: Instant,
}
#[derive(Resource)]
pub(crate) struct SocialClient {
    pub(crate) chat_open: bool,
    wheel: Wheel,
    blocked_frame: bool,
    opened_frame: bool,
    draft: String,
    preedit: String,
    ime_owned: bool,
    keyboard_requested: bool,
    channel: SocialChannel,
    namespace: Option<(u64, u64)>,
    server_sequence: u64,
    subscribed: bool,
    subscription: Option<(u64, Instant)>,
    event_sequence: u64,
    request_sequence: u64,
    pending: Option<PendingSend>,
    pub(crate) events: VecDeque<SocialEvent>,
    pub(crate) reactions: Vec<(SocialEvent, Instant)>,
    allowed: Vec<String>,
    status: String,
    wheel_ids: [String; 4],
    last_viewport: Option<Vec2>,
    mute_all: bool,
    muted: HashSet<u64>,
    /// What this frame's button presses may act on; `None` while social input
    /// is gated (another modal, no admitted match, window not usable).
    buttons: Option<ButtonFrame>,
}
/// Filled by `input` for `social_actions`, which runs after the kit's
/// dispatch in the same frame.
#[derive(Clone, Copy, Debug)]
struct ButtonFrame {
    hero: bool,
    hero_point: Option<Vec2>,
    viewport: Vec2,
    scale: f32,
}
impl Default for SocialClient {
    fn default() -> Self {
        Self {
            chat_open: false,
            wheel: Wheel::default(),
            blocked_frame: false,
            opened_frame: false,
            draft: String::new(),
            preedit: String::new(),
            ime_owned: false,
            keyboard_requested: true,
            channel: SocialChannel::Team,
            namespace: None,
            server_sequence: 0,
            subscribed: false,
            subscription: None,
            event_sequence: 0,
            request_sequence: 0,
            pending: None,
            events: VecDeque::new(),
            reactions: Vec::new(),
            allowed: Vec::new(),
            status: String::new(),
            wheel_ids: reaction_visuals::FREE_IDS.map(str::to_owned),
            last_viewport: None,
            mute_all: false,
            muted: HashSet::new(),
            buttons: None,
        }
    }
}
impl SocialClient {
    pub(crate) fn blocks_gameplay(&self) -> bool {
        self.chat_open || self.wheel.center.is_some() || self.blocked_frame
    }
    pub(crate) fn clear(&mut self) {
        let sequence = self.request_sequence;
        let ime_owned = self.ime_owned;
        *self = Self::default();
        self.request_sequence = sequence;
        self.ime_owned = ime_owned;
        self.blocked_frame = true;
    }
    pub(crate) fn bind(&mut self, epoch: u64, match_id: u64) {
        if epoch == 0 || match_id == 0 {
            return;
        }
        if self.namespace != Some((epoch, match_id)) {
            self.clear();
            self.namespace = Some((epoch, match_id));
        }
    }
    pub(crate) fn apply_view(
        &mut self,
        epoch: u64,
        match_id: u64,
        sequence: u64,
        mut view: SocialView,
    ) {
        if self.namespace != Some((epoch, match_id)) || sequence <= self.server_sequence {
            return;
        }
        self.server_sequence = sequence;
        self.subscribed = true;
        self.subscription = None;
        self.allowed = view.allowed_reactions.into_iter().take(256).collect();
        if let Some(pending) = &self.pending
            && view.request_id == Some(pending.id)
        {
            if let Some(error) = view.error {
                self.status = error;
            } else {
                if let SocialCommand::Chat { text, .. } = &pending.command
                    && self.draft.trim() == text
                {
                    self.draft.clear();
                }
                self.status = "Delivered".into();
            }
            self.pending = None;
        }
        view.events.sort_by_key(|event| event.id);
        for event in view.events {
            if event.id <= self.event_sequence {
                continue;
            }
            self.event_sequence = event.id;
            match &event.kind {
                SocialEventKind::Chat { .. } => {
                    self.events.push_back(event);
                    while self.events.len() > LOG_LIMIT {
                        self.events.pop_front();
                    }
                }
                SocialEventKind::Reaction { .. } => {
                    if event.age_ms < (REACTION_SECONDS * 1000.0) as u32 {
                        self.reactions
                            .retain(|(old, _)| old.player_id != event.player_id);
                        let age = Duration::from_millis(event.age_ms as u64);
                        self.reactions.push((event, Instant::now() - age));
                        if self.reactions.len() > shared::social::MAX_SOCIAL_EVENTS {
                            self.reactions.remove(0);
                        }
                    }
                }
            }
        }
    }
    pub(crate) fn request_failed(&mut self, request_id: u64, error: String) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == request_id)
        {
            self.pending = None;
            self.status = error;
        }
    }
    fn send(&mut self, command: SocialCommand, out: &mut MessageWriter<NetworkCommand>) {
        if self.pending.is_some() {
            self.status = "Wait for your previous message to finish.".into();
            return;
        }
        let Some(id) = self.request_sequence.checked_add(1) else {
            return;
        };
        self.request_sequence = id;
        let now = Instant::now();
        self.pending = Some(PendingSend {
            id,
            command: command.clone(),
            started: now,
            next: now + Duration::from_secs(1),
        });
        self.status = "Sending…".into();
        out.write(NetworkCommand::Social {
            request_id: id,
            command,
        });
    }
    fn send_chat(&mut self, out: &mut MessageWriter<NetworkCommand>) {
        match normalize_chat_text(&self.draft) {
            Ok(text) => self.send(
                SocialCommand::Chat {
                    channel: self.channel,
                    text,
                },
                out,
            ),
            Err(error) => self.status = error.into(),
        }
    }
    fn send_reaction(&mut self, index: usize, out: &mut MessageWriter<NetworkCommand>) {
        let id = self.wheel_ids[index].clone();
        if !self.allowed.iter().any(|allowed| allowed == &id) {
            self.status = "This reaction is not available on this server.".into();
            return;
        }
        self.send(SocialCommand::Reaction { reaction_id: id }, out);
    }
    fn open_chat(&mut self) {
        self.chat_open = true;
        self.keyboard_requested = true;
        self.wheel.cancel();
        self.opened_frame = true;
        self.blocked_frame = true;
    }
    fn close(&mut self) {
        self.chat_open = false;
        self.wheel.cancel();
        self.preedit.clear();
        self.blocked_frame = true;
    }
    #[cfg(feature = "qa")]
    pub(crate) fn wheel_center(&self) -> Option<Vec2> {
        self.wheel.center
    }
    #[cfg(feature = "qa")]
    pub(crate) fn qa_send_chat(&mut self, out: &mut MessageWriter<NetworkCommand>) {
        self.chat_open = true;
        self.opened_frame = true;
        self.draft = "QA: Привет 小明 — ready for practice!".into();
        self.send_chat(out);
    }
    #[cfg(feature = "qa")]
    pub(crate) fn qa_close(&mut self) {
        self.close();
    }
    #[cfg(feature = "qa")]
    pub(crate) fn qa_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "namespace": self.namespace,
            "subscribed": self.subscribed,
            "subscription_request_id": self.subscription.map(|(id, _)| id),
            "server_sequence": self.server_sequence,
            "pending": self.pending.as_ref().map(|pending| serde_json::json!({
                "request_id": pending.id,
                "kind": match pending.command {
                    SocialCommand::Subscribe => "subscribe",
                    SocialCommand::Chat { .. } => "chat",
                    SocialCommand::Reaction { .. } => "reaction",
                },
                "elapsed_secs": pending.started.elapsed().as_secs_f64(),
            })),
            "chat_open": self.chat_open,
            "wheel_open": self.wheel.center.is_some(),
            "events": self.events.len(),
            "reactions": self.reactions.len(),
            "allowed_reactions": self.allowed,
            "status": self.status,
        })
    }
    fn visible_sender(&self, id: u64) -> bool {
        !self.mute_all && !self.muted.contains(&id)
    }
}

pub(crate) struct SocialPlugin;
impl Plugin for SocialPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SocialClient>()
            .add_ui_action::<SocialAction>()
            .add_systems(
                Update,
                social_actions
                    .in_set(InputContextSet::Modal)
                    .after(UiSet::Dispatch)
                    .before(crate::career::CareerUiSet),
            )
            .add_systems(
                Update,
                input
                    .in_set(InputContextSet::Social)
                    .after(MobileControlsSet::Layout)
                    .after(crate::net::ClientNetPipeline::ApplySnapshot),
            )
            .add_systems(
                Update,
                chat_keyboard
                    .in_set(InputContextSet::Modal)
                    .after(crate::career::CareerUiSet)
                    .after(crate::mobile_ui::address_keyboard),
            )
            .add_systems(
                Update,
                cancel_orders
                    .after(InputContextSet::Modal)
                    .before(InputContextSet::Resolve),
            )
            .add_systems(Update, render.after(InputContextSet::Actions))
            .add_systems(Update, clear_on_scope_reset.in_set(SessionReactions))
            .add_systems(PostUpdate, scroll_chat.before(bevy::ui::UiSystems::Layout));
        configure_social_bubbles(app);
    }
}
fn configure_social_bubbles(app: &mut App) {
    app.add_systems(
        PostUpdate,
        render_bubbles
            .after(crate::net::NetworkGroundingSet)
            .after(bevy::camera::CameraUpdateSystems)
            .before(bevy::ui::UiSystems::Prepare)
            .before(bevy::ui::UiSystems::Layout)
            .before(bevy::transform::TransformSystems::Propagate),
    );
}
/// Chat, reactions and the namespace belong to the previous server once the
/// session announces [`SessionEvent::ServerScopeReset`]. Runs in
/// `SessionReactions`, in the frame of the reset, so the next frame's ingest
/// binds and fills the new server's view on a cleared client.
pub(crate) fn clear_on_scope_reset(
    mut session_events: MessageReader<SessionEvent>,
    mut social: ResMut<SocialClient>,
) {
    let mut scope_reset = false;
    for event in session_events.read() {
        scope_reset |= *event == SessionEvent::ServerScopeReset;
    }
    if scope_reset {
        social.clear();
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum SocialAction {
    Chat,
    Wheel,
    Close,
    Send,
    Keyboard,
    FocusComposer,
    Channel,
    MuteAll,
    Mute(u64),
    Reaction(usize),
}
#[derive(Component)]
struct SocialRoot;
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SocialBubble {
    pub(crate) event_id: u64,
    player_id: u64,
}
#[derive(bevy::ecs::system::SystemParam)]
struct SocialWorld<'w, 's> {
    session: Res<'w, ClientSession>,
    snapshot: Res<'w, GameStateSnapshot>,
    mobile: Res<'w, MobileControls>,
    career: Option<Res<'w, crate::career::CareerClient>>,
    pause: Option<Res<'w, crate::pause_menu::PauseMenuState>>,
    shop: Option<Res<'w, crate::shop::ShopState>>,
    scoreboard: Option<Res<'w, crate::edge_hud::ScoreboardState>>,
    help: Option<Res<'w, crate::help_overlay::HelpOverlayVisible>>,
    server: Option<Res<'w, crate::mobile_ui::ServerEntry>>,
    visuals: Option<Res<'w, ReactionVisuals>>,
    mode: Res<'w, PlayerVisualMode>,
    local: Query<'w, 's, (&'static Transform, &'static CombatStats), With<Player>>,
    camera: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<MainCamera>>,
    window: Query<'w, 's, (Entity, &'static Window), With<PrimaryWindow>>,
}
impl SocialWorld<'_, '_> {
    fn other_modal(&self) -> bool {
        self.career.as_ref().is_some_and(|v| v.modal_open())
            || self.pause.as_ref().is_some_and(|v| v.open)
            || self.shop.as_ref().is_some_and(|v| v.open)
            || self.scoreboard.as_ref().is_some_and(|v| v.open)
            || self.help.as_ref().is_some_and(|v| v.0)
            || self.server.as_ref().is_some_and(|v| v.open)
    }
}
pub(crate) fn hero_screen(
    camera: &Camera,
    transform: &GlobalTransform,
    mode: PlayerVisualMode,
    position: Vec3,
    height: f32,
) -> Option<Vec2> {
    let position = if mode == PlayerVisualMode::Sprite2d {
        crate::world2d::simulation_xz_to_render_xy(position).extend(crate::world2d::layer::ACTOR)
    } else {
        position + Vec3::Y * height
    };
    camera
        .world_to_viewport(transform, position)
        .ok()
        .filter(|p| p.is_finite())
}
fn input(
    mut social: ResMut<SocialClient>,
    world: SocialWorld,
    time: Res<Time>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut touches: MessageReader<TouchInput>,
    ui_hits: Query<
        (
            &ComputedNode,
            &UiGlobalTransform,
            &Node,
            Option<&InheritedVisibility>,
        ),
        With<Button>,
    >,
    mut out: MessageWriter<NetworkCommand>,
) {
    if let Some(visuals) = world.visuals.as_ref() {
        social.wheel_ids.clone_from(&visuals.wheel);
    }
    social.blocked_frame = false;
    social.opened_frame = false;
    social.buttons = None;
    social
        .reactions
        .retain(|(_, start)| start.elapsed().as_secs_f32() < REACTION_SECONDS);
    let Ok((window_id, window)) = world.window.single() else {
        social.close();
        touches.clear();
        return;
    };
    let admitted = chat_allowed(world.session.join_confirmed(), &world.snapshot.state);
    if !admitted || !window.focused || (world.mobile.enabled && !world.mobile.landscape) {
        if social.namespace.is_some() && !world.session.is_connected() {
            social.clear();
        }
        if social.chat_open || social.wheel.center.is_some() {
            social.close();
        } else {
            social.wheel.cancel();
        }
        touches.clear();
        return;
    }
    // Subscription is transport capability negotiation, independent of an open chat draft.
    if !social.subscribed && social.namespace.is_some() {
        let now = Instant::now();
        if social.subscription.is_none()
            && let Some(id) = social.request_sequence.checked_add(1)
        {
            social.request_sequence = id;
            social.subscription = Some((id, now));
        }
        if let Some((id, next)) = social.subscription.as_mut()
            && now >= *next
        {
            *next = now + Duration::from_secs(2);
            out.write(NetworkCommand::Social {
                request_id: *id,
                command: SocialCommand::Subscribe,
            });
        }
    }
    if world.other_modal() {
        if social.chat_open || social.wheel.center.is_some() {
            social.close();
        } else {
            social.wheel.cancel();
        }
        touches.clear();
        return;
    }
    let had_modal = social.blocks_gameplay();
    let viewport = Vec2::new(window.width(), window.height());
    let scale = world.mobile.scale();
    if social
        .last_viewport
        .is_some_and(|previous| previous != viewport)
    {
        social.wheel.cancel();
        social.blocked_frame = true;
    }
    social.last_viewport = Some(viewport);
    let hero = world.local.single().ok().filter(|(_, stats)| {
        stats.is_alive() && matches!(world.snapshot.state, GameState::Running)
    });
    let hero_point = hero.and_then(|(position, _)| {
        world.camera.single().ok().and_then(|(camera, transform)| {
            hero_screen(camera, transform, *world.mode, position.translation, 1.1)
        })
    });
    if hero.is_none() {
        social.wheel.cancel();
    }
    if keys.just_pressed(KeyCode::Escape) && social.blocks_gameplay() {
        social.close();
        keys.clear_just_pressed(KeyCode::Escape);
    } else if keys.just_pressed(KeyCode::Enter)
        && !social.chat_open
        && social.wheel.center.is_none()
    {
        social.open_chat();
    } else if keys.just_pressed(KeyCode::KeyT) && !social.chat_open && hero.is_some() {
        social.wheel.cancel();
        social
            .wheel
            .open(hero_point.unwrap_or(viewport * 0.5), viewport, scale);
        social.blocked_frame = true;
    }
    social.buttons = Some(ButtonFrame {
        hero: hero.is_some(),
        hero_point,
        viewport,
        scale,
    });
    if social.wheel.center.is_some() && social.wheel.owner.is_none() {
        for (index, key) in [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
        ]
        .into_iter()
        .enumerate()
        {
            if keys.just_pressed(key) {
                social.send_reaction(index, &mut out);
                social.close();
                break;
            }
        }
    }
    if world.mobile.has_active_gesture() {
        social.wheel.pending = None;
    }
    let touch_events: Vec<_> = touches
        .read()
        .filter(|event| event.window == window_id)
        .copied()
        .collect();
    let hud_started = touch_events.iter().any(|event| {
        event.phase == TouchPhase::Started && world.mobile.owns_control_point(event.position)
    });
    if hud_started {
        social.wheel.pending = None;
    }
    for event in &touch_events {
        let blocked = ui_hits.iter().any(|(computed, transform, node, visible)| {
            node.display != Display::None
                && visible.is_none_or(|v| v.get())
                && Rect::from_center_size(
                    transform.translation * computed.inverse_scale_factor(),
                    computed.size() * computed.inverse_scale_factor(),
                )
                .contains(event.position)
        });
        let start = !hud_started
            && world.mobile.enabled
            && !social.chat_open
            && !world.mobile.has_active_gesture()
            && !world.mobile.owns_control_point(event.position)
            && !blocked
            && hero_point.is_some_and(|point| point.distance(event.position) <= 32.0 * scale);
        // Another finger's HUD gesture wins over a pending hold, including its first frame.
        if event.phase == TouchPhase::Started
            && world.mobile.owns_control_point(event.position)
            && social.wheel.center.is_none()
        {
            social.wheel.pending = None;
        }
        let was_open = social.wheel.center.is_some();
        if let Some(index) = social
            .wheel
            .event(event.id, event.phase, event.position, start, scale)
        {
            social.send_reaction(index, &mut out);
        }
        if was_open && social.wheel.center.is_none() {
            social.blocked_frame = true;
        }
    }
    if !social.chat_open && hero.is_some() {
        social.wheel.advance(time.delta_secs(), viewport, scale);
    }
    if !had_modal && social.blocks_gameplay() {
        social.blocked_frame = true;
    }
    if let Some(pending) = social.pending.as_mut() {
        let now = Instant::now();
        if now.duration_since(pending.started) >= Duration::from_secs(5) {
            social.pending = None;
            social.status = "No confirmation. Your message was kept; try again.".into();
        } else if now >= pending.next {
            pending.next = now + Duration::from_secs(1);
            out.write(NetworkCommand::Social {
                request_id: pending.id,
                command: pending.command.clone(),
            });
        }
    }
}
/// Applies this frame's social button presses (`UiSet::Dispatch` ran after
/// `input` filled [`SocialClient::buttons`]). Gated frames drop the presses.
fn social_actions(
    mut social: ResMut<SocialClient>,
    mut activated: MessageReader<Activated<SocialAction>>,
    mut out: MessageWriter<NetworkCommand>,
) {
    let Some(frame) = social.buttons else {
        activated.clear();
        return;
    };
    for Activated { action, .. } in activated.read() {
        match action {
            SocialAction::Chat => {
                social.open_chat();
            }
            SocialAction::Wheel => {
                if frame.hero {
                    social.wheel.cancel();
                    social.wheel.open(
                        frame.hero_point.unwrap_or(frame.viewport * 0.5),
                        frame.viewport,
                        frame.scale,
                    );
                    social.blocked_frame = true;
                }
            }
            SocialAction::Close => {
                if social.wheel.owner.is_none() {
                    social.close();
                }
            }
            SocialAction::Send => social.send_chat(&mut out),
            SocialAction::Keyboard => {
                social.keyboard_requested = !social.keyboard_requested;
            }
            SocialAction::FocusComposer => social.keyboard_requested = true,
            SocialAction::Channel => {
                social.channel = if social.channel == SocialChannel::Team {
                    SocialChannel::Match
                } else {
                    SocialChannel::Team
                }
            }
            SocialAction::MuteAll => social.mute_all = !social.mute_all,
            SocialAction::Mute(id) => {
                if !social.muted.remove(id) {
                    social.muted.insert(*id);
                }
            }
            SocialAction::Reaction(index) => {
                if social.wheel.owner.is_none() {
                    social.send_reaction(*index, &mut out);
                    social.close();
                }
            }
        }
    }
}
fn append_chat(draft: &mut String, text: &str) -> Result<(), &'static str> {
    let next = format!("{draft}{text}");
    if next.chars().count() > shared::social::MAX_CHAT_CHARS
        || next.len() > shared::social::MAX_CHAT_BYTES
    {
        return Err("Use up to 160 characters.");
    }
    if text.chars().any(char::is_control) {
        return Err("Chat cannot contain control characters.");
    }
    *draft = next;
    Ok(())
}
fn is_chat_submit_text(value: &str) -> bool {
    matches!(value, "\n" | "\r" | "\r\n")
}
fn chat_keyboard(
    mut social: ResMut<SocialClient>,
    mut keys: MessageReader<KeyboardInput>,
    mut ime: MessageReader<Ime>,
    mut window: Query<&mut Window, With<PrimaryWindow>>,
    mut out: MessageWriter<NetworkCommand>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mobile: Option<Res<MobileControls>>,
) {
    let phone = mobile.as_ref().is_some_and(|mobile| mobile.enabled);
    if let Ok(mut window) = window.single_mut() {
        if social.chat_open && (!phone || social.keyboard_requested) {
            window.ime_enabled = true;
            social.ime_owned = true;
        } else if social.ime_owned {
            window.ime_enabled = false;
            social.ime_owned = false;
        }
    }
    let mut committed = false;
    for event in ime.read() {
        if !social.chat_open || social.opened_frame {
            continue;
        }
        match event {
            Ime::Preedit { value, .. } => social.preedit = value.clone(),
            Ime::Commit { value, .. } => {
                if is_chat_submit_text(value) {
                    social.send_chat(&mut out);
                } else if let Err(error) = append_chat(&mut social.draft, value) {
                    social.status = error.into();
                }
                social.preedit.clear();
                committed = true;
            }
            _ => {}
        }
    }
    for event in keys.read() {
        if !social.chat_open
            || social.opened_frame
            || !event.state.is_pressed()
            || committed
            || !social.preedit.is_empty()
        {
            continue;
        }
        match &event.logical_key {
            // UIKit's UIKeyInput emits Return as Character("\n"), not Key::Enter.
            Key::Character(value) if is_chat_submit_text(value) => social.send_chat(&mut out),
            Key::Character(value)
                if value.eq_ignore_ascii_case("v")
                    && keyboard.any_pressed([
                        KeyCode::ControlLeft,
                        KeyCode::ControlRight,
                        KeyCode::SuperLeft,
                        KeyCode::SuperRight,
                    ]) =>
            {
                match crate::career::clipboard_text()
                    .and_then(|text| append_chat(&mut social.draft, text.trim()))
                {
                    Ok(()) => {}
                    Err(error) => social.status = error.into(),
                }
            }
            Key::Enter => social.send_chat(&mut out),
            Key::Backspace => {
                social.draft.pop();
            }
            Key::Escape => social.close(),
            _ => {
                if keyboard.any_pressed([
                    KeyCode::ControlLeft,
                    KeyCode::ControlRight,
                    KeyCode::SuperLeft,
                    KeyCode::SuperRight,
                ]) {
                    continue;
                }
                if let Some(text) = event.text.as_deref().or_else(|| match &event.logical_key {
                    Key::Character(text) => Some(text.as_str()),
                    _ => None,
                }) {
                    if let Err(error) = append_chat(&mut social.draft, text) {
                        social.status = error.into();
                    }
                }
            }
        }
    }
}
fn cancel_orders(
    mut commands: Commands,
    social: Res<SocialClient>,
    local: Query<Entity, With<Player>>,
    mut pending: ResMut<PendingCast>,
    mut basic: ResMut<BasicAttackState>,
) {
    if social.blocks_gameplay() {
        for entity in &local {
            commands
                .entity(entity)
                .remove::<(MovementTarget, MovementRoute)>();
        }
        pending.cancel();
        basic.cancel();
    }
}
fn text(
    parent: &mut ChildSpawnerCommands,
    value: impl Into<String>,
    size: f32,
    color: Color,
    name: &str,
) {
    parent.spawn((
        Text::new(value),
        ui::text(size),
        TextColor(color),
        Name::new(name.to_owned()),
    ));
}
fn button(parent: &mut ChildSpawnerCommands, title: &str, action: SocialAction, name: &str) {
    let compact = matches!(name, "SocialOpenChat" | "SocialOpenWheel");
    parent
        .spawn((
            Button,
            Node {
                min_height: Val::Px(44.0),
                min_width: Val::Px(44.0),
                width: if compact { Val::Px(44.0) } else { Val::Auto },
                padding: if compact {
                    UiRect::ZERO
                } else {
                    UiRect::axes(Val::Px(12.0), Val::Px(7.0))
                },
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(ui::TILE),
            BorderColor::all(ui::EDGE),
            ButtonStyle::new(ButtonKind::Secondary),
            UiAction(action),
            TestId::new(name.to_owned()),
        ))
        .with_children(|p| {
            if !compact {
                text(p, title, 14.0, ui::IVORY, "SocialButtonText");
                return;
            }
            p.spawn((
                Node {
                    width: Val::Px(24.0),
                    height: Val::Px(if name == "SocialOpenChat" { 18.0 } else { 24.0 }),
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(if name == "SocialOpenChat" {
                        Val::Px(4.0)
                    } else {
                        Val::Percent(50.0)
                    }),
                    column_gap: Val::Px(3.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BorderColor::all(ui::GOLD),
            ))
            .with_children(|face| {
                for _ in 0..if name == "SocialOpenChat" { 3 } else { 2 } {
                    face.spawn((
                        Node {
                            width: Val::Px(3.0),
                            height: Val::Px(3.0),
                            border_radius: BorderRadius::all(Val::Percent(50.0)),
                            ..default()
                        },
                        BackgroundColor(ui::GOLD),
                    ));
                }
                if name == "SocialOpenWheel" {
                    face.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            bottom: Val::Px(4.0),
                            width: Val::Px(9.0),
                            height: Val::Px(2.0),
                            ..default()
                        },
                        BackgroundColor(ui::GOLD),
                    ));
                }
            });
        });
}
fn row() -> Node {
    Node {
        column_gap: Val::Px(6.0),
        row_gap: Val::Px(6.0),
        flex_wrap: FlexWrap::Wrap,
        align_items: AlignItems::Center,
        ..default()
    }
}
fn column() -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(6.0),
        ..default()
    }
}
fn wheel_render_key(ids: &[String; 4], images: &[Option<ImageNode>; 4]) -> String {
    let resolved: Vec<_> = images
        .iter()
        .map(|image| image.as_ref().map(|image| (image.image.id(), image.rect)))
        .collect();
    format!("{ids:?}{resolved:?}")
}

/// Native UIKit IME overlays the viewport and winit does not expose its frame.
/// Keep all essential composer controls in the first safe-area row, independent
/// of the keyboard's size. Hiding it expands history without closing chat.
#[derive(Clone, Copy, Debug)]
struct PhoneChatLayout {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}
impl PhoneChatLayout {
    fn new(viewport: Vec2, safe: crate::mobile_controls::MobileSafeInsets, keyboard: bool) -> Self {
        let width = (viewport.x - safe.left - safe.right - 12.0).max(0.0);
        let available = (viewport.y - safe.top - safe.bottom - 12.0).max(0.0);
        Self {
            left: safe.left + 6.0,
            top: safe.top + 6.0,
            width,
            height: if keyboard {
                // This is a conservative history budget, not an inferred OS inset.
                (viewport.y * 0.4).max(112.0).min(available)
            } else {
                available
            },
        }
    }
}
fn composer_preview(draft: &str, preedit: &str, width: f32) -> String {
    let capacity = ((width / 16.0) as usize).max(1);
    let value = format!("{draft}{preedit}");
    if value.is_empty() {
        return "Message your team…".into();
    }
    let skip = value.chars().count().saturating_sub(capacity);
    format!(
        "{}{} |",
        if skip > 0 { "…" } else { "" },
        value.chars().skip(skip).collect::<String>()
    )
}
fn render_phone_chat(
    commands: &mut Commands,
    social: &SocialClient,
    mobile: &MobileControls,
    viewport: Vec2,
    old_scroll: ScrollPosition,
) {
    let layout = PhoneChatLayout::new(viewport, mobile.safe, social.keyboard_requested);
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.02, 0.02, 0.96)),
            ZIndex(150),
            SocialRoot,
            Name::new("SocialChatRoot"),
        ))
        .with_children(|p| {
            p.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(layout.left),
                    top: Val::Px(layout.top),
                    width: Val::Px(layout.width),
                    height: Val::Px(layout.height),
                    padding: UiRect::all(Val::Px(6.0)),
                    overflow: Overflow::clip(),
                    ..column()
                },
                BackgroundColor(ui::PANEL),
                Name::new("SocialChatPanel"),
            ))
            .with_children(|p| {
                // Send and Close never move below history or a wrapping draft.
                p.spawn(Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(48.0),
                    flex_shrink: 0.0,
                    flex_wrap: FlexWrap::NoWrap,
                    ..row()
                })
                .with_children(|p| {
                    p.spawn((
                        Button,
                        Node {
                            height: Val::Px(48.0),
                            min_width: Val::Px(0.0),
                            flex_grow: 1.0,
                            flex_basis: Val::Px(0.0),
                            padding: UiRect::horizontal(Val::Px(10.0)),
                            align_items: AlignItems::Center,
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(ui::TILE),
                        UiAction(SocialAction::FocusComposer),
                        TestId::new("SocialPhoneComposer"),
                    ))
                    .with_children(|p| {
                        p.spawn((
                            Text::new(composer_preview(
                                &social.draft,
                                &social.preedit,
                                layout.width - 180.0,
                            )),
                            TextLayout::new_with_no_wrap(),
                            ui::text(16.0),
                            TextColor(ui::IVORY),
                            Name::new("SocialChatInput"),
                        ));
                    });
                    button(p, "Send", SocialAction::Send, "SocialSend");
                    button(p, "Close", SocialAction::Close, "SocialClose");
                });
                p.spawn(Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(44.0),
                    flex_shrink: 0.0,
                    flex_wrap: FlexWrap::NoWrap,
                    overflow: Overflow::clip(),
                    ..row()
                })
                .with_children(|p| {
                    button(
                        p,
                        if social.channel == SocialChannel::Team {
                            "Team"
                        } else {
                            "Match"
                        },
                        SocialAction::Channel,
                        "SocialChannel",
                    );
                    button(
                        p,
                        if social.keyboard_requested {
                            "Hide keys"
                        } else {
                            "Keyboard"
                        },
                        SocialAction::Keyboard,
                        "SocialKeyboard",
                    );
                    button(
                        p,
                        if social.mute_all {
                            "Unmute"
                        } else {
                            "Mute all"
                        },
                        SocialAction::MuteAll,
                        "SocialMuteAll",
                    );
                    p.spawn((
                        Text::new(format!(
                            "{}/160 {}",
                            social.draft.chars().count(),
                            social.status
                        )),
                        TextLayout::new_with_no_wrap(),
                        ui::text(12.0),
                        TextColor(ui::MUTED),
                        Node {
                            min_width: Val::Px(0.0),
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        Name::new("SocialSendStatus"),
                    ));
                });
                render_chat_log(p, social, old_scroll);
            });
        });
}

fn render_chat_log(
    p: &mut ChildSpawnerCommands,
    social: &SocialClient,
    old_scroll: ScrollPosition,
) {
    p.spawn((
        Node {
            flex_grow: 1.0,
            flex_basis: Val::Px(0.0),
            min_height: Val::Px(0.0),
            overflow: Overflow::scroll_y(),
            ..column()
        },
        old_scroll,
        Name::new("SocialChatLog"),
    ))
    .with_children(|p| {
        if social.events.is_empty() {
            text(
                p,
                "No messages yet. Say hello to your team.",
                14.0,
                ui::MUTED,
                "SocialEmpty",
            );
        }
        for event in social
            .events
            .iter()
            .filter(|e| social.visible_sender(e.player_id))
        {
            if let SocialEventKind::Chat {
                channel,
                text: message,
            } = &event.kind
            {
                p.spawn(row()).with_children(|p| {
                    text(
                        p,
                        format!(
                            "{} · {}: {}",
                            if *channel == SocialChannel::Team {
                                "Team"
                            } else {
                                "Match"
                            },
                            event.nickname,
                            message
                        ),
                        14.0,
                        ui::IVORY,
                        "SocialChatMessage",
                    );
                    button(
                        p,
                        "Mute",
                        SocialAction::Mute(event.player_id),
                        "SocialMuteSender",
                    );
                });
            }
        }
        if !social.muted.is_empty() {
            text(p, "Muted players", 12.0, ui::MUTED, "SocialMuted");
            for id in &social.muted {
                button(
                    p,
                    &format!("Unmute player {id}"),
                    SocialAction::Mute(*id),
                    "SocialUnmuteSender",
                );
            }
        }
    });
}

fn render(
    mut commands: Commands,
    social: Res<SocialClient>,
    world: SocialWorld,
    assets: Option<Res<ReactionVisuals>>,
    images: Res<Assets<Image>>,
    roots: Query<Entity, With<SocialRoot>>,
    mut previous: Local<String>,
    scrolls: Query<(&Name, &ScrollPosition)>,
) {
    let Ok((_, window)) = world.window.single() else {
        return;
    };
    let key = format!(
        "{:?}{:?}{:?}{:?}{}{}{}{}{}{}{}{:?}{}{:?}{:?}",
        social.chat_open,
        social.wheel,
        social.events,
        social.muted,
        social.draft,
        social.preedit,
        social.status,
        social.mute_all,
        window.width(),
        window.height(),
        world.session.join_confirmed(),
        world.snapshot.state,
        world.snapshot.match_mode,
        social.allowed,
        world.other_modal()
    );
    let wheel_images: [Option<ImageNode>; 4] = std::array::from_fn(|index| {
        assets
            .as_ref()
            .and_then(|assets| assets.image(&social.wheel_ids[index], &images))
    });
    let key = format!(
        "{key}{}{:?}{}{}{:?}",
        wheel_render_key(&social.wheel_ids, &wheel_images),
        world.mobile.safe,
        world.mobile.enabled,
        social.keyboard_requested,
        social.channel,
    );
    let old_scroll = scrolls
        .iter()
        .find(|(name, _)| name.as_str() == "SocialChatLog")
        .map(|(_, scroll)| scroll.clone())
        .unwrap_or_default();
    if *previous == key {
        return;
    }
    *previous = key;
    for root in &roots {
        commands.entity(root).despawn();
    }
    if !chat_allowed(world.session.join_confirmed(), &world.snapshot.state) || world.other_modal() {
        return;
    }
    let phone = world.mobile.enabled;
    let viewport = Vec2::new(window.width(), window.height());
    let scale = world.mobile.scale();
    let edge_right = if phone { world.mobile.safe.right } else { 16.0 };
    let edge_top = if phone {
        world.mobile.safe.top
    } else {
        desktop_social_top()
    };
    if !social.chat_open && social.wheel.center.is_none() {
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(edge_right + 50.0),
                    top: Val::Px(edge_top),
                    width: Val::Px(94.0),
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::NoWrap,
                    align_items: AlignItems::Center,
                    ..row()
                },
                ZIndex(85),
                SocialRoot,
                Name::new("SocialEntry"),
            ))
            .with_children(|p| {
                button(p, "Chat", SocialAction::Chat, "SocialOpenChat");
                if matches!(world.snapshot.state, GameState::Running) {
                    button(p, "Reactions", SocialAction::Wheel, "SocialOpenWheel");
                }
            });
        if !social.status.is_empty() {
            commands.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(edge_right + 50.0),
                    top: Val::Px(edge_top + 50.0),
                    max_width: Val::Px(180.0),
                    padding: UiRect::all(Val::Px(4.0)),
                    border_radius: BorderRadius::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(ui::PANEL),
                Text::new(&social.status),
                ui::text(12.0),
                TextColor(ui::IVORY),
                ZIndex(85),
                FocusPolicy::Pass,
                SocialRoot,
                Name::new("SocialStatus"),
            ));
        }
        return;
    }
    if social.chat_open && phone {
        render_phone_chat(&mut commands, &social, &world.mobile, viewport, old_scroll);
    } else if social.chat_open {
        let width = (viewport.x - 48.0).min(700.0);
        let height = (viewport.y - 48.0).min(560.0);
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.02, 0.02, 0.96)),
                ZIndex(150),
                SocialRoot,
                Name::new("SocialChatRoot"),
            ))
            .with_children(|p| {
                p.spawn((
                    Node {
                        width: Val::Px(width),
                        height: Val::Px(height),
                        padding: UiRect::all(Val::Px(10.0)),
                        ..column()
                    },
                    BackgroundColor(ui::PANEL),
                    Name::new("SocialChatPanel"),
                ))
                .with_children(|p| {
                    p.spawn(row()).with_children(|p| {
                        text(p, "Match chat", 22.0, ui::GOLD, "SocialChatTitle");
                        button(
                            p,
                            if social.channel == SocialChannel::Team {
                                "Team"
                            } else {
                                "Match"
                            },
                            SocialAction::Channel,
                            "SocialChannel",
                        );
                        button(
                            p,
                            if social.mute_all {
                                "Unmute all"
                            } else {
                                "Mute all"
                            },
                            SocialAction::MuteAll,
                            "SocialMuteAll",
                        );
                        button(p, "Close", SocialAction::Close, "SocialClose");
                    });
                    text(
                        p,
                        if world.snapshot.match_mode == "practice" {
                            "Practice · bots · no ranked or career rewards"
                        } else {
                            "Messages last for this match"
                        },
                        12.0,
                        ui::MUTED,
                        "SocialMode",
                    );
                    render_chat_log(p, &social, old_scroll);
                    text(
                        p,
                        format!("{}{} |", social.draft, social.preedit),
                        16.0,
                        ui::IVORY,
                        "SocialChatInput",
                    );
                    p.spawn(row()).with_children(|p| {
                        button(p, "Send", SocialAction::Send, "SocialSend");
                        text(
                            p,
                            format!("{}/160  {}", social.draft.chars().count(), social.status),
                            12.0,
                            ui::MUTED,
                            "SocialSendStatus",
                        );
                    });
                });
            });
    } else if let Some(center) = social.wheel.center {
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    ..default()
                },
                ZIndex(150),
                SocialRoot,
                Name::new("SocialWheelRoot"),
            ))
            .with_children(|p| {
                for index in 0..4 {
                    let id = &social.wheel_ids[index];
                    let point = center + choice_offset(index, scale);
                    p.spawn((
                        Button,
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(point.x - 29.0 * scale),
                            top: Val::Px(point.y - 29.0 * scale),
                            width: Val::Px(58.0 * scale),
                            height: Val::Px(58.0 * scale),
                            border: UiRect::all(Val::Px(2.0)),
                            ..default()
                        },
                        BackgroundColor(ui::PANEL),
                        BorderColor::all(if social.wheel.selected == Some(index) {
                            ui::GOLD
                        } else {
                            ui::EDGE
                        }),
                        UiAction(SocialAction::Reaction(index)),
                        TestId::new(format!("SocialWheelChoice{index}")),
                    ))
                    .with_children(|p| {
                        if let Some(image) = assets.as_ref().and_then(|a| a.image(id, &images)) {
                            p.spawn((
                                image,
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                                Name::new("SocialReactionImage"),
                            ));
                        } else {
                            text(
                                p,
                                reaction_visuals::label(id),
                                12.0,
                                ui::IVORY,
                                "SocialReactionFallback",
                            );
                        }
                    });
                }
                p.spawn(Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(center.x - 29.0),
                    top: Val::Px(center.y - 22.0),
                    ..default()
                })
                .with_children(|p| button(p, "Cancel", SocialAction::Close, "SocialWheelCancel"));
            });
    }
}
fn render_bubbles(
    mut commands: Commands,
    social: Res<SocialClient>,
    mode: Res<PlayerVisualMode>,
    camera: Query<(Entity, &Camera), With<MainCamera>>,
    transforms: bevy::transform::helper::TransformHelper,
    assets: Option<Res<ReactionVisuals>>,
    images: Res<Assets<Image>>,
    players: Query<(
        Entity,
        &NetworkPlayerId,
        Option<&NetworkBot>,
        &CombatStats,
        Option<&crate::model_scale::NormalizeModelScale>,
        Option<&crate::net::NetworkSpriteCharacter>,
    )>,
    bubbles: Query<(Entity, &SocialBubble)>,
) {
    let current: Vec<_> = bubbles.iter().map(|(entity, key)| (entity, *key)).collect();
    let mut desired = Vec::new();
    let Ok((camera_entity, camera)) = camera.single() else {
        reconcile_bubbles(&mut commands, &current, desired);
        return;
    };
    // These are sampled before UI layout, while propagated globals may still
    // contain last frame's camera or hero poses. Match the target UI pipeline.
    let Ok(camera_transform) = transforms.compute_global_transform(camera_entity) else {
        reconcile_bubbles(&mut commands, &current, desired);
        return;
    };
    let viewport = camera.logical_viewport_size().unwrap_or(Vec2::ZERO);
    for (entity, id, bot, stats, normalization, sprite) in &players {
        if !stats.is_alive() {
            continue;
        }
        let Ok(pose) = transforms.compute_global_transform(entity) else {
            continue;
        };
        let anchor = social_head_anchor(pose.translation(), *mode, normalization, sprite);
        let Ok(point) = camera.world_to_viewport(&camera_transform, anchor) else {
            continue;
        };
        if !point.is_finite() {
            continue;
        }
        if point.x < 32.0
            || point.y < 36.0
            || point.x > viewport.x - 32.0
            || point.y > viewport.y - 32.0
        {
            continue;
        }
        if bot.is_some_and(|bot| bot.0) {
            desired.push(BubbleDraw {
                node: Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(point.x - 24.0),
                    top: Val::Px(point.y - 16.0),
                    width: Val::Px(48.0),
                    height: Val::Px(16.0),
                    ..default()
                },
                image: None,
                key: SocialBubble {
                    event_id: 0,
                    player_id: id.0,
                },
            });
        }
        if !social.visible_sender(id.0) {
            continue;
        }
        if let Some((event, _)) = social.reactions.iter().find(|(event, start)| {
            event.player_id == id.0 && start.elapsed().as_secs_f32() < REACTION_SECONDS
        }) {
            let SocialEventKind::Reaction { reaction_id } = &event.kind else {
                continue;
            };
            let Some(image) = assets.as_ref().and_then(|a| a.image(reaction_id, &images)) else {
                continue;
            };
            desired.push(BubbleDraw {
                node: Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(point.x - 28.0),
                    top: Val::Px(point.y - 48.0),
                    width: Val::Px(56.0),
                    height: Val::Px(56.0),
                    ..default()
                },
                image: Some(image),
                key: SocialBubble {
                    event_id: event.id,
                    player_id: id.0,
                },
            });
        }
    }
    reconcile_bubbles(&mut commands, &current, desired);
}

fn social_head_anchor(
    position: Vec3,
    mode: PlayerVisualMode,
    normalization: Option<&crate::model_scale::NormalizeModelScale>,
    sprite: Option<&crate::net::NetworkSpriteCharacter>,
) -> Vec3 {
    if mode == PlayerVisualMode::Sprite2d {
        let height = crate::sprite_roster::sprite_character_render_definition(
            sprite.and_then(|sprite| sprite.0.as_deref()),
        )
        .map(|definition| (1.0 - definition.pivot[1]) * definition.world_height)
        .unwrap_or(2.6);
        let xy = crate::world2d::simulation_xz_to_render_xy(position);
        (xy + Vec2::Y * (height + 0.2)).extend(crate::world2d::layer::OVERHEAD)
    } else {
        // head_local_y already includes normalization. Do not rotate the offset
        // with animated limbs or multiply it by the model's root scale again.
        position + Vec3::Y * (normalization.and_then(|n| n.head_local_y).unwrap_or(2.6) + 0.2)
    }
}

struct BubbleDraw {
    key: SocialBubble,
    node: Node,
    image: Option<ImageNode>,
}

fn reconcile_bubbles(
    commands: &mut Commands,
    current: &[(Entity, SocialBubble)],
    desired: Vec<BubbleDraw>,
) {
    let mut retained = HashSet::new();
    for draw in desired {
        let existing = current.iter().find(|(_, key)| *key == draw.key);
        let entity = if let Some((entity, _)) = existing {
            *entity
        } else if let Some(image) = draw.image.clone() {
            commands
                .spawn((
                    image,
                    BackgroundColor(ui::PANEL),
                    FocusPolicy::Pass,
                    ZIndex(80),
                    draw.key,
                    Name::new("SocialReactionBubble"),
                ))
                .id()
        } else {
            commands
                .spawn((
                    Text::new("BOT"),
                    TextLayout::new_with_justify(Justify::Center),
                    ui::text(11.0),
                    TextColor(ui::GOLD),
                    FocusPolicy::Pass,
                    // World labels must stay below the opaque HUD cards.
                    ZIndex(-1),
                    draw.key,
                    Name::new("SocialBotLabel"),
                ))
                .id()
        };
        retained.insert(entity);
        commands.entity(entity).insert(draw.node);
        if let Some(image) = draw.image {
            commands.entity(entity).insert(image);
        }
    }
    for (entity, _) in current {
        if !retained.contains(entity) {
            commands.entity(*entity).despawn();
        }
    }
}

fn desktop_social_top() -> f32 {
    crate::minimap::DESKTOP_MINIMAP_INSET
}

fn scroll_chat(
    social: Res<SocialClient>,
    mobile: Res<MobileControls>,
    mut wheel: MessageReader<MouseWheel>,
    mut panels: Query<(&Name, &ComputedNode, &mut ScrollPosition)>,
) {
    let delta: f32 = wheel
        .read()
        .map(|event| {
            event.y
                * if event.unit == MouseScrollUnit::Line {
                    28.0
                } else {
                    1.0
                }
        })
        .sum();
    if !social.chat_open || mobile.enabled {
        return;
    }
    for (name, node, mut scroll) in &mut panels {
        if name.as_str() == "SocialChatLog" {
            let max =
                ((node.content_size().y - node.size().y) * node.inverse_scale_factor()).max(0.0);
            scroll.y = (scroll.y - delta).clamp(0.0, max);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phone_composer_stays_in_top_safe_row_and_history_expands_when_keyboard_hides() {
        let safe = crate::mobile_controls::MobileSafeInsets {
            left: 44.0,
            right: 44.0,
            top: 8.0,
            bottom: 21.0,
        };
        for viewport in [
            Vec2::new(568.0, 320.0),
            Vec2::new(844.0, 390.0),
            Vec2::new(932.0, 430.0),
        ] {
            let typing = PhoneChatLayout::new(viewport, safe, true);
            let reading = PhoneChatLayout::new(viewport, safe, false);
            assert!(typing.left >= safe.left);
            assert!(typing.left + typing.width <= viewport.x - safe.right);
            // Input, Send and Close are in the first 48px row + 6px padding.
            // Even an overlay covering the lower 75% leaves this row accessible.
            assert!(typing.top + 6.0 + 48.0 <= viewport.y * 0.25);
            assert!(typing.top + typing.height <= viewport.y - safe.bottom);
            assert!(reading.height > typing.height);
            assert_eq!(typing.top, reading.top);
            assert_eq!(typing.width, reading.width);
        }
        let preview = composer_preview("Привет 小明, this is a long draft", "中文", 128.0);
        assert!(preview.starts_with('…'));
        assert!(preview.ends_with("中文 |"));
    }
    #[test]
    fn phone_keyboard_can_hide_without_losing_draft_or_releasing_gameplay() {
        let mut app = App::new();
        let mut mobile = MobileControls::default();
        mobile.enabled = true;
        app.insert_resource(mobile)
            .insert_resource(SocialClient {
                chat_open: true,
                draft: "Keep me".into(),
                ..default()
            })
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<KeyboardInput>()
            .add_message::<Ime>()
            .add_message::<NetworkCommand>()
            .add_systems(Update, chat_keyboard);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        app.update();
        assert!(app.world().get::<Window>(window).unwrap().ime_enabled);
        app.world_mut()
            .resource_mut::<SocialClient>()
            .keyboard_requested = false;
        app.update();
        assert!(!app.world().get::<Window>(window).unwrap().ime_enabled);
        assert!(app.world().resource::<SocialClient>().blocks_gameplay());
        assert_eq!(app.world().resource::<SocialClient>().draft, "Keep me");
        app.world_mut()
            .resource_mut::<SocialClient>()
            .keyboard_requested = true;
        app.update();
        assert!(app.world().get::<Window>(window).unwrap().ime_enabled);
        app.world_mut().resource_mut::<SocialClient>().close();
        app.update();
        assert!(!app.world().get::<Window>(window).unwrap().ime_enabled);
        assert!(app.world().resource::<SocialClient>().blocks_gameplay());
        app.world_mut().resource_mut::<SocialClient>().open_chat();
        assert!(app.world().resource::<SocialClient>().keyboard_requested);
        assert_eq!(app.world().resource::<SocialClient>().draft, "Keep me");
    }
    #[test]
    fn ios_character_return_submits_once_and_does_not_insert_control_text() {
        use bevy::input::ButtonState;
        for ime_commit in [false, true] {
            let mut app = App::new();
            app.insert_resource(SocialClient {
                chat_open: true,
                draft: "Hello team".into(),
                ..default()
            })
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<KeyboardInput>()
            .add_message::<Ime>()
            .add_message::<NetworkCommand>()
            .add_systems(Update, chat_keyboard);
            let window = app
                .world_mut()
                .spawn((Window::default(), PrimaryWindow))
                .id();
            if ime_commit {
                app.world_mut().write_message(Ime::Commit {
                    window,
                    value: "\n".into(),
                });
            }
            for state in [ButtonState::Pressed, ButtonState::Released] {
                app.world_mut().write_message(KeyboardInput {
                    window,
                    key_code: KeyCode::Enter,
                    logical_key: Key::Character("\n".into()),
                    text: Some("\n".into()),
                    state,
                    repeat: false,
                });
            }
            app.update();
            let social = app.world().resource::<SocialClient>();
            assert_eq!(social.request_sequence, 1);
            assert_eq!(social.draft, "Hello team");
            assert!(
                matches!(&social.pending.as_ref().unwrap().command, SocialCommand::Chat { text, .. } if text == "Hello team")
            );
            assert_eq!(app.world().resource::<Messages<NetworkCommand>>().len(), 1);
        }
    }
    #[test]
    fn bot_labels_track_current_camera_and_grounded_head_without_yaw_jitter() {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin))
            .init_resource::<SocialClient>()
            .init_resource::<Assets<Image>>()
            .insert_resource(PlayerVisualMode::Models3d);
        configure_social_bubbles(&mut app);
        let mut normalization = crate::model_scale::NormalizeModelScale::for_player_model();
        normalization.head_local_y = Some(1.8);
        let bot = app
            .world_mut()
            .spawn((
                NetworkPlayerId(7),
                NetworkBot(true),
                CombatStats::default(),
                normalization,
                Transform::from_xyz(0.1, 0.0, 0.0).with_scale(Vec3::splat(3.0)),
            ))
            .id();
        let camera = app
            .world_mut()
            .spawn((
                MainCamera,
                Transform::IDENTITY,
                Camera {
                    computed: ComputedCameraValues {
                        clip_from_view: Mat4::from_scale(Vec3::new(1.0, 0.1, 0.01)),
                        target_info: Some(RenderTargetInfo {
                            physical_size: UVec2::new(800, 400),
                            scale_factor: 1.0,
                        }),
                        ..default()
                    },
                    ..default()
                },
            ))
            .id();
        app.update();
        let label = app
            .world_mut()
            .query_filtered::<Entity, With<SocialBubble>>()
            .single(app.world())
            .unwrap();
        app.add_systems(
            PostUpdate,
            (move |mut poses: Query<&mut Transform>| {
                poses.get_mut(bot).unwrap().translation.x = 0.4;
                poses.get_mut(camera).unwrap().translation.x = 0.15;
            })
            .in_set(crate::net::NetworkGroundingSet),
        );
        app.add_systems(
            PostUpdate,
            (move |nodes: Query<&Node>| {
                let node = nodes.get(label).unwrap();
                // Both root poses changed after Update. Old globals would miss x=500.
                assert_eq!(node.left, Val::Px(500.0 - 24.0));
                // Head offset is already normalized, independent of root scale=3.
                assert_eq!(node.top, Val::Px(144.0));
            })
            .in_set(bevy::ui::UiSystems::Layout),
        );
        for frame in 0..12 {
            app.world_mut().get_mut::<Transform>(bot).unwrap().rotation =
                Quat::from_rotation_y(frame as f32);
            app.update();
            assert!(app.world().get::<SocialBubble>(label).is_some());
        }
    }
    #[test]
    fn bubble_nodes_survive_motion_and_only_expire_or_replace_with_authoritative_event() {
        let mut world = World::new();
        let reaction = SocialBubble {
            player_id: 7,
            event_id: 41,
        };
        let bot = SocialBubble {
            player_id: 8,
            event_id: 0,
        };
        let draw = |key: SocialBubble, x: f32| BubbleDraw {
            key,
            node: Node {
                left: Val::Px(x),
                ..default()
            },
            image: (key.event_id != 0).then(ImageNode::default),
        };
        let apply = |world: &mut World, desired| {
            let current: Vec<_> = world
                .query::<(Entity, &SocialBubble)>()
                .iter(world)
                .map(|(entity, key)| (entity, *key))
                .collect();
            reconcile_bubbles(&mut world.commands(), &current, desired);
            world.flush();
        };
        apply(&mut world, vec![draw(reaction, 10.0), draw(bot, 20.0)]);
        let first: Vec<_> = world
            .query::<(Entity, &SocialBubble)>()
            .iter(&world)
            .map(|(entity, key)| (entity, *key))
            .collect();
        let reaction_entity = first.iter().find(|(_, key)| *key == reaction).unwrap().0;
        let bot_entity = first.iter().find(|(_, key)| *key == bot).unwrap().0;
        apply(&mut world, vec![draw(reaction, 30.0), draw(bot, 40.0)]);
        assert_eq!(
            world.get::<Node>(reaction_entity).unwrap().left,
            Val::Px(30.0)
        );
        assert_eq!(world.get::<Node>(bot_entity).unwrap().left, Val::Px(40.0));
        let replacement = SocialBubble {
            event_id: 42,
            ..reaction
        };
        apply(&mut world, vec![draw(replacement, 50.0), draw(bot, 60.0)]);
        assert!(world.get::<SocialBubble>(reaction_entity).is_none());
        assert_eq!(world.get::<SocialBubble>(bot_entity), Some(&bot));
        assert_eq!(world.query::<&SocialBubble>().iter(&world).count(), 2);
        apply(&mut world, Vec::new());
        assert_eq!(world.query::<&SocialBubble>().iter(&world).count(), 0);
    }
    #[test]
    fn wheel_cache_tracks_order_and_every_late_image_handle_and_crop() {
        let ids = reaction_visuals::FREE_IDS.map(str::to_owned);
        let mut images: [Option<ImageNode>; 4] = std::array::from_fn(|_| None);
        let before = wheel_render_key(&ids, &images);
        let mut swapped = ids.clone();
        swapped.swap(0, 1);
        assert_ne!(before, wheel_render_key(&swapped, &images));
        let mut assets = Assets::<Image>::default();
        let first = assets.add(Image::default());
        let second = assets.add(Image::default());
        images[2] = Some(ImageNode::new(first));
        let ready = wheel_render_key(&ids, &images);
        assert_ne!(before, ready);
        images[2].as_mut().unwrap().image = second;
        let changed = wheel_render_key(&ids, &images);
        assert_ne!(ready, changed);
        images[2].as_mut().unwrap().rect = Some(Rect::from_corners(Vec2::ZERO, Vec2::splat(627.0)));
        assert_ne!(changed, wheel_render_key(&ids, &images));
    }
    #[test]
    fn chat_is_available_after_match_but_not_before_admission() {
        let victory = GameState::Victory {
            winner: shared::map::Team::Green,
        };
        assert!(chat_allowed(true, &GameState::Running));
        assert!(chat_allowed(true, &victory));
        assert!(!chat_allowed(false, &victory));
        assert!(!chat_allowed(true, &GameState::Lobby));
    }
    fn event(id: u64, kind: SocialEventKind) -> SocialEvent {
        SocialEvent {
            id,
            player_id: 7,
            nickname: "小明".into(),
            team: shared::social::SocialTeam::Green,
            age_ms: 0,
            kind,
        }
    }
    fn view(events: Vec<SocialEvent>) -> SocialView {
        SocialView {
            events,
            allowed_reactions: reaction_visuals::FREE_IDS.map(str::to_owned).to_vec(),
            ..default()
        }
    }
    #[test]
    fn touch_hold_releases_only_exact_choice_once_and_other_touch_cannot_send() {
        let mut wheel = Wheel::default();
        let p = Vec2::new(420.0, 190.0);
        let size = Vec2::new(844.0, 390.0);
        let press = TouchInput {
            window: Entity::PLACEHOLDER,
            id: 5,
            phase: TouchPhase::Started,
            position: p,
            force: None,
        };
        wheel.event(press.id, press.phase, press.position, true, 1.0);
        wheel.advance(0.44, size, 1.0);
        assert!(wheel.center.is_none());
        wheel.advance(0.02, size, 1.0);
        let center = wheel.center.unwrap();
        let choice = center + choice_offset(2, 1.0);
        assert_eq!(wheel.event(9, TouchPhase::Ended, choice, true, 1.0), None);
        assert!(wheel.center.is_some());
        assert_eq!(wheel.event(5, TouchPhase::Moved, choice, false, 1.0), None);
        assert_eq!(wheel.selected, Some(2));
        assert_eq!(
            wheel.event(5, TouchPhase::Ended, choice, false, 1.0),
            Some(2)
        );
        assert_eq!(wheel.event(5, TouchPhase::Ended, choice, false, 1.0), None);
        assert!(wheel.center.is_none());
    }
    #[test]
    fn movement_cancel_invalid_release_and_os_cancel_never_select() {
        let size = Vec2::new(844.0, 390.0);
        let p = size * 0.5;
        let mut wheel = Wheel::default();
        wheel.event(1, TouchPhase::Started, p, false, 1.0);
        wheel.advance(1.0, size, 1.0);
        assert!(wheel.center.is_none());
        wheel.event(1, TouchPhase::Started, p, true, 1.0);
        wheel.event(1, TouchPhase::Moved, p + Vec2::X * 13.0, false, 1.0);
        wheel.advance(1.0, size, 1.0);
        assert!(wheel.center.is_none());
        for (phase, delta) in [
            (TouchPhase::Canceled, Vec2::NEG_Y * 74.0),
            (TouchPhase::Ended, Vec2::ZERO),
            (TouchPhase::Ended, Vec2::X * 300.0),
            (TouchPhase::Ended, Vec2::splat(f32::NAN)),
        ] {
            wheel.event(1, TouchPhase::Started, p, true, 1.0);
            wheel.advance(1.0, size, 1.0);
            assert_eq!(wheel.event(1, phase, p + delta, false, 1.0), None);
            assert!(wheel.center.is_none());
        }
    }
    #[test]
    fn authoritative_stream_rejects_stale_namespace_sequence_and_never_renews_reactions() {
        let mut social = SocialClient::default();
        social.bind(10, 20);
        let reaction = event(
            1,
            SocialEventKind::Reaction {
                reaction_id: "heart".into(),
            },
        );
        social.apply_view(9, 20, 1, view(vec![reaction.clone()]));
        assert!(social.reactions.is_empty());
        social.apply_view(10, 20, 1, view(vec![reaction.clone()]));
        let start = social.reactions[0].1;
        social.apply_view(10, 20, 2, view(vec![reaction]));
        assert_eq!(social.reactions.len(), 1);
        assert_eq!(social.reactions[0].1, start);
        social.apply_view(
            10,
            20,
            1,
            view(vec![event(
                2,
                SocialEventKind::Chat {
                    channel: SocialChannel::Team,
                    text: "old".into(),
                },
            )]),
        );
        assert!(social.events.is_empty());
        let mut old = event(
            3,
            SocialEventKind::Reaction {
                reaction_id: "heart".into(),
            },
        );
        old.age_ms = 4000;
        social.apply_view(10, 20, 3, view(vec![old]));
        assert_eq!(social.reactions.len(), 1);
        social.request_sequence = 8;
        social.bind(10, 21);
        assert!(social.events.is_empty() && social.reactions.is_empty());
        assert_eq!(social.request_sequence, 8);
        assert!(!social.subscribed);
    }
    #[test]
    fn log_is_bounded_mute_hides_chat_and_reactions_and_wrong_ack_keeps_draft() {
        let mut social = SocialClient::default();
        social.bind(1, 1);
        let events = (1..=80)
            .map(|id| {
                event(
                    id,
                    SocialEventKind::Chat {
                        channel: SocialChannel::Match,
                        text: format!("{id}"),
                    },
                )
            })
            .collect();
        social.apply_view(1, 1, 1, view(events));
        assert_eq!(social.events.len(), LOG_LIMIT);
        social.muted.insert(7);
        assert!(!social.visible_sender(7));
        social.muted.clear();
        social.mute_all = true;
        assert!(!social.visible_sender(7));
        social.draft = "keep me".into();
        let now = Instant::now();
        social.pending = Some(PendingSend {
            id: 9,
            command: SocialCommand::Chat {
                channel: SocialChannel::Team,
                text: "keep me".into(),
            },
            started: now,
            next: now,
        });
        social.apply_view(
            1,
            1,
            2,
            SocialView {
                request_id: Some(8),
                ..default()
            },
        );
        assert!(social.pending.is_some());
        assert_eq!(social.draft, "keep me");
        social.apply_view(
            1,
            1,
            3,
            SocialView {
                request_id: Some(9),
                ..default()
            },
        );
        assert!(social.pending.is_none());
        assert!(social.draft.is_empty());
    }
    #[test]
    fn chat_ime_commits_once_and_closed_frame_blocks_gameplay_orders() {
        use bevy::input::ButtonState;
        let mut app = App::new();
        app.insert_resource(SocialClient {
            chat_open: true,
            ..default()
        })
        .init_resource::<ButtonInput<KeyCode>>()
        .add_message::<KeyboardInput>()
        .add_message::<Ime>()
        .add_message::<NetworkCommand>()
        .add_systems(Update, chat_keyboard);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        app.world_mut().write_message(Ime::Commit {
            window,
            value: "Привет 小明".into(),
        });
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code: KeyCode::KeyA,
            logical_key: Key::Character("Привет 小明".into()),
            text: Some("Привет 小明".into()),
            state: ButtonState::Pressed,
            repeat: false,
        });
        app.update();
        assert_eq!(app.world().resource::<SocialClient>().draft, "Привет 小明");
        assert!(app.world().get::<Window>(window).unwrap().ime_enabled);
        app.world_mut().resource_mut::<SocialClient>().close();
        app.update();
        assert!(app.world().resource::<SocialClient>().blocks_gameplay());
        assert!(!app.world().get::<Window>(window).unwrap().ime_enabled);
        let mut value = "a".repeat(160);
        let before = value.clone();
        assert!(append_chat(&mut value, "小").is_err());
        assert_eq!(value, before);
    }
    #[test]
    fn opening_or_closing_social_cancels_existing_routes_and_basic_order() {
        let mut app = App::new();
        app.insert_resource(SocialClient {
            blocked_frame: true,
            ..default()
        })
        .init_resource::<PendingCast>()
        .init_resource::<BasicAttackState>()
        .add_systems(Update, cancel_orders);
        let hero = app
            .world_mut()
            .spawn((
                Player,
                MovementTarget { target: Vec3::X },
                MovementRoute {
                    requested_target: Vec3::X,
                    destination: Vec3::X,
                    structure_revision: 0,
                    waypoints: vec![Vec3::X],
                },
            ))
            .id();
        app.update();
        assert!(app.world().get::<MovementTarget>(hero).is_none());
        assert!(app.world().get::<MovementRoute>(hero).is_none());
        assert!(!app.world().resource::<PendingCast>().is_pending());
    }

    #[test]
    fn social_presses_apply_once_gated_frames_drop_them_and_disabled_do_nothing() {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.init_resource::<SocialClient>()
            .add_message::<NetworkCommand>()
            .add_ui_action::<SocialAction>()
            .add_systems(Update, social_actions.after(UiSet::Dispatch));
        harness::spawn_ui(app.world_mut(), |p| {
            button(p, "Close", SocialAction::Close, "SocialClose");
            button(p, "Mute all", SocialAction::MuteAll, "SocialMuteAll");
        });
        let frame = ButtonFrame {
            hero: false,
            hero_point: None,
            viewport: Vec2::new(1280.0, 720.0),
            scale: 1.0,
        };
        app.world_mut().resource_mut::<SocialClient>().chat_open = true;
        app.update();
        // Gated (input left no button frame): the press is dropped for good.
        harness::press(app.world_mut(), "SocialClose");
        app.update();
        app.world_mut().resource_mut::<SocialClient>().buttons = Some(frame);
        app.update();
        assert!(app.world().resource::<SocialClient>().chat_open);
        harness::press(app.world_mut(), "SocialClose");
        app.update();
        assert!(!app.world().resource::<SocialClient>().chat_open);
        harness::press(app.world_mut(), "SocialMuteAll");
        app.update();
        assert!(app.world().resource::<SocialClient>().mute_all);
        app.update();
        assert!(app.world().resource::<SocialClient>().mute_all, "no repeat");
        harness::set_disabled(app.world_mut(), "SocialMuteAll", true);
        harness::press(app.world_mut(), "SocialMuteAll");
        app.update();
        assert!(app.world().resource::<SocialClient>().mute_all);
    }
}
