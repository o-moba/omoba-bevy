//! Directional focus for kit buttons, driven by any source that is not a
//! pointer (today a gamepad's D-pad, left stick and South button).
//!
//! A driver sets [`UiFocus::set_enabled`] every frame it owns the input and
//! writes [`FocusNav`] messages. [`navigate_focus`] (in `UiSet::Focus`,
//! before the tap recognizer) keeps one focused button among the
//! candidates: kit [`Pressable`]s that are visible, measured, not disabled
//! and allowed by the modal stack (the top modal's buttons, or the whole
//! screen when no modal is open). Directions move to the nearest button that
//! way, preferring the same row or column, and stay put at an edge.
//! `Confirm` writes [`SyntheticPress`] for the focused button, so activation
//! goes through the same gate as a click or a tap. When the set of
//! candidates changes (a modal opened, a page changed, a section hid) or a
//! modal bumps [`GestureEpoch`], focus returns to the first button and the
//! next confirm must be a fresh press. A focused button inside a
//! [`ScrollArea`] scrolls into view. The ring is drawn by
//! [`super::widgets::paint_focus_ring`].
use bevy::prelude::*;

use super::{GestureEpoch, Pressable, ScrollArea, SyntheticPress, modal::ModalGate};

/// One navigation step from a focus driver.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FocusNav {
    Up,
    Down,
    Left,
    Right,
    Confirm,
}

impl FocusNav {
    fn direction(self) -> Option<Vec2> {
        match self {
            // UI space: +Y is down.
            FocusNav::Up => Some(Vec2::NEG_Y),
            FocusNav::Down => Some(Vec2::Y),
            FocusNav::Left => Some(Vec2::NEG_X),
            FocusNav::Right => Some(Vec2::X),
            FocusNav::Confirm => None,
        }
    }
}

/// A focusable control that takes Left/Right itself (slider, cycle row):
/// while it is focused those steps become [`FocusAdjust`] instead of moving
/// the focus.
#[derive(Component, Clone, Copy, Default)]
pub(crate) struct FocusAdjustable;

/// A pressable that pointer and touch can hit but focus skips (a cycle
/// row's chevrons: the row itself is the one focusable control).
#[derive(Component, Clone, Copy, Default)]
pub(crate) struct FocusSkip;

/// Left (-1) or Right (+1) on a focused [`FocusAdjustable`].
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FocusAdjust {
    pub entity: Entity,
    pub step: i8,
}

/// How a control takes the focus when its surface appears (the default is
/// the first candidate, top-to-bottom then left-to-right).
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FocusEntry {
    /// Focused first when the surface appears (a Retry that just appeared).
    Preferred,
    /// Never focused by the surface appearing; the first direction focuses
    /// it (a Cancel on a shared countdown: a stray confirm must not press it).
    Deferred,
}

/// The focused kit button, if a focus driver is active.
#[derive(Resource, Default, Debug)]
pub(crate) struct UiFocus {
    enabled: bool,
    focused: Option<Entity>,
    surface: Vec<Entity>,
    confirm_ready: bool,
    epoch: Option<u64>,
    revealed: Option<Entity>,
}

/// A focusable button and its rectangle in physical window pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    pub entity: Entity,
    pub rect: Rect,
}

