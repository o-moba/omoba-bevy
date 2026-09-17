//! Controller navigation feeds the same one-frame button presses as pointer UI.
//! Gameplay intents stay in `gamepad_controls`; this module never sends commands.
use bevy::{prelude::*, ui::InteractionDisabled, window::PrimaryWindow};

use crate::{
    gamepad_controls::{GamepadControls, GamepadInputSet, GamepadNavigation},
    help_overlay::HelpOverlayVisible,
    input_context::InputContextSet,
    net::{GameState, GameStateSnapshot},
    pause_menu::PauseMenuState,
};

pub(crate) struct GamepadUiPlugin;
impl Plugin for GamepadUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControllerFocus>()
            .add_systems(Startup, setup_controller_ui)
            .add_systems(
                PreUpdate,
                release_controller_press
                    .after(bevy::input::InputSystems)
                    .before(bevy::ui::UiSystems::Focus),
            )
            .add_systems(
                PreUpdate,
                navigate_controller_ui
                    .after(release_controller_press)
                    .after(GamepadInputSet::Sample)
                    .after(bevy::ui::UiSystems::Focus),
            )
            .add_systems(Update, draw_controller_ui.after(InputContextSet::Resolve));
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    entity: Entity,
    rect: Rect,
}

#[derive(Resource, Default)]
struct ControllerFocus {
    focused: Option<Entity>,
    pressed: Option<Entity>,
    surface: Vec<Entity>,
    confirm_ready: bool,
}

impl ControllerFocus {
    fn clear(&mut self) {
        self.focused = None;
        self.surface.clear();
        self.confirm_ready = false;
    }

    fn navigate(&mut self, candidates: &[Candidate], nav: &GamepadNavigation) -> Option<Entity> {
        let surface: Vec<_> = candidates
            .iter()
            .map(|candidate| candidate.entity)
            .collect();
        if surface != self.surface {
            // A new modal, a hidden section, or a rebuilt account screen must
            // never inherit a confirm intended for its previous buttons.
            self.surface = surface;
            self.focused = candidates.first().map(|candidate| candidate.entity);
            self.confirm_ready = false;
            return None;
        }
        let direction = Vec2::new(
            i32::from(nav.right) as f32 - i32::from(nav.left) as f32,
            i32::from(nav.down) as f32 - i32::from(nav.up) as f32,
        );
        if direction != Vec2::ZERO {
            self.focused = directional_neighbor(candidates, self.focused, direction);
            return None;
        }
        if !nav.confirm {
            self.confirm_ready = true;
        }
        if nav.confirm && self.confirm_ready {
            self.confirm_ready = false;
            return self.focused;
        }
        None
    }
}

fn directional_neighbor(
    candidates: &[Candidate],
    current: Option<Entity>,
    direction: Vec2,
) -> Option<Entity> {
    let Some(origin) = candidates
        .iter()
        .find(|candidate| Some(candidate.entity) == current)
    else {
        return candidates.first().map(|candidate| candidate.entity);
    };
    let direction = direction.normalize_or_zero();
    candidates
        .iter()
        .filter(|candidate| candidate.entity != origin.entity)
        .filter_map(|candidate| {
            let delta = candidate.rect.center() - origin.rect.center();
            let along = delta.dot(direction);
            // Prefer the same column/row over a closer diagonal button.
            (along > 1.0).then_some((candidate, along + delta.perp_dot(direction).abs() * 3.0))
        })
        .min_by(|a, b| {
            a.1.total_cmp(&b.1)
                .then_with(|| a.0.entity.cmp(&b.0.entity))
        })
        .map(|(candidate, _)| candidate.entity)
        .or(current)
}

fn release_controller_press(
    mut state: ResMut<ControllerFocus>,
    mut buttons: Query<&mut Interaction, With<Button>>,
) {
    if let Some(entity) = state.pressed.take() {
        if let Ok(mut interaction) = buttons.get_mut(entity) {
            if *interaction == Interaction::Pressed {
                *interaction = Interaction::None;
            }
        }
    }
}

type HierarchyNode<'a> = (
    Option<&'a Node>,
    Option<&'a Visibility>,
    Option<&'a InheritedVisibility>,
    Option<&'a ChildOf>,
    Option<&'a Name>,
    Option<&'a GlobalZIndex>,
);

