//! Native two-thumb controls. Touch ownership is per finger and survives crossing
//! another control; only a fresh Started event can capture an input.
use std::collections::HashMap;

use bevy::{
    input::touch::{TouchInput, TouchPhase},
    prelude::*,
    ui::FocusPolicy,
    window::{AppLifecycle, PrimaryWindow, WindowFocused},
};
use shared::{SkillSlot, ability_for_class_slot, scaled_mana_cost, unlocked_slots_for_level};

use crate::{
    combat::{CombatStats, LocalCastCooldown},
    input_context::{GameplayInputContext, InputContextSet},
    net::{NetworkHeroClass, PlayerProgression},
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
    pub ability_centers: [Vec2; 4],
    pub ability_radii: [f32; 4],
    pub upgrade_centers: [Vec2; 4],
    pub upgrade_radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MobileCastIntent {
    pub slot: usize,
    pub aim: Option<Vec2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Joystick,
    Ability(usize),
    Upgrade(usize),
}

#[derive(Debug, Clone, Copy)]
struct Capture {
    control: Control,
    origin: Vec2,
    position: Vec2,
    canceled: bool,
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
    pub upgrades: Vec<usize>,
    captures: HashMap<u64, Capture>,
    upgrade_enabled: [bool; 4],
    layout_changed: bool,
    round_identity: Option<(u64, u64)>,
}

impl Default for MobileControls {
    fn default() -> Self {
        Self {
            enabled: crate::platform::ui_profile() == crate::platform::UiProfile::Mobile,
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
            upgrades: Vec::new(),
            captures: HashMap::new(),
            upgrade_enabled: [false; 4],
            layout_changed: false,
            round_identity: None,
        }
    }
}

impl MobileControls {
    pub fn scale(&self) -> f32 {
        (self.viewport.y / 390.0).clamp(0.85, 1.25)
    }

    pub fn layout(&self) -> MobileLayout {
        let s = self.scale();
        let edge = self.viewport.x - self.safe.right;
        let bottom = self.viewport.y - self.safe.bottom;
        let attack = Vec2::new(edge - 48.0 * s, bottom - 48.0 * s);
        // Q is Omoba's basic attack. The three abilities form a compact
        // thumb fan above/left of it, with at least six logical pixels of
        // separation at our smallest supported phone viewport. Upgrades sit
        // outside the fan so they never steal an attack/ability touch.
        let centers = [
            attack,
            attack + Vec2::new(-91.0, -37.0) * s,
            attack + Vec2::new(-60.0, -98.0) * s,
            attack + Vec2::new(12.0, -98.0) * s,
        ];
        MobileLayout {
            joystick_center: Vec2::new(self.safe.left + 76.0 * s, bottom - 72.0 * s),
            joystick_radius: 58.0 * s,
            ability_centers: centers,
            ability_radii: [44.0 * s, 31.0 * s, 31.0 * s, 34.0 * s],
            upgrade_centers: [
                attack + Vec2::new(-65.0, 25.0) * s,
                attack + Vec2::new(-149.0, -37.0) * s,
                attack + Vec2::new(-67.0, -157.0) * s,
                attack + Vec2::new(12.0, -157.0) * s,
            ],
            upgrade_radius: 22.0 * s,
        }
    }

    fn clear(&mut self) {
        self.captures.clear();
        self.movement = Vec2::ZERO;
        self.casts.clear();
        self.upgrades.clear();
    }

    fn hit_control(&self, point: Vec2) -> Option<(Control, Vec2)> {
        let l = self.layout();
        for slot in 0..4 {
            if self.upgrade_enabled[slot]
                && point.distance(l.upgrade_centers[slot]) <= l.upgrade_radius
            {
                return Some((Control::Upgrade(slot), l.upgrade_centers[slot]));
            }
        }
        for slot in 0..4 {
            if point.distance(l.ability_centers[slot]) <= l.ability_radii[slot] {
                return Some((Control::Ability(slot), l.ability_centers[slot]));
            }
        }
        // Fixed anchor avoids moving the joystick under minimap/menu touches.
        (point.distance(l.joystick_center) <= l.joystick_radius * 1.3)
            .then_some((Control::Joystick, l.joystick_center))
    }

    fn event(&mut self, id: u64, phase: TouchPhase, position: Vec2) {
        if !position.is_finite() {
            self.captures.remove(&id);
            self.refresh_movement();
            return;
        }
        match phase {
            TouchPhase::Started => {
                let Some((control, origin)) = self.hit_control(position) else {
                    return;
                };
                if self.captures.values().any(|c| c.control == control) {
                    return;
                }
                self.captures.insert(
                    id,
                    Capture {
                        control,
                        origin,
                        position,
                        canceled: false,
                    },
                );
                if control == Control::Ability(0) {
                    self.casts.push(MobileCastIntent { slot: 0, aim: None });
                }
            }
            TouchPhase::Moved => {
                let cancel_distance = 158.0 * self.scale();
                if let Some(capture) = self.captures.get_mut(&id) {
                    capture.position = position;
                    capture.canceled = position.distance(capture.origin) > cancel_distance;
                }
            }
            TouchPhase::Ended | TouchPhase::Canceled => {
                if let Some(mut capture) = self.captures.remove(&id) {
                    capture.position = position;
                    capture.canceled |= position.distance(capture.origin) > 158.0 * self.scale();
                    if phase == TouchPhase::Ended && !capture.canceled {
                        match capture.control {
                            Control::Ability(slot) if slot != 0 => {
                                self.casts.push(MobileCastIntent {
                                    slot,
                                    aim: aim_vector(
                                        capture.position - capture.origin,
                                        self.scale(),
                                    ),
                                })
                            }
                            Control::Upgrade(slot)
                                if position.distance(capture.origin)
                                    <= self.layout().upgrade_radius * 1.3 =>
                            {
                                self.upgrades.push(slot)
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

    /// A held thumb remains a request; the combat system schedules it against
    /// the real rank/equipment cooldown each frame instead of a fixed repeat rate.
    pub(crate) fn held_attack(&self) -> Option<MobileCastIntent> {
        self.captures
            .values()
            .find(|capture| capture.control == Control::Ability(0) && !capture.canceled)
            .map(|capture| MobileCastIntent {
                slot: 0,
                aim: aim_vector(capture.position - capture.origin, self.scale()),
            })
    }

    #[cfg(test)]
    pub(crate) fn start_attack_hold_for_test(&mut self) {
        self.event(1, TouchPhase::Started, self.layout().ability_centers[0]);
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
fn aim_vector(delta: Vec2, scale: f32) -> Option<Vec2> {
    (delta.length() > 20.0 * scale).then(|| delta.normalize_or_zero())
}

pub(crate) struct MobileControlsPlugin;
impl Plugin for MobileControlsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MobileControls>()
            .add_systems(Startup, setup_mobile_controls)
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
    let s = mobile.scale();
    mobile.safe = MobileSafeInsets {
        left: 32.0 * s,
        right: 32.0 * s,
        top: 12.0 * s,
        bottom: 20.0 * s,
    };
}

fn read_mobile_controls(
    mut events: MessageReader<TouchInput>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    window: Query<(Entity, &Window), With<PrimaryWindow>>,
    context: Res<GameplayInputContext>,
    local: Query<(&CombatStats, Option<&PlayerProgression>), With<Player>>,
    mut mobile: ResMut<MobileControls>,
    snapshot: Option<Res<crate::net::GameStateSnapshot>>,
) {
    if let Some(snapshot) = snapshot {
        let identity = (snapshot.meta.server_epoch, snapshot.meta.match_id);
        if identity.0 != 0 && identity.1 != 0 {
            if mobile
                .round_identity
                .is_some_and(|previous| previous != identity)
            {
                mobile.clear();
                mobile.layout_changed = true;
            }
            mobile.round_identity = Some(identity);
        }
    }
    mobile.casts.clear();
    mobile.upgrades.clear();
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
                && unlocked_slots_for_level(prog.level.max(1))[slot]
        });
    }
    let Ok((window_entity, window)) = window.single() else {
        mobile.clear();
        events.clear();
        return;
    };
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
}

#[derive(Component)]
enum MobileVisual {
    Joystick,
    Thumb,
    Ability(usize),
    Upgrade(usize),
    AimHint,
    Rotate,
}

fn setup_mobile_controls(mut commands: Commands) {
    for visual in [
        MobileVisual::Joystick,
        MobileVisual::Thumb,
        MobileVisual::Ability(0),
        MobileVisual::Ability(1),
        MobileVisual::Ability(2),
        MobileVisual::Ability(3),
        MobileVisual::Upgrade(0),
        MobileVisual::Upgrade(1),
        MobileVisual::Upgrade(2),
        MobileVisual::Upgrade(3),
        MobileVisual::AimHint,
        MobileVisual::Rotate,
    ] {
        let is_rotate = matches!(visual, MobileVisual::Rotate);
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
                    ..default()
                },
                BackgroundColor(crate::ui_theme::PANEL),
                BorderColor::all(crate::ui_theme::EDGE),
                ZIndex(if is_rotate { 250 } else { 30 }),
                if is_rotate {
                    FocusPolicy::Block
                } else {
                    FocusPolicy::Pass
                },
                Name::new(match &visual {
                    MobileVisual::Joystick => "MobileJoystick".to_owned(),
                    MobileVisual::Thumb => "MobileThumb".to_owned(),
                    MobileVisual::Ability(slot) => format!("MobileAbility-{slot}"),
                    MobileVisual::Upgrade(slot) => format!("MobileUpgrade-{slot}"),
                    MobileVisual::AimHint => "MobileAimHint".to_owned(),
                    MobileVisual::Rotate => "MobileRotatePrompt".to_owned(),
                }),
                visual,
            ))
            .with_children(|parent| {
                parent.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: 15.0,
                        ..default()
                    },
                    TextColor(crate::ui_theme::IVORY),
                    TextLayout::new_with_justify(Justify::Center),
                ));
            });
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_mobile_controls(
    mobile: Res<MobileControls>,
    context: Res<GameplayInputContext>,
    local: Query<
        (
            &CombatStats,
            Option<&PlayerProgression>,
            Option<&NetworkHeroClass>,
        ),
        With<Player>,
    >,
    selection: Res<TeamSelection>,
    cooldown: Res<LocalCastCooldown>,
    mut visuals: Query<(
        &MobileVisual,
        &mut Node,
        &mut BackgroundColor,
        &mut BorderColor,
        &Children,
    )>,
    mut texts: Query<(&mut Text, &mut TextFont)>,
) {
    let layout = mobile.layout();
    let s = mobile.scale();
    let local = local.single().ok();
    let prog = local.and_then(|(_, p, _)| p).copied().unwrap_or_default();
    let class = local
        .and_then(|(_, _, c)| c)
        .map(|c| c.0)
        .unwrap_or(selection.hero_class);
    let visible = mobile.enabled
        && mobile.landscape
        && context.gameplay_allowed()
        && local.is_some_and(|(stats, _, _)| stats.is_alive());
    let aiming = mobile
        .captures
        .values()
        .find(|c| matches!(c.control, Control::Ability(slot) if slot != 0));
    for (visual, mut node, mut color, mut border, children) in &mut visuals {
        let (center, radius, label, show, fill, edge) = match *visual {
            MobileVisual::Joystick => (
                layout.joystick_center,
                layout.joystick_radius,
                String::new(),
                visible,
                Color::srgba(0.02, 0.08, 0.08, 0.45),
                crate::ui_theme::MUTED,
            ),
            MobileVisual::Thumb => (
                layout.joystick_center + mobile.movement * layout.joystick_radius * 0.7,
                24.0 * s,
                String::new(),
                visible,
                Color::srgba(0.43, 0.7, 0.62, 0.72),
                crate::ui_theme::JADE,
            ),
            MobileVisual::Ability(slot) => {
                let def = ability_for_class_slot(class, SkillSlot::from_index(slot as u8).unwrap());
                let unlocked = unlocked_slots_for_level(prog.level.max(1))[slot];
                let mana = local.is_some_and(|(stats, _, _)| {
                    stats.mana >= scaled_mana_cost(def, prog.ranks[slot].max(1))
                });
                let active = mobile
                    .captures
                    .values()
                    .any(|c| c.control == Control::Ability(slot) && !c.canceled);
                let tag = if slot == 0 {
                    "ATTACK"
                } else {
                    ["", "W", "E", "R"][slot]
                };
                let status = if !unlocked {
                    format!("Lv {}", shared::SLOT_UNLOCK_LEVELS[slot])
                } else if cooldown.remaining_secs[slot] > 0.0 {
                    format!("{:.1}", cooldown.remaining_secs[slot])
                } else if !mana {
                    "MANA".into()
                } else {
                    format!("{}", prog.ranks[slot].max(1))
                };
                (
                    layout.ability_centers[slot],
                    layout.ability_radii[slot],
                    format!("{tag}\n{status}"),
                    visible,
                    if active {
                        crate::ui_theme::HOVER
                    } else {
                        crate::ui_theme::PANEL
                    },
                    if !unlocked || !mana || cooldown.remaining_secs[slot] > 0.0 {
                        crate::ui_theme::EDGE
                    } else if slot == 0 {
                        crate::ui_theme::GOLD
                    } else {
                        crate::ui_theme::JADE
                    },
                )
            }
            MobileVisual::Upgrade(slot) => (
                layout.upgrade_centers[slot],
                layout.upgrade_radius,
                ["Q+", "W+", "E+", "R+"][slot].into(),
                visible && mobile.upgrade_enabled[slot],
                crate::ui_theme::HOVER,
                crate::ui_theme::GOLD,
            ),
            MobileVisual::AimHint => {
                let message = aiming
                    .map(|c| {
                        if c.canceled {
                            "CANCEL · release to discard"
                        } else {
                            "Drag to aim · release to cast\nDrag farther to cancel"
                        }
                    })
                    .unwrap_or("");
                (
                    Vec2::new(
                        mobile.viewport.x * 0.5,
                        mobile.viewport.y - mobile.safe.bottom - 36.0 * s,
                    ),
                    1.0,
                    message.into(),
                    visible && aiming.is_some(),
                    crate::ui_theme::PANEL,
                    crate::ui_theme::EDGE,
                )
            }
            MobileVisual::Rotate => (
                mobile.viewport * 0.5,
                1.0,
                "Rotate your phone\nPlay Omoba in landscape".into(),
                mobile.enabled && !mobile.landscape,
                crate::ui_theme::PANEL,
                crate::ui_theme::GOLD,
            ),
        };
        node.display = if show { Display::Flex } else { Display::None };
        let rectangular = matches!(visual, MobileVisual::AimHint | MobileVisual::Rotate);
        let size = if matches!(visual, MobileVisual::Rotate) {
            mobile.viewport
        } else if rectangular {
            Vec2::new(260.0 * s, 60.0 * s)
        } else {
            Vec2::splat(radius * 2.0)
        };
        node.left = Val::Px(center.x - size.x * 0.5);
        node.top = Val::Px(center.y - size.y * 0.5);
        node.width = Val::Px(size.x);
        node.height = Val::Px(size.y);
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
    fn controls() -> MobileControls {
        MobileControls {
            enabled: true,
            ..default()
        }
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
            m.event(
                1,
                TouchPhase::Moved,
                l.ability_centers[1] + Vec2::NEG_Y * 200.0,
            );
            m.event(1, phase, l.ability_centers[1] + Vec2::NEG_Y * 200.0);
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
                .insert_resource(crate::net::GameStateSnapshot::default())
                .add_message::<TouchInput>()
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
                    app.world_mut()
                        .resource_mut::<MobileControls>()
                        .round_identity = Some((1, 1));
                    let mut snapshot = app
                        .world_mut()
                        .resource_mut::<crate::net::GameStateSnapshot>();
                    snapshot.meta.server_epoch = 1;
                    snapshot.meta.match_id = 2;
                }
            }
            app.update();
            assert_eq!(
                app.world().resource::<MobileControls>().movement,
                Vec2::ZERO
            );
            assert!(app.world().resource::<MobileControls>().captures.is_empty());
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
    fn phone_controls_and_upgrade_targets_fit_safe_area_without_overlap() {
        for viewport in [
            Vec2::new(844.0, 390.0),
            Vec2::new(932.0, 430.0),
            Vec2::new(667.0, 375.0),
            Vec2::new(1280.0, 720.0),
        ] {
            let mut m = MobileControls {
                viewport,
                ..controls()
            };
            let scale = m.scale();
            m.safe = MobileSafeInsets {
                left: 32.0 * scale,
                right: 32.0 * scale,
                top: 12.0 * scale,
                bottom: 20.0 * scale,
            };
            let l = m.layout();
            let attack = l.ability_centers[0];
            // Reach bounds protect the compact fan while the spacing lower
            // bound prevents achieving compactness with ambiguous touch areas.
            for (slot, center) in l.ability_centers.iter().enumerate().skip(1) {
                assert!(center.distance(attack) <= 116.0 * scale);
                assert!(attack.y - (center.y - l.ability_radii[slot]) <= 133.0 * scale);
            }
            for (i, center) in l.ability_centers.iter().enumerate() {
                for j in i + 1..4 {
                    let gap = center.distance(l.ability_centers[j])
                        - l.ability_radii[i]
                        - l.ability_radii[j];
                    assert!(
                        gap >= 6.0,
                        "ability hit areas too close on {viewport:?}: {i}/{j}, {gap}"
                    );
                }
            }
            for center in l.upgrade_centers {
                assert!(center.distance(attack) <= 172.0 * scale);
                assert!(attack.y - (center.y - l.upgrade_radius) <= 180.0 * scale);
            }
            // Reserve the right HP/progression card above the combat fan.
            let edge = viewport.x - m.safe.right;
            let hero = Rect::from_corners(
                Vec2::new(edge - 300.0, m.safe.top),
                Vec2::new(edge - 128.0, m.safe.top + 108.0),
            );
            let circles: Vec<_> = l
                .ability_centers
                .iter()
                .copied()
                .zip(l.ability_radii)
                .chain(
                    l.upgrade_centers
                        .iter()
                        .copied()
                        .map(|p| (p, l.upgrade_radius)),
                )
                .chain([(l.joystick_center, l.joystick_radius)])
                .collect();
            for (i, (p, r)) in circles.iter().enumerate() {
                let nearest = p.clamp(hero.min, hero.max);
                assert!(
                    p.distance(nearest) >= *r,
                    "control overlaps right hero HUD on {viewport:?}"
                );
                assert!(p.x - r >= m.safe.left - 1.0 && p.x + r <= viewport.x - m.safe.right + 1.0);
                assert!(p.y - r >= m.safe.top && p.y + r <= viewport.y - m.safe.bottom);
                for (q, t) in &circles[i + 1..] {
                    assert!(
                        p.distance(*q) >= r + t,
                        "overlap at {viewport:?}: {p:?}, {q:?}"
                    );
                }
            }
        }
    }
}
