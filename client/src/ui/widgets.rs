//! Kit widgets: menu buttons, `-`/`+` stepper rows, toggle rows and value
//! labels in the overlay style the pause menu established, plus the front-end
//! screen buttons and tiles. Every control gets a [`UiAction`], a
//! [`ButtonStyle`] and a [`TestId`]; the module that owns it only handles
//! `Activated<T>`.
use bevy::prelude::*;

use super::{
    Pressable, TestId, UiAction,
    action::UiActionT,
    theme::{self, ButtonKind, metric},
};

/// Colour role of a kit button. Painted by [`paint_pressables`] from the
/// effective interaction, so a resting finger lights the button and a
/// completed tap flashes it exactly like a desktop click.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ButtonStyle {
    pub kind: ButtonKind,
    /// Selected tiles keep their colour under the pointer.
    pub selected: bool,
}

impl ButtonStyle {
    pub(crate) fn new(kind: ButtonKind) -> Self {
        Self {
            kind,
            selected: false,
        }
    }

    pub(crate) fn idle_color(&self) -> Color {
        theme::button_idle_color(self.kind, self.selected)
    }

    pub(crate) fn hover_color(&self) -> Color {
        theme::button_hover_color(self.kind, self.selected)
    }

    pub(crate) fn pressed_color(&self) -> Color {
        theme::button_pressed_color(self.kind, self.selected)
    }

    /// `selected`, set only when it changes (the painter reacts to changes).
    pub(crate) fn set_selected(style: &mut Mut<Self>, selected: bool) {
        if style.selected != selected {
            style.selected = selected;
        }
    }
}

/// Runs in `UiSet::Paint`; the one place kit buttons change colour.
pub(crate) fn paint_pressables(
    mut buttons: Query<
        (&Interaction, &Pressable, &ButtonStyle, &mut BackgroundColor),
        Or<(
            Changed<Interaction>,
            Changed<Pressable>,
            Changed<ButtonStyle>,
        )>,
    >,
) {
    for (interaction, pressable, style, mut color) in &mut buttons {
        let next = match pressable.effective(*interaction) {
            Interaction::Pressed => style.pressed_color(),
            Interaction::Hovered => style.hover_color(),
            Interaction::None => style.idle_color(),
        };
        if color.0 != next {
            color.0 = next;
        }
    }
}

fn button_bundle<T: UiActionT>(node: Node, kind: ButtonKind, action: T, id: TestId) -> impl Bundle {
    let style = ButtonStyle::new(kind);
    (
        Button,
        node,
        BorderColor::all(theme::EDGE),
        BackgroundColor(style.idle_color()),
        style,
        UiAction(action),
        id,
    )
}

fn menu_button_node() -> Node {
    Node {
        width: Val::Px(metric::MENU_W),
        height: Val::Px(metric::BUTTON_H),
        max_width: Val::Percent(100.0),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        flex_shrink: 0.0,
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    }
}

/// A full-width menu button with a centred label.
pub(crate) fn button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    kind: ButtonKind,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    spawn_menu_button(parent, label, kind, action, id.into(), ())
}

/// [`button`] whose label carries `label_marker` and its own id, for a
/// caption the owning module rewrites (a mute/unmute toggle).
pub(crate) fn button_with_label<T: UiActionT, M: Component>(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    kind: ButtonKind,
    action: T,
    id: impl Into<TestId>,
    label_marker: M,
    label_id: impl Into<TestId>,
) -> Entity {
    spawn_menu_button(
        parent,
        label,
        kind,
        action,
        id.into(),
        (label_marker, label_id.into()),
    )
}

fn spawn_menu_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    kind: ButtonKind,
    action: T,
    id: TestId,
    label_extra: impl Bundle,
) -> Entity {
    parent
        .spawn(button_bundle(menu_button_node(), kind, action, id))
        .with_children(|button| {
            button.spawn((
                Text::new(label),
                theme::text(17.0),
                TextColor(theme::IVORY),
                label_extra,
            ));
        })
        .id()
}

