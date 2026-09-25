//! Native two-thumb controls. Touch ownership is per finger and survives crossing
//! another control; only a fresh Started event can capture an input.
use std::collections::HashMap;

use crate::net::TargetKind;
use bevy::{
    input::touch::{TouchInput, TouchPhase},
    prelude::*,
    ui::FocusPolicy,
    window::{AppLifecycle, PrimaryWindow, WindowFocused},
};
use shared::utility::UtilityAction;
use shared::{SkillSlot, ability_for_class_slot, scaled_mana_cost};

use crate::{
    combat::{CombatStats, LocalCastCooldown},
    input_context::{GameplayInputContext, InputContextSet},
    net::{NetworkHeroClass, PlayerProgression, SessionEvent},
    player::Player,
    team::TeamSelection,
};

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MobileControlsSet {
    Layout,
    Input,
    Visuals,
}

/// Conservative logical-pixel safe area until native OS insets are available.
/// The edge gutter reserves room for camera cutouts and the home indicator.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MobileSafeInsets {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MobileLayout {
    pub joystick_center: Vec2,
    pub joystick_radius: f32,
    pub attack_center: Vec2,
    pub attack_radius: f32,
    pub cancel_center: Vec2,
    pub cancel_radius: f32,
    pub ability_centers: [Vec2; 4],
    pub ability_radii: [f32; 4],
    pub category_centers: [Vec2; 2],
    pub utility_centers: [Vec2; 2],
    pub auxiliary_radius: f32,
    pub upgrade_center: Vec2,
    pub upgrade_radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MobileCastIntent {
    pub slot: usize,
    pub aim: Option<Vec2>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MobileAttackAim {
    /// Logical screen axes, normalized. Positive Y points down.
    pub direction: Vec2,
    /// Fraction of the bounded visual handle; selection continues along its ray.
    pub extent: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MobileAttackIntent {
    pub aim: Option<MobileAttackAim>,
    pub gesture: u64,
}

const ATTACK_HOLD_SECONDS: f32 = 0.18;
const ATTACK_DRAG_DEAD_ZONE: f32 = 12.0;
const ATTACK_DRAG_REACH: f32 = 96.0;
const SKILL_DESCRIPTION_SECONDS: f32 = 0.45;
const SKILL_DRAG_DEAD_ZONE: f32 = 20.0;
const COMBAT_ORBIT_RADIUS: f32 = 92.0;
const SKILL_ORBIT_ANGLES: [f32; 4] = [150.0, 200.0, 250.0, 310.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Joystick,
    Attack,
    Ability(usize),
    Upgrade(usize),
    UpgradeMode,
    CategoryAttack(TargetKind),
    Utility(UtilityAction),
}

#[derive(Debug, Clone, Copy)]
struct Capture {
    control: Control,
    origin: Vec2,
    position: Vec2,
    canceled: bool,
    held_seconds: f32,
    hold_exposed: bool,
    dragged: bool,
    inspecting: bool,
    gesture: u64,
}

#[derive(Resource)]
pub(crate) struct MobileControls {
    /// Chosen from the compiled platform once; resizing never changes this.
    pub enabled: bool,
    pub viewport: Vec2,
    pub landscape: bool,
    pub safe: MobileSafeInsets,
    pub focused: bool,
    /// Screen axes: positive X right, positive Y down; magnitude is analog 0..1.
    pub movement: Vec2,
    pub casts: Vec<MobileCastIntent>,
    pub attacks: Vec<MobileAttackIntent>,
    pub upgrades: Vec<usize>,
    pub category_attacks: Vec<TargetKind>,
    pub utilities: Vec<(UtilityAction, Option<Vec2>)>,
    upgrade_mode: bool,
    captures: HashMap<u64, Capture>,
    upgrade_enabled: [bool; 4],
    layout_changed: bool,
    next_attack_gesture: u64,
    attack_canceled_this_frame: bool,
    skill_released_this_frame: bool,
}

impl Default for MobileControls {
    fn default() -> Self {
        Self {
            // `MobileControlsPlugin` copies the platform in; tests set it.
            enabled: false,
            viewport: Vec2::new(844.0, 390.0),
            landscape: true,
            safe: MobileSafeInsets {
                left: 32.0,
                right: 32.0,
                top: 12.0,
                bottom: 20.0,
            },
            focused: true,
            movement: Vec2::ZERO,
            casts: Vec::new(),
            attacks: Vec::new(),
            upgrades: Vec::new(),
            category_attacks: Vec::new(),
            utilities: Vec::new(),
            upgrade_mode: false,
            captures: HashMap::new(),
            upgrade_enabled: [false; 4],
            layout_changed: false,
            next_attack_gesture: 0,
            attack_canceled_this_frame: false,
            skill_released_this_frame: false,
        }
    }
}

impl MobileControls {
    pub(crate) fn owns_control_point(&self, point: Vec2) -> bool {
        self.hit_control(point).is_some()
    }
    pub(crate) fn has_active_gesture(&self) -> bool {
        !self.captures.is_empty()
    }

    pub fn scale(&self) -> f32 {
        (self.viewport.y / 390.0).clamp(0.85, 1.25)
    }

    /// Scale the complete combat group without shrinking its 44px touch targets.
    /// Keep this separate from frontend scale: small phones still need full-size
    /// controls, while menu typography and panels may use the smaller UI scale.
    pub(crate) fn combat_scale(&self) -> f32 {
        let usable_width = self.viewport.x - self.safe.left - self.safe.right;
        // 443.6 covers the joystick's expanded hit circle and leftmost utility.
        // Preserve at least 48px between them on narrow landscape viewports.
        self.scale().min((usable_width - 48.0) / 443.6).max(1.0)
    }

    pub fn layout(&self) -> MobileLayout {
        // All six satellites share one circle centered exactly on basic attack.
        // Anchor and scale the group together; clamping individual controls to
        // viewport edges would break both the orbit and the thumb's muscle memory.
        let s = self.combat_scale();
        let right = self.viewport.x - self.safe.right;
        let bottom = self.viewport.y - self.safe.bottom;
        let attack_center = Vec2::new(right - 114.0 * s, bottom - 114.0 * s);
        let orbit = |angle: f32| {
            let (sin, cos) = angle.to_radians().sin_cos();
            attack_center + Vec2::new(cos, sin) * COMBAT_ORBIT_RADIUS * s
        };
        let outside = |x: f32, y: f32| attack_center + Vec2::new(x, y) * s;
        MobileLayout {
            joystick_center: Vec2::new(self.safe.left + 72.0 * s, bottom - 70.0 * s),
            joystick_radius: 52.0 * s,
            attack_center,
            attack_radius: 38.0 * s,
            cancel_center: outside(-156.0, -92.0),
            cancel_radius: 22.0 * s,
            ability_centers: SKILL_ORBIT_ANGLES.map(orbit),
            ability_radii: [25.0 * s; 4],
            category_centers: [orbit(90.0), orbit(0.0)],
            utility_centers: [outside(-168.0, 92.0), outside(-118.0, 92.0)],
            auxiliary_radius: 22.0 * s,
            upgrade_center: outside(-150.0, -34.0),
            upgrade_radius: 22.0 * s,
        }
    }

    fn clear(&mut self) {
        self.attack_canceled_this_frame |= self
            .captures
            .values()
            .any(|capture| matches!(capture.control, Control::Attack | Control::Ability(_)));
        self.captures.clear();
        self.upgrade_mode = false;
        self.movement = Vec2::ZERO;
        self.casts.clear();
        self.attacks.clear();
        self.upgrades.clear();
        self.category_attacks.clear();
        self.utilities.clear();
    }

    fn hit_control(&self, point: Vec2) -> Option<(Control, Vec2)> {
        let l = self.layout();
        if self.upgrade_enabled.iter().any(|enabled| *enabled)
            && point.distance(l.upgrade_center) <= l.upgrade_radius
        {
            return Some((Control::UpgradeMode, l.upgrade_center));
        }
        for (index, kind) in [TargetKind::Minion, TargetKind::Structure]
            .into_iter()
            .enumerate()
        {
            if point.distance(l.category_centers[index]) <= l.auxiliary_radius {
                return Some((Control::CategoryAttack(kind), l.category_centers[index]));
            }
        }
        for (index, action) in [UtilityAction::Dash, UtilityAction::Haste]
            .into_iter()
            .enumerate()
        {
            if point.distance(l.utility_centers[index]) <= l.auxiliary_radius {
                return Some((Control::Utility(action), l.utility_centers[index]));
            }
        }
        if point.distance(l.attack_center) <= l.attack_radius {
            return Some((Control::Attack, l.attack_center));
        }
        for slot in 0..4 {
            if point.distance(l.ability_centers[slot]) <= l.ability_radii[slot] {
                return Some((
                    if self.upgrade_mode {
                        Control::Upgrade(slot)
                    } else {
                        Control::Ability(slot)
                    },
                    l.ability_centers[slot],
                ));
            }
        }
        // Fixed anchor avoids moving the joystick under minimap/menu touches.
        (point.distance(l.joystick_center) <= l.joystick_radius * 1.3)
            .then_some((Control::Joystick, l.joystick_center))
    }

    fn event(&mut self, id: u64, phase: TouchPhase, position: Vec2) {
        if !position.is_finite() {
            if let Some(capture) = self.captures.remove(&id) {
                self.attack_canceled_this_frame |=
                    matches!(capture.control, Control::Attack | Control::Ability(_));
                self.skill_released_this_frame |= matches!(capture.control, Control::Ability(_));
            }
            self.refresh_movement();
            return;
        }
        match phase {
            TouchPhase::Started => {
                if self.captures.contains_key(&id) {
                    return;
                }
                let Some((control, origin)) = self.hit_control(position) else {
                    return;
                };
                if self.captures.values().any(|c| {
                    c.control == control
                        || matches!(
                            (control, c.control),
                            (
                                Control::Ability(_) | Control::Upgrade(_),
                                Control::Ability(_) | Control::Upgrade(_)
                            )
                        )
                }) {
                    return;
                }
                if matches!(
                    control,
                    Control::Attack | Control::CategoryAttack(_) | Control::Utility(_)
                ) {
                    self.upgrade_mode = false;
                }
                let gesture = if control == Control::Attack {
                    let Some(next) = self.next_attack_gesture.checked_add(1) else {
                        return;
                    };
                    self.next_attack_gesture = next;
                    next
                } else {
                    0
                };
                self.captures.insert(
                    id,
                    Capture {
                        control,
                        origin: if control != Control::Joystick {
                            position
                        } else {
                            origin
                        },
                        position,
                        canceled: false,
                        held_seconds: 0.0,
                        hold_exposed: false,
                        dragged: false,
                        inspecting: false,
                        gesture,
                    },
                );
            }
            TouchPhase::Moved => {
                let layout = self.layout();
                let scale = self.combat_scale();
                if let Some(capture) = self.captures.get_mut(&id) {
                    capture.position = position;
                    capture.dragged |= position.distance(capture.origin)
                        > control_drag_dead_zone(capture.control) * scale;
                    capture.canceled =
                        position.distance(layout.cancel_center) <= layout.cancel_radius;
                }
            }
            TouchPhase::Ended | TouchPhase::Canceled => {
                if let Some(mut capture) = self.captures.remove(&id) {
                    self.skill_released_this_frame |=
                        matches!(capture.control, Control::Ability(_));
                    capture.position = position;
                    capture.dragged |= position.distance(capture.origin)
                        > control_drag_dead_zone(capture.control) * self.combat_scale();
                    capture.canceled = position.distance(self.layout().cancel_center)
                        <= self.layout().cancel_radius;
                    self.attack_canceled_this_frame |= capture.control == Control::Attack
                        && (phase == TouchPhase::Canceled
                            || capture.canceled
                            || (capture.dragged
                                && attack_aim_vector(
                                    capture.position - capture.origin,
                                    self.combat_scale(),
                                )
                                .is_none()));
                    if phase == TouchPhase::Ended && !capture.canceled {
                        match capture.control {
                            Control::Attack if !self.skill_aiming() && self.casts.is_empty() => {
                                if capture.dragged {
                                    if let Some(aim) = attack_aim_vector(
                                        capture.position - capture.origin,
                                        self.combat_scale(),
                                    ) {
                                        self.attacks.push(MobileAttackIntent {
                                            aim: Some(aim),
                                            gesture: capture.gesture,
                                        });
                                    }
                                } else if !capture.hold_exposed {
                                    // A stationary hold has already requested repeats; releasing it
                                    // must not enqueue an extra attack at the next cooldown boundary.
                                    self.attacks.push(MobileAttackIntent {
                                        aim: None,
                                        gesture: capture.gesture,
                                    });
                                }
                            }
                            Control::Ability(slot) if !capture.inspecting => {
                                self.casts.push(MobileCastIntent {
                                    slot,
                                    aim: aim_vector(
                                        capture.position - capture.origin,
                                        self.combat_scale(),
                                    ),
                                })
                            }
                            Control::Upgrade(slot)
                                if self.upgrade_enabled[slot]
                                    && position.distance(self.layout().ability_centers[slot])
                                        <= self.layout().ability_radii[slot] =>
                            {
                                self.upgrades.push(slot);
                                self.upgrade_mode = false;
                            }
                            Control::UpgradeMode if !capture.dragged => {
                                self.upgrade_mode = !self.upgrade_mode;
                            }
                            Control::CategoryAttack(kind) if !capture.dragged => {
                                self.category_attacks.push(kind);
                            }
                            Control::Utility(action) => {
                                self.utilities.push((
                                    action,
                                    aim_vector(
                                        capture.position - capture.origin,
                                        self.combat_scale(),
                                    ),
                                ));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        self.refresh_movement();
    }

    fn refresh_movement(&mut self) {
        self.movement = self
            .captures
            .values()
            .find(|c| c.control == Control::Joystick)
            .map(|c| joystick_vector(c.position - c.origin, self.layout().joystick_radius))
            .unwrap_or(Vec2::ZERO);
    }

    /// Includes a skill release for the current frame even after combat drains casts.
    pub(crate) fn skill_aiming(&self) -> bool {
        self.skill_released_this_frame
            || self
                .captures
                .values()
                .any(|capture| matches!(capture.control, Control::Ability(_)))
    }

    pub(crate) fn attack_pressed(&self) -> bool {
        self.attack_gesture().is_some()
    }

    pub(crate) fn attack_gesture(&self) -> Option<u64> {
        self.captures
            .values()
            .find(|capture| capture.control == Control::Attack)
            .map(|capture| capture.gesture)
    }

    fn begin_input_frame(&mut self) {
        self.casts.clear();
        self.attacks.clear();
        self.upgrades.clear();
        self.category_attacks.clear();
        self.utilities.clear();
        self.attack_canceled_this_frame = false;
        self.skill_released_this_frame = false;
    }

    fn advance_hold_time(&mut self, seconds: f32) {
        if !seconds.is_finite() || seconds <= 0.0 {
            return;
        }
        for capture in self.captures.values_mut() {
            capture.held_seconds += seconds;
            capture.inspecting |= matches!(capture.control, Control::Ability(_))
                && !capture.dragged
                && !capture.canceled
                && capture.held_seconds >= SKILL_DESCRIPTION_SECONDS;
        }
    }

    fn inspected_skill(&self) -> Option<usize> {
        self.captures
            .values()
            .find_map(|capture| match capture.control {
                Control::Ability(slot) if capture.inspecting && !capture.canceled => Some(slot),
                _ => None,
            })
    }

    fn acknowledge_hold(&mut self) {
        if self.held_basic_attack() {
            for capture in self
                .captures
                .values_mut()
                .filter(|capture| capture.control == Control::Attack)
            {
                capture.hold_exposed = true;
            }
        }
    }

    /// Repetition is scheduled by combat against the independent basic cooldown.
    pub(crate) fn held_basic_attack(&self) -> bool {
        !self.skill_aiming()
            && self.casts.is_empty()
            && self.captures.values().any(|capture| {
                capture.control == Control::Attack
                    && !capture.canceled
                    && !capture.dragged
                    && capture.held_seconds >= ATTACK_HOLD_SECONDS
            })
    }

    pub(crate) fn attack_aim(&self) -> Option<MobileAttackAim> {
        if self.skill_aiming() {
            return None;
        }
        self.captures
            .values()
            .find(|capture| {
                capture.control == Control::Attack && capture.dragged && !capture.canceled
            })
            .and_then(|capture| {
                attack_aim_vector(capture.position - capture.origin, self.combat_scale())
            })
    }

    pub(crate) fn attack_cancelled(&self) -> bool {
        self.attack_canceled_this_frame
            || self.captures.values().any(|capture| {
                capture.control == Control::Attack
                    && (capture.canceled || (capture.dragged && self.attack_aim().is_none()))
            })
    }

    #[cfg(test)]
    pub(crate) fn start_attack_hold_for_test(&mut self) {
        self.event(1, TouchPhase::Started, self.layout().attack_center);
        self.advance_hold_time(ATTACK_HOLD_SECONDS);
        self.acknowledge_hold();
    }
}

fn joystick_vector(delta: Vec2, radius: f32) -> Vec2 {
    let distance = delta.length();
    let dead_zone = radius * 0.16;
    if distance <= dead_zone {
        return Vec2::ZERO;
    }
    delta.normalize_or_zero() * ((distance - dead_zone) / (radius - dead_zone)).clamp(0.0, 1.0)
}
fn control_drag_dead_zone(control: Control) -> f32 {
    if matches!(control, Control::Ability(_)) {
        SKILL_DRAG_DEAD_ZONE
    } else {
        ATTACK_DRAG_DEAD_ZONE
    }
}
fn aim_vector(delta: Vec2, scale: f32) -> Option<Vec2> {
    (delta.length() > SKILL_DRAG_DEAD_ZONE * scale).then(|| delta.normalize_or_zero())
}

fn attack_aim_vector(delta: Vec2, scale: f32) -> Option<MobileAttackAim> {
    let distance = delta.length() / scale;
    (distance > ATTACK_DRAG_DEAD_ZONE).then(|| MobileAttackAim {
        direction: delta.normalize_or_zero(),
        extent: ((distance - ATTACK_DRAG_DEAD_ZONE) / (ATTACK_DRAG_REACH - ATTACK_DRAG_DEAD_ZONE))
            .clamp(0.0, 1.0),
    })
}

pub(crate) struct MobileControlsPlugin;
impl Plugin for MobileControlsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MobileControls>();
        if let Some(platform) = app.world().get_resource::<crate::ui::UiPlatform>() {
            let enabled = platform.is_mobile();
            app.world_mut().resource_mut::<MobileControls>().enabled = enabled;
        }
        app.add_systems(Startup, setup_mobile_controls)
            .add_systems(
                Update,
                refresh_mobile_layout
                    .in_set(MobileControlsSet::Layout)
                    .before(InputContextSet::Modal),
            )
            .add_systems(
                Update,
                read_mobile_controls
                    .in_set(MobileControlsSet::Input)
                    .after(InputContextSet::Resolve)
                    .before(crate::combat::CombatPointerInputSet),
            )
            .add_systems(
                Update,
                draw_mobile_controls
                    .in_set(MobileControlsSet::Visuals)
                    .after(MobileControlsSet::Input)
                    .after(crate::combat::WorldMovementInputSet),
            );
    }
}

fn refresh_mobile_layout(
    window: Query<&Window, With<PrimaryWindow>>,
    mut mobile: ResMut<MobileControls>,
    mut focus_events: MessageReader<WindowFocused>,
    mut lifecycle: MessageReader<AppLifecycle>,
) {
    let Ok(window) = window.single() else {
        return;
    };
    let viewport = Vec2::new(window.width(), window.height());
    mobile.layout_changed = viewport != mobile.viewport
        || focus_events.read().any(|event| !event.focused)
        || lifecycle.read().any(|event| {
            matches!(
                event,
                AppLifecycle::WillSuspend | AppLifecycle::Suspended | AppLifecycle::WillResume
            )
        });
    mobile.viewport = viewport;
    mobile.landscape = viewport.x >= viewport.y;
    mobile.focused = window.focused;
    mobile.safe = MobileSafeInsets {
        left: 32.0,
        right: 32.0,
        top: 12.0,
        bottom: 20.0,
    };
}

fn read_mobile_controls(
    time: Res<Time>,
    mut events: MessageReader<TouchInput>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    window: Query<(Entity, &Window), With<PrimaryWindow>>,
    context: Res<GameplayInputContext>,
    local: Query<(&CombatStats, Option<&PlayerProgression>), With<Player>>,
    mut mobile: ResMut<MobileControls>,
    mut session_events: MessageReader<SessionEvent>,
) {
    mobile.begin_input_frame();
    // A new round (`net` skips zero ids and same-round reconnects) releases
    // every held finger, like a layout change. The event is written at the
    // end of `ApplySnapshot`, before this input stage.
    let mut round_changed = false;
    for event in session_events.read() {
        round_changed |= matches!(event, SessionEvent::RoundChanged { .. });
    }
    if round_changed {
        mobile.clear();
        mobile.layout_changed = true;
    }
    let alive = local.single().is_ok_and(|(stats, _)| stats.is_alive());
    if !mobile.enabled
        || !mobile.landscape
        || !mobile.focused
        || !context.gameplay_allowed()
        || !alive
        || mobile.layout_changed
    {
        mobile.clear();
        events.clear();
        return;
    }
    if let Ok((_, prog)) = local.single() {
        let prog = prog.copied().unwrap_or_default();
        mobile.upgrade_enabled = std::array::from_fn(|slot| {
            prog.skill_points > 0
                && prog.ranks[slot] < shared::MAX_ABILITY_RANK
                && prog.unlocked()[slot]
        });
        mobile.upgrade_mode &= mobile.upgrade_enabled.iter().any(|enabled| *enabled);
    }
    let Ok((window_entity, window)) = window.single() else {
        mobile.clear();
        events.clear();
        return;
    };
    mobile.advance_hold_time(time.delta_secs());
    for event in events.read() {
        if event.window == window_entity {
            mobile.event(event.id, event.phase, event.position);
        }
    }
    // Desktop QA only. On real touch devices synthesized mouse input is ignored,
    // otherwise one finger could own two controls or produce duplicate casts.
    if !cfg!(any(target_os = "android", target_os = "ios"))
        && touches.iter().next().is_none()
        && !touches.any_just_released()
        && !touches.any_just_canceled()
    {
        const MOUSE_ID: u64 = u64::MAX;
        if let Some(position) = window.cursor_position() {
            if mouse.just_pressed(MouseButton::Left) {
                mobile.event(MOUSE_ID, TouchPhase::Started, position);
            } else if mouse.pressed(MouseButton::Left) {
                mobile.event(MOUSE_ID, TouchPhase::Moved, position);
            }
            if mouse.just_released(MouseButton::Left) {
                mobile.event(MOUSE_ID, TouchPhase::Ended, position);
            }
        } else {
            mobile.captures.remove(&MOUSE_ID);
            mobile.refresh_movement();
        }
    }
    mobile.acknowledge_hold();
}

#[derive(Component)]
enum MobileVisual {
    Joystick,
    Thumb,
    Attack,
    AttackVector,
    AttackThumb,
    Cancel,
    Ability(usize),
    UpgradeMode,
    CategoryAttack(usize),
    Utility(usize),
    RankRing(usize),
    AimHint,
    SkillDescription,
    Rotate,
}

#[derive(Component)]
struct SkillIcon(usize);

#[derive(Resource)]
struct SkillRingTextures {
    ranks: Vec<Handle<Image>>,
    cooldowns: Vec<Handle<Image>>,
    glyphs: Vec<Handle<Image>>,
}

#[derive(Component)]
enum SkillOverlay {
    Rank(usize),
    Cooldown(usize),
}

// Each actual rank occupies one arc, separated by a visible gap. The transparent
// center preserves the skill artwork; cooldown sectors are a separate layer.
fn skill_overlay_pixels(rank: u8, capacity: u8, cooldown: Option<f32>) -> Vec<u8> {
    const SIZE: usize = 112;
    let mut pixels = vec![0; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let delta = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - Vec2::splat(SIZE as f32 * 0.5);
            let radius = delta.length() / (SIZE as f32 * 0.5);
            let turn = (delta.y.atan2(delta.x) + std::f32::consts::FRAC_PI_2)
                .rem_euclid(std::f32::consts::TAU)
                / std::f32::consts::TAU;
            let rgba = if let Some(fraction) = cooldown {
                if radius <= 0.98 && turn <= fraction {
                    [1, 9, 14, 185]
                } else {
                    [0; 4]
                }
            } else {
                let segment = turn * f32::from(capacity.max(1));
                if (0.87..=0.98).contains(&radius) && (0.045..0.955).contains(&segment.fract()) {
                    if (segment.floor() as u8) < rank {
                        [123, 228, 192, 255]
                    } else {
                        [63, 85, 83, 240]
                    }
                } else {
                    [0; 4]
                }
            };
            pixels[(y * SIZE + x) * 4..(y * SIZE + x) * 4 + 4].copy_from_slice(&rgba);
        }
    }
    pixels
}

fn combat_glyph_pixels(kind: usize) -> Vec<u8> {
    // Small original geometric symbols: minion helmet, tower, dash arrow,
    // haste chevrons, and crossed swords. They do not depend on platform Unicode coverage.
    let mut pixels = vec![0; 112 * 112 * 4];
    for y in 0..112 {
        for x in 0..112 {
            let p = Vec2::new(x as f32 / 112.0, y as f32 / 112.0);
            let line = |a: Vec2, b: Vec2| {
                let t = ((p - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
                p.distance(a + (b - a) * t) < 0.055
            };
            let solid = match kind {
                0 => {
                    let d = p - Vec2::new(0.5, 0.48);
                    (d.length() < 0.34
                        && p.y < 0.72
                        && !(p.y > 0.39 && p.y < 0.51 && p.x > 0.26 && p.x < 0.74))
                        || (p.x > 0.46 && p.x < 0.54 && p.y > 0.14 && p.y < 0.83)
                }
                1 => {
                    (p.x > 0.25 && p.x < 0.75 && p.y > 0.22 && p.y < 0.83)
                        && !(p.y < 0.38
                            && ((p.x > 0.35 && p.x < 0.43) || (p.x > 0.57 && p.x < 0.65)))
                        && !(p.y > 0.59 && p.x > 0.43 && p.x < 0.57)
                }
                2 => {
                    line(Vec2::new(0.12, 0.50), Vec2::new(0.85, 0.50))
                        || line(Vec2::new(0.56, 0.22), Vec2::new(0.85, 0.50))
                        || line(Vec2::new(0.56, 0.78), Vec2::new(0.85, 0.50))
                }
                3 => [0.27, 0.56].into_iter().any(|offset| {
                    line(Vec2::new(offset, 0.22), Vec2::new(offset + 0.23, 0.5))
                        || line(Vec2::new(offset + 0.23, 0.5), Vec2::new(offset, 0.78))
                }),
                _ => {
                    line(Vec2::new(0.25, 0.82), Vec2::new(0.78, 0.18))
                        || line(Vec2::new(0.75, 0.82), Vec2::new(0.22, 0.18))
                        || line(Vec2::new(0.17, 0.64), Vec2::new(0.42, 0.84))
                        || line(Vec2::new(0.58, 0.84), Vec2::new(0.83, 0.64))
                }
            };
            if solid {
                pixels[(y * 112 + x) * 4..(y * 112 + x) * 4 + 4]
                    .copy_from_slice(&[214, 196, 143, 255]);
            }
        }
    }
    pixels
}

fn setup_mobile_controls(
    mut commands: Commands,
    assets: Option<Res<AssetServer>>,
    mut images: Option<ResMut<Assets<Image>>>,
) {
    let textures = images.as_mut().map(|images| {
        let mut add = |pixels| {
            images.add(Image::new(
                bevy::render::render_resource::Extent3d {
                    width: 112,
                    height: 112,
                    depth_or_array_layers: 1,
                },
                bevy::render::render_resource::TextureDimension::D2,
                pixels,
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::default(),
            ))
        };
        SkillRingTextures {
            glyphs: (0..5).map(|kind| add(combat_glyph_pixels(kind))).collect(),
            ranks: (0..=shared::MAX_ABILITY_RANK)
                .map(|rank| add(skill_overlay_pixels(rank, shared::MAX_ABILITY_RANK, None)))
                .collect(),
            cooldowns: (0..=32)
                .map(|step| add(skill_overlay_pixels(0, 0, Some(step as f32 / 32.0))))
                .collect(),
        }
    });
    for visual in [
        MobileVisual::Joystick,
        MobileVisual::Thumb,
        MobileVisual::Attack,
        MobileVisual::AttackVector,
        MobileVisual::AttackThumb,
        MobileVisual::Cancel,
        MobileVisual::Ability(0),
        MobileVisual::Ability(1),
        MobileVisual::Ability(2),
        MobileVisual::Ability(3),
        MobileVisual::UpgradeMode,
        MobileVisual::CategoryAttack(0),
        MobileVisual::CategoryAttack(1),
        MobileVisual::Utility(0),
        MobileVisual::Utility(1),
        MobileVisual::RankRing(0),
        MobileVisual::RankRing(1),
        MobileVisual::RankRing(2),
        MobileVisual::RankRing(3),
        MobileVisual::AimHint,
        MobileVisual::SkillDescription,
        MobileVisual::Rotate,
    ] {
        let rank_slot = if let MobileVisual::RankRing(slot) = visual {
            Some(slot)
        } else {
            None
        };
        let is_rotate = matches!(visual, MobileVisual::Rotate);
        let is_description = matches!(visual, MobileVisual::SkillDescription);
        let icon_slot = if let MobileVisual::Ability(slot) = visual {
            Some(slot)
        } else {
            None
        };
        let attack_icon = matches!(visual, MobileVisual::Attack);
        let glyph = match visual {
            MobileVisual::CategoryAttack(index) => Some(index),
            MobileVisual::Utility(index) => Some(index + 2),
            MobileVisual::Attack => Some(4),
            _ => None,
        };
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    display: Display::None,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Percent(50.0)),
                    padding: UiRect::all(Val::Px(4.0)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(crate::ui::theme::PANEL),
                UiTransform::default(),
                BorderColor::all(crate::ui::theme::EDGE),
                ZIndex(if is_rotate {
                    250
                } else if is_description {
                    180
                } else {
                    30
                }),
                if is_rotate {
                    FocusPolicy::Block
                } else {
                    FocusPolicy::Pass
                },
                Name::new(match &visual {
                    MobileVisual::Joystick => "MobileJoystick".to_owned(),
                    MobileVisual::Thumb => "MobileThumb".to_owned(),
                    MobileVisual::Attack => "MobileAttack".to_owned(),
                    MobileVisual::AttackVector => "MobileAttackVector".to_owned(),
                    MobileVisual::AttackThumb => "MobileAttackThumb".to_owned(),
                    MobileVisual::Cancel => "MobileAttackCancel".to_owned(),
                    MobileVisual::Ability(slot) => format!("MobileAbility-{slot}"),
                    MobileVisual::UpgradeMode => "MobileRankMode".to_owned(),
                    MobileVisual::CategoryAttack(index) => {
                        ["MobileMinionAttack", "MobileTowerAttack"][*index].to_owned()
                    }
                    MobileVisual::Utility(index) => {
                        ["MobileDash", "MobileHaste"][*index].to_owned()
                    }
                    MobileVisual::RankRing(slot) => format!("MobileRankRing-{slot}"),
                    MobileVisual::AimHint => "MobileAimHint".to_owned(),
                    MobileVisual::SkillDescription => "MobileSkillDescription".to_owned(),
                    MobileVisual::Rotate => "MobileRotatePrompt".to_owned(),
                }),
                visual,
            ))
            .with_children(|parent| {
                if let (Some(glyph), Some(textures)) = (glyph, textures.as_ref()) {
                    parent.spawn((
                        ImageNode::new(textures.glyphs[glyph].clone()),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Percent(if attack_icon { 17.0 } else { 22.0 }),
                            top: Val::Percent(if attack_icon { 10.0 } else { 1.0 }),
                            width: Val::Percent(if attack_icon { 66.0 } else { 56.0 }),
                            height: Val::Percent(if attack_icon { 66.0 } else { 56.0 }),
                            ..default()
                        },
                        FocusPolicy::Pass,
                    ));
                }
                if let (Some(slot), Some(textures)) = (rank_slot, textures.as_ref()) {
                    parent.spawn((
                        SkillOverlay::Rank(slot),
                        ImageNode::new(textures.ranks[0].clone()),
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        FocusPolicy::Pass,
                    ));
                }
                if let (Some(slot), Some(assets)) = (icon_slot, assets.as_ref()) {
                    parent.spawn((
                        SkillIcon(slot),
                        ImageNode::new(assets.load(crate::skill_icons::ATLAS_PATH)),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            border_radius: BorderRadius::all(Val::Percent(50.0)),
                            ..default()
                        },
                        FocusPolicy::Pass,
                    ));
                }
                if let (Some(slot), Some(textures)) = (icon_slot, textures.as_ref()) {
                    parent.spawn((
                        SkillOverlay::Cooldown(slot),
                        ImageNode::new(textures.cooldowns[0].clone()),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        FocusPolicy::Pass,
                    ));
                }
                parent.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: 15.0,
                        ..default()
                    },
                    TextColor(crate::ui::theme::IVORY),
                    TextLayout::new_with_justify(Justify::Center),
                    Node {
                        align_self: if icon_slot.is_some() || attack_icon || glyph.is_some() {
                            AlignSelf::End
                        } else {
                            AlignSelf::Center
                        },
                        ..default()
                    },
                    BackgroundColor(if icon_slot.is_some() || attack_icon {
                        Color::srgba(0.01, 0.025, 0.04, 0.84)
                    } else {
                        Color::NONE
                    }),
                    ZIndex(1),
                ));
            });
    }
    if let Some(textures) = textures {
        commands.insert_resource(textures);
    }
}