/// Inspect ancestors directly: propagated visibility/layout can still contain
/// last frame's values when a parent was hidden or a new section was selected.
fn visible_in_hierarchy(entity: Entity, nodes: &Query<HierarchyNode>) -> bool {
    let mut current = Some(entity);
    for _ in 0..128 {
        let Some(entity) = current else { return true };
        let Ok((node, visibility, inherited, parent, _, _)) = nodes.get(entity) else {
            return false;
        };
        if node.is_some_and(|node| node.display == Display::None)
            || visibility.is_some_and(|visibility| *visibility == Visibility::Hidden)
            || inherited.is_some_and(|visibility| !visibility.get())
        {
            return false;
        }
        current = parent.map(ChildOf::parent);
    }
    false
}

fn has_ancestor(entity: Entity, ancestor: Entity, nodes: &Query<HierarchyNode>) -> bool {
    let mut current = Some(entity);
    for _ in 0..128 {
        let Some(entity) = current else { return false };
        if entity == ancestor {
            return true;
        }
        current = nodes
            .get(entity)
            .ok()
            .and_then(|(_, _, _, parent, _, _)| parent.map(ChildOf::parent));
    }
    false
}

fn modal_priority(name: Option<&Name>, global_z: Option<&GlobalZIndex>) -> Option<i32> {
    // Supporter uses an unnamed root, while its child close button has z=1301.
    if global_z.is_some_and(|z| z.0 == 1300) {
        return Some(1300);
    }
    match name.map(Name::as_str) {
        Some("HelpOverlayRoot") => Some(40),
        Some("ShopRoot") => Some(45),
        Some("PauseMenuRoot") => Some(100),
        Some("CareerMobileRoot" | "CareerDesktopRoot") => Some(120),
        Some("ServerEntryRoot" | "SocialChatRoot" | "SocialWheelRoot") => Some(150),
        _ => None,
    }
}

fn visible_rect(
    node: &ComputedNode,
    transform: &UiGlobalTransform,
    clip: Option<&bevy::ui::CalculatedClip>,
    viewport: Vec2,
) -> Option<Rect> {
    let factor = node.inverse_scale_factor();
    let size = node.size() * transform.to_scale_angle_translation().0.abs() * factor;
    let center = transform.translation * factor;
    if !size.is_finite() || !center.is_finite() || size.min_element() <= 0.0 {
        return None;
    }
    let mut rect =
        Rect::from_center_size(center, size).intersect(Rect::from_corners(Vec2::ZERO, viewport));
    if let Some(clip) = clip {
        rect = rect.intersect(Rect::from_corners(
            clip.clip.min * factor,
            clip.clip.max * factor,
        ));
    }
    (rect.width() > 1.0 && rect.height() > 1.0).then_some(rect)
}

fn scroll_at_boundary(
    current: Option<Entity>,
    next: Option<Entity>,
    direction: f32,
    nodes: &Query<HierarchyNode>,
    scrolls: &mut Query<(&ComputedNode, &mut ScrollPosition)>,
) -> bool {
    if direction == 0.0 {
        return false;
    }
    let mut ancestor = current;
    for _ in 0..128 {
        let Some(entity) = ancestor else { return false };
        if let Ok((computed, mut scroll)) = scrolls.get_mut(entity) {
            // At the last visible row, reveal the rest of this panel before
            // jumping to buttons outside it. Fully clipped buttons never act.
            if next != current && next.is_some_and(|next| has_ancestor(next, entity, nodes)) {
                return false;
            }
            let maximum = ((computed.content_size().y - computed.size().y)
                * computed.inverse_scale_factor())
            .max(0.0);
            let next_y = (scroll.y + direction * 120.0).clamp(0.0, maximum);
            if (next_y - scroll.y).abs() > 0.1 {
                scroll.y = next_y;
                return true;
            }
        }
        ancestor = nodes
            .get(entity)
            .ok()
            .and_then(|(_, _, _, parent, _, _)| parent.map(ChildOf::parent));
    }
    false
}

