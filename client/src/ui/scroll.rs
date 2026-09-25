//! The one scroll system for overlay panels.
//!
//! Bevy lays a `Overflow::scroll_y()` node out but never moves it. Every
//! scrolling panel carries a [`ScrollArea`] that says which inputs move it
//! (mouse wheel, PageUp/PageDown, touch drag), how far, and on which
//! platform; [`scroll_areas`] (in `UiSet::Scroll`) does the rest. The touch
//! path is the one `mobile_ui` shipped: the first finger that starts on an
//! overflowing area owns it until release, nothing scrolls until the finger
//! travelled past the drag threshold (the tap slop on phone menus, so a drag
//! never also presses a button), then the content follows the finger in
//! window pixels. Offsets are clamped to the content.
use bevy::{
    ecs::message::MessageCursor,
    input::{
        mouse::{MouseScrollUnit, MouseWheel},
        touch::{TouchInput, TouchPhase},
    },
    prelude::*,
    window::PrimaryWindow,
};

use super::gesture::{TAP_SLOP, logical_ui_rect};
use super::modal::ModalGate;

/// Which build an input path is live on. `Phone` also requires the phone HUD
/// to be landscape and focused, like the tap recognizer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum ScrollPlatform {
    #[default]
    All,
    Desktop,
    Phone,
}

/// Mouse wheel and page keys.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WheelScroll {
    /// UI pixels per `MouseScrollUnit::Line` notch.
    pub line_step: f32,
    /// UI pixels per `MouseScrollUnit::Pixel` unit.
    pub pixel_step: f32,
    /// UI pixels per PageUp/PageDown press; 0 leaves the keys alone.
    pub page_step: f32,
    /// Only while the cursor is over the area (several panes side by side).
    pub hover_only: bool,
    pub platform: ScrollPlatform,
}

/// Touch drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DragScroll {
    /// Logical pixels a finger travels before the content moves.
    pub threshold: f32,
    pub platform: ScrollPlatform,
}

/// A scrolling panel. The node also needs `Overflow::scroll_y()`.
#[derive(Component, Clone, Copy, Debug, PartialEq, Default)]
#[require(ScrollPosition)]
pub(crate) struct ScrollArea {
    pub wheel: Option<WheelScroll>,
    pub drag: Option<DragScroll>,
    /// A stable identity for panels that are despawned and rebuilt while a
    /// finger is down (the draft panes): the drag follows the key to the new
    /// entity, and motion made while the rebuilt node is still unmeasured is
    /// applied once layout measured it.
    pub key: Option<u64>,
}

impl ScrollArea {
    /// A touch-only phone panel (help, shop): drag past the tap slop.
    pub(crate) fn phone_panel() -> Self {
        Self {
            drag: Some(DragScroll {
                threshold: TAP_SLOP,
                platform: ScrollPlatform::Phone,
            }),
            ..default()
        }
    }

    /// A modal body: wheel on desktop, drag past the tap slop on a phone
    /// (pause menu sections, career body).
    pub(crate) fn menu(line_step: f32) -> Self {
        Self::phone_panel().desktop_wheel(line_step)
    }

    /// Wheel on every build.
    pub(crate) fn wheel(line_step: f32) -> Self {
        Self::default().with_wheel(line_step, ScrollPlatform::All)
    }

    /// Wheel on desktop builds only.
    pub(crate) fn desktop_wheel(self, line_step: f32) -> Self {
        self.with_wheel(line_step, ScrollPlatform::Desktop)
    }

    fn with_wheel(mut self, line_step: f32, platform: ScrollPlatform) -> Self {
        self.wheel = Some(WheelScroll {
            line_step,
            pixel_step: 1.0,
            page_step: 0.0,
            hover_only: false,
            platform,
        });
        self
    }

    fn wheel_mut(&mut self) -> &mut WheelScroll {
        self.wheel.get_or_insert(WheelScroll {
            line_step: 0.0,
            pixel_step: 1.0,
            page_step: 0.0,
            hover_only: false,
            platform: ScrollPlatform::All,
        })
    }

