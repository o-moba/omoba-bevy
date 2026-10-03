//! Native two-thumb controls. Touch ownership is per finger and survives crossing
//! another control; only a fresh Started event can capture an input.
// i18n-strict
use std::collections::HashMap;

use crate::i18n::{tr, trf};
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
    ui::kit_assets::{Icon, KitImage, Sprite},
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
    pub category_radius: f32,
    pub recall_center: Vec2,
    pub upgrade_center: Vec2,
    pub upgrade_radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MobileCastIntent {
    pub slot: usize,
    pub extent: f32,
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
const SKILL_DESCRIPTION_SECONDS: f32 = crate::combat::inspection::HOLD_SECONDS as f32;
const SKILL_DRAG_DEAD_ZONE: f32 = 20.0;
/// ATK 96 is anchored 124px from safe right and 76px from safe bottom.
/// Four abilities and two compact category targets share one R104 arc;
/// utilities (48) use the outer R168 arc, both centred on ATK. Angles are
/// clockwise from +x with y down (screen axes).
/// The 48px horizontal shift fits MINION's complete 44px touch target:
/// 104 * cos(12°) + 22 = 123.727px. Skill heights and relative positions stay.
const ATTACK_INSET: Vec2 = Vec2::new(124.0, 76.0);
const ATTACK_RADIUS: f32 = crate::ui::tokens::size::ABILITY_ATTACK_PHONE * 0.5;
const ABILITY_RADIUS: f32 = crate::ui::tokens::size::ABILITY.phone * 0.5;
const UTILITY_RADIUS: f32 = crate::ui::tokens::size::ABILITY_UTILITY_PHONE * 0.5;
const ABILITY_ORBIT: f32 = crate::ui::tokens::size::COMBAT_ORBIT_ABILITY_PHONE;
const UTILITY_ORBIT: f32 = crate::ui::tokens::size::COMBAT_ORBIT_UTILITY_PHONE;
/// Q W E R, 42° apart (≥ 10 px between rims).
const SKILL_ORBIT_ANGLES: [f32; 4] = [162.0, 204.0, 246.0, 288.0];
/// Continue the skills' shared arc at equal 42° intervals: R288, TOWER330,
/// MINION372 (12°). Storage order follows TargetKind: MINION, TOWER.
const CATEGORY_ORBIT_ANGLES: [f32; 2] = [372.0, 330.0];
/// DASH, HASTE, CANCEL (aiming only), RANK (skill points), RECALL — 22° apart.
const DASH_ANGLE: f32 = 166.0;
const HASTE_ANGLE: f32 = 188.0;
const CANCEL_ANGLE: f32 = 210.0;
const RANK_ANGLE: f32 = 232.0;
const RECALL_ANGLE: f32 = 254.0;
/// Narrow phones tuck HASTE between Q/W and move DASH out enough to retain
/// separate 48px controls, leaving the hero neighborhood clear.
const COMPACT_UTILITY_WIDTH: f32 = 700.0;
const COMPACT_DASH_ORBIT: f32 = 176.0;
const COMPACT_HASTE_ORBIT: f32 = 150.0;
const COMPACT_HASTE_ANGLE: f32 = 182.6;
/// Joystick centre: safe left + 68, safe bottom − 65; base r 52, knob r 24;
/// the capture circle is 1.3 × r.
const JOYSTICK_INSET: Vec2 = Vec2::new(68.0, 65.0);
const JOYSTICK_RADIUS: f32 = crate::ui::tokens::size::JOYSTICK_PHONE * 0.5;
const JOYSTICK_CAPTURE: f32 = 1.3;
pub(crate) const KNOB_RADIUS: f32 = crate::ui::tokens::size::JOYSTICK_KNOB_PHONE * 0.5;
/// Width the group needs at scale 1: the joystick capture circle from the
/// safe left edge to the leftmost utility's rim.
fn combat_group_span(compact: bool) -> f32 {
    let utility_left = if compact {
        -COMPACT_DASH_ORBIT * DASH_ANGLE.to_radians().cos()
    } else {
        -UTILITY_ORBIT * HASTE_ANGLE.to_radians().cos()
    };
    JOYSTICK_INSET.x
        + JOYSTICK_RADIUS * JOYSTICK_CAPTURE
        + ATTACK_INSET.x
        + utility_left
        + UTILITY_RADIUS
}

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

/// Player adjustments are bounded and applied to each whole control group.
#[derive(
    Resource, Debug, Default, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize,
)]
#[serde(default)]
pub(crate) struct HudPositionSettings {
    pub joystick_offset: Vec2,
    pub combat_offset: Vec2,
}
impl HudPositionSettings {
    pub(crate) fn sanitized(self) -> Self {
        let clean = |v: Vec2| {
            if v.is_finite() {
                v.clamp(Vec2::splat(-60.0), Vec2::splat(60.0))
            } else {
                Vec2::ZERO
            }
        };
        Self {
            joystick_offset: clean(self.joystick_offset),
            combat_offset: clean(self.combat_offset),
        }
    }
}

#[derive(Resource)]
pub(crate) struct MobileControls {
    /// Chosen from the compiled platform once; resizing never changes this.
    pub enabled: bool,
    pub viewport: Vec2,
    pub hud_position: HudPositionSettings,
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
    utilities_available: bool,
    recall_active: bool,
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
            hud_position: HudPositionSettings::default(),
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
            utilities_available: true,
            recall_active: false,
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

    /// Scale the complete combat group without shrinking its touch targets
    /// (today's rule, ≥ 1). Keep this separate from frontend scale: small
    /// phones still need full-size controls, while menu typography and panels
    /// may use the smaller UI scale.
    fn compact_utilities(&self) -> bool {
        self.viewport.x - self.safe.left - self.safe.right < COMPACT_UTILITY_WIDTH
    }

    pub(crate) fn combat_scale(&self) -> f32 {
        let usable_width = self.viewport.x - self.safe.left - self.safe.right;
        // Preserve at least 48px between the joystick's expanded hit circle
        // and the leftmost utility on narrow landscape viewports.
        self.scale()
            .min((usable_width - 48.0) / combat_group_span(self.compact_utilities()))
            .max(1.0)
    }