/// A square glyph button (the header close `×`).
pub(crate) fn icon_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    glyph: &str,
    kind: ButtonKind,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    let node = Node {
        width: Val::Px(metric::BUTTON_H),
        height: Val::Px(metric::BUTTON_H),
        flex_shrink: 0.0,
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        ..default()
    };
    parent
        .spawn(button_bundle(node, kind, action, id.into()))
        .with_children(|button| {
            button.spawn((Text::new(glyph), theme::text(28.0), TextColor(theme::IVORY)));
        })
        .id()
}

fn adjust_button<T: UiActionT>(row: &mut ChildSpawnerCommands, glyph: &str, action: T, id: TestId) {
    let node = Node {
        width: Val::Px(metric::ADJUST_BTN),
        height: Val::Px(metric::ADJUST_BTN),
        flex_shrink: 0.0,
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    };
    row.spawn(button_bundle(node, ButtonKind::Secondary, action, id))
        .with_children(|button| {
            button.spawn((Text::new(glyph), theme::text(22.0), TextColor(theme::IVORY)));
        });
}

/// A centred value the owning module rewrites through `marker`.
pub(crate) fn value_label<M: Component>(
    parent: &mut ChildSpawnerCommands,
    value: String,
    marker: M,
    id: impl Into<TestId>,
) -> Entity {
    parent
        .spawn((
            Text::new(value),
            Node {
                width: Val::Px(62.0),
                flex_shrink: 0.0,
                ..default()
            },
            TextLayout::new_with_justify(Justify::Center),
            theme::text(18.0),
            TextColor(theme::IVORY),
            marker,
            id.into(),
        ))
        .id()
}

/// `label  [-] value [+]`; the controls are `{id}-Down`, `{id}-Value` and
/// `{id}-Up`. Returns the row.
pub(crate) fn adjust_row<T: UiActionT, M: Component>(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    value: String,
    value_marker: M,
    decrease: T,
    increase: T,
    id: impl Into<TestId>,
) -> Entity {
    let id = id.into();
    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                flex_shrink: 0.0,
                column_gap: Val::Px(10.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            id.clone(),
        ))
        .with_children(|row| {
            row.spawn((
                Text::new(label),
                Node {
                    width: Val::Px(110.0),
                    flex_shrink: 0.0,
                    ..default()
                },
                theme::text(18.0),
                TextColor(theme::IVORY),
            ));
            adjust_button(row, "-", decrease, id.child("-Down"));
            value_label(row, value, value_marker, id.child("-Value"));
            adjust_button(row, "+", increase, id.child("-Up"));
        })
        .id()
}

/// A menu-width button showing `label` on the left and the current `value`
/// on the right; pressing it is the toggle. The button is `{id}Button` and
/// the value `{id}Value`. Returns the button.
pub(crate) fn toggle_row<T: UiActionT, M: Component>(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    value: &str,
    value_marker: M,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    let id = id.into();
    let node = Node {
        padding: UiRect::horizontal(Val::Px(14.0)),
        justify_content: JustifyContent::SpaceBetween,
        ..menu_button_node()
    };
    parent
        .spawn(button_bundle(
            node,
            ButtonKind::Secondary,
            action,
            id.child("Button"),
        ))
        .with_children(|button| {
            button.spawn((Text::new(label), theme::text(17.0), TextColor(theme::IVORY)));
            button.spawn((
                Text::new(value),
                theme::text(17.0),
                TextColor(theme::GOLD),
                value_marker,
                id.child("Value"),
            ));
        })
        .id()
}

/// Font size a front-end label was designed at; `frontend::widgets` applies
/// `metric::menu_font` to it every frame (a readable minimum on a phone).
#[derive(Component)]
pub(crate) struct MenuTypography {
    pub size: f32,
    pub heading: bool,
}

/// Height a front-end control was designed at; `metric::menu_control_height`
/// raises it to the touch minimum on a phone, in the same pass as
/// [`MenuTypography`].
#[derive(Component)]
pub(crate) struct MenuControl {
    pub height: f32,
}

/// A front-end screen label in `color`, with its phone readability metric.
pub(crate) fn screen_label(text: &str, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text.to_owned()),
        theme::text(size),
        TextColor(color),
        MenuTypography {
            size,
            heading: false,
        },
    )
}