impl UiFocus {
    /// A driver owns the input this frame. Without one the focus is dropped
    /// and no ring is drawn, so mouse and touch players never see it.
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
        }
    }

    pub(crate) fn focused(&self) -> Option<Entity> {
        self.focused.filter(|_| self.enabled)
    }

    fn clear(&mut self) {
        self.focused = None;
        self.surface.clear();
        self.confirm_ready = false;
        self.revealed = None;
    }

    /// [`Self::step_with`] with the default entry for every candidate.
    #[cfg(test)]
    pub(crate) fn step(
        &mut self,
        candidates: &[Candidate],
        nav: impl IntoIterator<Item = FocusNav>,
    ) -> Option<Entity> {
        self.step_with(candidates, nav, |_| None)
    }

    /// One frame of navigation over `candidates` (sorted top-to-bottom, then
    /// left-to-right), with each candidate's [`FocusEntry`] (`None` = the
    /// default). Returns the button a confirm activates.
    pub(crate) fn step_with(
        &mut self,
        candidates: &[Candidate],
        nav: impl IntoIterator<Item = FocusNav>,
        entry: impl Fn(Entity) -> Option<FocusEntry>,
    ) -> Option<Entity> {
        let surface: Vec<Entity> = candidates.iter().map(|c| c.entity).collect();
        if surface != self.surface {
            // A new modal, page or section never inherits a confirm meant for
            // the buttons it replaced.
            self.surface = surface;
            self.focused = candidates
                .iter()
                .find(|c| entry(c.entity) == Some(FocusEntry::Preferred))
                .or_else(|| {
                    candidates
                        .iter()
                        .find(|c| entry(c.entity) != Some(FocusEntry::Deferred))
                })
                .map(|c| c.entity);
            self.confirm_ready = false;
            return None;
        }
        let mut moved = false;
        let mut confirm = false;
        for step in nav {
            match step.direction() {
                Some(direction) => {
                    self.focused = directional_neighbor(candidates, self.focused, direction);
                    moved = true;
                }
                None => confirm = true,
            }
        }
        if !confirm {
            // Any frame without a confirm arms the next one.
            self.confirm_ready = true;
            return None;
        }
        if moved {
            return None;
        }
        if self.confirm_ready {
            self.confirm_ready = false;
            return self.focused;
        }
        None
    }
}

/// The nearest candidate in `direction` from `current`; ties prefer the same
/// row or column (score `along + |across| * 3`). Stays on `current` at an
/// edge; without a current focus, the first candidate.
pub(crate) fn directional_neighbor(
    candidates: &[Candidate],
    current: Option<Entity>,
    direction: Vec2,
) -> Option<Entity> {
    let Some(origin) = candidates.iter().find(|c| Some(c.entity) == current) else {
        return candidates.first().map(|c| c.entity);
    };
    let direction = direction.normalize_or_zero();
    candidates
        .iter()
        .filter(|c| c.entity != origin.entity)
        .filter_map(|c| {
            let delta = c.rect.center() - origin.rect.center();
            let along = delta.dot(direction);
            (along > 1.0).then_some((c, along + delta.perp_dot(direction).abs() * 3.0))
        })
        .min_by(|a, b| {
            a.1.total_cmp(&b.1)
                .then_with(|| a.0.entity.cmp(&b.0.entity))
        })
        .map(|(c, _)| c.entity)
        .or(current)
}

/// Physical-pixel rectangle of a node (unclipped).
pub(crate) fn node_rect(node: &ComputedNode, transform: &UiGlobalTransform) -> Rect {
    Rect::from_center_size(
        transform.translation,
        node.size() * transform.to_scale_angle_translation().0.abs(),
    )
}

type Hierarchy<'a> = (
    Option<&'a Node>,
    Option<&'a Visibility>,
    Option<&'a InheritedVisibility>,
    Option<&'a ChildOf>,
);

/// Walks the ancestors directly: a parent hidden this frame has not
/// propagated its visibility or layout yet.
fn visible_in_hierarchy(entity: Entity, nodes: &Query<Hierarchy>) -> bool {
    let mut current = Some(entity);
    for _ in 0..128 {
        let Some(entity) = current else { return true };
        let Ok((node, visibility, inherited, parent)) = nodes.get(entity) else {
            return false;
        };
        if node.is_some_and(|node| node.display == Display::None)
            || visibility == Some(&Visibility::Hidden)
            || inherited.is_some_and(|inherited| !inherited.get())
        {
            return false;
        }
        current = parent.map(ChildOf::parent);
    }
    false
}

fn scroll_ancestors(
    entity: Entity,
    nodes: &Query<Hierarchy>,
    is_area: impl Fn(Entity) -> bool,
) -> Vec<Entity> {
    let mut found = Vec::new();
    let mut current = nodes
        .get(entity)
        .ok()
        .and_then(|n| n.3)
        .map(ChildOf::parent);
    for _ in 0..128 {
        let Some(entity) = current else { break };
        if is_area(entity) {
            found.push(entity);
        }
        current = nodes
            .get(entity)
            .ok()
            .and_then(|n| n.3)
            .map(ChildOf::parent);
    }
    found
}