fn draw_mobile_controls(
    mobile: Res<MobileControls>,
    game: Option<Res<crate::net::GameStateSnapshot>>,
    context: Res<GameplayInputContext>,
    local: Query<
        (
            &CombatStats,
            Option<&PlayerProgression>,
            Option<&NetworkHeroClass>,
            Option<&crate::net::PlayerEquipment>,
        ),
        With<Player>,
    >,
    selection: Res<TeamSelection>,
    cooldown: Res<LocalCastCooldown>,
    basic_attack: Option<Res<crate::targeting::BasicAttackState>>,
    images: Option<Res<Assets<Image>>>,
    mut icons: Query<(&SkillIcon, &mut ImageNode), Without<SkillOverlay>>,
    textures: Option<Res<SkillRingTextures>>,
    mut overlays: Query<(&SkillOverlay, &mut ImageNode), Without<SkillIcon>>,
    utilities: Query<&crate::net::PlayerUtility, With<Player>>,
    mut visuals: Query<(
        &MobileVisual,
        &mut Node,
        &mut BackgroundColor,
        &mut BorderColor,
        &mut UiTransform,
        &Children,
    )>,
    mut texts: Query<(&mut Text, &mut TextFont)>,
) {
    let layout = mobile.layout();
    let s = mobile.combat_scale();
    let local = local.single().ok();
    let prog = local
        .and_then(|(_, p, _, _)| p)
        .copied()
        .unwrap_or_default();
    let class = local
        .and_then(|(_, _, c, _)| c)
        .map(|c| c.0)
        .unwrap_or(selection.hero_class);
    let bonuses = local
        .and_then(|(_, _, _, equipment)| equipment)
        .map_or_else(Default::default, |equipment| equipment.item_bonuses);
    let sandbox = game.as_ref().and_then(|g| g.sandbox.as_ref());
    let visible = mobile.enabled
        && mobile.landscape
        && context.gameplay_allowed()
        && local.is_some_and(|(stats, _, _, _)| stats.is_alive());
    let aiming = mobile
        .captures
        .values()
        .find(|c| matches!(c.control, Control::Ability(_) | Control::Utility(_)))
        .or_else(|| {
            mobile
                .captures
                .values()
                .find(|c| c.control == Control::Attack)
        });
    let attack = mobile
        .captures
        .values()
        .find(|c| c.control == Control::Attack);
    let attack_remaining = basic_attack.map_or(0.0, |state| state.remaining_secs);
    let attack_cooling = attack_remaining > 0.0;
    let attack_held = mobile.held_basic_attack();
    let drag = attack
        .filter(|capture| capture.dragged)
        .map(|capture| (capture.position - capture.origin).clamp_length_max(ATTACK_DRAG_REACH * s));
    let canceled_color = Color::srgb(0.90, 0.28, 0.24);
    for (SkillIcon(slot), mut icon) in &mut icons {
        let def = ability_for_class_slot(class, SkillSlot::from_index(*slot as u8).unwrap());
        if let Some(image) = images.as_ref().and_then(|images| images.get(&icon.image)) {
            icon.rect = crate::skill_icons::icon_rect(def.id, image.size().as_vec2());
        }
        let ready = prog.unlocked()[*slot]
            && cooldown.remaining_secs[*slot] <= 0.0
            && local.is_some_and(|(stats, _, _, _)| {
                stats.mana >= scaled_mana_cost(def, prog.ranks[*slot].max(1))
            });
        icon.color = if ready {
            Color::WHITE
        } else {
            Color::srgb(0.38, 0.42, 0.48)
        };
    }
    if let Some(textures) = textures {
        for (overlay, mut image) in &mut overlays {
            image.image = match *overlay {
                SkillOverlay::Rank(slot) => textures.ranks
                    [usize::from(prog.ranks[slot].min(shared::MAX_ABILITY_RANK))]
                .clone(),
                SkillOverlay::Cooldown(slot) => {
                    let fraction = cooldown.remaining_fraction(slot);
                    textures.cooldowns[(fraction.clamp(0.0, 1.0) * 32.0).ceil() as usize].clone()
                }
            };
        }
    }
    let utility = utilities
        .single()
        .map(|utility| utility.state)
        .unwrap_or_default();
    for (visual, mut node, mut color, mut border, mut ui_transform, children) in &mut visuals {
        let (center, radius, label, show, fill, edge) = match *visual {
            MobileVisual::Joystick => (
                layout.joystick_center,
                layout.joystick_radius,
                String::new(),
                visible,
                Color::srgba(0.02, 0.08, 0.08, 0.45),
                crate::ui::theme::MUTED,
            ),
            MobileVisual::Thumb => (
                layout.joystick_center + mobile.movement * layout.joystick_radius * 0.7,
                22.0 * s,
                String::new(),
                visible,
                Color::srgba(0.43, 0.7, 0.62, 0.72),
                crate::ui::theme::JADE,
            ),
            MobileVisual::Attack => (
                layout.attack_center,
                layout.attack_radius,
                if attack_cooling {
                    format!("{attack_remaining:.1}")
                } else {
                    "ATK".into()
                },
                visible,
                if attack_cooling {
                    if attack_held {
                        crate::ui::theme::TILE
                    } else {
                        crate::ui::theme::PANEL
                    }
                } else if attack.is_some() {
                    crate::ui::theme::HOVER
                } else {
                    crate::ui::theme::TILE
                },
                if mobile.attack_cancelled() {
                    canceled_color
                } else if attack_cooling {
                    crate::ui::theme::EDGE
                } else if attack_held {
                    crate::ui::theme::JADE
                } else {
                    crate::ui::theme::GOLD
                },
            ),
            MobileVisual::AttackVector | MobileVisual::AttackThumb => (
                layout.attack_center
                    + drag.unwrap_or_default()
                        * if matches!(visual, MobileVisual::AttackVector) {
                            0.5
                        } else {
                            1.0
                        },
                16.0 * s,
                String::new(),
                visible && drag.is_some() && !mobile.skill_aiming(),
                if mobile.attack_cancelled() {
                    canceled_color
                } else {
                    crate::ui::theme::JADE
                },
                crate::ui::theme::IVORY,
            ),
            MobileVisual::Cancel => (
                layout.cancel_center,
                layout.cancel_radius,
                "×".into(),
                visible && aiming.is_some() && mobile.inspected_skill().is_none(),
                if aiming.is_some_and(|capture| capture.canceled) {
                    canceled_color
                } else {
                    crate::ui::theme::PANEL
                },
                canceled_color,
            ),
            MobileVisual::Ability(slot) => {
                let def = ability_for_class_slot(class, SkillSlot::from_index(slot as u8).unwrap());
                let unlocked = prog.unlocked()[slot];
                let mana = local.is_some_and(|(stats, _, _, _)| {
                    stats.mana >= scaled_mana_cost(def, prog.ranks[slot].max(1))
                });
                let active = mobile
                    .captures
                    .values()
                    .any(|c| c.control == Control::Ability(slot) && !c.canceled);
                let status = if mobile.upgrade_mode && mobile.upgrade_enabled[slot] {
                    format!("{} +", ["Q", "W", "E", "R"][slot])
                } else if !unlocked {
                    format!("Lv {}", shared::SLOT_UNLOCK_LEVELS[slot])
                } else if cooldown.remaining_secs[slot] > 0.0 {
                    format!("{:.1}", cooldown.remaining_secs[slot])
                } else if !mana {
                    "MANA".into()
                } else {
                    ["Q", "W", "E", "R"][slot].into()
                };
                (
                    layout.ability_centers[slot],
                    layout.ability_radii[slot],
                    status,
                    visible,
                    if active {
                        crate::ui::theme::HOVER
                    } else {
                        crate::ui::theme::PANEL
                    },
                    if mobile.upgrade_enabled[slot] {
                        crate::ui::theme::GOLD
                    } else if !unlocked || !mana || cooldown.remaining_secs[slot] > 0.0 {
                        crate::ui::theme::EDGE
                    } else {
                        crate::ui::theme::JADE
                    },
                )
            }
            MobileVisual::RankRing(slot) => (
                layout.ability_centers[slot],
                layout.ability_radii[slot] + 3.0 * s,
                String::new(),
                visible,
                Color::NONE,
                Color::NONE,
            ),
            MobileVisual::UpgradeMode => (
                layout.upgrade_center,
                layout.upgrade_radius,
                if mobile.upgrade_mode {
                    "BACK".into()
                } else {
                    format!("+{}\nRANK", prog.skill_points)
                },
                visible && mobile.upgrade_enabled.iter().any(|enabled| *enabled),
                if mobile.upgrade_mode {
                    crate::ui::theme::HOVER
                } else {
                    crate::ui::theme::PANEL
                },
                crate::ui::theme::GOLD,
            ),
            MobileVisual::CategoryAttack(index) => (
                layout.category_centers[index],
                layout.auxiliary_radius,
                ["MIN", "TWR"][index].into(),
                visible,
                crate::ui::theme::PANEL,
                crate::ui::theme::GOLD,
            ),
            MobileVisual::Utility(index) => {
                let remaining = [utility.dash_remaining_secs, utility.haste_remaining_secs][index];
                let active = index == 1 && utility.haste_active_secs > 0.0;
                let label = if active {
                    format!("{:.1}\nFAST", utility.haste_active_secs)
                } else if remaining > 0.0 {
                    format!("{remaining:.0}")
                } else {
                    ["DASH", "HASTE"][index].into()
                };
                (
                    layout.utility_centers[index],
                    layout.auxiliary_radius,
                    label,
                    visible,
                    if active {
                        crate::ui::theme::HOVER
                    } else {
                        crate::ui::theme::PANEL
                    },
                    if active {
                        crate::ui::theme::JADE
                    } else if remaining > 0.0 {
                        crate::ui::theme::EDGE
                    } else {
                        crate::ui::theme::GOLD
                    },
                )
            }
            MobileVisual::AimHint => {
                let message = aiming
                    .map(|c| {
                        if c.canceled || (c.control == Control::Attack && mobile.attack_cancelled())
                        {
                            "CANCEL\nRelease to discard"
                        } else if c.control == Control::Attack {
                            "Point toward an enemy · release to lock\nMove to × to cancel"
                        } else {
                            "Drag to aim · release to use\nMove to × to cancel"
                        }
                    })
                    .unwrap_or("");
                (
                    Vec2::new(
                        (layout.joystick_center.x
                            + layout.joystick_radius
                            + layout.utility_centers[0].x
                            - layout.upgrade_radius)
                            * 0.5,
                        mobile.viewport.y - mobile.safe.bottom - 36.0 * s,
                    ),
                    1.0,
                    message.into(),
                    visible && aiming.is_some() && mobile.inspected_skill().is_none(),
                    crate::ui::theme::PANEL,
                    crate::ui::theme::EDGE,
                )
            }
            MobileVisual::SkillDescription => {
                let slot = mobile.inspected_skill();
                let label = slot.map(|slot| {
                    let def = ability_for_class_slot(class, SkillSlot::from_index(slot as u8).unwrap());
                    let rank = prog.ranks[slot].max(1);
                    let duration = if sandbox.is_some_and(|s| s.config.player.no_cooldowns) {
                        0.0
                    } else {
                        crate::combat::effective_cast_duration(class, prog.level, rank,
                            SkillSlot::ALL[slot], bonuses, sandbox.is_some())
                    };
                    let availability = if prog.unlocked()[slot] {
                        format!("Rank {rank}")
                    } else { format!("Unlocks at level {}", shared::SLOT_UNLOCK_LEVELS[slot]) };
                    format!("{}  ·  {}\n{}\n\n{:.0} mana  ·  {:.1}s cooldown\nRelease to close · tap or drag to cast", def.name, availability, def.description,
                        scaled_mana_cost(def, rank), duration)
                }).unwrap_or_default();
                (
                    Vec2::new(mobile.viewport.x * 0.5, mobile.safe.top + 100.0 * s),
                    1.0,
                    label,
                    visible && slot.is_some(),
                    Color::srgb(0.025, 0.055, 0.065),
                    crate::ui::theme::GOLD,
                )
            }
            MobileVisual::Rotate => (
                mobile.viewport * 0.5,
                1.0,
                "Rotate your phone\nPlay Omoba in landscape".into(),
                mobile.enabled && !mobile.landscape,
                crate::ui::theme::PANEL,
                crate::ui::theme::GOLD,
            ),
        };
        node.display = if show { Display::Flex } else { Display::None };
        let is_ring = matches!(visual, MobileVisual::RankRing(_));
        let rectangular = matches!(
            visual,
            MobileVisual::AimHint | MobileVisual::Rotate | MobileVisual::SkillDescription
        );
        let is_vector = matches!(visual, MobileVisual::AttackVector);
        let size = if is_vector {
            Vec2::new(drag.unwrap_or_default().length(), 3.0 * s)
        } else if matches!(visual, MobileVisual::Rotate) {
            mobile.viewport
        } else if matches!(visual, MobileVisual::SkillDescription) {
            Vec2::new(
                (330.0 * s).min(mobile.viewport.x - mobile.safe.left - mobile.safe.right),
                172.0 * s,
            )
        } else if rectangular {
            let available = layout.utility_centers[0].x
                - layout.upgrade_radius
                - layout.joystick_center.x
                - layout.joystick_radius
                - 24.0 * s;
            Vec2::new(available.clamp(120.0 * s, 260.0 * s), 60.0 * s)
        } else {
            Vec2::splat(radius * 2.0)
        };
        node.left = Val::Px(center.x - size.x * 0.5);
        node.top = Val::Px(center.y - size.y * 0.5);
        node.width = Val::Px(size.x);
        node.height = Val::Px(size.y);
        node.padding = UiRect::all(Val::Px(if is_vector || is_ring { 0.0 } else { 3.0 }));
        node.border = UiRect::all(Val::Px(if is_vector || is_ring { 0.0 } else { 1.0 }));
        ui_transform.rotation = if is_vector {
            let delta = drag.unwrap_or_default();
            Rot2::radians(delta.y.atan2(delta.x))
        } else {
            Rot2::IDENTITY
        };
        node.border_radius = BorderRadius::all(if rectangular {
            Val::Px(8.0)
        } else {
            Val::Percent(50.0)
        });
        *color = fill.into();
        *border = BorderColor::all(edge);
        for child in children.iter() {
            if let Ok((mut text, mut font)) = texts.get_mut(child) {
                text.0.clone_from(&label);
                font.font_size = if matches!(visual, MobileVisual::Rotate) {
                    24.0 * s
                } else if matches!(visual, MobileVisual::Cancel) {
                    28.0 * s
                } else if matches!(
                    visual,
                    MobileVisual::AimHint
                        | MobileVisual::SkillDescription
                        | MobileVisual::Ability(_)
                        | MobileVisual::Utility(_)
                        | MobileVisual::CategoryAttack(_)
                        | MobileVisual::UpgradeMode
                ) {
                    12.0 * s
                } else {
                    14.0 * s
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skill_description_shows_level_item_and_sandbox_cooldowns() {
        for (slot, slow_sandbox, no_cooldowns, expected) in [
            (0, false, false, "1.1s cooldown"),
            (1, false, false, "4.5s cooldown"),
            (0, true, false, "5.0s cooldown"),
            (0, true, true, "0.0s cooldown"),
        ] {
            let mut mobile = controls();
            let point = mobile.layout().ability_centers[slot];
            mobile.event(1, TouchPhase::Started, point);
            mobile.advance_hold_time(SKILL_DESCRIPTION_SECONDS);
            assert_eq!(mobile.inspected_skill(), Some(slot));
            let mut app = App::new();
            app.insert_resource(mobile)
                .init_resource::<GameplayInputContext>()
                .init_resource::<TeamSelection>()
                .init_resource::<LocalCastCooldown>()
                .add_systems(Update, draw_mobile_controls);
            let mut bonuses = shared::shop::item_bonuses(&[
                shared::shop::ItemId::SwiftGrip,
                shared::shop::ItemId::FocusCharm,
            ]);
            if slow_sandbox {
                bonuses.attack_speed_multiplier = 0.25;
                let mut config = shared::sandbox::SandboxConfig::default();
                config.player.no_cooldowns = no_cooldowns;
                app.insert_resource(crate::net::GameStateSnapshot {
                    sandbox: Some(shared::sandbox::SandboxSnapshot {
                        config,
                        ack: None,
                        last_request_id: 0,
                        actors: vec![],
                        analytics: default(),
                        simulation_secs: 0.0,
                        frame: 0,
                    }),
                    ..default()
                });
            }
            app.world_mut().spawn((
                Player,
                CombatStats::default(),
                PlayerProgression {
                    level: 10,
                    ..default()
                },
                NetworkHeroClass(shared::HeroClass::Warrior),
                crate::net::PlayerEquipment {
                    item_bonuses: bonuses,
                    ..default()
                },
            ));
            let label = app
                .world_mut()
                .spawn((Text::default(), TextFont::default()))
                .id();
            app.world_mut()
                .spawn((
                    MobileVisual::SkillDescription,
                    Node::default(),
                    BackgroundColor::default(),
                    BorderColor::default(),
                    UiTransform::default(),
                ))
                .add_child(label);
            app.update();
            let text = &app.world().get::<Text>(label).unwrap().0;
            assert!(text.contains(expected), "expected {expected}, got {text}");
        }
    }

    fn controls() -> MobileControls {
        MobileControls {
            enabled: true,
            ..default()
        }
    }
    #[test]
    fn stationary_skill_hold_describes_without_casting_and_movement_keeps_working() {
        let mut m = controls();
        let layout = m.layout();
        let skill = layout.ability_centers[1] + Vec2::X * 16.0;
        m.event(1, TouchPhase::Started, skill);
        m.event(2, TouchPhase::Started, layout.joystick_center);
        m.event(
            2,
            TouchPhase::Moved,
            layout.joystick_center + Vec2::X * 40.0,
        );
        m.event(1, TouchPhase::Moved, skill + Vec2::X * 15.0);
        m.advance_hold_time(SKILL_DESCRIPTION_SECONDS);
        assert_eq!(m.inspected_skill(), Some(1));
        assert!(m.movement.x > 0.0);
        // After opening help, later finger movement must never cast accidentally.
        m.event(1, TouchPhase::Moved, skill + Vec2::NEG_X * 70.0);
        m.event(1, TouchPhase::Ended, skill + Vec2::NEG_X * 70.0);
        assert!(m.casts.is_empty());
        assert_eq!(m.inspected_skill(), None);
        assert!(m.movement.x > 0.0);
    }

    #[test]
    fn deliberate_skill_drag_still_aims_after_a_long_hold() {
        let mut m = controls();
        let skill = m.layout().ability_centers[0];
        m.event(1, TouchPhase::Started, skill);
        m.event(1, TouchPhase::Moved, skill + Vec2::NEG_X * 50.0);
        m.advance_hold_time(1.0);
        assert_eq!(m.inspected_skill(), None);
        m.event(1, TouchPhase::Ended, skill + Vec2::NEG_X * 50.0);
        assert_eq!(
            m.casts,
            [MobileCastIntent {
                slot: 0,
                aim: Some(Vec2::NEG_X)
            }]
        );
    }
    #[test]
    fn basic_taps_and_holds_are_independent_from_all_four_skills() {
        let mut m = controls();
        let layout = m.layout();
        let press = layout.attack_center + Vec2::X * 30.0;
        m.event(1, TouchPhase::Started, press);
        assert!(m.attacks.is_empty() && m.casts.is_empty());
        m.advance_hold_time(0.179);
        assert!(!m.held_basic_attack());
        m.event(1, TouchPhase::Moved, press);
        m.advance_hold_time(0.002);
        assert!(m.held_basic_attack());
        assert!(m.attack_aim().is_none());
        m.acknowledge_hold();
        m.event(1, TouchPhase::Ended, press);
        assert!(
            m.attacks.is_empty(),
            "releasing a repeating hold must not attack again"
        );
        m.event(1, TouchPhase::Started, press);
        m.event(1, TouchPhase::Ended, press);
        assert_eq!(
            m.attacks,
            [MobileAttackIntent {
                aim: None,
                gesture: 2
            }]
        );
        m.attacks.clear();
        // Even a long frame ending the gesture before combat saw a hold is one tap.
        m.event(1, TouchPhase::Started, press);
        m.advance_hold_time(0.3);
        m.event(1, TouchPhase::Ended, press);
        assert_eq!(
            m.attacks,
            [MobileAttackIntent {
                aim: None,
                gesture: 3
            }]
        );
        m.attacks.clear();
        for slot in 0..4 {
            m.event(2, TouchPhase::Started, layout.ability_centers[slot]);
            assert_eq!(m.casts.len(), slot);
            m.event(2, TouchPhase::Ended, layout.ability_centers[slot]);
            assert_eq!(m.casts[slot], MobileCastIntent { slot, aim: None });
            assert!(m.attacks.is_empty());
        }
    }

    #[test]
    fn attack_drag_grows_caps_and_releases_exact_aim_without_repeat_or_fallback() {
        let mut m = controls();
        let center = m.layout().attack_center;
        m.event(1, TouchPhase::Started, center);
        m.event(1, TouchPhase::Moved, center + Vec2::NEG_X * 54.0);
        let short = m.attack_aim().unwrap();
        assert_eq!(short.direction, Vec2::NEG_X);
        assert!((short.extent - 0.5).abs() < 0.001);
        m.advance_hold_time(1.0);
        assert!(!m.held_basic_attack());
        assert!(m.attacks.is_empty());
        let far = center + Vec2::NEG_X * 300.0;
        m.event(1, TouchPhase::Moved, far);
        let full = m.attack_aim().unwrap();
        assert_eq!(full.extent, 1.0);
        assert!(
            !m.attack_cancelled(),
            "long drag caps reach rather than canceling"
        );
        m.event(1, TouchPhase::Ended, far);
        assert_eq!(
            m.attacks,
            [MobileAttackIntent {
                aim: Some(full),
                gesture: 1
            }]
        );
        m.attacks.clear();
        m.event(1, TouchPhase::Started, center);
        m.event(1, TouchPhase::Moved, far);
        m.event(1, TouchPhase::Moved, center);
        assert!(m.attack_cancelled());
        assert!(!m.held_basic_attack());
        m.event(1, TouchPhase::Ended, center);
        assert!(
            m.attacks.is_empty(),
            "returning a drag to dead zone cannot auto-select"
        );
    }

    #[test]
    fn attack_cancel_region_and_os_cancel_discard_only_the_owned_gesture() {
        let mut m = controls();
        let layout = m.layout();
        m.event(1, TouchPhase::Started, layout.joystick_center);
        m.event(
            1,
            TouchPhase::Moved,
            layout.joystick_center + Vec2::X * 58.0,
        );
        for phase in [TouchPhase::Ended, TouchPhase::Canceled] {
            m.event(2, TouchPhase::Started, layout.attack_center);
            m.event(2, TouchPhase::Moved, layout.cancel_center);
            assert!(m.attack_cancelled());
            assert!(m.attack_aim().is_none());
            m.event(2, phase, layout.cancel_center);
            assert!(m.attacks.is_empty() && !m.held_basic_attack());
            assert_eq!(m.movement, Vec2::X);
        }
        m.event(2, TouchPhase::Started, layout.attack_center);
        m.event(2, TouchPhase::Canceled, layout.attack_center);
        assert!(m.attacks.is_empty());
    }

    #[test]
    fn skill_owner_suspends_basic_hold_without_stealing_movement() {
        let mut m = controls();
        let layout = m.layout();
        m.event(1, TouchPhase::Started, layout.joystick_center);
        m.event(
            1,
            TouchPhase::Moved,
            layout.joystick_center + Vec2::X * 58.0,
        );
        m.event(2, TouchPhase::Started, layout.attack_center);
        m.advance_hold_time(0.2);
        assert!(m.held_basic_attack());
        m.event(3, TouchPhase::Started, layout.ability_centers[0]);
        m.event(4, TouchPhase::Started, layout.ability_centers[1]);
        assert!(!m.held_basic_attack());
        assert!(!m.captures.contains_key(&4));
        // Repeated Started with an owned id cannot turn the joystick into attack.
        m.event(1, TouchPhase::Started, layout.attack_center);
        assert_eq!(m.movement, Vec2::X);
        m.event(3, TouchPhase::Ended, layout.ability_centers[0]);
        m.event(4, TouchPhase::Ended, layout.ability_centers[1]);
        assert_eq!(m.casts, [MobileCastIntent { slot: 0, aim: None }]);
        assert!(!m.held_basic_attack(), "skill release wins this frame");
        m.casts.clear();
        assert!(
            !m.held_basic_attack(),
            "draining skill requests must not erase their frame priority"
        );
        m.begin_input_frame();
        assert!(m.held_basic_attack());
    }

    #[test]
    fn release_cancellation_survives_capture_removal_and_touch_ids_are_not_gesture_ids() {
        let mut m = controls();
        let layout = m.layout();
        let mut previous = 0;
        for (phase, position) in [
            (TouchPhase::Ended, layout.cancel_center),
            (TouchPhase::Canceled, layout.attack_center),
            (TouchPhase::Ended, Vec2::splat(f32::NAN)),
        ] {
            m.begin_input_frame();
            m.event(17, TouchPhase::Started, layout.attack_center);
            let gesture = m.attack_gesture().unwrap();
            assert!(gesture > previous);
            previous = gesture;
            assert!(m.attack_pressed());
            m.event(17, phase, position);
            assert!(!m.attack_pressed());
            assert!(
                m.attack_cancelled(),
                "root must observe even a same-frame cancellation"
            );
            assert!(m.attacks.is_empty());
            m.begin_input_frame();
            assert!(!m.attack_cancelled());
        }
        m.event(17, TouchPhase::Started, layout.attack_center);
        let final_gesture = m.attack_gesture().unwrap();
        assert!(final_gesture > previous);
        m.event(
            17,
            TouchPhase::Moved,
            layout.attack_center + Vec2::NEG_X * 54.0,
        );
        m.clear();
        assert!(
            m.attack_cancelled(),
            "layout reset must cancel queued attacks too"
        );
        m.event(17, TouchPhase::Ended, layout.attack_center);
        assert!(m.attacks.is_empty());
    }

    #[test]
    fn independent_fingers_move_and_cast_without_stealing_joystick() {
        let mut m = controls();
        let l = m.layout();
        m.event(10, TouchPhase::Started, l.joystick_center);
        m.event(
            10,
            TouchPhase::Moved,
            l.joystick_center + Vec2::X * l.joystick_radius,
        );
        m.event(20, TouchPhase::Started, l.ability_centers[2]);
        m.event(
            20,
            TouchPhase::Moved,
            l.ability_centers[2] + Vec2::NEG_Y * 60.0,
        );
        m.event(
            20,
            TouchPhase::Ended,
            l.ability_centers[2] + Vec2::NEG_Y * 60.0,
        );
        assert_eq!(m.movement, Vec2::X);
        assert_eq!(
            m.casts,
            [MobileCastIntent {
                slot: 2,
                aim: Some(Vec2::NEG_Y)
            }]
        );
        m.event(10, TouchPhase::Ended, l.joystick_center);
        assert_eq!(m.movement, Vec2::ZERO);
    }
    #[test]
    fn resize_dpi_and_rotation_preserve_the_selected_interface_family() {
        for mobile in [false, true] {
            let mut app = App::new();
            app.insert_resource(MobileControls {
                enabled: mobile,
                ..default()
            })
            .add_message::<WindowFocused>()
            .add_message::<AppLifecycle>()
            .add_systems(Update, refresh_mobile_layout);
            let entity = app
                .world_mut()
                .spawn((Window::default(), PrimaryWindow))
                .id();
            for (width, height, dpi) in [
                (1920, 1080, 1.0),
                (667, 375, 1.0),
                (390, 844, 1.0),
                (3840, 2160, 3.0),
                (1280, 720, 2.0),
            ] {
                {
                    let mut window = app.world_mut().get_mut::<Window>(entity).unwrap();
                    window.resolution.set_scale_factor_override(Some(dpi));
                    window.resolution.set_physical_resolution(width, height);
                }
                app.update();
                let controls = app.world().resource::<MobileControls>();
                assert_eq!(controls.enabled, mobile);
                assert_eq!(controls.landscape, width >= height);
                assert_eq!(
                    controls.viewport,
                    Vec2::new(width as f32 / dpi, height as f32 / dpi)
                );
            }
        }
    }
    #[test]
    fn dead_zone_clamping_and_capture_survive_crossing_controls() {
        let mut m = controls();
        let l = m.layout();
        m.event(1, TouchPhase::Started, l.joystick_center + Vec2::X * 3.0);
        assert_eq!(m.movement, Vec2::ZERO);
        m.event(1, TouchPhase::Moved, l.ability_centers[0]);
        assert!((m.movement.length() - 1.0).abs() < 0.001);
        assert!(m.casts.is_empty());
        m.event(2, TouchPhase::Started, l.joystick_center);
        m.event(1, TouchPhase::Canceled, l.ability_centers[0]);
        assert_eq!(m.movement, Vec2::ZERO);
        m.event(2, TouchPhase::Moved, l.joystick_center + Vec2::X * 50.0);
        assert_eq!(m.movement, Vec2::ZERO);
    }
    #[test]
    fn cancel_or_gate_reset_never_casts_and_requires_a_new_touch() {
        let mut m = controls();
        let l = m.layout();
        for phase in [TouchPhase::Canceled, TouchPhase::Ended] {
            m.event(1, TouchPhase::Started, l.ability_centers[1]);
            m.event(1, TouchPhase::Moved, l.cancel_center);
            m.event(1, phase, l.cancel_center);
        }
        m.event(1, TouchPhase::Started, l.ability_centers[2]);
        m.event(2, TouchPhase::Started, l.joystick_center);
        m.clear();
        m.event(1, TouchPhase::Ended, l.ability_centers[2]);
        m.event(2, TouchPhase::Moved, l.joystick_center + Vec2::X * 40.0);
        assert!(m.casts.is_empty());
        assert_eq!(m.movement, Vec2::ZERO);
    }
    #[test]
    fn ecs_gates_flush_held_fingers_on_modal_death_focus_rotation_and_round_change() {
        for gate in 0..5 {
            let mut app = App::new();
            app.init_resource::<Time>()
                .init_resource::<Touches>()
                .init_resource::<ButtonInput<MouseButton>>()
                .init_resource::<GameplayInputContext>()
                .insert_resource(controls())
                .add_message::<TouchInput>()
                .add_message::<SessionEvent>()
                .add_systems(Update, read_mobile_controls);
            let window = app
                .world_mut()
                .spawn((Window::default(), PrimaryWindow))
                .id();
            let player = app.world_mut().spawn((Player, CombatStats::default())).id();
            let l = app.world().resource::<MobileControls>().layout();
            for (id, phase, position) in [
                (1, TouchPhase::Started, l.joystick_center),
                (1, TouchPhase::Moved, l.joystick_center + Vec2::X * 55.0),
                (2, TouchPhase::Started, l.ability_centers[2]),
                (3, TouchPhase::Started, l.attack_center),
                (3, TouchPhase::Moved, l.attack_center + Vec2::NEG_X * 54.0),
            ] {
                app.world_mut().write_message(TouchInput {
                    id,
                    phase,
                    position,
                    window,
                    force: None,
                });
            }
            app.update();
            assert!(app.world().resource::<MobileControls>().movement.x > 0.8);
            match gate {
                0 => {
                    app.world_mut()
                        .resource_mut::<GameplayInputContext>()
                        .modal_open = true
                }
                1 => {
                    app.world_mut()
                        .entity_mut(player)
                        .get_mut::<CombatStats>()
                        .unwrap()
                        .hp = 0.0
                }
                2 => app.world_mut().resource_mut::<MobileControls>().focused = false,
                3 => app.world_mut().resource_mut::<MobileControls>().landscape = false,
                _ => {
                    use crate::domain::RoundId;
                    app.world_mut().write_message(SessionEvent::RoundChanged {
                        previous: RoundId {
                            server_epoch: 1,
                            match_id: 1,
                        },
                        current: RoundId {
                            server_epoch: 1,
                            match_id: 2,
                        },
                    });
                }
            }
            app.update();
            assert_eq!(
                app.world().resource::<MobileControls>().movement,
                Vec2::ZERO
            );
            assert!(app.world().resource::<MobileControls>().captures.is_empty());
            assert!(app.world().resource::<MobileControls>().attacks.is_empty());
            assert!(!app.world().resource::<MobileControls>().held_basic_attack());
            assert!(
                app.world()
                    .resource::<MobileControls>()
                    .attack_aim()
                    .is_none()
            );
            app.world_mut()
                .resource_mut::<GameplayInputContext>()
                .modal_open = false;
            app.world_mut()
                .entity_mut(player)
                .get_mut::<CombatStats>()
                .unwrap()
                .hp = 100.0;
            {
                let mut m = app.world_mut().resource_mut::<MobileControls>();
                m.focused = true;
                m.landscape = true;
                m.layout_changed = false;
            }
            app.world_mut().write_message(TouchInput {
                id: 2,
                phase: TouchPhase::Ended,
                position: l.ability_centers[2],
                window,
                force: None,
            });
            app.world_mut().write_message(TouchInput {
                id: 1,
                phase: TouchPhase::Moved,
                position: l.joystick_center + Vec2::X * 55.0,
                window,
                force: None,
            });
            app.update();
            let m = app.world().resource::<MobileControls>();
            assert!(m.casts.is_empty());
            assert_eq!(m.movement, Vec2::ZERO);
        }
    }

    #[test]
    fn phone_controls_and_rank_mode_fit_safe_area_without_overlap() {
        for viewport in [
            Vec2::new(693.0, 320.0),
            Vec2::new(844.0, 390.0),
            Vec2::new(932.0, 430.0),
            Vec2::new(568.0, 320.0),
            Vec2::new(667.0, 375.0),
            Vec2::new(1280.0, 720.0),
        ] {
            let m = MobileControls {
                viewport,
                ..controls()
            };
            let l = m.layout();
            let circles: Vec<_> = l
                .ability_centers
                .iter()
                .copied()
                // Rank artwork extends 3px beyond each skill's hit circle.
                .zip(
                    l.ability_radii
                        .map(|radius| radius + 3.0 * m.combat_scale()),
                )
                .chain(
                    l.category_centers
                        .iter()
                        .chain(l.utility_centers.iter())
                        .copied()
                        .map(|p| (p, l.auxiliary_radius)),
                )
                .chain([
                    // Touch ownership extends beyond the visible joystick disc.
                    (l.joystick_center, l.joystick_radius * 1.3),
                    (l.attack_center, l.attack_radius),
                    (l.upgrade_center, l.upgrade_radius),
                    (l.cancel_center, l.cancel_radius),
                ])
                .collect();
            for (i, (center, radius)) in circles.iter().enumerate() {
                assert!(*radius * 2.0 >= 44.0);
                assert!(
                    center.x - radius >= m.safe.left
                        && center.x + radius <= viewport.x - m.safe.right + 0.01
                );
                assert!(
                    center.y - radius >= m.safe.top
                        && center.y + radius <= viewport.y - m.safe.bottom + 0.01
                );
                for (other, other_radius) in &circles[i + 1..] {
                    assert!(
                        center.distance(*other) + 0.01 >= radius + other_radius + 3.5,
                        "overlap at {viewport:?}: {center:?}, {other:?}"
                    );
                }
            }
        }
        let l = controls().layout();
        assert_eq!(l.joystick_center, Vec2::new(104.0, 300.0));
        assert_eq!(l.attack_center, Vec2::new(698.0, 256.0));
        assert_eq!(
            l.utility_centers,
            [Vec2::new(530.0, 348.0), Vec2::new(580.0, 348.0)]
        );
    }

    #[test]
    fn all_six_satellites_share_the_attack_center_and_one_uniform_orbit() {
        for viewport in [
            Vec2::new(693.0, 320.0),
            Vec2::new(844.0, 390.0),
            Vec2::new(932.0, 430.0),
            Vec2::new(1280.0, 720.0),
        ] {
            let m = MobileControls {
                viewport,
                ..controls()
            };
            let l = m.layout();
            let s = m.combat_scale();
            let centers = l.ability_centers.into_iter().chain(l.category_centers);
            for (center, angle) in centers.zip([150.0_f32, 200.0, 250.0, 310.0, 90.0, 0.0]) {
                let delta = center - l.attack_center;
                assert!((delta.length() - 92.0 * s).abs() < 0.001);
                let expected_direction =
                    Vec2::new(angle.to_radians().cos(), angle.to_radians().sin());
                assert!(delta.normalize().distance(expected_direction) < 0.0001);
            }
            assert!((l.attack_center.x + 114.0 * s - (viewport.x - m.safe.right)).abs() < 0.001);
            assert!((l.attack_center.y + 114.0 * s - (viewport.y - m.safe.bottom)).abs() < 0.001);
            for center in l
                .utility_centers
                .into_iter()
                .chain([l.upgrade_center, l.cancel_center])
            {
                assert!(center.distance(l.attack_center) > 92.0 * s + l.auxiliary_radius);
            }
            // Geometry used by drawing and input must remain the same at every size.
            assert!(matches!(
                m.hit_control(l.attack_center),
                Some((Control::Attack, _))
            ));
            for (slot, center) in l.ability_centers.into_iter().enumerate() {
                assert!(
                    matches!(m.hit_control(center), Some((Control::Ability(actual), _)) if actual == slot)
                );
            }
            assert!(matches!(
                m.hit_control(l.category_centers[0]),
                Some((Control::CategoryAttack(TargetKind::Minion), _))
            ));
            assert!(matches!(
                m.hit_control(l.category_centers[1]),
                Some((Control::CategoryAttack(TargetKind::Structure), _))
            ));
        }
    }

    #[test]
    fn minimum_combat_target_size_does_not_change_frontend_scaling() {
        let m = MobileControls {
            viewport: Vec2::new(693.0, 320.0),
            ..controls()
        };
        assert_eq!(m.scale(), 0.85);
        assert_eq!(m.combat_scale(), 1.0);
        assert_eq!(m.layout().auxiliary_radius * 2.0, 44.0);
        let narrow = MobileControls {
            viewport: Vec2::new(568.0, 430.0),
            ..controls()
        };
        let layout = narrow.layout();
        let opening = layout.utility_centers[0].x
            - layout.auxiliary_radius
            - layout.joystick_center.x
            - layout.joystick_radius * 1.3;
        assert!(opening >= 48.0 - 0.001);
    }

    #[test]
    fn runtime_resize_keeps_conservative_safe_insets_at_short_and_tall_heights() {
        let mut app = App::new();
        app.insert_resource(controls())
            .add_message::<WindowFocused>()
            .add_message::<AppLifecycle>()
            .add_systems(Update, refresh_mobile_layout);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        for (width, height) in [(693.0, 320.0), (844.0, 390.0), (932.0, 430.0)] {
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .resolution
                .set(width, height);
            app.update();
            let mobile = app.world().resource::<MobileControls>();
            assert_eq!(mobile.viewport, Vec2::new(width, height));
            assert_eq!(
                [
                    mobile.safe.left,
                    mobile.safe.right,
                    mobile.safe.top,
                    mobile.safe.bottom
                ],
                [32.0, 32.0, 12.0, 20.0]
            );
            assert!(mobile.layout().auxiliary_radius >= 22.0);
        }
    }

    #[test]
    fn rank_mode_uses_the_skill_hit_circle_and_never_casts_or_upgrades_locked_slots() {
        let mut m = controls();
        m.upgrade_enabled = [true, true, false, false];
        let l = m.layout();
        m.event(1, TouchPhase::Started, l.upgrade_center);
        m.event(1, TouchPhase::Ended, l.upgrade_center);
        assert!(m.upgrade_mode);
        m.event(2, TouchPhase::Started, l.ability_centers[2]);
        m.event(2, TouchPhase::Ended, l.ability_centers[2]);
        assert!(m.upgrades.is_empty() && m.casts.is_empty());
        m.event(2, TouchPhase::Started, l.ability_centers[1]);
        m.event(2, TouchPhase::Ended, l.ability_centers[1]);
        assert_eq!(m.upgrades, [1]);
        assert!(m.casts.is_empty() && !m.upgrade_mode);
        m.upgrade_mode = true;
        m.clear();
        assert!(!m.upgrade_mode);
    }

    #[test]
    fn auxiliary_touch_ownership_keeps_category_and_utility_intents_separate() {
        let mut m = controls();
        let l = m.layout();
        m.event(1, TouchPhase::Started, l.joystick_center);
        m.event(1, TouchPhase::Moved, l.joystick_center + Vec2::X * 60.0);
        for (index, kind) in [TargetKind::Minion, TargetKind::Structure]
            .into_iter()
            .enumerate()
        {
            m.event(2, TouchPhase::Started, l.category_centers[index]);
            m.event(2, TouchPhase::Ended, l.category_centers[index]);
            assert_eq!(m.category_attacks[index], kind);
        }
        m.event(3, TouchPhase::Started, l.utility_centers[0]);
        m.event(
            3,
            TouchPhase::Ended,
            l.utility_centers[0] + Vec2::NEG_X * 60.0,
        );
        assert_eq!(m.utilities, [(UtilityAction::Dash, Some(Vec2::NEG_X))]);
        m.event(4, TouchPhase::Started, l.utility_centers[1]);
        m.event(4, TouchPhase::Canceled, l.utility_centers[1]);
        assert_eq!(m.utilities.len(), 1);
        assert_eq!(m.movement, Vec2::X);
        assert!(m.attacks.is_empty() && m.casts.is_empty());
        m.clear();
        assert!(m.utilities.is_empty() && m.category_attacks.is_empty());
    }

    #[test]
    fn auxiliary_controls_accept_stationary_taps_at_the_edge_of_the_full_hit_circle() {
        let mut m = controls();
        m.upgrade_enabled[0] = true;
        let l = m.layout();
        for center in [
            l.upgrade_center,
            l.category_centers[0],
            l.utility_centers[0],
        ] {
            let edge = center + Vec2::X * 21.0;
            m.event(1, TouchPhase::Started, edge);
            m.event(1, TouchPhase::Ended, edge);
        }
        assert_eq!(m.category_attacks, [TargetKind::Minion]);
        assert_eq!(m.utilities, [(UtilityAction::Dash, None)]);
        m.event(1, TouchPhase::Started, l.upgrade_center + Vec2::X * 21.0);
        m.event(1, TouchPhase::Ended, l.upgrade_center + Vec2::X * 21.0);
        assert!(m.upgrade_mode);
    }

    #[test]
    fn rank_ring_has_distinct_capacity_segments_and_learned_rank_pixels() {
        let count = |rank| {
            let pixels = skill_overlay_pixels(rank, 3, None);
            assert_eq!(
                &pixels[(56 * 112 + 56) * 4..(56 * 112 + 56) * 4 + 4],
                &[0, 0, 0, 0]
            );
            pixels
                .chunks_exact(4)
                .filter(|pixel| pixel[0] == 123)
                .count()
        };
        assert_eq!(count(0), 0);
        assert!(count(1) > 500);
        assert!((count(2) as i32 - 2 * count(1) as i32).abs() < 10);
        assert!((count(3) as i32 - 3 * count(1) as i32).abs() < 10);
        let dark = skill_overlay_pixels(0, 3, None);
        assert!(dark.chunks_exact(4).any(|pixel| pixel == [63, 85, 83, 240]));
        assert_ne!(
            skill_overlay_pixels(0, 0, Some(0.25)),
            skill_overlay_pixels(0, 0, Some(0.75))
        );
    }
}
