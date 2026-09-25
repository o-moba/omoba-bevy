//! Stable identifiers for QA harnesses and tests.
//!
//! A `TestId` mirrors itself into `Name` when the entity has none, so the
//! existing QA node dumps and `Name` lookups keep resolving while modules
//! migrate. Policy: every kit button and value label gets one; layout-only
//! nodes keep a plain `Name`.
use std::borrow::Cow;

use bevy::{
    ecs::{lifecycle::HookContext, world::DeferredWorld},
    prelude::*,
};

#[derive(Component, Clone, Debug, PartialEq, Eq, Hash)]
#[component(on_insert = mirror_into_name)]
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

fn mirror_into_name(mut world: DeferredWorld, context: HookContext) {
    if world.get::<Name>(context.entity).is_some() {
        return;
    }
    let id = world
        .get::<TestId>(context.entity)
        .map(|id| id.as_str().to_owned())
        .unwrap_or_default();
    world
        .commands()
        .entity(context.entity)
        .insert(Name::new(id));
}

/// The string a layout pass or a QA dump matches a node by: a kit control's
/// `TestId`, otherwise its `Name`.
pub(crate) fn node_key<'a>(name: Option<&'a Name>, id: Option<&'a TestId>) -> Option<&'a str> {
    id.map(TestId::as_str).or(name.map(Name::as_str))
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
    fn test_id_mirrors_into_name_only_when_absent() {
        let mut app = App::new();
        app.add_message::<super::super::SyntheticPress>();
        let mirrored = app.world_mut().spawn(TestId::new("KitButton")).id();
        let kept = app
            .world_mut()
            .spawn((TestId::new("KitOther"), Name::new("Custom")))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<Name>(mirrored).unwrap().as_str(),
            "KitButton"
        );
        assert_eq!(app.world().get::<Name>(kept).unwrap().as_str(), "Custom");
        assert_eq!(harness::find(app.world_mut(), "KitOther"), Some(kept));
        assert_eq!(harness::press(app.world_mut(), "KitButton"), mirrored);
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