#[allow(clippy::type_complexity)]
fn navigate_controller_ui(
    controls: Res<GamepadControls>,
    mut state: ResMut<ControllerFocus>,
    windows: Query<&Window, With<PrimaryWindow>>,
    game: Option<Res<GameStateSnapshot>>,
    mut pause: Option<ResMut<PauseMenuState>>,
    mut help: Option<ResMut<HelpOverlayVisible>>,
    mut supporter: Option<ResMut<crate::supporter::SupporterUiState>>,
    nodes: Query<HierarchyNode>,
    roots: Query<Entity, With<Node>>,
    mut scrolls: Query<(&ComputedNode, &mut ScrollPosition)>,
    mut buttons: Query<
        (
            Entity,
            &ComputedNode,
            &UiGlobalTransform,
            Option<&bevy::ui::CalculatedClip>,
            Option<&Name>,
            &mut Interaction,
        ),
        (With<Button>, Without<InteractionDisabled>),
    >,
) {
    let Ok(window) = windows.single() else {
        state.clear();
        return;
    };
    if !controls.active || !controls.connected || !window.focused {
        state.clear();
        return;
    }
    let modal = roots
        .iter()
        .filter_map(|entity| {
            let (_, _, _, _, name, global_z) = nodes.get(entity).ok()?;
            let priority = modal_priority(name, global_z)?;
            visible_in_hierarchy(entity, &nodes).then_some((entity, priority))
        })
        .max_by_key(|(entity, priority)| (*priority, *entity))
        .map(|(entity, _)| entity);
    let pause_root = modal.is_some_and(|entity| {
        nodes.get(entity).is_ok_and(|(_, _, _, _, name, _)| {
            name.is_some_and(|name| name.as_str() == "PauseMenuRoot")
        })
    });
    let help_root = modal.is_some_and(|entity| {
        nodes.get(entity).is_ok_and(|(_, _, _, _, name, _)| {
            name.is_some_and(|name| name.as_str() == "HelpOverlayRoot")
        })
    });
    if controls.nav.menu {
        if modal.is_none() || pause_root || help_root {
            if let Some(help) = help.as_deref_mut() {
                help.0 = false;
            }
            if let Some(pause) = pause.as_deref_mut() {
                let open = !pause.open;
                *pause = PauseMenuState::default();
                pause.open = open;
            }
        }
        state.clear();
        return;
    }
    let viewport = Vec2::new(window.width(), window.height());
    let in_lobby = game
        .as_ref()
        .is_none_or(|game| !matches!(game.state, GameState::Running))
        || roots.iter().any(|entity| {
            nodes.get(entity).is_ok_and(|(_, _, _, _, name, _)| {
                name.is_some_and(|name| name.as_str() == "TeamSelectOverlay")
                    && visible_in_hierarchy(entity, &nodes)
            })
        });
    if modal.is_none() && !in_lobby {
        state.clear();
        return;
    }
    let mut candidates = Vec::new();
    let mut close_button = None;
    for (entity, computed, transform, clip, name, _) in &mut buttons {
        if !visible_in_hierarchy(entity, &nodes)
            || modal.is_some_and(|modal| !has_ancestor(entity, modal, &nodes))
        {
            continue;
        }
        let Some(rect) = visible_rect(computed, transform, clip, viewport) else {
            continue;
        };
        if name.is_some_and(|name| {
            matches!(
                name.as_str(),
                "CareerClose" | "SocialClose" | "ServerCloseButton" | "ShopCloseButton"
            )
        }) {
            close_button = Some(entity);
        }
        candidates.push(Candidate { entity, rect });
    }
    candidates.sort_by(|a, b| {
        a.rect
            .center()
            .y
            .total_cmp(&b.rect.center().y)
            .then_with(|| a.rect.center().x.total_cmp(&b.rect.center().x))
            .then_with(|| a.entity.cmp(&b.entity))
    });
    let pressed = if controls.nav.cancel {
        state.clear();
        if supporter.as_ref().is_some_and(|supporter| supporter.open) {
            if let Some(supporter) = supporter.as_deref_mut() {
                supporter.open = false;
            }
            None
        } else if pause_root {
            if let Some(pause) = pause.as_deref_mut() {
                *pause = PauseMenuState::default();
            }
            None
        } else if help_root {
            if let Some(help) = help.as_deref_mut() {
                help.0 = false;
            }
            None
        } else {
            close_button
        }
    } else {
        let prior = state.focused;
        let pressed = state.navigate(&candidates, &controls.nav);
        let direction = i32::from(controls.nav.down) as f32 - i32::from(controls.nav.up) as f32;
        if scroll_at_boundary(prior, state.focused, direction, &nodes, &mut scrolls) {
            state.focused = prior;
            state.confirm_ready = false;
            None
        } else {
            pressed
        }
    };
    if let Some(entity) = pressed {
        if let Ok((_, _, _, _, _, mut interaction)) = buttons.get_mut(entity) {
            *interaction = Interaction::Pressed;
            state.pressed = Some(entity);
        }
    }
}

