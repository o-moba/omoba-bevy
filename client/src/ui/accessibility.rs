use accesskit::Role;
use bevy::{
    a11y::{AccessibilityNode, AccessibilitySystems},
    prelude::*,
    text::TextElement,
    ui::AccessibilityUiSystems,
};

use super::Button;

pub(super) fn button_node() -> AccessibilityNode {
    accesskit::Node::new(Role::Button).into()
}

pub(super) fn add_systems(app: &mut App) {
    app.add_systems(
        PostUpdate,
        sync_button_accessibility
            .after(AccessibilityUiSystems)
            .before(AccessibilitySystems::Update),
    );
}

fn sync_button_accessibility(
    mut buttons: Query<
        (
            Entity,
            &mut AccessibilityNode,
            Option<&Children>,
            Option<&AccessibleLabel>,
        ),
        With<Button>,
    >,
    mut text: TextUiReader,
    mut label: Local<String>,
) {
    for (entity, mut node, children, explicit_label) in &mut buttons {
        // ImageNode also supplies accessibility metadata. Keep the button
        // role even when a kit texture is attached or changes.
        if node.role() != Role::Button {
            node.set_role(Role::Button);
        }
        label.clear();
        let next = if let Some(explicit_label) = explicit_label {
            explicit_label.0.as_str()
        } else {
            for entity in std::iter::once(entity).chain(children.into_iter().flatten().copied()) {
                for (_, _, item) in text.iter(entity) {
                    if let TextElement::Text { text, .. } = item
                        && !text.trim().is_empty()
                    {
                        if !label.is_empty() {
                            label.push(' ');
                        }
                        label.push_str(text.trim());
                    }
                }
            }
            label.as_str()
        };
        let next = (!next.is_empty()).then_some(next);
        if node.label() != next {
            if let Some(next) = next {
                node.set_label(next);
            } else {
                node.clear_label();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_buttons_keep_their_roles_and_follow_label_changes() {
        let mut app = App::new();
        add_systems(&mut app);
        app.add_systems(
            PostUpdate,
            (|mut images: Query<&mut AccessibilityNode, Changed<ImageNode>>| {
                for mut image in &mut images {
                    image.set_role(Role::Image);
                }
            })
            .in_set(AccessibilityUiSystems),
        );

        let button = app
            .world_mut()
            .spawn((Button, ImageNode::default(), children![Text::new("Equip")]))
            .id();
        let descendant = app.world().get::<Children>(button).unwrap()[0];

        // The role written by Bevy's ImageNode system is overridden before
        // it can reach the platform accessibility adapter.
        app.world_mut().run_schedule(PostUpdate);
        let node = app.world().get::<AccessibilityNode>(button).unwrap();
        assert_eq!(node.role(), Role::Button);
        assert_eq!(node.label(), Some("Equip"));
        app.world_mut().get_mut::<Text>(descendant).unwrap().0 = "Equipped".into();
        app.world_mut().run_schedule(PostUpdate);
        let node = app.world().get::<AccessibilityNode>(button).unwrap();
        assert_eq!(node.label(), Some("Equipped"));
    }
}
