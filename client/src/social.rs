//! Ephemeral server-confirmed match communication and exclusive gesture ownership.
use crate::{
    camera::MainCamera,
    combat::{CombatStats, PendingCast},
    input_context::InputContextSet,
    mobile_controls::{MobileControls, MobileControlsSet},
    net::{
        ClientSession, GameState, GameStateSnapshot, NetworkBot, NetworkCommand, NetworkPlayerId,
    },
    player::{MovementRoute, MovementTarget, Player},
    reaction_visuals::{self, ReactionVisuals},
    sprite::PlayerVisualMode,
    targeting::BasicAttackState,
    ui_theme as ui,
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
    fn close(&mut self) {
        self.chat_open = false;
        self.wheel.cancel();
        self.preedit.clear();
        self.blocked_frame = true;
    }
    pub(crate) fn wheel_center(&self) -> Option<Vec2> {
        self.wheel.center
    }
    pub(crate) fn qa_send_chat(&mut self, out: &mut MessageWriter<NetworkCommand>) {
        self.chat_open = true;
        self.opened_frame = true;
        self.draft = "QA: Привет 小明 — ready for practice!".into();
        self.send_chat(out);
    }
    pub(crate) fn qa_close(&mut self) {
        self.close();
    }
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
            .add_systems(Update, render_bubbles.after(InputContextSet::Actions))
            .add_systems(PostUpdate, scroll_chat.before(bevy::ui::UiSystems::Layout));
    }
}
#[derive(Component, Clone)]
enum SocialAction {
    Chat,
    Wheel,
    Close,
    Send,
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
    buttons: Query<(&Interaction, &SocialAction), (With<Button>, Changed<Interaction>)>,
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
        social.chat_open = true;
        social.opened_frame = true;
        social.blocked_frame = true;
    } else if keys.just_pressed(KeyCode::KeyT) && !social.chat_open && hero.is_some() {
        social.wheel.cancel();
        social
            .wheel
            .open(hero_point.unwrap_or(viewport * 0.5), viewport, scale);
        social.blocked_frame = true;
    }
    for (interaction, action) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            SocialAction::Chat => {
                social.chat_open = true;
                social.wheel.cancel();
                social.opened_frame = true;
                social.blocked_frame = true;
            }
            SocialAction::Wheel => {
                if hero.is_some() {
                    social.wheel.cancel();
                    social
                        .wheel
                        .open(hero_point.unwrap_or(viewport * 0.5), viewport, scale);
                    social.blocked_frame = true;
                }
            }
            SocialAction::Close => {
                if social.wheel.owner.is_none() {
                    social.close();
                }
            }
            SocialAction::Send => social.send_chat(&mut out),
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
fn chat_keyboard(
    mut social: ResMut<SocialClient>,
    mut keys: MessageReader<KeyboardInput>,
    mut ime: MessageReader<Ime>,
    mut window: Query<&mut Window, With<PrimaryWindow>>,
    mut out: MessageWriter<NetworkCommand>,
    keyboard: Res<ButtonInput<KeyCode>>,
) {
    if let Ok(mut window) = window.single_mut() {
        if social.chat_open {
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
                if let Err(error) = append_chat(&mut social.draft, value) {
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
    parent
        .spawn((
            Button,
            Node {
                min_height: Val::Px(44.0),
                min_width: Val::Px(44.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(ui::TILE),
            action,
            Name::new(name.to_owned()),
        ))
        .with_children(|p| text(p, title, 14.0, ui::IVORY, "SocialButtonText"));
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

#[allow(clippy::too_many_arguments)]
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
        "{key}{}",
        wheel_render_key(&social.wheel_ids, &wheel_images)
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
    if !social.chat_open && social.wheel.center.is_none() {
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(if phone {
                        viewport.x * 0.5 - 100.0
                    } else {
                        crate::minimap::DESKTOP_MINIMAP_INSET
                    }),
                    top: if phone {
                        Val::Auto
                    } else {
                        Val::Px(desktop_social_top())
                    },
                    bottom: if phone {
                        Val::Px(world.mobile.safe.bottom + 4.0)
                    } else {
                        Val::Auto
                    },
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
                    left: if phone {
                        Val::Percent(35.0)
                    } else {
                        Val::Px(crate::minimap::DESKTOP_MINIMAP_INSET)
                    },
                    top: if phone {
                        Val::Auto
                    } else {
                        Val::Px(desktop_social_top() + 52.0)
                    },
                    bottom: if phone {
                        Val::Px(world.mobile.safe.bottom + 52.0)
                    } else {
                        Val::Auto
                    },
                    max_width: if phone {
                        Val::Auto
                    } else {
                        Val::Px(crate::minimap::MINIMAP_SIZE)
                    },
                    ..default()
                },
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
    if social.chat_open {
        let width = if phone {
            viewport.x - world.mobile.safe.left - world.mobile.safe.right - 12.0
        } else {
            (viewport.x - 48.0).min(700.0)
        };
        let height = if phone {
            viewport.y - world.mobile.safe.top - world.mobile.safe.bottom - 12.0
        } else {
            (viewport.y - 48.0).min(560.0)
        };
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
                        SocialAction::Reaction(index),
                        Name::new(format!("SocialWheelChoice{index}")),
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
#[allow(clippy::too_many_arguments)]
fn render_bubbles(
    mut commands: Commands,
    social: Res<SocialClient>,
    world: SocialWorld,
    assets: Option<Res<ReactionVisuals>>,
    images: Res<Assets<Image>>,
    players: Query<(
        &NetworkPlayerId,
        &Transform,
        Option<&NetworkBot>,
        &CombatStats,
    )>,
    bubbles: Query<(Entity, &SocialBubble)>,
) {
    let current: Vec<_> = bubbles.iter().map(|(entity, key)| (entity, *key)).collect();
    let mut desired = Vec::new();
    let Ok((camera, camera_transform)) = world.camera.single() else {
        reconcile_bubbles(&mut commands, &current, desired);
        return;
    };
    let viewport = camera.logical_viewport_size().unwrap_or(Vec2::ZERO);
    for (id, position, bot, stats) in &players {
        if !stats.is_alive() {
            continue;
        }
        let Some(point) = hero_screen(
            camera,
            camera_transform,
            *world.mode,
            position.translation,
            2.6,
        ) else {
            continue;
        };
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
                    left: Val::Px(point.x - 12.0),
                    top: Val::Px(point.y + 24.0),
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
    crate::minimap::DESKTOP_MINIMAP_INSET + crate::minimap::MINIMAP_SIZE + 8.0
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
            winner: crate::team::Team::Green,
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
}