#[derive(Component)]
struct ControllerLegend;
#[derive(Component)]
struct ControllerFocusRing;

fn setup_controller_ui(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        crate::ui_theme::text(12.0),
        TextColor(crate::ui_theme::IVORY),
        BackgroundColor(crate::ui_theme::PANEL),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(4.0),
            left: Val::Percent(20.0),
            max_width: Val::Percent(60.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
            display: Display::None,
            ..default()
        },
        GlobalZIndex(2000),
        bevy::ui::FocusPolicy::Pass,
        Pickable::IGNORE,
        ControllerLegend,
        Name::new("ControllerLegend"),
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            display: Display::None,
            ..default()
        },
        Outline::new(Val::Px(3.0), Val::Px(3.0), crate::ui_theme::GOLD),
        GlobalZIndex(1999),
        bevy::ui::FocusPolicy::Pass,
        Pickable::IGNORE,
        ControllerFocusRing,
        Name::new("ControllerFocusRing"),
    ));
}

fn legend(playstation: bool, menu: bool, phone: bool) -> &'static str {
    match (playstation, menu, phone) {
        (true, true, _) => "D-pad / LS: navigate · Cross: select · Circle: back",
        (false, true, _) => "D-pad / LS: navigate · A: select · B: back",
        // The phone skill bar carries all four trigger labels above this strip.
        (true, false, true) => {
            "LS move · RS aim · R3 lock · R2 attack · Circle cancel · Options menu\nHold skill: aim; release: cast · Triangle + skill: upgrade · D-pad right: shop; left: reactions"
        }
        (false, false, true) => {
            "LS move · RS aim · RS click lock · RT attack · B cancel · Menu\nHold skill: aim; release: cast · Y + skill: upgrade · D-pad right: shop; left: reactions"
        }
        (true, false, false) => {
            "Left stick: move   Right stick: aim   R2: attack   R3: lock\nHold L1 / R1 / L2: aim skill, release: cast   L2+R2: ultimate\nCircle: cancel   Triangle + skill: upgrade   Options: menu   D-pad right: shop; left: reactions"
        }
        (false, false, false) => {
            "Left stick: move   Right stick: aim   RT: attack   RS click: lock\nHold LB / RB / LT: aim skill, release: cast   LT+RT: ultimate\nB: cancel   Y + skill: upgrade   Menu: menu   D-pad right: shop; left: reactions"
        }
    }
}

