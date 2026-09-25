//! Stable identifiers for QA harnesses and tests.
//!
//! Policy: every kit button and rewritable value label gets a `TestId`;
//! layout-only nodes keep a plain `Name`. The two are independent (a
//! `TestId` no longer mirrors itself into `Name`): QA presses look buttons up
//! by `TestId` (`crate::qa::TestIdPresses`), and layout passes and QA dumps
//! that address both kinds of node read [`NodeKey`].
use std::borrow::Cow;

use bevy::{ecs::query::QueryData, prelude::*};

#[derive(Component, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TestId(pub Cow<'static, str>);

impl TestId {
    pub(crate) fn new(id: impl Into<Cow<'static, str>>) -> Self {
        Self(id.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// `"{id}{suffix}"`, for a widget's child controls.
    pub(crate) fn child(&self, suffix: &str) -> Self {
        Self::new(format!("{}{suffix}", self.0))
    }
}

impl From<&str> for TestId {
    fn from(id: &str) -> Self {
        Self(Cow::Owned(id.to_owned()))
    }
}

impl From<String> for TestId {
    fn from(id: String) -> Self {
        Self(Cow::Owned(id))
    }
}

/// The string a layout pass or a QA dump matches a node by: a kit control's
/// `TestId`, otherwise its `Name`.
pub(crate) fn node_key<'a>(name: Option<&'a Name>, id: Option<&'a TestId>) -> Option<&'a str> {
    id.map(TestId::as_str).or(name.map(Name::as_str))
}

/// Query data for [`node_key`]: a node's `Name` and `TestId`, either absent.
#[derive(QueryData)]
pub(crate) struct NodeKey {
    name: Option<&'static Name>,
    id: Option<&'static TestId>,
}

impl NodeKeyItem<'_, '_> {
    /// The `TestId`, else the `Name`; empty for a node that has neither.
    pub(crate) fn as_str(&self) -> &str {
        node_key(self.name, self.id).unwrap_or_default()
    }
}

/// Lookup and press by identifier for tests and harness systems.
#[cfg(test)]
pub(crate) mod harness {
    use super::*;
    use bevy::ecs::system::SystemParam;

    #[derive(SystemParam)]
    pub(crate) struct TestIds<'w, 's> {
        ids: Query<'w, 's, (Entity, &'static TestId)>,
        presses: MessageWriter<'w, super::super::SyntheticPress>,
    }

    impl TestIds<'_, '_> {
        pub(crate) fn find(&self, id: &str) -> Option<Entity> {
            self.ids
                .iter()
                .find(|(_, test_id)| test_id.as_str() == id)
                .map(|(entity, _)| entity)
        }

        /// Queues a synthetic press; false when no entity carries the id.
        pub(crate) fn press(&mut self, id: &str) -> bool {
            match self.find(id) {
                Some(entity) => {
                    self.presses.write(super::super::SyntheticPress(entity));
                    true
                }
                None => false,
            }
        }
    }

    /// Entity carrying `id`, for tests that hold a `World`.
    pub(crate) fn find(world: &mut World, id: &str) -> Option<Entity> {
        world
            .query::<(Entity, &TestId)>()
            .iter(world)
            .find(|(_, test_id)| test_id.as_str() == id)
            .map(|(entity, _)| entity)
    }

    /// Queues a synthetic press for the entity carrying `id`.
    pub(crate) fn press(world: &mut World, id: &str) -> Entity {
        let entity = find(world, id).unwrap_or_else(|| panic!("no TestId {id}"));
        world.write_message(super::super::SyntheticPress(entity));
        entity
    }

    /// A bare app with the kit's frame order: recognizer, then the
    /// dispatchers added with `add_ui_action`, then the painter. Screen tests
    /// add their action type and handler (`.after(UiSet::Dispatch)`).
    pub(crate) fn kit_app() -> App {
        use super::super::{UiSet, gesture, widgets};
        let mut app = App::new();
        app.add_message::<super::super::SyntheticPress>()
            .add_message::<bevy::input::touch::TouchInput>()
            .configure_sets(
                Update,
                (UiSet::Gesture, UiSet::Dispatch, UiSet::Paint).chain(),
            )
            .add_systems(
                Update,
                (
                    gesture::recognize_presses.in_set(UiSet::Gesture),
                    widgets::paint_pressables.in_set(UiSet::Paint),
                ),
            );
        app
    }

    /// Spawns `children` under a fresh root node and applies the commands.
    pub(crate) fn spawn_ui(
        world: &mut World,
        children: impl FnOnce(&mut ChildSpawnerCommands),
    ) -> Entity {
        let root = world
            .commands()
            .spawn(Node::default())
            .with_children(children)
            .id();
        world.flush();
        root
    }

    /// Marks the button carrying `id` disabled (or enabled again).
    pub(crate) fn set_disabled(world: &mut World, id: &str, disabled: bool) {
        let entity = find(world, id).unwrap_or_else(|| panic!("no TestId {id}"));
        world
            .get_mut::<super::super::Pressable>(entity)
            .unwrap_or_else(|| panic!("{id} is not pressable"))
            .disabled = disabled;
    }

    /// Every `Activated<T>` action still queued, oldest first; clears them.
    pub(crate) fn drain_actions<T: super::super::action::UiActionT>(world: &mut World) -> Vec<T> {
        world
            .resource_mut::<Messages<super::super::Activated<T>>>()
            .drain()
            .map(|activated| activated.action)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_and_name_are_independent_and_node_key_prefers_the_id() {
        let mut app = App::new();
        app.add_message::<super::super::SyntheticPress>();
        let kit = app.world_mut().spawn(TestId::new("KitButton")).id();
        let both = app
            .world_mut()
            .spawn((TestId::new("KitOther"), Name::new("Custom")))
            .id();
        let layout = app.world_mut().spawn(Name::new("Layout")).id();
        app.update();
        assert!(app.world().get::<Name>(kit).is_none(), "no Name mirror");
        let keys: Vec<(Entity, String)> = app
            .world_mut()
            .query::<(Entity, NodeKey)>()
            .iter(app.world())
            .map(|(entity, key)| (entity, key.as_str().to_owned()))
            .collect();
        for (entity, key) in [(kit, "KitButton"), (both, "KitOther"), (layout, "Layout")] {
            assert!(keys.contains(&(entity, key.to_owned())), "{key}");
        }
        assert_eq!(harness::find(app.world_mut(), "KitOther"), Some(both));
        assert_eq!(harness::find(app.world_mut(), "Custom"), None);
        assert_eq!(harness::press(app.world_mut(), "KitButton"), kit);
        assert_eq!(TestId::new("Row").child("-Up").as_str(), "Row-Up");
    }

    #[test]
    fn test_ids_param_finds_and_presses() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.add_message::<super::super::SyntheticPress>();
        let entity = app.world_mut().spawn(TestId::new("Kit")).id();
        let outcome = app
            .world_mut()
            .run_system_once(move |mut ids: harness::TestIds| {
                (
                    ids.find("Kit") == Some(entity),
                    ids.press("Kit"),
                    ids.press("Missing"),
                )
            })
            .unwrap();
        assert_eq!(outcome, (true, true, false));
        let queued: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<super::super::SyntheticPress>>()
            .drain()
            .collect();
        assert_eq!(queued, vec![super::super::SyntheticPress(entity)]);
    }
}