/// Offset change (in `ScrollPosition` units) that brings `target` inside
/// `area` vertically; zero when it already is. Rects are physical pixels.
pub(crate) fn reveal_delta(area: Rect, target: Rect, inverse_scale_factor: f32) -> f32 {
    let delta = if target.min.y < area.min.y || target.height() > area.height() {
        target.min.y - area.min.y
    } else if target.max.y > area.max.y {
        target.max.y - area.max.y
    } else {
        0.0
    };
    delta * inverse_scale_factor
}

type ButtonItem<'a> = (
    Entity,
    &'a Pressable,
    &'a ComputedNode,
    &'a UiGlobalTransform,
    Option<&'a bevy::ui::CalculatedClip>,
);

/// Runs in `UiSet::Focus`, after the modal registry synced and before the
/// tap recognizer, which applies the synthetic press this writes.
pub(crate) fn navigate_focus(
    mut focus: ResMut<UiFocus>,
    mut nav: MessageReader<FocusNav>,
    epoch: Option<Res<GestureEpoch>>,
    window: Query<&Window, With<bevy::window::PrimaryWindow>>,
    gate: ModalGate,
    buttons: Query<ButtonItem>,
    nodes: Query<Hierarchy>,
    mut areas: Query<(&ComputedNode, &UiGlobalTransform, &mut ScrollPosition), With<ScrollArea>>,
    mut presses: MessageWriter<SyntheticPress>,
    adjustable: Query<(), With<FocusAdjustable>>,
    skipped: Query<(), With<FocusSkip>>,
    entries: Query<&FocusEntry>,
    mut adjust: MessageWriter<FocusAdjust>,
) {
    let mut steps: Vec<FocusNav> = nav.read().copied().collect();
    if let Some(focused) = focus
        .focused()
        .filter(|entity| adjustable.contains(*entity))
    {
        steps.retain(|step| {
            let horizontal = match step {
                FocusNav::Left => -1,
                FocusNav::Right => 1,
                _ => return true,
            };
            adjust.write(FocusAdjust {
                entity: focused,
                step: horizontal,
            });
            false
        });
    }
    if !focus.enabled {
        if focus.focused.is_some() || !focus.surface.is_empty() {
            focus.clear();
        }
        return;
    }
    let epoch = epoch.map(|epoch| epoch.0);
    if focus.epoch != epoch {
        // A modal navigated: start over on its new page.
        focus.epoch = epoch;
        focus.clear();
    }
    let viewport = window
        .single()
        .ok()
        .map(|window| Rect::from_corners(Vec2::ZERO, window.physical_size().as_vec2()));
    let mut candidates: Vec<Candidate> = buttons
        .iter()
        .filter(|(entity, pressable, node, ..)| {
            !pressable.disabled
                && !skipped.contains(*entity)
                && gate.allows(*entity)
                && node.size().min_element() > 0.0
                && visible_in_hierarchy(*entity, &nodes)
        })
        .filter_map(|(entity, _, node, transform, clip)| {
            let rect = node_rect(node, transform);
            if !(rect.min.is_finite() && rect.max.is_finite()) {
                return None;
            }
            // Clipped out of sight is fine inside a scroll area (focus
            // scrolls it into view); anywhere else it is not a candidate.
            let mut shown = rect;
            if let Some(clip) = clip {
                shown = shown.intersect(clip.clip);
            }
            if let Some(viewport) = viewport {
                shown = shown.intersect(viewport);
            }
            let scrollable = || !scroll_ancestors(entity, &nodes, |e| areas.contains(e)).is_empty();
            (!shown.is_empty() || scrollable()).then_some(Candidate { entity, rect })
        })
        .collect();
    candidates.sort_by(|a, b| {
        a.rect
            .center()
            .y
            .total_cmp(&b.rect.center().y)
            .then_with(|| a.rect.center().x.total_cmp(&b.rect.center().x))
            .then_with(|| a.entity.cmp(&b.entity))
    });
    if let Some(entity) = focus.step_with(&candidates, steps, |entity| {
        entries.get(entity).ok().copied()
    }) {
        presses.write(SyntheticPress(entity));
    }
    let Some(focused) = focus.focused else {
        focus.revealed = None;
        return;
    };
    if focus.revealed == Some(focused) {
        return;
    }
    focus.revealed = Some(focused);
    let Some(target) = candidates
        .iter()
        .find(|c| c.entity == focused)
        .map(|c| c.rect)
    else {
        return;
    };
    for area in scroll_ancestors(focused, &nodes, |e| areas.contains(e)) {
        let Ok((node, transform, mut scroll)) = areas.get_mut(area) else {
            continue;
        };
        let delta = reveal_delta(
            node_rect(node, transform),
            target,
            node.inverse_scale_factor(),
        );
        let next = (scroll.y + delta).clamp(0.0, super::scroll::max_offset(node));
        if (next - scroll.y).abs() > 0.01 {
            scroll.y = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(index: u32, center: Vec2) -> Candidate {
        Candidate {
            entity: Entity::from_raw_u32(index).unwrap(),
            rect: Rect::from_center_size(center, Vec2::splat(20.0)),
        }
    }

    #[test]
    fn spatial_navigation_prefers_rows_and_columns_and_stays_at_edge() {
        let buttons = [
            candidate(1, Vec2::ZERO),
            candidate(2, Vec2::new(40.0, 0.0)),
            candidate(3, Vec2::new(0.0, 40.0)),
            candidate(4, Vec2::new(15.0, 15.0)),
        ];
        let from = |direction| directional_neighbor(&buttons, Some(buttons[0].entity), direction);
        assert_eq!(from(Vec2::X), Some(buttons[1].entity));
        assert_eq!(from(Vec2::Y), Some(buttons[2].entity));
        assert_eq!(from(Vec2::NEG_X), Some(buttons[0].entity), "edge");
        assert_eq!(from(Vec2::NEG_Y), Some(buttons[0].entity), "edge");
        assert_eq!(
            directional_neighbor(&buttons, None, Vec2::X),
            Some(buttons[0].entity)
        );
    }

    #[test]
    fn a_preferred_control_takes_the_new_surface_and_a_deferred_one_waits_for_a_direction() {
        let mut focus = UiFocus {
            enabled: true,
            ..default()
        };
        let cancel = candidate(1, Vec2::ZERO);
        let retry = candidate(2, Vec2::Y * 200.0);
        let entry = |entity: Entity| {
            if entity == cancel.entity {
                Some(FocusEntry::Deferred)
            } else if entity == retry.entity {
                Some(FocusEntry::Preferred)
            } else {
                None
            }
        };
        // Cancel alone: nothing focused, a confirm presses nothing.
        assert_eq!(focus.step_with(&[cancel], [FocusNav::Confirm], entry), None);
        assert_eq!(focus.focused(), None);
        focus.step_with(&[cancel], [], entry);
        assert_eq!(focus.step_with(&[cancel], [FocusNav::Confirm], entry), None);
        // A direction focuses it.
        focus.step_with(&[cancel], [FocusNav::Down], entry);
        assert_eq!(focus.focused(), Some(cancel.entity));
        // Retry appears below Cancel and takes the focus.
        focus.step_with(&[cancel, retry], [], entry);
        assert_eq!(focus.focused(), Some(retry.entity));
    }

    #[test]
    fn changed_surface_refocuses_the_first_button_and_requires_a_fresh_confirm() {
        let mut focus = UiFocus {
            enabled: true,
            ..default()
        };
        let buttons = [candidate(1, Vec2::ZERO), candidate(2, Vec2::X * 40.0)];
        // A confirm arriving with the new surface is not a press.
        assert_eq!(focus.step(&buttons, [FocusNav::Confirm]), None);
        assert_eq!(focus.focused(), Some(buttons[0].entity));
        assert_eq!(focus.step(&buttons, [FocusNav::Confirm]), None, "not fresh");
        assert_eq!(focus.step(&buttons, []), None);
        assert_eq!(
            focus.step(&buttons, [FocusNav::Confirm]),
            Some(buttons[0].entity)
        );
        assert_eq!(focus.step(&buttons, [FocusNav::Confirm]), None, "one press");
        assert_eq!(focus.step(&buttons, [FocusNav::Right]), None);
        assert_eq!(focus.focused(), Some(buttons[1].entity));
        assert_eq!(focus.step(&buttons, [FocusNav::Right]), None);
        assert_eq!(focus.focused(), Some(buttons[1].entity), "edge");
        focus.step(&buttons, []);
        assert_eq!(
            focus.step(&buttons, [FocusNav::Confirm]),
            Some(buttons[1].entity)
        );
        let replaced = [candidate(3, Vec2::ZERO)];
        assert_eq!(focus.step(&replaced, [FocusNav::Confirm]), None);
        assert_eq!(focus.focused(), Some(replaced[0].entity));
        focus.set_enabled(false);
        assert_eq!(focus.focused(), None, "no driver, no focus");
    }

    #[test]
    fn reveal_delta_scrolls_just_enough_either_way() {
        let area = Rect::new(0.0, 100.0, 300.0, 300.0);
        let inside = Rect::new(0.0, 150.0, 100.0, 200.0);
        assert_eq!(reveal_delta(area, inside, 1.0), 0.0);
        let below = Rect::new(0.0, 320.0, 100.0, 360.0);
        assert_eq!(reveal_delta(area, below, 1.0), 60.0);
        assert_eq!(reveal_delta(area, below, 0.5), 30.0, "physical to UI px");
        let above = Rect::new(0.0, 60.0, 100.0, 90.0);
        assert_eq!(reveal_delta(area, above, 1.0), -40.0);
    }

    // --- ECS: the real system through the kit's recognizer and dispatch ---

    use crate::ui::{
        ModalRoot, UiAction, UiActionAppExt, UiSet,
        modal::{ModalAppExt, ModalId},
        test_id::harness::{drain_actions, kit_app},
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Probe {
        A,
        B,
        Modal,
        Disabled,
        Scrolled,
    }

    #[derive(Resource, Default)]
    struct ModalFlag(bool);

    fn focus_app() -> App {
        let mut app = kit_app();
        app.init_resource::<UiFocus>()
            .init_resource::<GestureEpoch>()
            .init_resource::<ModalFlag>()
            .add_message::<FocusNav>()
            .add_message::<FocusAdjust>()
            .add_ui_action::<Probe>()
            .configure_sets(Update, UiSet::Focus.before(UiSet::Gesture))
            .add_systems(Update, navigate_focus.in_set(UiSet::Focus))
            .add_systems(
                Update,
                crate::ui::widgets::paint_focus_ring.in_set(UiSet::Paint),
            )
            .register_modal::<ModalFlag>(ModalId::Pause, |flag| flag.0);
        app.world_mut().resource_mut::<UiFocus>().set_enabled(true);
        app
    }

    fn button(app: &mut App, probe: Probe, center: Vec2, parent: Option<Entity>) -> Entity {
        let entity = app
            .world_mut()
            .spawn((
                Button,
                UiAction(probe),
                Node::default(),
                InheritedVisibility::VISIBLE,
                ComputedNode {
                    size: Vec2::new(60.0, 30.0),
                    inverse_scale_factor: 1.0,
                    ..default()
                },
                UiGlobalTransform::from_translation(center),
            ))
            .id();
        if let Some(parent) = parent {
            app.world_mut().entity_mut(entity).insert(ChildOf(parent));
        }
        entity
    }

    fn nav(app: &mut App, step: FocusNav) {
        app.world_mut().write_message(step);
        app.update();
    }

    fn focused(app: &App) -> Option<Entity> {
        app.world().resource::<UiFocus>().focused()
    }

    #[test]
    fn ecs_confirm_presses_through_the_kit_gate_inside_the_top_modal() {
        let mut app = focus_app();
        let a = button(&mut app, Probe::A, Vec2::new(100.0, 100.0), None);
        let b = button(&mut app, Probe::B, Vec2::new(200.0, 100.0), None);
        app.update();
        assert_eq!(focused(&app), Some(a));
        nav(&mut app, FocusNav::Right);
        assert_eq!(focused(&app), Some(b));
        nav(&mut app, FocusNav::Confirm);
        assert_eq!(drain_actions::<Probe>(app.world_mut()), [Probe::B]);
        app.update();
        assert!(
            drain_actions::<Probe>(app.world_mut()).is_empty(),
            "one frame"
        );
        let ring = |app: &mut App| {
            app.world_mut()
                .query_filtered::<&Node, With<crate::ui::widgets::FocusRing>>()
                .single(app.world())
                .unwrap()
                .clone()
        };
        let drawn = ring(&mut app);
        assert_eq!(
            (drawn.left, drawn.top, drawn.width, drawn.height),
            (Val::Px(170.0), Val::Px(85.0), Val::Px(60.0), Val::Px(30.0)),
            "the ring sits on the focused button"
        );

        // A modal opens: only its buttons are candidates; disabled ones are skipped.
        let root = app
            .world_mut()
            .spawn((
                Node::default(),
                InheritedVisibility::VISIBLE,
                ModalRoot(ModalId::Pause),
            ))
            .id();
        let modal = button(&mut app, Probe::Modal, Vec2::new(100.0, 300.0), Some(root));
        let disabled = button(
            &mut app,
            Probe::Disabled,
            Vec2::new(200.0, 300.0),
            Some(root),
        );
        app.world_mut()
            .get_mut::<Pressable>(disabled)
            .unwrap()
            .disabled = true;
        app.world_mut().resource_mut::<ModalFlag>().0 = true;
        nav(&mut app, FocusNav::Confirm);
        assert_eq!(focused(&app), Some(modal));
        assert!(
            drain_actions::<Probe>(app.world_mut()).is_empty(),
            "fresh confirm"
        );
        nav(&mut app, FocusNav::Right);
        assert_eq!(focused(&app), Some(modal), "disabled is not a neighbour");
        nav(&mut app, FocusNav::Up);
        assert_eq!(
            focused(&app),
            Some(modal),
            "behind the modal is not a neighbour"
        );
        nav(&mut app, FocusNav::Confirm);
        assert_eq!(drain_actions::<Probe>(app.world_mut()), [Probe::Modal]);

        // A parent hidden this frame drops its buttons before layout catches up.
        app.world_mut().get_mut::<Node>(root).unwrap().display = Display::None;
        app.world_mut().resource_mut::<ModalFlag>().0 = false;
        app.update();
        assert_eq!(focused(&app), Some(a));
        // The driver lets go: focus and presses stop.
        app.world_mut().resource_mut::<UiFocus>().set_enabled(false);
        nav(&mut app, FocusNav::Confirm);
        assert_eq!(focused(&app), None);
        assert!(drain_actions::<Probe>(app.world_mut()).is_empty());
        assert_eq!(ring(&mut app).display, Display::None, "no driver, no ring");
    }

    #[test]
    fn ecs_epoch_bump_resets_focus_and_scroll_areas_reveal_the_focused_button() {
        let mut app = focus_app();
        let panel = app
            .world_mut()
            .spawn((
                Node::default(),
                InheritedVisibility::VISIBLE,
                ScrollArea::wheel(32.0),
                ComputedNode {
                    size: Vec2::new(300.0, 100.0),
                    content_size: Vec2::new(300.0, 500.0),
                    inverse_scale_factor: 1.0,
                    ..default()
                },
                UiGlobalTransform::from_translation(Vec2::new(150.0, 50.0)),
            ))
            .id();
        let top = button(&mut app, Probe::A, Vec2::new(150.0, 40.0), Some(panel));
        // Clipped far below the panel's visible 100 px.
        let low = button(
            &mut app,
            Probe::Scrolled,
            Vec2::new(150.0, 260.0),
            Some(panel),
        );
        app.world_mut()
            .entity_mut(low)
            .insert(bevy::ui::CalculatedClip {
                clip: Rect::new(0.0, 0.0, 300.0, 100.0),
            });
        app.update();
        assert_eq!(focused(&app), Some(top));
        nav(&mut app, FocusNav::Down);
        assert_eq!(focused(&app), Some(low), "clipped but scrollable");
        let scroll = app.world().get::<ScrollPosition>(panel).unwrap().y;
        assert_eq!(scroll, 175.0, "bottom edge brought to the panel's bottom");
        app.update();
        nav(&mut app, FocusNav::Confirm);
        assert_eq!(drain_actions::<Probe>(app.world_mut()), [Probe::Scrolled]);

        app.world_mut().resource_mut::<GestureEpoch>().bump();
        nav(&mut app, FocusNav::Confirm);
        assert_eq!(
            focused(&app),
            Some(top),
            "epoch returns to the first button"
        );
        assert!(drain_actions::<Probe>(app.world_mut()).is_empty());
    }
}