/// A front-end screen button: `Primary` is the one big call to action with a
/// gold edge, everything else a 44 px pill sized to its label.
pub(crate) fn screen_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    kind: ButtonKind,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    spawn_screen_button(
        parent,
        text,
        ButtonStyle::new(kind),
        action,
        id.into(),
        false,
    )
}

/// A grid tile whose selected state is owned by the screen (it flips
/// `ButtonStyle::selected`; the painter repaints).
pub(crate) fn screen_tile<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    selected: bool,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    compact_screen_tile(parent, text, selected, action, id, false)
}

/// [`screen_tile`] that shrinks on a phone (the collection's clip strip).
pub(crate) fn compact_screen_tile<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    selected: bool,
    action: T,
    id: impl Into<TestId>,
    compact: bool,
) -> Entity {
    let style = ButtonStyle {
        kind: ButtonKind::Tile,
        selected,
    };
    spawn_screen_button(parent, text, style, action, id.into(), compact)
}

fn spawn_screen_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    style: ButtonStyle,
    action: T,
    id: TestId,
    compact: bool,
) -> Entity {
    let primary = style.kind == ButtonKind::Primary;
    let (width, height, font) = if primary {
        (Val::Px(metric::PRIMARY.0), metric::PRIMARY.1, 22.0)
    } else {
        (Val::Auto, metric::TOUCH_MIN, 15.0)
    };
    parent
        .spawn((
            Button,
            Node {
                width,
                height: Val::Px(height),
                min_width: Val::Px(if compact { 86.0 } else { 120.0 }),
                padding: UiRect::axes(Val::Px(if compact { 10.0 } else { 18.0 }), Val::Px(8.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BorderColor::all(if primary { theme::GOLD } else { theme::EDGE }),
            BackgroundColor(style.idle_color()),
            MenuControl { height },
            style,
            UiAction(action),
            id,
        ))
        .with_children(|button| {
            button.spawn(screen_label(text, font, theme::IVORY));
        })
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Probe {
        Down,
        Up,
        Toggle,
    }
    #[derive(Component)]
    struct Value;

    #[test]
    fn widgets_name_their_controls_and_paint_from_the_effective_interaction() {
        let mut app = App::new();
        app.add_message::<super::super::SyntheticPress>()
            .add_systems(Update, paint_pressables);
        let root = app.world_mut().spawn(Node::default()).id();
        app.world_mut()
            .commands()
            .entity(root)
            .with_children(|parent| {
                adjust_row(
                    parent,
                    "Level",
                    "5".into(),
                    Value,
                    Probe::Down,
                    Probe::Up,
                    "Row",
                );
                toggle_row(parent, "God mode", "OFF", Value, Probe::Toggle, "Practice");
                button(parent, "Resume", ButtonKind::Primary, Probe::Up, "Resume");
            });
        app.world_mut().flush();
        app.update();
        let names: Vec<String> = app
            .world_mut()
            .query::<&Name>()
            .iter(app.world())
            .map(|name| name.as_str().to_owned())
            .collect();
        for wanted in [
            "Row",
            "Row-Down",
            "Row-Value",
            "Row-Up",
            "PracticeButton",
            "PracticeValue",
            "Resume",
        ] {
            assert!(names.iter().any(|name| name == wanted), "{wanted} missing");
        }
        let resume = super::super::test_id::harness::find(app.world_mut(), "Resume").unwrap();
        assert_eq!(
            app.world().get::<BackgroundColor>(resume).unwrap().0,
            theme::PRIMARY
        );
        app.world_mut()
            .entity_mut(resume)
            .insert(Interaction::Hovered);
        app.update();
        assert_eq!(
            app.world().get::<BackgroundColor>(resume).unwrap().0,
            theme::PRIMARY_HOVER
        );
        // A disabled button never lights.
        app.world_mut()
            .get_mut::<Pressable>(resume)
            .unwrap()
            .disabled = true;
        app.update();
        assert_eq!(
            app.world().get::<BackgroundColor>(resume).unwrap().0,
            theme::PRIMARY
        );
    }
}