    pub fn layout(&self) -> MobileLayout {
        // Two concentric arcs centred exactly on basic attack (hud.md phone
        // redline). Anchor and scale the group together; clamping individual
        // controls to viewport edges would break the arcs and the thumb's
        // muscle memory.
        let s = self.combat_scale();
        let right = self.viewport.x - self.safe.right;
        let bottom = self.viewport.y - self.safe.bottom;
        let attack_center = Vec2::new(right, bottom) - ATTACK_INSET * s;
        let arc = |radius: f32| {
            move |angle: f32| {
                let (sin, cos) = angle.to_radians().sin_cos();
                attack_center + Vec2::new(cos, sin) * radius * s
            }
        };
        let inner = arc(ABILITY_ORBIT);
        let outer = arc(UTILITY_ORBIT);
        let mut layout = MobileLayout {
            joystick_center: Vec2::new(
                self.safe.left + JOYSTICK_INSET.x * s,
                bottom - JOYSTICK_INSET.y * s,
            ),
            joystick_radius: JOYSTICK_RADIUS * s,
            attack_center,
            attack_radius: ATTACK_RADIUS * s,
            cancel_center: outer(CANCEL_ANGLE),
            cancel_radius: UTILITY_RADIUS * s,
            ability_centers: SKILL_ORBIT_ANGLES.map(inner),
            ability_radii: [ABILITY_RADIUS * s; 4],
            category_centers: CATEGORY_ORBIT_ANGLES.map(inner),
            category_radius: 22.0 * s,
            recall_center: outer(RECALL_ANGLE),
            utility_centers: if self.compact_utilities() {
                [
                    arc(COMPACT_DASH_ORBIT)(DASH_ANGLE),
                    arc(COMPACT_HASTE_ORBIT)(COMPACT_HASTE_ANGLE),
                ]
            } else {
                [outer(DASH_ANGLE), outer(HASTE_ANGLE)]
            },
            auxiliary_radius: UTILITY_RADIUS * s,
            upgrade_center: outer(RANK_ANGLE),
            upgrade_radius: UTILITY_RADIUS * s,
        };
        let settings = self.hud_position.sanitized();
        let safe_min = Vec2::new(self.safe.left, self.safe.top);
        let safe_max = self.viewport - Vec2::new(self.safe.right, self.safe.bottom);
        let clamp_shift = |requested: Vec2, min: Vec2, max: Vec2| {
            requested.clamp(
                (safe_min - min).min(Vec2::ZERO),
                (safe_max - max).max(Vec2::ZERO),
            )
        };
        layout.joystick_center += clamp_shift(
            settings.joystick_offset,
            layout.joystick_center - Vec2::splat(layout.joystick_radius),
            layout.joystick_center + Vec2::splat(layout.joystick_radius),
        );
        let circles = layout
            .ability_centers
            .into_iter()
            .zip(layout.ability_radii)
            .chain(
                layout
                    .utility_centers
                    .into_iter()
                    .map(|c| (c, layout.auxiliary_radius)),
            )
            .chain(
                layout
                    .category_centers
                    .into_iter()
                    .map(|c| (c, layout.category_radius)),
            )
            .chain([
                (layout.attack_center, layout.attack_radius),
                (layout.cancel_center, layout.cancel_radius),
                (layout.upgrade_center, layout.upgrade_radius),
                (layout.recall_center, layout.auxiliary_radius),
            ]);
        let (min, max) = circles.fold(
            (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
            |(min, max), (center, radius)| {
                (
                    min.min(center - Vec2::splat(radius)),
                    max.max(center + Vec2::splat(radius)),
                )
            },
        );
        let mut shift = clamp_shift(settings.combat_offset, min, max);
        // Keep a passage between two independently adjusted thumb groups.
        shift.x = shift.x.max(
            (layout.joystick_center.x + layout.joystick_radius * JOYSTICK_CAPTURE + 32.0 - min.x)
                .min(0.0),
        );
        layout.attack_center += shift;
        layout.cancel_center += shift;
        layout.upgrade_center += shift;
        layout.recall_center += shift;
        for center in layout
            .ability_centers
            .iter_mut()
            .chain(layout.utility_centers.iter_mut())
            .chain(layout.category_centers.iter_mut())
        {
            *center += shift;
        }
        layout
    }

    pub(crate) fn clear(&mut self) {
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

    fn upgrade_badge_center(&self, slot: usize) -> Vec2 {
        let layout = self.layout();
        layout.ability_centers[slot] + Vec2::new(1.0, -1.0) * (layout.ability_radii[slot] - 6.0)
    }

    fn upgrade_badge_hit(&self, slot: usize, point: Vec2) -> bool {
        // A 44-point touch target around the smaller visible badge.
        self.upgrade_enabled[slot] && point.distance(self.upgrade_badge_center(slot)) <= 22.0
    }

    fn hit_control(&self, point: Vec2) -> Option<(Control, Vec2)> {
        let l = self.layout();
        // Badges take priority over the ability face and own the entire gesture.
        for slot in 0..4 {
            if self.upgrade_badge_hit(slot, point) {
                return Some((Control::Upgrade(slot), self.upgrade_badge_center(slot)));
            }
        }
        if self.upgrade_enabled.iter().any(|enabled| *enabled)
            && point.distance(l.upgrade_center) <= l.upgrade_radius
        {
            return Some((Control::UpgradeMode, l.upgrade_center));
        }
        for (index, kind) in [TargetKind::Minion, TargetKind::Structure]
            .into_iter()
            .enumerate()
        {
            if point.distance(l.category_centers[index]) <= l.category_radius {
                return Some((Control::CategoryAttack(kind), l.category_centers[index]));
            }
        }
        for (index, action) in [UtilityAction::Dash, UtilityAction::Haste]
            .into_iter()
            .enumerate()
        {
            if self.utilities_available
                && point.distance(l.utility_centers[index]) <= l.auxiliary_radius
            {
                return Some((Control::Utility(action), l.utility_centers[index]));
            }
        }
        if self.utilities_available && point.distance(l.recall_center) <= l.auxiliary_radius {
            return Some((
                Control::Utility(if self.recall_active {
                    UtilityAction::CancelRecall
                } else {
                    UtilityAction::Recall
                }),
                l.recall_center,
            ));
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
        (point.distance(l.joystick_center) <= l.joystick_radius * JOYSTICK_CAPTURE)
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
                    capture.canceled = position.distance(layout.cancel_center)
                        <= layout.cancel_radius
                        || (capture.control == Control::Utility(UtilityAction::Dash)
                            && position.distance(capture.origin) > ATTACK_DRAG_REACH * scale * 2.0);
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
                        <= self.layout().cancel_radius
                        || (capture.control == Control::Utility(UtilityAction::Dash)
                            && position.distance(capture.origin)
                                > ATTACK_DRAG_REACH * self.combat_scale() * 2.0);
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
                                    extent: ((capture.position - capture.origin).length()
                                        / (ATTACK_DRAG_REACH * self.combat_scale()))
                                    .clamp(0.15, 1.0),
                                    aim: aim_vector(
                                        capture.position - capture.origin,
                                        self.combat_scale(),
                                    ),
                                })
                            }
                            Control::Upgrade(slot)
                                if self.upgrade_enabled[slot]
                                    && (self.upgrade_badge_hit(slot, position)
                                        || (self.upgrade_mode
                                            && position.distance(
                                                self.layout().ability_centers[slot],
                                            ) <= self.layout().ability_radii[slot])) =>
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
                            Control::Utility(
                                UtilityAction::Recall | UtilityAction::CancelRecall,
                            ) if capture.dragged => {}
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

    /// Screen direction for the dash preview, following the same axes as skill aiming.
    pub(crate) fn dash_aim(&self) -> Option<Vec2> {
        self.captures.values().find_map(|capture| {
            (capture.control == Control::Utility(UtilityAction::Dash)
                && capture.dragged
                && !capture.canceled)
                .then(|| aim_vector(capture.position - capture.origin, self.combat_scale()))
                .flatten()
        })
    }

    pub(crate) fn aimed_skill(&self) -> Option<MobileCastIntent> {
        self.captures.values().find_map(|c| match c.control {
            Control::Ability(slot) if c.dragged && !c.canceled && !c.inspecting => {
                Some(MobileCastIntent {
                    slot,
                    aim: aim_vector(c.position - c.origin, self.combat_scale()),
                    extent: ((c.position - c.origin).length()
                        / (ATTACK_DRAG_REACH * self.combat_scale()))
                    .clamp(0.15, 1.0),
                })
            }
            _ => None,
        })
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

    pub(crate) fn inspected_skill(&self) -> Option<usize> {
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
        app.init_resource::<MobileControls>()
            .init_resource::<HudPositionSettings>();
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
    settings: Option<Res<HudPositionSettings>>,
    mut focus_events: MessageReader<WindowFocused>,
    mut lifecycle: MessageReader<AppLifecycle>,
) {
    let Ok(window) = window.single() else {
        return;
    };
    let viewport = Vec2::new(window.width(), window.height());
    let position = settings.as_deref().copied().unwrap_or_default().sanitized();
    mobile.layout_changed = position != mobile.hud_position
        || viewport != mobile.viewport
        || focus_events.read().any(|event| !event.focused)
        || lifecycle.read().any(|event| {
            matches!(
                event,
                AppLifecycle::WillSuspend | AppLifecycle::Suspended | AppLifecycle::WillResume
            )
        });
    mobile.hud_position = position;
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
    local: Query<
        (
            &CombatStats,
            Option<&PlayerProgression>,
            Option<&NetworkHeroClass>,
        ),
        With<Player>,
    >,
    mut mobile: ResMut<MobileControls>,
    mut session_events: MessageReader<SessionEvent>,
    gamepad: Option<Res<crate::gamepad::GamepadControls>>,
    utilities: Query<&crate::net::PlayerUtility, With<Player>>,
) {
    mobile.recall_active = utilities
        .single()
        .is_ok_and(|utility| utility.state.recall_remaining_secs > 0.0);
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
    let alive = local.single().is_ok_and(|(stats, _, _)| stats.is_alive());
    mobile.utilities_available = true;
    // A controller that owns input hides the touch HUD; the first touch
    // takes ownership back before this runs, so no finger is lost.
    let controller = gamepad.as_ref().is_some_and(|pad| pad.active);
    if !mobile.enabled
        || controller
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
    if let Ok((_, prog, _)) = local.single() {
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
    DashVector,
    DashThumb,
    Recall,
    Cancel,
    Ability(usize),
    UpgradeMode,
    CategoryAttack(usize),
    Utility(usize),
    /// Gold rim around an upgradable ability in rank mode.
    RankRing(usize),
    UpgradeBadge(usize),
    AimHint,
    SkillDescription,
    Rotate,
}

/// Parts of a touch disc (`hud.md` phone combat group).
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum DiscPart {
    Sweep,
    Seconds,
    Icon,
    Label,
}

/// How a touch disc is drawn at rest (`hud.md` phone § combat-cluster,
/// attack): fill, rim width and colour, icon.
struct DiscLook {
    fill: Color,
    rim: f32,
    edge: Color,
    icon: Option<Icon>,
    icon_size: f32,
}

fn disc_look(visual: &MobileVisual) -> Option<DiscLook> {
    use crate::ui::tokens::{border, color, size};
    let utility = |icon| DiscLook {
        fill: color::EMERALD_800,
        rim: border::HAIRLINE,
        edge: color::GOLD_600,
        icon: Some(icon),
        icon_size: size::ICON_MD,
    };
    Some(match visual {
        MobileVisual::Attack => DiscLook {
            fill: color::EMERALD_700,
            rim: border::FRAME,
            edge: color::GOLD_500,
            icon: Some(Icon::HudAttack),
            icon_size: size::ICON_XL,
        },
        MobileVisual::Recall => utility(Icon::HudRecall),
        MobileVisual::Utility(0) => utility(Icon::HudDash),
        MobileVisual::Utility(_) => utility(Icon::HudHaste),
        MobileVisual::CategoryAttack(0) => utility(Icon::HudMinion),
        MobileVisual::CategoryAttack(_) => utility(Icon::HudTower),
        MobileVisual::UpgradeMode => DiscLook {
            fill: color::EMERALD_400,
            rim: border::FRAME,
            edge: color::GOLD_400,
            icon: None,
            icon_size: 0.0,
        },
        // Dashed in the redline; Bevy borders are solid.
        MobileVisual::Cancel => DiscLook {
            fill: crate::ui::theme::perceptual(color::SURFACE_GLASS_STRONG),
            rim: border::FRAME,
            edge: color::STATE_DANGER,
            icon: Some(Icon::NavX),
            icon_size: size::ICON_MD,
        },
        _ => return None,
    })
}

fn visual_name(visual: &MobileVisual) -> String {
    match visual {
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
        MobileVisual::Utility(index) => ["MobileDash", "MobileHaste"][*index].to_owned(),
        MobileVisual::RankRing(slot) => format!("MobileRankRing-{slot}"),
        MobileVisual::UpgradeBadge(slot) => format!("MobileUpgrade-{slot}"),
        MobileVisual::Recall => "MobileRecall".to_owned(),
        MobileVisual::DashVector => "MobileDashVector".to_owned(),
        MobileVisual::DashThumb => "MobileDashThumb".to_owned(),
        MobileVisual::AimHint => "MobileAimHint".to_owned(),
        MobileVisual::SkillDescription => "MobileSkillDescription".to_owned(),
        MobileVisual::Rotate => "MobileRotatePrompt".to_owned(),
    }
}

fn setup_mobile_controls(mut commands: Commands) {
    use crate::ui::{
        theme::{self, TextStyle},
        tokens::{TextRole, border, color, radius, size},
        widgets::{game, icon_node},
    };
    let hidden = || Node {
        position_type: PositionType::Absolute,
        display: Display::None,
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    };
    for visual in [
        MobileVisual::Joystick,
        MobileVisual::Thumb,
        MobileVisual::AttackVector,
        MobileVisual::AttackThumb,
        MobileVisual::DashVector,
        MobileVisual::DashThumb,
        MobileVisual::Recall,
        MobileVisual::Attack,
        MobileVisual::Cancel,
        MobileVisual::UpgradeMode,
        MobileVisual::CategoryAttack(0),
        MobileVisual::CategoryAttack(1),
        MobileVisual::Utility(0),
        MobileVisual::Utility(1),
        MobileVisual::AimHint,
        MobileVisual::Rotate,
    ] {
        let name = visual_name(&visual);
        let z = ZIndex(match visual {
            MobileVisual::Rotate => 250,
            // A held finger can cross another skill. Keep the gesture above
            // every control and upgrade badge, independent of spawn order.
            MobileVisual::AttackVector | MobileVisual::DashVector => 40,
            MobileVisual::AttackThumb | MobileVisual::DashThumb => 41,
            _ => 30,
        });
        let policy = if matches!(visual, MobileVisual::Rotate) {
            FocusPolicy::Block
        } else {
            FocusPolicy::Pass
        };
        let mut root = commands.spawn((
            hidden(),
            UiTransform::default(),
            BackgroundColor(Color::NONE),
            BorderColor::all(Color::NONE),
            z,
            policy,
            Name::new(name),
        ));
        match &visual {
            // hud.md phone `joystick`: base glass + hairline, knob
            // `color.emerald.600` with a 2 px `color.gold.500` rim.
            MobileVisual::Joystick => {
                root.insert((
                    Node {
                        border: UiRect::all(Val::Px(border::HAIRLINE)),
                        ..hidden()
                    },
                    BackgroundColor(theme::perceptual(color::SURFACE_GLASS)),
                    BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)),
                ));
            }
            MobileVisual::Thumb => {
                root.insert((
                    Node {
                        border: UiRect::all(Val::Px(border::FRAME)),
                        ..hidden()
                    },
                    BackgroundColor(color::EMERALD_600),
                    BorderColor::all(color::GOLD_500),
                ));
            }
            MobileVisual::AttackVector
            | MobileVisual::AttackThumb
            | MobileVisual::DashVector
            | MobileVisual::DashThumb => {
                let thumb = matches!(visual, MobileVisual::AttackThumb | MobileVisual::DashThumb);
                root.insert((
                    Node {
                        border: UiRect::all(Val::Px(if thumb { 2.0 } else { 0.0 })),
                        ..hidden()
                    },
                    BackgroundColor(if thumb {
                        color::EMERALD_600
                    } else {
                        color::GOLD_400
                    }),
                    BorderColor::all(color::GOLD_400),
                ));
            }
            MobileVisual::AimHint => {
                root.insert((
                    BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
                    BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)),
                ))
                .with_child((
                    Text::new(""),
                    theme::styled_text(TextStyle::keep_case(TextRole::Caption)),
                    TextColor(color::TEXT_PRIMARY),
                    TextLayout::new_with_justify(Justify::Center),
                    DiscPart::Label,
                ));
            }
            MobileVisual::Rotate => {
                root.insert((
                    BackgroundColor(color::SURFACE_1_OPAQUE),
                    BorderColor::all(color::GOLD_500),
                ))
                .with_child((
                    Text::new(""),
                    theme::role_text(TextRole::Heading),
                    TextColor(color::TEXT_PRIMARY),
                    TextLayout::new_with_justify(Justify::Center),
                    DiscPart::Label,
                ));
            }
            other => {
                let Some(look) = disc_look(other) else {
                    continue;
                };
                let attack = matches!(other, MobileVisual::Attack);
                root.insert((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(0.0),
                        border: UiRect::all(Val::Px(look.rim)),
                        border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                        overflow: Overflow::clip(),
                        ..hidden()
                    },
                    BackgroundColor(look.fill),
                    BorderColor::all(look.edge),
                ))
                .with_children(|disc| {
                    disc.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            right: Val::Px(0.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            ..default()
                        },
                        KitImage::atlas(
                            Sprite::CooldownSweepAtlas,
                            theme::perceptual(color::COOLDOWN_OVERLAY),
                            0,
                        ),
                        Visibility::Hidden,
                        DiscPart::Sweep,
                    ));
                    if let Some(icon) = look.icon {
                        disc.spawn((
                            icon_node(icon, look.icon_size, color::TEXT_GOLD),
                            DiscPart::Icon,
                        ));
                    }
                    disc.spawn((
                        Text::new(""),
                        if attack {
                            theme::role_text(TextRole::Button)
                        } else {
                            theme::styled_text(
                                TextStyle::keep_case(TextRole::Label)
                                    .sized(TextRole::Caption.style().size),
                            )
                        },
                        TextColor(if matches!(other, MobileVisual::UpgradeMode) {
                            color::TEXT_ON_PRIMARY
                        } else {
                            color::TEXT_GOLD
                        }),
                        TextLayout::new_with_justify(Justify::Center),
                        bevy::text::LineHeight::RelativeToFont(1.0),
                        DiscPart::Label,
                    ));
                    disc.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            right: Val::Px(0.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        Visibility::Hidden,
                        DiscPart::Seconds,
                        children![(
                            Text::new(""),
                            theme::styled_text(
                                TextStyle::new(TextRole::NumberLg)
                                    .sized(crate::ui::tokens::Metric::new(20.0, 20.0)),
                            ),
                            TextColor(color::TEXT_PRIMARY),
                            TextShadow::default(),
                        )],
                    ));
                });
            }
        }
        root.insert(visual);
    }
    // Abilities: the kit face (cost pill kept, key and pips hidden), with
    // the rank ring (+ badge, gold rim in rank mode) over each.
    for slot in 0..4 {
        let view = game::AbilityView {
            ability: None,
            icon: Icon::HudAttack,
            key: None,
            cost: Some(0),
            rank: 1,
            cooldown: None,
            locked: false,
            unlock_level: Some(shared::SLOT_UNLOCK_LEVELS[slot] as u8),
            no_mana: false,
            pips: false,
            // hud.md phone: rank is the segmented ring on the rim (R10).
            ring: true,
        };
        commands
            .spawn((
                hidden(),
                UiTransform::default(),
                ZIndex(30),
                FocusPolicy::Pass,
                MobileVisual::Ability(slot),
                Name::new(visual_name(&MobileVisual::Ability(slot))),
            ))
            .with_children(|root| {
                game::ability_face(
                    root,
                    view,
                    size::ABILITY.phone,
                    game::AbilityFace::default(),
                );
            });
        commands.spawn((
            Node {
                border: UiRect::all(Val::Px(border::FRAME)),
                border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                ..hidden()
            },
            UiTransform::default(),
            BackgroundColor(Color::NONE),
            BorderColor::all(Color::NONE),
            ZIndex(31),
            FocusPolicy::Pass,
            Pickable::IGNORE,
            MobileVisual::RankRing(slot),
            Name::new(visual_name(&MobileVisual::RankRing(slot))),
        ));
        commands.spawn((
            Node {
                border: UiRect::all(Val::Px(border::FRAME)),
                border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                ..hidden()
            },
            UiTransform::default(),
            BackgroundColor(color::EMERALD_400),
            BorderColor::all(color::GOLD_400),
            ZIndex(32),
            FocusPolicy::Pass,
            Pickable::IGNORE,
            MobileVisual::UpgradeBadge(slot),
            Name::new(visual_name(&MobileVisual::UpgradeBadge(slot))),
            children![icon_node(
                Icon::NavPlus,
                game::ABILITY_UPGRADE_GLYPH,
                color::TEXT_ON_PRIMARY
            )],
        ));
    }
    // The hold card (`skill-description.md`, phone): the shared tooltip.
    crate::combat::skill_card::spawn_skill_card(
        &mut commands,
        crate::ui::theme::Form::Phone,
        (
            MobileVisual::SkillDescription,
            Name::new(visual_name(&MobileVisual::SkillDescription)),
        ),
    );
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
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
    gamepad: Option<Res<crate::gamepad::GamepadControls>>,
    utilities: Query<&crate::net::PlayerUtility, With<Player>>,
    mut visuals: Query<(
        &MobileVisual,
        &mut Node,
        &mut UiTransform,
        Option<&mut BackgroundColor>,
        Option<&mut BorderColor>,
        Option<&Children>,
        Option<&mut crate::combat::skill_card::SkillCardView>,
    )>,
    mut faces: Query<(
        &mut crate::ui::widgets::game::AbilityView,
        &mut crate::ui::widgets::game::AbilityFace,
    )>,
    mut parts: Query<
        (
            &DiscPart,
            &mut Visibility,
            Option<&mut KitImage>,
            Option<&Children>,
            Option<&mut Node>,
        ),
        Without<MobileVisual>,
    >,
    mut texts: Query<(&mut Text, &mut TextColor), Without<MobileVisual>>,
) {
    use crate::ui::tokens::color;
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
    let mana = local.map_or(0.0, |(stats, _, _, _)| stats.mana);
    // hud.md § States, Dead: the combat group stays drawn but veiled (input
    // is off: `read_mobile_controls` clears every finger while dead).
    let dead = local.is_some_and(|(stats, _, _, _)| !stats.is_alive());
    let visible = mobile.enabled
        && !gamepad.as_ref().is_some_and(|pad| pad.active)
        && mobile.landscape
        && context.gameplay_allowed()
        && local.is_some();
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
    let attack_remaining = basic_attack
        .as_ref()
        .map_or(0.0, |state| state.remaining_secs);
    let attack_total = basic_attack
        .as_ref()
        .map_or(0.0, |state| state.duration_secs)
        .max(attack_remaining);
    let attack_held = mobile.held_basic_attack();
    let drag = attack
        .filter(|capture| capture.dragged)
        .map(|capture| (capture.position - capture.origin).clamp_length_max(ATTACK_DRAG_REACH * s));
    let dash_drag = mobile
        .captures
        .values()
        .find(|c| c.control == Control::Utility(UtilityAction::Dash) && c.dragged && !c.canceled)
        .map(|c| (c.position - c.origin).clamp_length_max(ATTACK_DRAG_REACH * s));
    let inspected = mobile.inspected_skill();
    let utility = utilities
        .single()
        .map(|utility| utility.state)
        .unwrap_or_default();
    let place = |node: &mut Node, center: Vec2, size: Vec2| {
        let (left, top) = (
            Val::Px(center.x - size.x * 0.5),
            Val::Px(center.y - size.y * 0.5),
        );
        if node.left != left || node.top != top {
            node.left = left;
            node.top = top;
        }
        if node.width != Val::Px(size.x) || node.height != Val::Px(size.y) {
            node.width = Val::Px(size.x);
            node.height = Val::Px(size.y);
        }
    };
    for (visual, mut node, mut transform, fill, edge, children, card) in &mut visuals {
        // (centre, diameter, label, shown, cooldown (remaining, total), fill, edge)
        let mut cooldown_state: Option<(f32, f32)> = None;
        let mut label = String::new();
        let mut look_fill = None;
        let mut look_edge = None;
        let (center, size, show) = match *visual {
            MobileVisual::Joystick => (
                layout.joystick_center,
                Vec2::splat(layout.joystick_radius * 2.0),
                visible,
            ),
            MobileVisual::Thumb => (
                layout.joystick_center + mobile.movement * layout.joystick_radius * 0.7,
                Vec2::splat(KNOB_RADIUS * 2.0 * s),
                visible,
            ),
            MobileVisual::Attack => {
                label = tr("touch.attack").into();
                if attack_remaining > 0.0 {
                    cooldown_state = Some((attack_remaining, attack_total));
                }
                look_fill = Some(if attack.is_some() {
                    color::EMERALD_600
                } else {
                    color::EMERALD_700
                });
                look_edge = Some(if mobile.attack_cancelled() {
                    color::STATE_DANGER
                } else if attack_held {
                    color::GOLD_300
                } else {
                    color::GOLD_500
                });
                (
                    layout.attack_center,
                    Vec2::splat(layout.attack_radius * 2.0),
                    visible,
                )
            }
            MobileVisual::AttackVector | MobileVisual::AttackThumb => {
                let vector = matches!(visual, MobileVisual::AttackVector);
                let reach = drag.unwrap_or_default();
                let center = layout.attack_center + reach * if vector { 0.5 } else { 1.0 };
                let size = if vector {
                    Vec2::new(reach.length(), 3.0 * s)
                } else {
                    Vec2::splat(32.0 * s)
                };
                look_fill = Some(if mobile.attack_cancelled() {
                    color::STATE_DANGER
                } else if vector {
                    color::GOLD_400
                } else {
                    color::EMERALD_600
                });
                (
                    center,
                    size,
                    visible && drag.is_some() && !mobile.skill_aiming(),
                )
            }
            MobileVisual::DashVector | MobileVisual::DashThumb => {
                let vector = matches!(visual, MobileVisual::DashVector);
                let reach = dash_drag.unwrap_or_default();
                (
                    layout.utility_centers[0] + reach * if vector { 0.5 } else { 1.0 },
                    if vector {
                        Vec2::new(reach.length(), 3.0 * s)
                    } else {
                        Vec2::splat(28.0 * s)
                    },
                    visible && dash_drag.is_some(),
                )
            }
            MobileVisual::Recall => {
                label = tr("touch.recall").into();
                if utility.recall_remaining_secs > 0.0 {
                    cooldown_state = Some((
                        utility.recall_remaining_secs,
                        shared::utility::RECALL_CHANNEL_SECS,
                    ));
                    look_fill = Some(color::EMERALD_600);
                }
                (
                    layout.recall_center,
                    Vec2::splat(layout.auxiliary_radius * 2.0),
                    visible,
                )
            }
            MobileVisual::Cancel => {
                look_fill = Some(if aiming.is_some_and(|capture| capture.canceled) {
                    color::STATE_DANGER
                } else {
                    crate::ui::theme::perceptual(color::SURFACE_GLASS_STRONG)
                });
                (
                    layout.cancel_center,
                    Vec2::splat(layout.cancel_radius * 2.0),
                    visible && aiming.is_some() && inspected.is_none(),
                )
            }
            MobileVisual::Ability(slot) => {
                let def = ability_for_class_slot(class, SkillSlot::from_index(slot as u8).unwrap());
                let rank = prog.ranks[slot].max(1);
                let cost = if cooldown.recast[slot] {
                    0.0
                } else {
                    scaled_mana_cost(def, rank)
                };
                let remaining = cooldown.remaining_secs[slot];
                let fraction = cooldown.remaining_fraction(slot);
                if let Some(children) = children {
                    for child in children.iter() {
                        if let Ok((mut view, mut face)) = faces.get_mut(child) {
                            let unlocked = prog.unlocked()[slot];
                            let next = crate::ui::widgets::game::AbilityView {
                                ability: Some(def.id),
                                cost: Some(cost.round() as u32),
                                // A locked ability shows the ring empty.
                                rank: if unlocked { prog.ranks[slot] } else { 0 },
                                cooldown: (remaining > 0.0).then(|| {
                                    (
                                        remaining,
                                        if fraction > 0.0 {
                                            remaining / fraction
                                        } else {
                                            remaining
                                        },
                                    )
                                }),
                                locked: !unlocked || dead,
                                unlock_level: (!unlocked && !dead)
                                    .then_some(shared::SLOT_UNLOCK_LEVELS[slot] as u8),
                                no_mana: mana < cost,
                                ..view.clone()
                            };
                            if *view != next {
                                *view = next;
                            }
                            let active = mobile
                                .captures
                                .values()
                                .any(|c| c.control == Control::Ability(slot) && !c.canceled);
                            let next_face = crate::ui::widgets::game::AbilityFace {
                                glow: active && inspected != Some(slot),
                                held: inspected == Some(slot),
                            };
                            if *face != next_face {
                                *face = next_face;
                            }
                        }
                    }
                }
                (
                    layout.ability_centers[slot],
                    Vec2::splat(layout.ability_radii[slot] * 2.0),
                    visible,
                )
            }
            MobileVisual::RankRing(slot) => {
                look_edge = Some(if mobile.upgrade_mode && mobile.upgrade_enabled[slot] {
                    color::GOLD_400
                } else {
                    Color::NONE
                });
                (
                    layout.ability_centers[slot],
                    Vec2::splat(layout.ability_radii[slot] * 2.0),
                    visible && mobile.upgrade_enabled[slot],
                )
            }
            MobileVisual::UpgradeBadge(slot) => (
                mobile.upgrade_badge_center(slot),
                Vec2::splat(crate::ui::widgets::game::ABILITY_UPGRADE),
                visible && !dead && mobile.upgrade_enabled[slot],
            ),
            MobileVisual::UpgradeMode => {
                label = if mobile.upgrade_mode {
                    tr("touch.rank.back").into()
                } else {
                    trf("touch.rank.points", &[("points", &prog.skill_points)])
                };
                look_fill = Some(if mobile.upgrade_mode {
                    color::EMERALD_300
                } else {
                    color::EMERALD_400
                });
                (
                    layout.upgrade_center,
                    Vec2::splat(layout.upgrade_radius * 2.0),
                    visible && mobile.upgrade_enabled.iter().any(|enabled| *enabled),
                )
            }
            MobileVisual::CategoryAttack(index) => {
                label = tr(["touch.target.minion", "touch.target.tower"][index]).into();
                (
                    layout.category_centers[index],
                    Vec2::splat(36.0 * s),
                    visible,
                )
            }
            MobileVisual::Utility(index) => {
                let remaining = [utility.dash_remaining_secs, utility.haste_remaining_secs][index];
                let total = [
                    shared::utility::DASH_COOLDOWN_SECS,
                    shared::utility::HASTE_COOLDOWN_SECS,
                ][index];
                let active = index == 1 && utility.haste_active_secs > 0.0;
                label = if active {
                    trf(
                        "touch.haste.active",
                        &[("seconds", &format!("{:.1}", utility.haste_active_secs))],
                    )
                } else {
                    tr(["touch.dash", "touch.haste"][index]).into()
                };
                if remaining > 0.0 && !active {
                    cooldown_state = Some((remaining, total.max(remaining)));
                }
                look_fill = Some(if active {
                    color::EMERALD_600
                } else {
                    color::EMERALD_800
                });
                (
                    layout.utility_centers[index],
                    Vec2::splat(layout.auxiliary_radius * 2.0),
                    visible,
                )
            }
            // Directional gestures communicate through vectors. Error feedback remains
            // in the skill feedback layer; never add a tutorial box over combat.
            MobileVisual::AimHint => (Vec2::ZERO, Vec2::ZERO, false),
            MobileVisual::SkillDescription => {
                let slot = inspected;
                let Some(mut card) = card else { continue };
                if let Some(slot) = slot {
                    let rank = prog.ranks[slot].max(1);
                    let duration = if sandbox.is_some_and(|s| s.config.player.no_cooldowns) {
                        0.0
                    } else {
                        crate::combat::effective_cast_duration(
                            class,
                            prog.level,
                            rank,
                            SkillSlot::ALL[slot],
                            bonuses,
                            sandbox.is_some(),
                        )
                    };
                    let mut next = crate::combat::skill_card::SkillCardView::of(
                        class, &prog, slot, mana, duration,
                    );
                    if cooldown.recast[slot] {
                        next.mana = 0;
                        next.no_mana = false;
                    }
                    next.hint = true;
                    next.visible = visible;
                    if *card != next {
                        *card = next;
                    }
                } else if card.visible {
                    card.visible = false;
                }
                let rect = crate::hud_layout::HudLayout::phone(&mobile).skill_card;
                // The card owns its size and display (`paint_skill_card`).
                let left = Val::Px(rect.min.x);
                let top = Val::Px(rect.min.y);
                if node.left != left || node.top != top {
                    node.left = left;
                    node.top = top;
                }
                continue;
            }
            MobileVisual::Rotate => {
                label = tr("touch.rotate").into();
                (
                    mobile.viewport * 0.5,
                    mobile.viewport,
                    mobile.enabled && !mobile.landscape,
                )
            }
        };
        let display = if show { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        if !show {
            continue;
        }
        match visual {
            // The aim hint hugs its text, centred on the slot's top edge.
            MobileVisual::AimHint => {
                let left = Val::Px(center.x);
                let top = Val::Px(center.y);
                if node.left != left || node.top != top {
                    node.left = left;
                    node.top = top;
                }
                let shift = Val2::new(Val::Percent(-50.0), Val::Px(0.0));
                if transform.translation != shift {
                    transform.translation = shift;
                }
            }
            _ => place(&mut node, center, size),
        }
        let rotation = if matches!(
            visual,
            MobileVisual::AttackVector | MobileVisual::DashVector
        ) {
            let delta = if matches!(visual, MobileVisual::DashVector) {
                dash_drag
            } else {
                drag
            }
            .unwrap_or_default();
            Rot2::radians(delta.y.atan2(delta.x))
        } else {
            Rot2::IDENTITY
        };
        if transform.rotation != rotation {
            transform.rotation = rotation;
        }
        let round = !matches!(
            visual,
            MobileVisual::AimHint
                | MobileVisual::Rotate
                | MobileVisual::AttackVector
                | MobileVisual::DashVector
        );
        if round {
            let corner = BorderRadius::all(Val::Percent(50.0));
            if node.border_radius != corner {
                node.border_radius = corner;
            }
        }
        // Dead: ATK and the utilities are veiled like the abilities.
        let veiled = dead && disc_look(visual).is_some();
        let look_fill = if veiled {
            Some(color::SURFACE_3)
        } else {
            look_fill
        };
        let look_edge = if veiled {
            Some(color::BORDER_DISABLED)
        } else {
            look_edge
        };
        if let (Some(next), Some(mut fill)) = (look_fill, fill) {
            if fill.0 != next {
                fill.0 = next;
            }
        }
        if let (Some(next), Some(mut edge)) = (look_edge, edge) {
            let next = BorderColor::all(next);
            if *edge != next {
                *edge = next;
            }
        }
        let Some(children) = children else { continue };
        for child in children.iter() {
            let Ok((part, mut visibility, image, grandchildren, _)) = parts.get_mut(child) else {
                if let Ok((mut text, _)) = texts.get_mut(child) {
                    if text.0 != label {
                        text.0.clone_from(&label);
                    }
                }
                continue;
            };
            let part = *part;
            match part {
                DiscPart::Sweep => {
                    let next = if cooldown_state.is_some() {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    };
                    if *visibility != next {
                        *visibility = next;
                    }
                    if let (Some(mut image), Some((remaining, total))) = (image, cooldown_state) {
                        let frame =
                            Some(crate::ui::widgets::game::cooldown_frame(remaining, total));
                        if image.frame != frame {
                            image.frame = frame;
                        }
                    }
                }
                DiscPart::Seconds => {
                    let next = if cooldown_state.is_some() {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    };
                    if *visibility != next {
                        *visibility = next;
                    }
                    if let (Some(grandchildren), Some((remaining, _))) =
                        (grandchildren, cooldown_state)
                    {
                        for text_entity in grandchildren.iter() {
                            if let Ok((mut text, _)) = texts.get_mut(text_entity) {
                                let next = crate::ui::widgets::game::cooldown_text(remaining);
                                if text.0 != next {
                                    text.0 = next;
                                }
                            }
                        }
                    }
                }
                DiscPart::Icon | DiscPart::Label => {
                    // Icon and label step aside while the sweep shows seconds.
                    let next = if cooldown_state.is_some() {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    };
                    if *visibility != next {
                        *visibility = next;
                    }
                }
            }
            if part == DiscPart::Label {
                if let Ok((mut text, _)) = texts.get_mut(child) {
                    if text.0 != label {
                        text.0.clone_from(&label);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ipad_upgrade_badges_capture_taps_without_casting_or_moving() {
        for slot in 0..4 {
            for offset in [Vec2::ZERO, Vec2::X * 21.0, Vec2::NEG_Y * 21.0] {
                let mut m = controls();
                m.viewport = Vec2::new(1180.0, 820.0);
                m.upgrade_enabled = [true; 4];
                let point = m.upgrade_badge_center(slot) + offset;
                // The map's world-touch gate uses this same ownership predicate.
                assert!(m.owns_control_point(point));
                m.event(1, TouchPhase::Started, point);
                assert_eq!(m.captures[&1].control, Control::Upgrade(slot));
                m.event(1, TouchPhase::Ended, point);
                assert_eq!(m.upgrades, [slot]);
                assert!(m.casts.is_empty() && m.attacks.is_empty());
                assert_eq!(m.movement, Vec2::ZERO);
                assert!(!m.has_active_gesture());
            }
        }
    }

    #[test]
    fn upgrade_badges_cancel_and_do_not_steal_the_joystick_or_another_finger() {
        let mut m = controls();
        m.viewport = Vec2::new(1180.0, 820.0);
        m.upgrade_enabled = [true; 4];
        let l = m.layout();
        m.event(9, TouchPhase::Started, l.joystick_center);
        m.event(
            9,
            TouchPhase::Moved,
            l.joystick_center + Vec2::X * l.joystick_radius,
        );
        let point = m.upgrade_badge_center(0);
        m.event(1, TouchPhase::Started, point);
        m.event(2, TouchPhase::Started, point);
        m.event(2, TouchPhase::Ended, point);
        assert!(m.upgrades.is_empty());
        m.event(1, TouchPhase::Canceled, point);
        assert!(m.upgrades.is_empty());
        m.event(1, TouchPhase::Started, point);
        m.event(1, TouchPhase::Ended, Vec2::new(500.0, 300.0));
        assert!(m.upgrades.is_empty());
        m.event(1, TouchPhase::Started, point);
        m.upgrade_enabled[0] = false; // Authoritative points/rank changed mid-touch.
        m.event(1, TouchPhase::Ended, point);
        assert!(m.upgrades.is_empty() && m.casts.is_empty() && m.attacks.is_empty());
        assert_eq!(m.movement, Vec2::X);
    }

    #[test]
    fn gesture_overlay_stays_above_skills_and_upgrade_badges() {
        let mut app = App::new();
        app.add_systems(Startup, setup_mobile_controls);
        app.update();
        let mut nodes = app.world_mut().query::<(&MobileVisual, &ZIndex, &Node)>();
        let control_z = nodes
            .iter(app.world())
            .filter_map(|(visual, z, _)| {
                matches!(
                    visual,
                    MobileVisual::Ability(_)
                        | MobileVisual::RankRing(_)
                        | MobileVisual::UpgradeBadge(_)
                        | MobileVisual::Attack
                        | MobileVisual::CategoryAttack(_)
                        | MobileVisual::Utility(_)
                        | MobileVisual::Cancel
                        | MobileVisual::Recall
                )
                .then_some(z.0)
            })
            .max()
            .unwrap();
        let mut overlays = 0;
        for (visual, z, node) in nodes.iter(app.world()) {
            if matches!(
                visual,
                MobileVisual::AttackVector
                    | MobileVisual::DashVector
                    | MobileVisual::AttackThumb
                    | MobileVisual::DashThumb
            ) {
                assert!(
                    z.0 > control_z,
                    "finger preview must stay visible over another control"
                );
                if matches!(visual, MobileVisual::AttackThumb | MobileVisual::DashThumb) {
                    assert_eq!(node.border, UiRect::all(Val::Px(2.0)));
                }
                overlays += 1;
            }
        }
        assert_eq!(overlays, 4);
    }

    #[test]
    fn directional_gestures_never_show_instruction_boxes() {
        let mut app = App::new();
        let mut m = controls();
        m.viewport = Vec2::new(1180.0, 820.0);
        m.event(1, TouchPhase::Started, m.layout().attack_center);
        app.insert_resource(m)
            .init_resource::<GameplayInputContext>()
            .init_resource::<TeamSelection>()
            .init_resource::<LocalCastCooldown>()
            .add_systems(Update, draw_mobile_controls);
        app.world_mut().spawn((Player, CombatStats::default()));
        let hint = app
            .world_mut()
            .spawn((
                MobileVisual::AimHint,
                Node::default(),
                UiTransform::default(),
            ))
            .id();
        let cancel = app
            .world_mut()
            .spawn((
                MobileVisual::Cancel,
                Node::default(),
                UiTransform::default(),
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<Node>(hint).unwrap().display,
            Display::None
        );
        assert_eq!(
            app.world().get::<Node>(cancel).unwrap().display,
            Display::Flex
        );
        let mut m = app.world_mut().resource_mut::<MobileControls>();
        m.clear();
        let p = m.layout().ability_centers[0];
        m.event(2, TouchPhase::Started, p);
        app.update();
        assert_eq!(
            app.world().get::<Node>(hint).unwrap().display,
            Display::None
        );
    }

    /// The hold card is the shared skill card (`skill-description.md`;
    /// `touch.skill.card` was one composed string): its cooldown follows
    /// level, items and the sandbox exactly as before.
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
            let card = app
                .world_mut()
                .spawn((
                    MobileVisual::SkillDescription,
                    Node::default(),
                    UiTransform::default(),
                    crate::combat::skill_card::SkillCardView::default(),
                ))
                .id();
            app.update();
            let view = app
                .world()
                .get::<crate::combat::skill_card::SkillCardView>(card)
                .unwrap();
            assert!(view.visible && view.hint && view.status.is_none());
            assert_eq!(view.slot, slot);
            let text = crate::combat::skill_card::cooldown_line(view.cooldown);
            assert!(text.contains(expected), "expected {expected}, got {text}");
        }
    }

    #[test]
    fn an_owning_controller_hides_the_touch_hud_and_giving_input_back_restores_it() {
        let mut app = App::new();
        app.insert_resource(controls())
            .init_resource::<GameplayInputContext>()
            .init_resource::<TeamSelection>()
            .init_resource::<LocalCastCooldown>()
            .init_resource::<crate::gamepad::GamepadControls>()
            .add_systems(Update, draw_mobile_controls);
        app.world_mut()
            .spawn((Player, CombatStats::default(), PlayerProgression::default()));
        let joystick = app
            .world_mut()
            .spawn((
                MobileVisual::Joystick,
                Node::default(),
                BackgroundColor::default(),
                BorderColor::default(),
                UiTransform::default(),
            ))
            .id();
        let label = app
            .world_mut()
            .spawn((Text::default(), TextFont::default()))
            .id();
        app.world_mut().entity_mut(joystick).add_child(label);
        let shown = |app: &App| app.world().get::<Node>(joystick).unwrap().display != Display::None;
        app.update();
        assert!(shown(&app), "touch owns input");
        app.world_mut()
            .resource_mut::<crate::gamepad::GamepadControls>()
            .active = true;
        app.update();
        assert!(!shown(&app), "the controller owns input");
        app.world_mut()
            .resource_mut::<crate::gamepad::GamepadControls>()
            .active = false;
        app.update();
        assert!(shown(&app), "a touch took input back");
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
        m.advance_hold_time(1.49);
        assert_eq!(m.inspected_skill(), None, "short holds must not inspect");
        m.advance_hold_time(0.02);
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
        m.advance_hold_time(SKILL_DESCRIPTION_SECONDS + 1.0);
        assert_eq!(m.inspected_skill(), None);
        m.event(1, TouchPhase::Ended, skill + Vec2::NEG_X * 50.0);
        assert_eq!(
            m.casts,
            [MobileCastIntent {
                slot: 0,
                extent: (50.0 / (ATTACK_DRAG_REACH * m.combat_scale())).clamp(0.15, 1.0),
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
            assert_eq!(
                m.casts[slot],
                MobileCastIntent {
                    slot,
                    aim: None,
                    extent: 0.15
                }
            );
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
        assert_eq!(
            m.casts,
            [MobileCastIntent {
                slot: 0,
                aim: None,
                extent: 0.15
            }]
        );
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
                extent: (60.0 / (ATTACK_DRAG_REACH * m.combat_scale())).clamp(0.15, 1.0),
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
    fn hud_position_settings_preserve_skill_shape_and_clamp_to_safe_edges() {
        let baseline = controls().layout();
        let mut m = controls();
        m.hud_position = HudPositionSettings {
            joystick_offset: Vec2::splat(60.0),
            combat_offset: Vec2::new(-60.0, 60.0),
        };
        let adjusted = m.layout();
        let shift = adjusted.attack_center - baseline.attack_center;
        for slot in 0..4 {
            assert!(
                (adjusted.ability_centers[slot] - baseline.ability_centers[slot] - shift).length()
                    < 0.001
            );
        }
        assert!(
            adjusted.joystick_center.y + adjusted.joystick_radius
                <= m.viewport.y - m.safe.bottom + 0.001
        );
        for center in adjusted.category_centers {
            assert!(center.y + adjusted.category_radius <= m.viewport.y - m.safe.bottom + 0.001);
            assert!(center.x + adjusted.category_radius <= m.viewport.x - m.safe.right + 0.001);
        }
        assert_eq!(
            HudPositionSettings {
                joystick_offset: Vec2::splat(f32::NAN),
                combat_offset: Vec2::splat(1e6)
            }
            .sanitized(),
            HudPositionSettings {
                joystick_offset: Vec2::ZERO,
                combat_offset: Vec2::splat(60.0)
            }
        );
    }

    #[test]
    fn dash_drag_previews_direction_and_cancel_never_fires() {
        let mut m = controls();
        let center = m.layout().utility_centers[0];
        m.event(1, TouchPhase::Started, center);
        m.event(1, TouchPhase::Moved, center + Vec2::NEG_Y * 70.0);
        assert_eq!(m.dash_aim(), Some(Vec2::NEG_Y));
        m.event(1, TouchPhase::Ended, center + Vec2::NEG_Y * 70.0);
        assert_eq!(m.utilities, [(UtilityAction::Dash, Some(Vec2::NEG_Y))]);
        m.utilities.clear();
        for end in [center + Vec2::X * 220.0, m.layout().cancel_center] {
            m.event(2, TouchPhase::Started, center);
            m.event(2, TouchPhase::Moved, end);
            assert!(m.dash_aim().is_none());
            m.event(2, TouchPhase::Ended, end);
            assert!(m.utilities.is_empty());
        }
    }

    #[test]
    fn recall_tap_starts_or_cancels_without_stealing_other_controls() {
        let mut m = controls();
        let center = m.layout().recall_center;
        for (active, expected) in [
            (false, UtilityAction::Recall),
            (true, UtilityAction::CancelRecall),
        ] {
            m.recall_active = active;
            m.event(1, TouchPhase::Started, center);
            m.event(1, TouchPhase::Ended, center);
            assert_eq!(m.utilities.pop(), Some((expected, None)));
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
                    l.utility_centers
                        .iter()
                        .copied()
                        .map(|p| (p, l.auxiliary_radius))
                        .chain(
                            l.category_centers
                                .iter()
                                .copied()
                                .map(|p| (p, 18.0 * m.combat_scale())),
                        ),
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
                assert!(*radius * 2.0 >= 36.0);
                assert!(l.category_radius * 2.0 >= 44.0);
                // hud.md places the joystick at safe bottom − 65 with a
                // 1.3 × r capture circle, 2.6 px past the safe bottom: the
                // visible base stays inside the safe area, the capture
                // circle inside the viewport.
                let visible = if *center == l.joystick_center {
                    l.joystick_radius
                } else {
                    *radius
                };
                let within = |r: f32, left: f32, top: f32, right: f32, bottom: f32| {
                    center.x - r >= left - 0.01
                        && center.y - r >= top - 0.01
                        && center.x + r <= right + 0.01
                        && center.y + r <= bottom + 0.01
                };
                assert!(
                    within(*radius, 0.0, 0.0, viewport.x, viewport.y),
                    "{viewport:?}: {center:?} leaves the viewport"
                );
                assert!(
                    within(
                        visible,
                        m.safe.left,
                        m.safe.top,
                        viewport.x - m.safe.right,
                        viewport.y - m.safe.bottom
                    ),
                    "{viewport:?}: {center:?} leaves the safe area"
                );
                for (other, other_radius) in &circles[i + 1..] {
                    assert!(
                        center.distance(*other) + 0.01 >= radius + other_radius + 3.5,
                        "overlap at {viewport:?}: {center:?}, {other:?}"
                    );
                }
            }
        }
        // hud.md at the runtime insets 32/32/12/20 (844 × 390): joystick at
        // safe left + 68, safe bottom − 65; ATK at right −124, bottom −76.
        let l = controls().layout();
        assert!(l.joystick_center.distance(Vec2::new(100.0, 305.0)) < 0.01);
        assert!(l.attack_center.distance(Vec2::new(688.0, 294.0)) < 0.01);
        let dash = Vec2::new(688.0, 294.0) + 168.0 * Vec2::from_angle(166f32.to_radians());
        assert!(l.utility_centers[0].distance(dash) < 0.01);
    }

    /// hud-phone redline at the reference safe area (47/47/0/21, 844 × 390):
    /// Skill heights stay within ±2px of the original layout; the entire
    /// combat group moves 48px left so category targets extend its R104 arc.
    /// ATK 96 / abilities 64 / utilities 48 retain their original dimensions,
    /// and non-category controls retain at least 8px between rims.
    #[test]
    fn combat_group_preserves_skill_heights_and_continues_categories_on_the_same_arc() {
        let mut m = controls();
        m.safe = MobileSafeInsets {
            left: 47.0,
            right: 47.0,
            top: 0.0,
            bottom: 21.0,
        };
        let l = m.layout();
        let near = |a: Vec2, x: f32, y: f32| a.distance(Vec2::new(x, y)) <= 2.0;
        assert!(near(l.attack_center, 673.0, 293.0));
        assert!(near(l.joystick_center, 115.0, 304.0));
        for (center, (x, y)) in l.ability_centers.into_iter().zip([
            (574.0, 325.0),
            (578.0, 251.0),
            (631.0, 198.0),
            (705.0, 194.0),
        ]) {
            assert!(near(center, x, y), "{center:?} vs ({x}, {y})");
        }
        assert!(near(l.utility_centers[0], 510.0, 334.0), "DASH");
        assert!(near(l.utility_centers[1], 507.0, 270.0), "HASTE");
        assert!(near(l.cancel_center, 528.0, 209.0), "CANCEL");
        assert!(near(l.upgrade_center, 570.0, 161.0), "RANK");
        assert!(near(l.category_centers[0], 774.73, 314.62), "MIN");
        assert!(near(l.category_centers[1], 763.07, 241.0), "TWR");
        assert_eq!(l.attack_radius * 2.0, 96.0);
        assert_eq!(l.ability_radii, [32.0; 4]);
        assert_eq!(l.auxiliary_radius * 2.0, 48.0);
        assert_eq!(l.joystick_radius * 2.0, 104.0);
        for viewport in [
            Vec2::new(568.0, 320.0),
            Vec2::new(693.0, 320.0),
            Vec2::new(844.0, 390.0),
            Vec2::new(852.0, 393.0),
            Vec2::new(932.0, 430.0),
            Vec2::new(1280.0, 720.0),
        ] {
            let m = MobileControls {
                viewport,
                ..controls()
            };
            let l = m.layout();
            let s = m.combat_scale();
            for center in l.ability_centers {
                assert!((center.distance(l.attack_center) - 104.0 * s).abs() < 0.01);
            }
            let outer: Vec<Vec2> = l
                .utility_centers
                .into_iter()
                .chain([l.recall_center, l.upgrade_center, l.cancel_center])
                .collect();
            for (index, center) in outer.iter().enumerate() {
                let orbit = if m.compact_utilities() && index < 2 {
                    [176.0, 150.0][index]
                } else {
                    168.0
                };
                assert!((center.distance(l.attack_center) - orbit * s).abs() < 0.01);
            }
            let discs: Vec<(Vec2, f32)> = l
                .ability_centers
                .into_iter()
                .zip(l.ability_radii)
                .chain(outer.iter().map(|center| (*center, l.auxiliary_radius)))
                .chain([(l.attack_center, l.attack_radius)])
                .collect();
            for (i, (a, ra)) in discs.iter().enumerate() {
                for (b, rb) in &discs[i + 1..] {
                    let gap = if m.compact_utilities()
                        && *a == l.utility_centers[0]
                        && *b == l.utility_centers[1]
                    {
                        5.0
                    } else {
                        8.0
                    };
                    assert!(
                        a.distance(*b) - ra - rb >= gap * s - 0.01,
                        "rims closer than {gap}px at {viewport:?}: {a:?} {b:?}"
                    );
                }
            }
            let [minion, tower] = l.category_centers;
            assert!(minion.y > l.attack_center.y && tower.y < l.attack_center.y);
            let sequence: Vec<_> = l
                .ability_centers
                .into_iter()
                .chain([tower, minion])
                .collect();
            for (center, angle) in sequence.iter().zip([162_f32, 204., 246., 288., 330., 372.]) {
                let offset = *center - l.attack_center;
                assert!((offset.length() - 104.0 * s).abs() < 0.01);
                assert!(offset.distance(Vec2::from_angle(angle.to_radians()) * 104.0 * s) < 0.01);
            }
            let step = 2.0 * 104.0 * s * 21_f32.to_radians().sin();
            for pair in sequence.windows(2) {
                assert!((pair[0].distance(pair[1]) - step).abs() < 0.01);
            }
            // The shared arc only moves left from the original anchor.
            let original_attack = Vec2::new(
                viewport.x - m.safe.right - 76.0 * s,
                viewport.y - m.safe.bottom - 76.0 * s,
            );
            assert!(
                (l.attack_center - original_attack - Vec2::new(-48.0 * s, 0.0)).length() < 0.01
            );
            // The full 44px targets fit the safe corner at the amended
            // anchor. Check ownership around every target's perimeter,
            // including the sides closest to ATK and the upper-right skill.
            for (center, kind) in l
                .category_centers
                .into_iter()
                .zip([TargetKind::Minion, TargetKind::Structure])
            {
                assert!(center.x + l.category_radius <= viewport.x - m.safe.right + 0.001);
                assert!(center.y + l.category_radius <= viewport.y - m.safe.bottom + 0.001);
                for (other, radius) in &discs {
                    assert!(center.distance(*other) >= l.category_radius + radius - 0.01);
                }
                for step in 0..16 {
                    let edge = center
                        + Vec2::from_angle(step as f32 * std::f32::consts::TAU / 16.0)
                            * (l.category_radius - 0.01);
                    assert!(
                        matches!(m.hit_control(edge), Some((Control::CategoryAttack(actual), _)) if actual == kind),
                        "category hit stolen at {viewport:?}: {edge:?}"
                    );
                }
            }
            assert!(minion.distance(tower) >= l.category_radius * 2.0);
            // Geometry used by drawing and input is the same at every size.
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
        // `size.ability.utility.phone` (44 before the hud.md sizes).
        assert_eq!(m.layout().auxiliary_radius * 2.0, 48.0);
        let compact = m.layout();
        assert!((compact.utility_centers[0].distance(compact.attack_center) - 176.0).abs() < 0.01);
        assert!(
            (compact.utility_centers[1]
                - compact.attack_center
                - Vec2::from_angle(182.6_f32.to_radians()) * 150.0)
                .length()
                < 0.01
        );
        // The compact HASTE rect sits wholly below the protected hero box,
        // also with the phone reference's 21px bottom safe area.
        assert!(
            compact.utility_centers[1].y - compact.auxiliary_radius >= m.viewport.y * 0.60 + 1.0
        );
        let normal = MobileControls {
            viewport: Vec2::new(852.0, 393.0),
            ..controls()
        };
        let normal_layout = normal.layout();
        for (center, angle) in normal_layout
            .utility_centers
            .into_iter()
            .zip([166_f32, 188.])
        {
            assert!(
                (center
                    - normal_layout.attack_center
                    - Vec2::from_angle(angle.to_radians()) * 168.0 * normal.combat_scale())
                .length()
                    < 0.01
            );
        }
        let narrow = MobileControls {
            viewport: Vec2::new(568.0, 430.0),
            ..controls()
        };
        let layout = narrow.layout();
        let opening = layout
            .utility_centers
            .iter()
            .map(|center| center.x)
            .fold(f32::INFINITY, f32::min)
            - layout.auxiliary_radius
            - layout.joystick_center.x
            - layout.joystick_radius * 1.3;
        assert!(opening >= 48.0 - 0.001);
        // Even an unusually wide pair of landscape safe insets on a 568px
        // viewport retains separate 44px targets at the minimum scale.
        let extra_safe = MobileControls {
            safe: MobileSafeInsets {
                left: 47.0,
                right: 47.0,
                ..narrow.safe
            },
            ..narrow
        };
        let layout = extra_safe.layout();
        assert_eq!(extra_safe.combat_scale(), 1.0);
        let opening = layout
            .utility_centers
            .iter()
            .map(|center| center.x)
            .fold(f32::INFINITY, f32::min)
            - layout.auxiliary_radius
            - layout.joystick_center.x
            - layout.joystick_radius * JOYSTICK_CAPTURE;
        assert!(opening >= 19.5);
        for center in layout.category_centers {
            assert!(
                center.x + layout.category_radius
                    <= extra_safe.viewport.x - extra_safe.safe.right + 0.001
            );
        }
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
    fn unavailable_utilities_do_not_own_invisible_touch_regions() {
        let mobile = MobileControls {
            utilities_available: false,
            ..default()
        };
        for point in mobile.layout().utility_centers {
            assert!(!matches!(
                mobile.hit_control(point),
                Some((Control::Utility(_), _))
            ));
        }
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
            l.category_centers[1],
            l.utility_centers[0],
        ] {
            let edge = center + Vec2::X * 21.0;
            m.event(1, TouchPhase::Started, edge);
            m.event(1, TouchPhase::Ended, edge);
        }
        assert_eq!(
            m.category_attacks,
            [TargetKind::Minion, TargetKind::Structure]
        );
        assert_eq!(m.utilities, [(UtilityAction::Dash, None)]);
        m.event(1, TouchPhase::Started, l.upgrade_center + Vec2::X * 21.0);
        m.event(1, TouchPhase::Ended, l.upgrade_center + Vec2::X * 21.0);
        assert!(m.upgrade_mode);
    }

    /// hud.md phone § Level-up: the + badge sits on every upgradable
    /// ability (a direct upgrade control) and the gold rim only in rank mode
    /// (the procedural rank-capacity ring of 0.26 is gone: no pips on phone).
    #[test]
    fn rank_rings_badge_upgradable_abilities_and_rim_them_in_rank_mode() {
        let mut app = App::new();
        let mut mobile = controls();
        mobile.upgrade_enabled = [true, false, true, false];
        app.insert_resource(mobile)
            .init_resource::<GameplayInputContext>()
            .init_resource::<TeamSelection>()
            .init_resource::<LocalCastCooldown>()
            .add_systems(Update, draw_mobile_controls);
        app.world_mut()
            .spawn((Player, CombatStats::default(), PlayerProgression::default()));
        let rings: Vec<Entity> = (0..4)
            .map(|slot| {
                app.world_mut()
                    .spawn((
                        MobileVisual::RankRing(slot),
                        Node::default(),
                        UiTransform::default(),
                        BackgroundColor::default(),
                        BorderColor::default(),
                    ))
                    .id()
            })
            .collect();
        app.update();
        let shown = |app: &App, slot: usize| {
            app.world().get::<Node>(rings[slot]).unwrap().display != Display::None
        };
        let rim = |app: &App, slot: usize| app.world().get::<BorderColor>(rings[slot]).unwrap().top;
        assert!(shown(&app, 0) && !shown(&app, 1) && shown(&app, 2) && !shown(&app, 3));
        assert_eq!(rim(&app, 0), Color::NONE, "a hint outside rank mode");
        app.world_mut()
            .resource_mut::<MobileControls>()
            .upgrade_mode = true;
        app.update();
        assert_eq!(rim(&app, 0), crate::ui::tokens::color::GOLD_400);
        let layout = app.world().resource::<MobileControls>().layout();
        let node = app.world().get::<Node>(rings[2]).unwrap();
        assert_eq!(node.width, Val::Px(layout.ability_radii[2] * 2.0));
    }
}