#[allow(clippy::type_complexity)]
fn draw_controller_ui(
    controls: Res<GamepadControls>,
    state: Res<ControllerFocus>,
    context: Res<crate::input_context::GameplayInputContext>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    geometry: Query<
        (
            &ComputedNode,
            &UiGlobalTransform,
            Option<&bevy::ui::CalculatedClip>,
            Option<&InheritedVisibility>,
            Has<Button>,
            Has<Text>,
        ),
        Without<ControllerLegend>,
    >,
    mut legend_query: Query<
        (&mut Node, &mut Text, &mut TextFont),
        (With<ControllerLegend>, Without<ControllerFocusRing>),
    >,
    mut focus_query: Query<&mut Node, (With<ControllerFocusRing>, Without<ControllerLegend>)>,
) {
    let focused_window = windows.single().ok().filter(|window| window.focused);
    let visible = controls.active && controls.connected && focused_window.is_some();
    let phone = mobile.as_ref().filter(|mobile| mobile.enabled);
    let menu = !context.gameplay_allowed();
    let menu_rect = focused_window.filter(|_| menu).map(|window| {
        let width = 340.0_f32.min(window.width() - 72.0);
        Rect::from_corners(
            Vec2::new(
                (window.width() - width) * 0.5,
                phone.map_or(12.0, |mobile| mobile.safe.top + 8.0),
            ),
            Vec2::new(
                (window.width() + width) * 0.5,
                phone.map_or(12.0, |mobile| mobile.safe.top + 8.0) + 20.0,
            ),
        )
    });
    let menu_space_clear = menu_rect.is_none_or(|hint| {
        let viewport = focused_window
            .map(|window| Vec2::new(window.width(), window.height()))
            .unwrap_or_default();
        geometry
            .iter()
            .all(|(node, transform, clip, visibility, button, text)| {
                if (!button && !text) || visibility.is_some_and(|visibility| !visibility.get()) {
                    return true;
                }
                visible_rect(node, transform, clip, viewport).is_none_or(|rect| {
                    let overlap = rect.intersect(hint);
                    overlap.width() <= 0.0 || overlap.height() <= 0.0
                })
            })
    });
    for (mut node, mut text, mut font) in &mut legend_query {
        node.display = if visible && menu_space_clear {
            Display::Flex
        } else {
            Display::None
        };
        node.top = Val::Auto;
        node.width = Val::Auto;
        if let Some(rect) = menu_rect {
            node.left = Val::Px(rect.min.x);
            node.right = Val::Auto;
            node.top = Val::Px(rect.min.y);
            node.bottom = Val::Auto;
            node.width = Val::Px(rect.width());
            node.max_width = Val::Px(rect.width());
            font.font_size = 10.0.into();
        } else if let Some(mobile) = phone {
            node.left = Val::Px(mobile.safe.left + 4.0);
            node.right = Val::Px(mobile.safe.right + 4.0);
            node.bottom = Val::Px(mobile.safe.bottom);
            node.max_width = Val::Percent(100.0);
            font.font_size = 10.0.into();
        } else {
            node.left = Val::Percent(20.0);
            node.right = Val::Auto;
            node.bottom = Val::Px(4.0);
            node.max_width = Val::Percent(60.0);
            font.font_size = 12.0.into();
        }
        let value = legend(controls.playstation, menu, phone.is_some());
        if text.0 != value {
            text.0 = value.to_owned();
        }
    }
    let rect = visible
        .then_some(state.focused)
        .flatten()
        .and_then(|entity| geometry.get(entity).ok())
        .and_then(|(node, transform, clip, _, _, _)| {
            focused_window.and_then(|window| {
                visible_rect(
                    node,
                    transform,
                    clip,
                    Vec2::new(window.width(), window.height()),
                )
            })
        });
    for mut node in &mut focus_query {
        node.display = if rect.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if let Some(rect) = rect {
            node.left = Val::Px(rect.min.x);
            node.top = Val::Px(rect.min.y);
            node.width = Val::Px(rect.width());
            node.height = Val::Px(rect.height());
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
        assert_eq!(
            directional_neighbor(&buttons, Some(buttons[0].entity), Vec2::X),
            Some(buttons[1].entity)
        );
        assert_eq!(
            directional_neighbor(&buttons, Some(buttons[0].entity), Vec2::Y),
            Some(buttons[2].entity)
        );
        assert_eq!(
            directional_neighbor(&buttons, Some(buttons[0].entity), Vec2::NEG_X),
            Some(buttons[0].entity)
        );
    }

    #[test]
    fn changed_menu_requires_a_fresh_confirm_and_emits_one_press() {
        let mut focus = ControllerFocus::default();
        let buttons = [candidate(1, Vec2::ZERO), candidate(2, Vec2::X * 40.0)];
        let mut nav = GamepadNavigation::default();
        nav.confirm = true;
        assert_eq!(focus.navigate(&buttons, &nav), None);
        assert_eq!(focus.navigate(&buttons, &nav), None);
        nav.confirm = false;
        focus.navigate(&buttons, &nav);
        nav.confirm = true;
        assert_eq!(focus.navigate(&buttons, &nav), Some(buttons[0].entity));
        assert_eq!(focus.navigate(&buttons, &nav), None);
        let replaced = [candidate(3, Vec2::ZERO)];
        assert_eq!(focus.navigate(&replaced, &nav), None);
        nav.confirm = false;
        focus.navigate(&replaced, &nav);
        nav.confirm = true;
        assert_eq!(focus.navigate(&replaced, &nav), Some(replaced[0].entity));
    }

    #[test]
    fn clipping_excludes_invisible_and_invalid_button_geometry() {
        let mut node = ComputedNode {
            size: Vec2::splat(20.0),
            inverse_scale_factor: 1.0,
            ..default()
        };
        let transform = UiGlobalTransform::from_translation(Vec2::new(20.0, 20.0));
        assert!(visible_rect(&node, &transform, None, Vec2::splat(100.0)).is_some());
        let offscreen = UiGlobalTransform::from_translation(Vec2::new(200.0, 200.0));
        assert!(visible_rect(&node, &offscreen, None, Vec2::splat(100.0)).is_none());
        node.size.x = f32::NAN;
        assert!(visible_rect(&node, &transform, None, Vec2::splat(100.0)).is_none());
    }

    #[test]
    fn hidden_ancestors_are_rejected_before_visibility_propagates() {
        fn inspect(nodes: Query<HierarchyNode>, mut result: ResMut<VisibilityResult>) {
            result.visible = visible_in_hierarchy(result.entity, &nodes);
        }
        #[derive(Resource)]
        struct VisibilityResult {
            entity: Entity,
            visible: bool,
        }
        let mut app = App::new();
        let root = app.world_mut().spawn(Node::default()).id();
        let button = app
            .world_mut()
            .spawn((Node::default(), ChildOf(root), InheritedVisibility::VISIBLE))
            .id();
        app.insert_resource(VisibilityResult {
            entity: button,
            visible: false,
        })
        .add_systems(Update, inspect);
        // Node's required visibility initially defaults hidden until propagation.
        app.world_mut()
            .entity_mut(root)
            .insert(InheritedVisibility::VISIBLE);
        app.update();
        assert!(app.world().resource::<VisibilityResult>().visible);
        app.world_mut().get_mut::<Node>(root).unwrap().display = Display::None;
        app.update();
        assert!(!app.world().resource::<VisibilityResult>().visible);
    }

    #[test]
    fn labels_match_controller_family_and_explain_upgrade_modifier() {
        assert!(legend(true, true, false).contains("Cross"));
        assert!(legend(false, true, false).contains("A: select"));
        assert!(legend(true, false, false).contains("Triangle + skill"));
        assert!(legend(false, false, false).contains("Y + skill"));
        for playstation in [true, false] {
            assert_eq!(legend(playstation, false, true).lines().count(), 2);
            assert!(
                legend(playstation, false, true).contains("D-pad right: shop; left: reactions")
            );
        }
    }

    #[derive(Resource, Default)]
    struct PressedButtons(Vec<Entity>);

    fn ui_app() -> App {
        let mut app = App::new();
        let mut controls = GamepadControls::default();
        controls.active = true;
        controls.connected = true;
        app.insert_resource(controls)
            .init_resource::<ControllerFocus>()
            .init_resource::<PressedButtons>()
            .init_resource::<PauseMenuState>()
            .init_resource::<HelpOverlayVisible>()
            .add_systems(
                PreUpdate,
                (release_controller_press, navigate_controller_ui).chain(),
            )
            .add_systems(
                Update,
                |buttons: Query<(Entity, &Interaction), (With<Button>, Changed<Interaction>)>,
                 mut pressed: ResMut<PressedButtons>| {
                    for (entity, interaction) in &buttons {
                        if *interaction == Interaction::Pressed {
                            pressed.0.push(entity);
                        }
                    }
                },
            );
        app.world_mut().spawn((
            Window {
                focused: true,
                ..default()
            },
            PrimaryWindow,
        ));
        app
    }

    fn ui_button(app: &mut App, parent: Option<Entity>, center: Vec2) -> Entity {
        let entity = app
            .world_mut()
            .spawn((
                Button,
                Node::default(),
                InheritedVisibility::VISIBLE,
                ComputedNode {
                    size: Vec2::splat(30.0),
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

    #[test]
    fn ecs_confirm_is_one_frame_and_disconnect_discards_focus() {
        let mut app = ui_app();
        let first = ui_button(&mut app, None, Vec2::new(100.0, 100.0));
        let second = ui_button(&mut app, None, Vec2::new(200.0, 100.0));
        app.update();
        app.update();
        app.world_mut().resource_mut::<GamepadControls>().nav.right = true;
        app.update();
        assert_eq!(
            app.world().resource::<ControllerFocus>().focused,
            Some(second)
        );
        app.world_mut().resource_mut::<GamepadControls>().nav = GamepadNavigation {
            confirm: true,
            ..default()
        };
        app.update();
        assert_eq!(app.world().resource::<PressedButtons>().0, [second]);
        app.world_mut().resource_mut::<GamepadControls>().nav = default();
        app.update();
        assert_eq!(
            *app.world().get::<Interaction>(second).unwrap(),
            Interaction::None
        );
        assert_eq!(app.world().resource::<PressedButtons>().0, [second]);
        let mut controls = app.world_mut().resource_mut::<GamepadControls>();
        controls.connected = false;
        controls.nav.confirm = true;
        app.update();
        assert_eq!(app.world().resource::<ControllerFocus>().focused, None);
        assert_eq!(app.world().resource::<PressedButtons>().0, [second]);
        assert_eq!(
            *app.world().get::<Interaction>(first).unwrap(),
            Interaction::None
        );
    }

    #[test]
    fn ecs_modal_traps_focus_and_disabled_buttons_cannot_activate() {
        let mut app = ui_app();
        let underlying = ui_button(&mut app, None, Vec2::new(40.0, 40.0));
        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<ControllerFocus>().focused,
            Some(underlying)
        );
        let root = app
            .world_mut()
            .spawn((
                Node::default(),
                InheritedVisibility::VISIBLE,
                Name::new("HelpOverlayRoot"),
            ))
            .id();
        let modal_button = ui_button(&mut app, Some(root), Vec2::new(100.0, 100.0));
        let disabled = ui_button(&mut app, Some(root), Vec2::new(200.0, 100.0));
        app.world_mut()
            .entity_mut(disabled)
            .insert(InteractionDisabled);
        app.world_mut()
            .resource_mut::<GamepadControls>()
            .nav
            .confirm = true;
        app.update();
        assert_eq!(
            app.world().resource::<ControllerFocus>().focused,
            Some(modal_button)
        );
        assert!(app.world().resource::<PressedButtons>().0.is_empty());
        app.world_mut().resource_mut::<GamepadControls>().nav = default();
        app.update();
        app.world_mut().resource_mut::<GamepadControls>().nav = GamepadNavigation {
            right: true,
            ..default()
        };
        app.update();
        assert_eq!(
            app.world().resource::<ControllerFocus>().focused,
            Some(modal_button)
        );
        app.world_mut().resource_mut::<GamepadControls>().nav = GamepadNavigation {
            confirm: true,
            ..default()
        };
        app.update();
        assert_eq!(app.world().resource::<PressedButtons>().0, [modal_button]);
        // A parent hidden this frame cannot use a stale layout/visibility value.
        app.world_mut().get_mut::<Node>(root).unwrap().display = Display::None;
        app.update();
        assert_eq!(app.world().resource::<PressedButtons>().0, [modal_button]);
    }

    #[test]
    fn ecs_options_and_cancel_change_pause_without_pressing_a_button() {
        let mut app = ui_app();
        ui_button(&mut app, None, Vec2::new(100.0, 100.0));
        app.world_mut().resource_mut::<GamepadControls>().nav = GamepadNavigation {
            menu: true,
            confirm: true,
            ..default()
        };
        app.update();
        assert!(app.world().resource::<PauseMenuState>().open);
        assert!(app.world().resource::<PressedButtons>().0.is_empty());
        app.world_mut().spawn((
            Node::default(),
            InheritedVisibility::VISIBLE,
            Name::new("PauseMenuRoot"),
        ));
        app.world_mut().resource_mut::<GamepadControls>().nav = GamepadNavigation {
            cancel: true,
            ..default()
        };
        app.update();
        assert!(!app.world().resource::<PauseMenuState>().open);
        assert!(app.world().resource::<PressedButtons>().0.is_empty());
    }

    #[test]
    fn ecs_scroll_boundary_reveals_panel_content_without_pressing_hidden_controls() {
        let mut app = ui_app();
        let panel = app
            .world_mut()
            .spawn((
                Node::default(),
                InheritedVisibility::VISIBLE,
                ComputedNode {
                    size: Vec2::new(300.0, 100.0),
                    content_size: Vec2::new(300.0, 500.0),
                    inverse_scale_factor: 1.0,
                    ..default()
                },
                ScrollPosition::default(),
            ))
            .id();
        let button = ui_button(&mut app, Some(panel), Vec2::new(100.0, 100.0));
        app.update();
        app.update();
        app.world_mut().resource_mut::<GamepadControls>().nav.down = true;
        app.update();
        assert_eq!(app.world().get::<ScrollPosition>(panel).unwrap().y, 120.0);
        assert_eq!(
            app.world().resource::<ControllerFocus>().focused,
            Some(button)
        );
        assert!(app.world().resource::<PressedButtons>().0.is_empty());
    }
}
