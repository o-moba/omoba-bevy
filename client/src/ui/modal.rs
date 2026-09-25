//! The modal registry: which overlays are open and which one is on top.
//!
//! A module registers its modal once with [`ModalAppExt::register_modal`],
//! naming the resource flag that says whether it is open. The registry keeps
//! [`ModalStack`] in step with those flags twice a frame (before the kit's
//! gesture recognizer and again before the input context resolves), so the
//! recognizer, the scroll areas and the gameplay input context all read the
//! same answer. The modal's root node carries [`ModalRoot`]; a `Pressable`
//! or `ScrollArea` belongs to the nearest `ModalRoot` ancestor, and while any
//! modal is open only the top modal's controls react.
use bevy::{ecs::system::SystemParam, prelude::*};

/// Every registered modal. The stack is ordered by [`ModalId::layer`] (the
/// z-order the modal's root is drawn at), then by the order of opening.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ModalId {
    Pause,
    Career,
    Shop,
    Supporter,
    Scoreboard,
    /// The phone server-address overlay (`mobile_ui::ServerEntry`).
    ServerEntry,
}

impl ModalId {
    /// The z-index the modal's root is drawn at. The top of the stack is the
    /// modal the player sees in front, so the phone server-address entry
    /// (`ZIndex(150)`) stays on top of the pause menu (`ZIndex(100)`) even
    /// when the menu was opened while the entry was up. Keep in step with
    /// the roots' `ZIndex`/`GlobalZIndex`.
    pub(crate) fn layer(self) -> i32 {
        match self {
            ModalId::Shop => 45,
            ModalId::Scoreboard => 90,
            ModalId::Pause => 100,
            ModalId::Career => 120,
            ModalId::ServerEntry => 150,
            // `GlobalZIndex(1300)`: above every local z-index.
            ModalId::Supporter => 1300,
        }
    }
}

/// Open modals, bottom first.
#[derive(Resource, Default, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModalStack(Vec<ModalId>);

impl ModalStack {
    /// Opens `id` on top of its layer; opening an open modal changes nothing.
    pub(crate) fn push(&mut self, id: ModalId) {
        if self.0.contains(&id) {
            return;
        }
        let at = self
            .0
            .iter()
            .rposition(|open| open.layer() <= id.layer())
            .map_or(0, |index| index + 1);
        self.0.insert(at, id);
    }

    /// Closes `id` wherever it is in the stack.
    pub(crate) fn pop(&mut self, id: ModalId) {
        self.0.retain(|open| *open != id);
    }

    pub(crate) fn set(&mut self, id: ModalId, open: bool) {
        if open {
            self.push(id);
        } else {
            self.pop(id);
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        !self.0.is_empty()
    }

    pub(crate) fn contains(&self, id: ModalId) -> bool {
        self.0.contains(&id)
    }

    pub(crate) fn top(&self) -> Option<ModalId> {
        self.0.last().copied()
    }
}

/// Marks the root node of a registered modal. Controls below it belong to it.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModalRoot(pub ModalId);

/// Where the registry syncs: `Early` before the kit's gesture recognizer
/// (inside `InputContextSet::Modal`), `Late` at the start of
/// `InputContextSet::Resolve`, after every modal toggled and before the
/// gameplay input context is resolved.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ModalSet {
    Early,
    Late,
}

pub(crate) trait ModalAppExt {
    /// Registers `id` as open whenever the resource `R` exists and `is_open`
    /// says so. The resource may be missing (a desktop build has no
    /// `ServerEntry`), which counts as closed.
    fn register_modal<R: Resource>(&mut self, id: ModalId, is_open: fn(&R) -> bool) -> &mut Self;
}

impl ModalAppExt for App {
    fn register_modal<R: Resource>(&mut self, id: ModalId, is_open: fn(&R) -> bool) -> &mut Self {
        if !self.world().contains_resource::<ModalStack>() {
            self.init_resource::<ModalStack>().configure_sets(
                Update,
                (
                    ModalSet::Early
                        .in_set(crate::input_context::InputContextSet::Modal)
                        .before(super::UiSet::Gesture),
                    ModalSet::Late.in_set(crate::input_context::InputContextSet::Resolve),
                ),
            );
        }
        let sync = move |source: Option<Res<R>>, mut stack: ResMut<ModalStack>| {
            let open = source.is_some_and(|source| is_open(&source));
            if stack.contains(id) != open {
                stack.set(id, open);
            }
        };
        self.add_systems(Update, sync.in_set(ModalSet::Early))
            .add_systems(Update, sync.in_set(ModalSet::Late))
    }
}