    /// UI pixels per pixel-unit wheel delta (default 1).
    pub(crate) fn pixel_step(mut self, step: f32) -> Self {
        self.wheel_mut().pixel_step = step;
        self
    }

    pub(crate) fn page_keys(mut self, step: f32) -> Self {
        self.wheel_mut().page_step = step;
        self
    }

    pub(crate) fn hover_only(mut self) -> Self {
        self.wheel_mut().hover_only = true;
        self
    }

    /// Touch drag on every build, starting after `threshold` logical px.
    pub(crate) fn touch_drag(mut self, threshold: f32) -> Self {
        self.drag = Some(DragScroll {
            threshold,
            platform: ScrollPlatform::All,
        });
        self
    }

    pub(crate) fn keyed(mut self, key: u64) -> Self {
        self.key = Some(key);
        self
    }
}

/// Largest offset the content allows, in `ScrollPosition` units.
pub(crate) fn max_offset(node: &ComputedNode) -> f32 {
    ((node.content_size().y - node.size().y) * node.inverse_scale_factor()).max(0.0)
}

struct Held {
    finger: u64,
    area: Entity,
    key: Option<u64>,
    threshold: f32,
    start: Vec2,
    previous: Vec2,
    moved: bool,
    /// Finger travel (logical px) not yet applied to the area.
    pending: f32,
    ended: bool,
}

#[derive(Default)]
pub(crate) struct ScrollState {
    held: Option<Held>,
    wheel: MessageCursor<MouseWheel>,
    touch: MessageCursor<TouchInput>,
}

type AreaItem<'a> = (
    Entity,
    &'a ScrollArea,
    &'a ComputedNode,
    Option<&'a UiGlobalTransform>,
    Option<&'a InheritedVisibility>,
    Option<&'a bevy::ui::CalculatedClip>,
    &'a mut ScrollPosition,
);

