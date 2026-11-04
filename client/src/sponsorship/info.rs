//! Approved creator text is a plain, read-only modal, never a gameplay target.
use super::{Desired, Sculpture};
use crate::{
    camera::MainCamera,
    input_context::{GameplayInputContext, InputContextSet},
    ui::{
        Activated, BackInput, GestureEpoch, ModalAppExt, ModalId, ModalRoot, ModalStack,
        ScrollArea, TestId, UiActionAppExt, UiSet, gesture::TapTracker, theme, widgets,
    },
};
use bevy::{
    ecs::system::SystemParam,
    input::touch::{TouchInput, TouchPhase},
    picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings},
    prelude::*,
    window::PrimaryWindow,
};
use omoba_passport::sponsorship::Placement;

#[derive(Resource, Default)]
pub(crate) struct SculptureInfo {
    pub(crate) open: Option<Placement>,
    // Modal flags alone would become false on the closing frame, allowing the
    // same mouse/touch release to issue a move or attack. Keep its entire tail.
    consumed_frame: bool,
    swallow_mouse: bool,
    captured_touch: Option<u64>,
}
impl SculptureInfo {
    pub(crate) fn blocks_world(&self) -> bool {
        self.open.is_some()
            || self.consumed_frame
            || self.swallow_mouse
            || self.captured_touch.is_some()
    }
    fn close(&mut self, epoch: &mut GestureEpoch) {
        if self.open.take().is_some() {
            self.consumed_frame = true;
            epoch.bump();
        }
    }
}
#[derive(Clone, Copy)]
enum Action {
    Close,
}
#[derive(Component)]
struct InfoRoot;
#[derive(Component)]
pub(crate) struct InfoScroll;
pub(super) struct SculptureInfoPlugin;
impl Plugin for SculptureInfoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SculptureInfo>()
            .init_resource::<GestureEpoch>()
            .register_modal::<SculptureInfo>(ModalId::Sculpture, |info| info.open.is_some())
            .add_ui_action::<Action>()
            .add_systems(
                Update,
                (validate_open, interact)
                    .chain()
                    .in_set(InputContextSet::Modal)
                    .after(UiSet::Dispatch)
                    .before(crate::pause_menu::toggle_pause_menu),
            )
            .add_systems(Update, render.after(InputContextSet::Modal))
            .add_systems(Last, |mut info: ResMut<SculptureInfo>| {
                info.consumed_frame = false
            });
    }
}
fn validate_open(
    desired: Res<Desired>,
    mut info: ResMut<SculptureInfo>,
    mut epoch: ResMut<GestureEpoch>,
) {
    if info
        .open
        .as_ref()
        .is_some_and(|open| desired.0.as_ref().map(|ready| &ready.placement) != Some(open))
    {
        info.close(&mut epoch);
    }
}
#[derive(SystemParam)]
struct Picking<'w, 's> {
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<MainCamera>>,
    roots: Query<'w, 's, &'static Sculpture>,
    parents: Query<'w, 's, &'static ChildOf>,
    rays: MeshRayCast<'w, 's>,
}
impl Picking<'_, '_> {
    fn sculpture(&mut self, point: Vec2, desired: &Desired) -> Option<Entity> {
        let ready = desired.0.as_ref()?;
        let (camera, transform) = self.cameras.single().ok()?;
        let ray = camera.viewport_to_world(transform, point).ok()?;
        // Use actual rendered triangles, nearest visible geometry first. An
        // intervening tree or hero must not turn into a hidden advert click.
        let mut entity = self
            .rays
            .cast_ray(ray, &MeshRayCastSettings::default())
            .first()?
            .0;
        for _ in 0..67 {
            if let Ok(root) = self.roots.get(entity) {
                return (root.identity
                    == format!(
                        "{}:{}",
                        ready.placement.id, ready.placement.rendition_sha256
                    ))
                .then_some(entity);
            }
            entity = self.parents.get(entity).ok()?.parent();
        }
        None
    }
}
fn interact(
    mut info: ResMut<SculptureInfo>,
    desired: Res<Desired>,
    mut picking: Picking,
    mut tracker: Local<TapTracker>,
    mut epoch: ResMut<GestureEpoch>,
    mut prior_epoch: Local<Option<u64>>,
    mut back: BackInput,
    mut actions: MessageReader<Activated<Action>>,
    modals: Res<ModalStack>,
    context: Res<GameplayInputContext>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut touches: MessageReader<TouchInput>,
    buttons: Query<&Interaction, With<Node>>,
) {
    if info.swallow_mouse {
        info.consumed_frame = true;
        info.swallow_mouse = mouse.pressed(MouseButton::Left);
    }
    if *prior_epoch != Some(epoch.0) {
        tracker.clear();
        // Navigation cancels activation, but the held finger still owns its
        // release: do not hand that tail back to world movement.
        *prior_epoch = Some(epoch.0);
    }
    let Ok((window_entity, window)) = windows.single() else {
        touches.clear();
        return;
    };
    if !window.focused {
        tracker.clear();
        info.captured_touch = None;
        touches.clear();
        return;
    }
    if info.open.is_some() {
        info.swallow_mouse |= mouse.pressed(MouseButton::Left);
        touches.clear();
        if modals.top() == Some(ModalId::Sculpture) {
            let close = actions.read().next().is_some();
            if close || back.just_pressed() {
                if back.just_pressed() {
                    back.consume();
                }
                info.close(&mut epoch);
            }
        } else {
            actions.clear();
        }
        return;
    }
    actions.clear();
    if modals.is_open() || !context.gameplay_allowed() && info.captured_touch.is_none() {
        tracker.clear();
        if let Some(id) = info.captured_touch {
            info.consumed_frame = true;
            if touches.read().any(|touch| {
                touch.window == window_entity
                    && touch.id == id
                    && matches!(touch.phase, TouchPhase::Ended | TouchPhase::Canceled)
            }) {
                info.captured_touch = None;
            }
        } else {
            touches.clear();
        }
        return;
    }
    let over_ui = buttons.iter().any(|value| *value != Interaction::None);
    let mut activate = false;
    for touch in touches.read().filter(|touch| touch.window == window_entity) {
        let hit = (!over_ui)
            .then(|| picking.sculpture(touch.position, &desired))
            .flatten();
        let targets = hit.map(|entity| (entity, Rect::from_center_size(touch.position, Vec2::ONE)));
        if touch.phase == TouchPhase::Started && hit.is_some() && info.captured_touch.is_none() {
            info.captured_touch = Some(touch.id);
        }
        if info.captured_touch == Some(touch.id) {
            info.consumed_frame = true;
            activate |= tracker
                .event(touch.id, touch.phase, touch.position, targets.as_slice())
                .is_some();
            if matches!(touch.phase, TouchPhase::Ended | TouchPhase::Canceled) {
                info.captured_touch = None;
            }
        }
    }
    if !over_ui
        && mouse.just_pressed(MouseButton::Left)
        && let Some(point) = window.cursor_position()
        && picking.sculpture(point, &desired).is_some()
    {
        info.swallow_mouse = true;
        activate = true;
    }
    if activate && let Some(ready) = &desired.0 {
        info.open = Some(ready.placement.clone());
        info.consumed_frame = true;
        info.captured_touch = None;
        tracker.clear();
        epoch.bump();
    }
}
fn render(
    mut commands: Commands,
    info: Res<SculptureInfo>,
    form: Res<theme::UiForm>,
    roots: Query<Entity, With<InfoRoot>>,
    mut previous: Local<Option<Placement>>,
) {
    if *previous == info.open {
        return;
    }
    *previous = info.open.clone();
    for root in &roots {
        commands.entity(root).despawn();
    }
    let Some(placement) = &info.open else { return };
    let mut panel = Entity::PLACEHOLDER;
    commands
        .spawn((
            InfoRoot,
            ModalRoot(ModalId::Sculpture),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.),
                height: Val::Percent(100.),
                ..default()
            },
            ZIndex(ModalId::Sculpture.layer()),
        ))
        .with_children(|root| {
            panel = widgets::surfaces::modal(
                root,
                crate::i18n::Localized::new("sculpture.title"),
                Action::Close,
                form.0,
                "SculptureInfo",
            );
        });
    commands.entity(panel).with_children(|panel| {
        panel
            .spawn((
                InfoScroll,
                TestId::new("SculptureInfoScroll"),
                Node {
                    max_height: Val::Vh(62.),
                    min_height: Val::Px(0.),
                    overflow: Overflow::scroll_y(),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(12.),
                    ..default()
                },
                ScrollArea::menu(32.).page_keys(200.),
            ))
            .with_children(|body| {
                body.spawn((
                    Text::new(&placement.title),
                    TextLayout::linebreak(bevy::text::LineBreak::WordOrCharacter),
                    theme::styled_text(theme::TextStyle::keep_case(
                        crate::ui::tokens::TextRole::Heading,
                    )),
                    TextColor(theme::GOLD),
                    Node {
                        flex_shrink: 0.,
                        max_width: Val::Percent(100.),
                        ..default()
                    },
                ));
                if !placement.credit.is_empty() {
                    body.spawn((
                        Text::new(&placement.credit),
                        TextLayout::linebreak(bevy::text::LineBreak::WordOrCharacter),
                        theme::styled_text(theme::TextStyle::keep_case(
                            crate::ui::tokens::TextRole::Body,
                        )),
                        TextColor(theme::JADE),
                        Node {
                            flex_shrink: 0.,
                            max_width: Val::Percent(100.),
                            ..default()
                        },
                    ));
                }
                if !placement.description.is_empty() {
                    // Bevy Text is literal. No Markdown/HTML parser or link handler.
                    body.spawn((
                        Text::new(&placement.description),
                        TextLayout::linebreak(bevy::text::LineBreak::WordOrCharacter),
                        theme::styled_text(theme::TextStyle::keep_case(
                            crate::ui::tokens::TextRole::Body,
                        )),
                        TextColor(theme::IVORY),
                        Node {
                            flex_shrink: 0.,
                            max_width: Val::Percent(100.),
                            ..default()
                        },
                    ));
                }
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        input_context::InputContextPlugin,
        net::{GameState, GameStateSnapshot},
    };
    use bevy::camera::{
        CameraProjection, ComputedCameraValues, RenderTargetInfo, primitives::Aabb,
    };

    fn app() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Desired>()
            .insert_resource(theme::UiForm(theme::Form::Desktop))
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .add_message::<TouchInput>()
            .add_plugins((InputContextPlugin, SculptureInfoPlugin))
            .configure_sets(
                Update,
                UiSet::Dispatch.after(crate::ui::modal::ModalSet::Early),
            );
        let mut window = Window {
            resolution: (1280, 720).into(),
            ..default()
        };
        window.set_cursor_position(Some(Vec2::new(640., 360.)));
        let window = app.world_mut().spawn((window, PrimaryWindow)).id();
        let projection = PerspectiveProjection {
            aspect_ratio: 1280. / 720.,
            ..default()
        };
        let camera = Camera {
            computed: ComputedCameraValues {
                clip_from_view: projection.get_clip_from_view(),
                target_info: Some(RenderTargetInfo {
                    physical_size: UVec2::new(1280, 720),
                    scale_factor: 1.,
                }),
                ..default()
            },
            ..default()
        };
        app.world_mut().spawn((
            camera,
            MainCamera,
            GlobalTransform::from_translation(Vec3::new(0., 0., 5.)),
        ));
        let ready = omoba_passport::sponsorship::Ready {
            placement: Placement {
                id: "one".into(),
                slot_id: "opposite-dragon".into(),
                asset_id: "asset".into(),
                revision_id: "revision".into(),
                rendition_sha256: "hash".into(),
                rendition_bytes: 1,
                rendition_url: "https://registry.test".into(),
                title: "  <b>Cat</b>  ".into(),
                credit: "Artist".into(),
                description: "  First line\n<b>Plain</b> https://test.invalid  ".into(),
                starts_at: String::new(),
                expires_at: String::new(),
            },
            asset_path: String::new(),
            fit: omoba_passport::sponsorship::model::ModelFit {
                scale: 1.,
                offset: [0.; 3],
            },
        };
        app.world_mut().resource_mut::<Desired>().0 = Some(ready);
        let root = app
            .world_mut()
            .spawn(Sculpture {
                identity: "one:hash".into(),
                gltf: default(),
                graph: None,
            })
            .id();
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::new(2., 2., 2.));
        app.world_mut().spawn((
            Mesh3d(mesh),
            GlobalTransform::IDENTITY,
            Aabb::from_min_max(Vec3::splat(-1.), Vec3::ONE),
            InheritedVisibility::VISIBLE,
            ViewVisibility::VISIBLE,
            ChildOf(root),
        ));
        app.update();
        (app, window)
    }
    fn mouse(app: &mut App, pressed: bool) {
        let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        buttons.clear();
        if pressed {
            buttons.press(MouseButton::Left);
        } else {
            buttons.release(MouseButton::Left);
        }
        app.update();
    }
    fn assert_blocked(app: &App) {
        let context = app.world().resource::<GameplayInputContext>();
        assert!(
            !context.gameplay_allowed(),
            "movement/combat gate must remain closed in this frame"
        );
        assert!(!context.camera_allowed());
    }
    #[test]
    fn click_actual_mesh_close_control_and_release_never_pass_through() {
        let (mut app, window) = app();
        mouse(&mut app, true);
        let expected = app
            .world()
            .resource::<Desired>()
            .0
            .as_ref()
            .unwrap()
            .placement
            .clone();
        assert_eq!(
            app.world().resource::<SculptureInfo>().open,
            Some(expected.clone())
        );
        assert_eq!(
            app.world().resource::<ModalStack>().top(),
            Some(ModalId::Sculpture)
        );
        assert_blocked(&app);
        // The card carries exact literal text, not parsed HTML or a URL action.
        let texts: Vec<_> = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|text| text.0.clone())
            .collect();
        assert!(texts.contains(&expected.title));
        assert!(texts.contains(&expected.description));
        mouse(&mut app, false);
        let close = app
            .world_mut()
            .query::<(Entity, &TestId)>()
            .iter(app.world())
            .find(|(_, id)| id.as_str() == "SculptureInfoClose")
            .unwrap()
            .0;
        // The same typed action as a real kit button click/tap.
        app.world_mut().write_message(Activated {
            action: Action::Close,
            source: close,
        });
        mouse(&mut app, true);
        assert!(app.world().resource::<SculptureInfo>().open.is_none());
        assert_blocked(&app);
        mouse(&mut app, false);
        assert_blocked(&app);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.update();
        assert!(
            app.world()
                .resource::<GameplayInputContext>()
                .gameplay_allowed()
        );
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(10., 10.)));
        mouse(&mut app, true);
        assert!(app.world().resource::<SculptureInfo>().open.is_none());
        assert!(
            app.world()
                .resource::<GameplayInputContext>()
                .gameplay_allowed()
        );
    }
    fn touch(app: &mut App, window: Entity, phase: TouchPhase, point: Vec2) {
        app.world_mut().write_message(TouchInput {
            window,
            phase,
            position: point,
            id: 7,
            force: None,
        });
        app.update();
    }
    #[test]
    fn touch_tap_uses_actual_mesh_and_consumes_start_drag_release_and_back() {
        let (mut app, window) = app();
        let point = Vec2::new(640., 360.);
        touch(&mut app, window, TouchPhase::Started, point);
        assert!(app.world().resource::<SculptureInfo>().open.is_none());
        assert_blocked(&app);
        touch(&mut app, window, TouchPhase::Ended, point);
        assert!(app.world().resource::<SculptureInfo>().open.is_some());
        assert_blocked(&app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        assert!(app.world().resource::<SculptureInfo>().open.is_none());
        assert!(
            !app.world()
                .resource::<ButtonInput<KeyCode>>()
                .just_pressed(KeyCode::Escape)
        );
        assert_blocked(&app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.update();
        touch(&mut app, window, TouchPhase::Started, point);
        touch(
            &mut app,
            window,
            TouchPhase::Moved,
            point + Vec2::new(30., 0.),
        );
        assert_blocked(&app);
        touch(&mut app, window, TouchPhase::Ended, point);
        assert!(
            app.world().resource::<SculptureInfo>().open.is_none(),
            "drag back is not a tap"
        );
        assert_blocked(&app);
    }
    #[test]
    fn other_modal_occlusion_and_missing_authorization_cannot_open_card() {
        let (mut app, _) = app();
        app.insert_resource(crate::pause_menu::PauseMenuState {
            open: true,
            ..default()
        });
        app.update();
        mouse(&mut app, true);
        assert!(app.world().resource::<SculptureInfo>().open.is_none());
        app.world_mut()
            .resource_mut::<crate::pause_menu::PauseMenuState>()
            .open = false;
        mouse(&mut app, false);
        app.update();
        // Visible foreground geometry occludes the sculpture's triangles.
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::new(3., 3., 1.));
        let obstruction = app
            .world_mut()
            .spawn((
                Mesh3d(mesh),
                GlobalTransform::from_translation(Vec3::new(0., 0., 3.)),
                Aabb::from_min_max(Vec3::new(-1.5, -1.5, -0.5), Vec3::new(1.5, 1.5, 0.5)),
                InheritedVisibility::VISIBLE,
                ViewVisibility::VISIBLE,
            ))
            .id();
        mouse(&mut app, true);
        assert!(app.world().resource::<SculptureInfo>().open.is_none());
        app.world_mut().despawn(obstruction);
        mouse(&mut app, false);
        app.world_mut().resource_mut::<Desired>().0 = None;
        mouse(&mut app, true);
        assert!(app.world().resource::<SculptureInfo>().open.is_none());
    }
    #[test]
    fn any_authorization_loss_or_replacement_closes_and_consumes_current_frame() {
        for variant in 0..3 {
            let (mut app, _) = app();
            mouse(&mut app, true);
            assert!(app.world().resource::<SculptureInfo>().open.is_some());
            mouse(&mut app, false);
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .clear();
            match variant {
                0 => app.world_mut().resource_mut::<Desired>().0 = None,
                1 => {
                    app.world_mut()
                        .resource_mut::<Desired>()
                        .0
                        .as_mut()
                        .unwrap()
                        .placement
                        .id = "replacement".into()
                }
                _ => {
                    app.world_mut()
                        .resource_mut::<Desired>()
                        .0
                        .as_mut()
                        .unwrap()
                        .placement
                        .description = "changed text".into()
                }
            }
            app.update();
            assert!(app.world().resource::<SculptureInfo>().open.is_none());
            assert!(
                app.world_mut()
                    .query::<&InfoRoot>()
                    .iter(app.world())
                    .next()
                    .is_none()
            );
            assert_blocked(&app);
        }
    }
    #[test]
    fn empty_description_is_clean_and_creator_text_never_changes_case_or_clips_long_words() {
        let (mut app, _) = app();
        app.world_mut()
            .resource_mut::<Desired>()
            .0
            .as_mut()
            .unwrap()
            .placement
            .description
            .clear();
        mouse(&mut app, true);
        let texts: Vec<_> = app
            .world_mut()
            .query::<(&Text, &theme::TextStyle, &TextLayout)>()
            .iter(app.world())
            .filter(|(text, _, _)| text.0 == "  <b>Cat</b>  " || text.0 == "Artist")
            .collect();
        assert_eq!(texts.len(), 2);
        for (_, style, layout) in texts {
            assert!(
                style.keep_case,
                "the kit must not uppercase approved creator content"
            );
            assert_eq!(layout.linebreak, bevy::text::LineBreak::WordOrCharacter);
        }
        assert!(
            !app.world_mut()
                .query::<&Text>()
                .iter(app.world())
                .any(|text| text.0.is_empty())
        );
    }
    #[test]
    fn modal_navigation_cancels_a_held_tap_but_retains_its_release_guard() {
        let (mut app, window) = app();
        let point = Vec2::new(640., 360.);
        touch(&mut app, window, TouchPhase::Started, point);
        app.insert_resource(crate::pause_menu::PauseMenuState {
            open: true,
            ..default()
        });
        app.world_mut().resource_mut::<GestureEpoch>().bump();
        app.update();
        assert_blocked(&app);
        app.world_mut()
            .resource_mut::<crate::pause_menu::PauseMenuState>()
            .open = false;
        app.world_mut().resource_mut::<GestureEpoch>().bump();
        app.update();
        assert_blocked(&app);
        touch(&mut app, window, TouchPhase::Ended, point);
        assert!(app.world().resource::<SculptureInfo>().open.is_none());
        assert_blocked(&app);
        app.update();
        assert!(
            app.world()
                .resource::<GameplayInputContext>()
                .gameplay_allowed()
        );
    }
}