/// Top-only gating for kit controls: answers whether an entity may react to
/// input given the open modals.
#[derive(SystemParam)]
pub(crate) struct ModalGate<'w, 's> {
    stack: Option<Res<'w, ModalStack>>,
    parents: Query<'w, 's, &'static ChildOf>,
    roots: Query<'w, 's, &'static ModalRoot>,
}

impl ModalGate<'_, '_> {
    /// The modal `entity` belongs to: its own or its nearest ancestor's
    /// [`ModalRoot`].
    pub(crate) fn owner(&self, entity: Entity) -> Option<ModalId> {
        let mut current = entity;
        loop {
            if let Ok(root) = self.roots.get(current) {
                return Some(root.0);
            }
            current = self.parents.get(current).ok()?.parent();
        }
    }

    /// With no modal open everything reacts; otherwise only controls of the
    /// top modal do (controls outside every modal wait).
    pub(crate) fn allows(&self, entity: Entity) -> bool {
        match self.stack.as_ref().and_then(|stack| stack.top()) {
            None => true,
            Some(top) => self.owner(entity) == Some(top),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn stack_orders_by_draw_layer_and_pops_from_anywhere() {
        let mut stack = ModalStack::default();
        assert!(!stack.is_open());
        assert_eq!(stack.top(), None);
        stack.push(ModalId::Pause);
        assert_eq!(stack.top(), Some(ModalId::Pause));
        stack.push(ModalId::ServerEntry);
        assert_eq!(stack.top(), Some(ModalId::ServerEntry));
        stack.push(ModalId::Pause);
        assert_eq!(
            stack.top(),
            Some(ModalId::ServerEntry),
            "re-opening is a no-op"
        );
        // Opened last but drawn below: the shop does not take the top.
        stack.push(ModalId::Shop);
        stack.push(ModalId::Career);
        assert_eq!(
            stack.0,
            [
                ModalId::Shop,
                ModalId::Pause,
                ModalId::Career,
                ModalId::ServerEntry
            ]
        );
        assert_eq!(stack.top(), Some(ModalId::ServerEntry));
        stack.pop(ModalId::ServerEntry);
        assert_eq!(stack.top(), Some(ModalId::Career));
        stack.pop(ModalId::Pause);
        assert_eq!(stack.top(), Some(ModalId::Career), "pops from the middle");
        stack.pop(ModalId::Career);
        stack.pop(ModalId::Shop);
        assert!(!stack.is_open());
    }

    #[derive(Resource, Default)]
    struct Flag(bool);
    #[derive(Resource, Default)]
    struct Other(bool);

    #[test]
    fn registered_sources_drive_the_stack_and_the_gate_allows_only_the_top_modal() {
        let mut app = App::new();
        app.init_resource::<Flag>()
            .init_resource::<Other>()
            .register_modal::<Flag>(ModalId::Pause, |flag| flag.0)
            .register_modal::<Other>(ModalId::Career, |other| other.0)
            // A missing resource counts as closed.
            .register_modal::<crate::supporter::SupporterUiState>(ModalId::Supporter, |s| s.open);
        let pause = app.world_mut().spawn(ModalRoot(ModalId::Pause)).id();
        let pause_button = app.world_mut().spawn(ChildOf(pause)).id();
        let nested = app.world_mut().spawn(ChildOf(pause_button)).id();
        let career = app.world_mut().spawn(ModalRoot(ModalId::Career)).id();
        let career_button = app.world_mut().spawn(ChildOf(career)).id();
        let loose = app.world_mut().spawn_empty().id();
        let allowed = |app: &mut App| {
            let entities = [nested, career_button, loose];
            app.world_mut()
                .run_system_once(move |gate: ModalGate| entities.map(|e| gate.allows(e)))
                .unwrap()
        };
        app.update();
        assert_eq!(allowed(&mut app), [true, true, true]);
        app.world_mut().resource_mut::<Flag>().0 = true;
        app.update();
        assert_eq!(
            app.world().resource::<ModalStack>().top(),
            Some(ModalId::Pause)
        );
        assert_eq!(allowed(&mut app), [true, false, false]);
        app.world_mut().resource_mut::<Other>().0 = true;
        app.update();
        assert_eq!(allowed(&mut app), [false, true, false]);
        app.world_mut().resource_mut::<Other>().0 = false;
        app.update();
        assert_eq!(allowed(&mut app), [true, false, false]);
        app.world_mut().resource_mut::<Flag>().0 = false;
        app.update();
        assert!(!app.world().resource::<ModalStack>().is_open());
        assert_eq!(allowed(&mut app), [true, true, true]);
    }
}