/// Runs in `UiSet::Scroll` for every [`ScrollArea`]. An area reacts only
/// while it is visible, measured and allowed by the modal stack (with a modal
/// open, only areas of the top modal scroll).
pub(crate) fn scroll_areas(
    mut state: Local<ScrollState>,
    platform: Option<Res<super::UiPlatform>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    wheel: Option<Res<Messages<MouseWheel>>>,
    touches: Option<Res<Messages<TouchInput>>>,
    window: Query<(Entity, &Window), With<PrimaryWindow>>,
    gate: ModalGate,
    mut areas: Query<AreaItem>,
) {
    let state = &mut *state;
    let wheel: Vec<MouseWheel> = wheel
        .as_ref()
        .map(|events| state.wheel.read(events).copied().collect())
        .unwrap_or_default();
    let touches: Vec<TouchInput> = touches
        .as_ref()
        .map(|events| state.touch.read(events).copied().collect())
        .unwrap_or_default();
    let Ok((window_entity, window)) = window.single() else {
        state.held = None;
        return;
    };
    let dpi = window.scale_factor();
    let phone = platform.as_ref().is_some_and(|p| p.is_mobile())
        || mobile.as_ref().is_some_and(|m| m.enabled);
    let on = |platform: ScrollPlatform| match platform {
        ScrollPlatform::All => true,
        ScrollPlatform::Desktop => !phone,
        ScrollPlatform::Phone => phone,
    };
    // Without the mobile HUD resource (unit tests) orientation and focus do
    // not veto; at runtime it always exists on a phone.
    let oriented = mobile
        .as_ref()
        .is_none_or(|mobile| mobile.landscape && mobile.focused);
    let touch_on = |drag: &DragScroll| {
        window.focused && on(drag.platform) && (drag.platform != ScrollPlatform::Phone || oriented)
    };
    let live = |entity: Entity, node: &ComputedNode, visibility: Option<&InheritedVisibility>| {
        visibility.is_none_or(|v| v.get()) && node.size().min_element() > 0.0 && gate.allows(entity)
    };

    // Wheel and page keys.
    let page = keys.as_ref().map_or(0.0, |keys| {
        f32::from(keys.just_pressed(KeyCode::PageDown))
            - f32::from(keys.just_pressed(KeyCode::PageUp))
    });
    if !wheel.is_empty() || page != 0.0 {
        let cursor = window.cursor_position();
        for (entity, area, node, transform, visibility, clip, mut scroll) in &mut areas {
            let Some(config) = area.wheel.filter(|config| on(config.platform)) else {
                continue;
            };
            if !live(entity, node, visibility) {
                continue;
            }
            if config.hover_only
                && !cursor.zip(transform).is_some_and(|(point, transform)| {
                    logical_ui_rect(node, transform, clip, dpi).contains(point)
                })
            {
                continue;
            }
            let delta = page * config.page_step
                - wheel
                    .iter()
                    .map(|event| {
                        event.y
                            * match event.unit {
                                MouseScrollUnit::Line => config.line_step,
                                MouseScrollUnit::Pixel => config.pixel_step,
                            }
                    })
                    .sum::<f32>();
            if delta != 0.0 {
                scroll.y = (scroll.y + delta).clamp(0.0, max_offset(node));
            }
        }
    }

    // Touch drag: the first finger on an overflowing area owns it.
    for event in touches.iter().filter(|e| e.window == window_entity) {
        if event.phase == TouchPhase::Started {
            if state.held.as_ref().is_none_or(|held| held.ended) {
                state.held = areas
                    .iter()
                    .filter_map(|(entity, area, node, transform, visibility, clip, _)| {
                        let drag = area.drag.filter(|drag| touch_on(drag))?;
                        let transform = transform?;
                        (live(entity, node, visibility)
                            && node.content_size().y > node.size().y
                            && logical_ui_rect(node, transform, clip, dpi).contains(event.position))
                        .then(|| {
                            (
                                node.size().x * node.size().y,
                                entity,
                                area.key,
                                drag.threshold,
                            )
                        })
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, area, key, threshold)| Held {
                        finger: event.id,
                        area,
                        key,
                        threshold,
                        start: event.position,
                        previous: event.position,
                        moved: false,
                        pending: 0.0,
                        ended: false,
                    });
            }
            continue;
        }
        let Some(held) = state
            .held
            .as_mut()
            .filter(|held| held.finger == event.id && !held.ended)
        else {
            continue;
        };
        if event.phase == TouchPhase::Canceled {
            state.held = None;
            continue;
        }
        let was_moved = held.moved;
        held.moved |= held.start.distance(event.position) > held.threshold;
        if held.moved {
            // The first scrolling event catches up from the start point.
            held.pending += if was_moved {
                held.previous.y
            } else {
                held.start.y
            } - event.position.y;
        }
        held.previous = event.position;
        held.ended = event.phase == TouchPhase::Ended;
    }

    // Apply the held finger's travel to the area it owns.
    let Some(held) = state.held.as_mut() else {
        return;
    };
    if areas.get(held.area).is_err()
        && let Some(key) = held.key
        && let Some((entity, ..)) = areas.iter().find(|item| item.1.key == Some(key))
    {
        held.area = entity;
    }
    let keep = match areas.get_mut(held.area) {
        // A rebuilt keyed area keeps the motion until it exists again.
        Err(_) => held.key.is_some(),
        Ok((entity, area, node, _, visibility, _, mut scroll)) => {
            if node.size().min_element() <= 0.0 {
                // Unmeasured rebuild frame (keyed) or a hidden panel.
                held.key.is_some()
            } else if !live(entity, node, visibility) || !area.drag.is_some_and(|d| touch_on(&d)) {
                false
            } else {
                if held.pending != 0.0 {
                    // ScrollPosition is in UI units; TouchInput is in logical
                    // window pixels.
                    scroll.y = (scroll.y + held.pending * dpi * node.inverse_scale_factor())
                        .clamp(0.0, max_offset(node));
                    held.pending = 0.0;
                }
                true
            }
        }
    };
    if !keep || (held.ended && held.pending == 0.0) {
        state.held = None;
    }
}

/// Test helpers for modules that check their panel still scrolls.
#[cfg(test)]
pub(crate) mod harness {
    use super::*;

    /// Adds the scroll system, its messages and a primary window to `app`.
    pub(crate) fn install(app: &mut App, profile: crate::platform::UiProfile) -> Entity {
        app.insert_resource(crate::ui::UiPlatform(profile))
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<MouseWheel>()
            .add_message::<TouchInput>()
            .add_systems(Update, scroll_areas);
        app.world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id()
    }

    /// Gives `entity` a measured 400×200 viewport over 800 px of content at
    /// `center`, visible (no layout or visibility pass runs in unit tests).
    pub(crate) fn measure(app: &mut App, entity: Entity, center: Vec2) {
        app.world_mut().entity_mut(entity).insert((
            ComputedNode {
                size: Vec2::new(400.0, 200.0),
                content_size: Vec2::new(400.0, 800.0),
                inverse_scale_factor: 1.0,
                ..default()
            },
            UiGlobalTransform::from_translation(center),
            InheritedVisibility::VISIBLE,
        ));
    }

    pub(crate) fn wheel_lines(app: &mut App, window: Entity, lines: f32) {
        app.world_mut().write_message(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: lines,
            window,
        });
        app.update();
    }

    /// One finger from `from` moving up by `dy` and releasing.
    pub(crate) fn drag(app: &mut App, window: Entity, id: u64, from: Vec2, dy: f32) {
        for (phase, offset) in [
            (TouchPhase::Started, 0.0),
            (TouchPhase::Moved, dy),
            (TouchPhase::Ended, dy),
        ] {
            app.world_mut().write_message(TouchInput {
                phase,
                position: from - Vec2::Y * offset,
                window,
                id,
                force: None,
            });
        }
        app.update();
    }

    pub(crate) fn offset(app: &App, entity: Entity) -> f32 {
        app.world().get::<ScrollPosition>(entity).unwrap().y
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        Pressable,
        gesture::recognize_presses,
        modal::{ModalAppExt, ModalId, ModalRoot},
    };

    fn app(mobile: bool) -> (App, Entity) {
        let mut app = App::new();
        app.insert_resource(crate::ui::UiPlatform(if mobile {
            crate::platform::UiProfile::Mobile
        } else {
            crate::platform::UiProfile::Desktop
        }))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<crate::ui::GestureEpoch>()
        .add_message::<MouseWheel>()
        .add_message::<TouchInput>()
        .add_message::<crate::ui::SyntheticPress>()
        .add_systems(Update, (recognize_presses, scroll_areas).chain());
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        (app, window)
    }

    fn area(app: &mut App, area: ScrollArea, center: Vec2) -> Entity {
        app.world_mut()
            .spawn((
                area,
                ComputedNode {
                    size: Vec2::new(400.0, 200.0),
                    content_size: Vec2::new(400.0, 800.0),
                    inverse_scale_factor: 1.0,
                    ..default()
                },
                UiGlobalTransform::from_translation(center),
            ))
            .id()
    }

    fn y(app: &App, entity: Entity) -> f32 {
        app.world().get::<ScrollPosition>(entity).unwrap().y
    }

    fn wheel(app: &mut App, window: Entity, unit: MouseScrollUnit, y: f32) {
        app.world_mut().write_message(MouseWheel {
            unit,
            x: 0.0,
            y,
            window,
        });
        app.update();
    }

    fn touch(app: &mut App, window: Entity, id: u64, phase: TouchPhase, position: Vec2) {
        app.world_mut().write_message(TouchInput {
            phase,
            position,
            window,
            id,
            force: None,
        });
    }

    #[test]
    fn wheel_uses_the_panel_step_and_clamps_to_the_content() {
        let (mut app, window) = app(false);
        let panel = area(&mut app, ScrollArea::menu(32.0), Vec2::new(300.0, 300.0));
        let pixels = area(
            &mut app,
            ScrollArea::wheel(24.0).pixel_step(24.0).hover_only(),
            Vec2::new(900.0, 300.0),
        );
        wheel(&mut app, window, MouseScrollUnit::Line, -2.0);
        assert_eq!(y(&app, panel), 64.0);
        assert_eq!(y(&app, pixels), 0.0, "hover-only area without the cursor");
        wheel(&mut app, window, MouseScrollUnit::Pixel, -50.0);
        assert_eq!(y(&app, panel), 114.0);
        wheel(&mut app, window, MouseScrollUnit::Line, -100.0);
        assert_eq!(y(&app, panel), 600.0, "clamped to content minus viewport");
        wheel(&mut app, window, MouseScrollUnit::Line, 100.0);
        assert_eq!(y(&app, panel), 0.0);
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(900.0, 300.0)));
        wheel(&mut app, window, MouseScrollUnit::Pixel, -1.0);
        assert_eq!(y(&app, pixels), 24.0);
        assert_eq!(
            y(&app, panel),
            1.0,
            "pixel units pass through 1:1 by default"
        );
        // Page keys only where configured.
        let paged = area(
            &mut app,
            ScrollArea::wheel(48.0).page_keys(180.0),
            Vec2::ZERO,
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::PageDown);
        app.update();
        assert_eq!(y(&app, paged), 180.0);
        assert_eq!(y(&app, panel), 1.0, "no page keys configured");
        // A desktop-only wheel ignores the wheel on a phone build.
        let (mut phone, window) = self::app(true);
        let panel = area(&mut phone, ScrollArea::menu(32.0), Vec2::new(300.0, 300.0));
        wheel(&mut phone, window, MouseScrollUnit::Line, -2.0);
        assert_eq!(y(&phone, panel), 0.0);
    }

    #[test]
    fn drag_scrolls_past_the_threshold_and_never_activates_the_button_under_it() {
        let (mut app, window) = app(true);
        let panel = area(&mut app, ScrollArea::menu(32.0), Vec2::new(300.0, 300.0));
        let button = app
            .world_mut()
            .spawn((
                Pressable::default(),
                ComputedNode {
                    size: Vec2::new(180.0, 46.0),
                    inverse_scale_factor: 1.0,
                    stack_index: 1,
                    ..default()
                },
                UiGlobalTransform::from_translation(Vec2::new(300.0, 300.0)),
                ChildOf(panel),
            ))
            .id();
        // A short tap presses and does not scroll.
        touch(
            &mut app,
            window,
            1,
            TouchPhase::Started,
            Vec2::new(300.0, 300.0),
        );
        touch(
            &mut app,
            window,
            1,
            TouchPhase::Ended,
            Vec2::new(300.0, 296.0),
        );
        app.update();
        assert!(app.world().get::<Pressable>(button).unwrap().activated);
        assert_eq!(y(&app, panel), 0.0);
        // Within the slop: nothing moves yet.
        touch(
            &mut app,
            window,
            2,
            TouchPhase::Started,
            Vec2::new(300.0, 300.0),
        );
        touch(
            &mut app,
            window,
            2,
            TouchPhase::Moved,
            Vec2::new(300.0, 292.0),
        );
        app.update();
        assert_eq!(y(&app, panel), 0.0);
        // Past it: the content catches up from the start point, then follows.
        touch(
            &mut app,
            window,
            2,
            TouchPhase::Moved,
            Vec2::new(300.0, 280.0),
        );
        app.update();
        assert_eq!(y(&app, panel), 20.0);
        // An unrelated finger cannot steal the drag.
        touch(
            &mut app,
            window,
            3,
            TouchPhase::Started,
            Vec2::new(300.0, 300.0),
        );
        touch(
            &mut app,
            window,
            3,
            TouchPhase::Moved,
            Vec2::new(300.0, 100.0),
        );
        touch(
            &mut app,
            window,
            2,
            TouchPhase::Moved,
            Vec2::new(300.0, 250.0),
        );
        touch(
            &mut app,
            window,
            2,
            TouchPhase::Ended,
            Vec2::new(300.0, 250.0),
        );
        app.update();
        assert_eq!(y(&app, panel), 50.0);
        assert!(
            !app.world().get::<Pressable>(button).unwrap().activated,
            "a drag is not a press"
        );
        // Coming back over the button after a drag does not revive the tap.
        touch(
            &mut app,
            window,
            4,
            TouchPhase::Started,
            Vec2::new(300.0, 300.0),
        );
        touch(
            &mut app,
            window,
            4,
            TouchPhase::Moved,
            Vec2::new(300.0, 360.0),
        );
        touch(
            &mut app,
            window,
            4,
            TouchPhase::Ended,
            Vec2::new(300.0, 300.0),
        );
        app.update();
        assert!(!app.world().get::<Pressable>(button).unwrap().activated);
        assert_eq!(y(&app, panel), 50.0, "net travel zero");
        // Touch drag is phone-only for menu panels.
        let (mut desktop, window) = self::app(false);
        let panel = area(
            &mut desktop,
            ScrollArea::menu(32.0),
            Vec2::new(300.0, 300.0),
        );
        touch(
            &mut desktop,
            window,
            1,
            TouchPhase::Started,
            Vec2::new(300.0, 300.0),
        );
        touch(
            &mut desktop,
            window,
            1,
            TouchPhase::Moved,
            Vec2::new(300.0, 200.0),
        );
        desktop.update();
        assert_eq!(y(&desktop, panel), 0.0);
    }

    #[test]
    fn drag_clamps_and_only_the_top_modal_scrolls() {
        let (mut app, window) = app(true);
        #[derive(Resource, Default)]
        struct Open(bool, bool);
        app.init_resource::<Open>()
            .register_modal::<Open>(ModalId::Pause, |open| open.0)
            .register_modal::<Open>(ModalId::ServerEntry, |open| open.1);
        let pause = app.world_mut().spawn(ModalRoot(ModalId::Pause)).id();
        let panel = area(&mut app, ScrollArea::menu(32.0), Vec2::new(300.0, 300.0));
        app.world_mut().entity_mut(panel).insert(ChildOf(pause));
        let drag = |app: &mut App, id, dy: f32| {
            touch(
                app,
                window,
                id,
                TouchPhase::Started,
                Vec2::new(300.0, 300.0),
            );
            touch(
                app,
                window,
                id,
                TouchPhase::Moved,
                Vec2::new(300.0, 300.0 - dy),
            );
            touch(
                app,
                window,
                id,
                TouchPhase::Ended,
                Vec2::new(300.0, 300.0 - dy),
            );
            app.update();
        };
        app.world_mut().resource_mut::<Open>().0 = true;
        app.update();
        drag(&mut app, 1, 90.0);
        assert_eq!(y(&app, panel), 90.0);
        drag(&mut app, 2, -150.0);
        assert_eq!(y(&app, panel), 0.0, "clamped at the top");
        app.world_mut().resource_mut::<Open>().1 = true;
        app.update();
        drag(&mut app, 3, 90.0);
        assert_eq!(y(&app, panel), 0.0, "covered by the server entry");
        app.world_mut().resource_mut::<Open>().1 = false;
        app.update();
        drag(&mut app, 4, 90.0);
        assert_eq!(y(&app, panel), 90.0);
    }

    #[test]
    fn keyed_drag_survives_a_rebuild_and_keeps_unmeasured_motion() {
        let (mut app, window) = app(false);
        let pane = ScrollArea::wheel(24.0).touch_drag(0.0).keyed(2);
        let old = area(&mut app, pane, Vec2::ZERO);
        touch(
            &mut app,
            window,
            5,
            TouchPhase::Started,
            Vec2::new(0.0, 20.0),
        );
        touch(
            &mut app,
            window,
            5,
            TouchPhase::Moved,
            Vec2::new(0.0, -10.0),
        );
        app.update();
        assert_eq!(y(&app, old), 30.0);
        app.world_mut().despawn(old);
        let new = app
            .world_mut()
            .spawn((
                pane,
                ComputedNode::default(),
                UiGlobalTransform::default(),
                ScrollPosition(Vec2::new(0.0, 30.0)),
            ))
            .id();
        touch(
            &mut app,
            window,
            5,
            TouchPhase::Moved,
            Vec2::new(0.0, -30.0),
        );
        app.update();
        assert_eq!(y(&app, new), 30.0);
        app.world_mut().entity_mut(new).insert(ComputedNode {
            size: Vec2::new(400.0, 200.0),
            content_size: Vec2::new(400.0, 800.0),
            inverse_scale_factor: 1.0,
            ..default()
        });
        touch(
            &mut app,
            window,
            5,
            TouchPhase::Moved,
            Vec2::new(0.0, -60.0),
        );
        app.update();
        assert_eq!(y(&app, new), 80.0);
    }
}
