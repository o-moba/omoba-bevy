//! Waiting and failure feedback: the status ring (a timer ring that can
//! count down, turn while nothing is known, or show an error), the small
//! spinner and skeleton bars that pulse where a value has not arrived yet
//! (`omoba-ui/handoff/screens/{loading-shell,prematch-countdown,result,
//! post-match}.md`; the ring itself is `components/timer-ring.md`).
//!
//! The owner keeps [`RingMode`] (and nothing else) up to date; the painters
//! below turn it into the atlas frame, the arc rotation, the number and the
//! alert icon.
// i18n-strict
use std::f32::consts::TAU;

use bevy::prelude::*;

use super::{
    KitParts,
    game::{RingArc, RingSize, ring_frame, timer_ring_layers},
    icon_node,
};
use crate::ui::{
    kit_assets::{Icon, KitImage, Sprite},
    theme,
    tokens::{border, color, radius, size},
};

/// One turn of an indeterminate arc (`loading-shell.md`: 1200 ms, linear).
pub(crate) const SPIN_PERIOD_SECS: f32 = 1.2;
/// The indeterminate arc: a quarter of the ring.
pub(crate) const SPIN_ARC: f32 = 0.25;
/// Skeleton bars pulse over 900 ms (`result.md`, `post-match.md`).
pub(crate) const SKELETON_PULSE_SECS: f32 = 0.9;
/// Spinner sides: 12 px in a caption line, 16 px in a button.
pub(crate) const SPINNER_SM: f32 = 12.0;
pub(crate) const SPINNER_MD: f32 = 16.0;

/// What a status ring shows.
#[derive(Component, Clone, PartialEq, Debug)]
pub(crate) enum RingMode {
    /// A draining countdown: `progress` of the arc left, `number` in the disc.
    Countdown { progress: f32, number: u32 },
    /// Waiting with no known end: a 90° arc turning once per
    /// [`SPIN_PERIOD_SECS`], empty disc.
    Indeterminate,
    /// Failed: the track only and `nav/alert-triangle` in `color.text.danger`.
    Error,
}

/// An arc layer that turns on its own (indeterminate rings and spinners).
#[derive(Component, Clone, Copy, Default)]
pub(crate) struct Spin;

/// A status ring of `ring_size`: track `color.surface.3`, arc
/// `color.gold.400` (never the ≤ 5 s warning colour: the screens that use it
/// count 3 s), centre disc `color.surface.1.opaque`, the number in the
/// size's number role and a hidden alert icon for [`RingMode::Error`].
pub(crate) fn status_ring(
    parent: &mut ChildSpawnerCommands,
    mode: RingMode,
    ring_size: RingSize,
) -> Entity {
    let (side, _, role) = ring_size.metrics();
    let disc = ring_size.disc();
    let icon = if ring_size == RingSize::Small {
        size::ICON_MD
    } else {
        size::ICON_LG
    };
    let mut parts = KitParts::default();
    let mut root = parent.spawn((
        Node {
            width: Val::Px(side),
            height: Val::Px(side),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        Pickable::IGNORE,
    ));
    root.with_children(|root| {
        timer_ring_layers(root, 1.0, color::SURFACE_3, color::GOLD_400);
        root.spawn((
            Node {
                width: Val::Px(disc),
                height: Val::Px(disc),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                ..default()
            },
            BackgroundColor(color::SURFACE_1_OPAQUE),
            Pickable::IGNORE,
        ))
        .with_children(|disc| {
            parts.label = Some(
                disc.spawn((
                    Text::new(""),
                    theme::role_text(role),
                    TextColor(color::TEXT_PRIMARY),
                ))
                .id(),
            );
            parts.icon = Some(
                disc.spawn((
                    icon_node(Icon::NavAlertTriangle, icon, color::TEXT_DANGER),
                    Visibility::Hidden,
                ))
                .id(),
            );
        });
    });
    root.insert((mode, parts)).id()
}

/// A small indeterminate spinner `side` px (ring `color.surface.3`, arc
/// `color.gold.400`): the saving line and a pressed Play again.
pub(crate) fn spinner(parent: &mut ChildSpawnerCommands, side: f32) -> Entity {
    parent
        .spawn((
            Node {
                width: Val::Px(side),
                height: Val::Px(side),
                flex_shrink: 0.0,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|spinner| {
            spinner.spawn((
                full_layer(),
                KitImage::atlas(Sprite::TimerRingAtlas, color::SURFACE_3, ring_frame(1.0)),
                crate::ui::kit_assets::LowDensity,
                Pickable::IGNORE,
            ));
            spinner.spawn((
                full_layer(),
                KitImage::atlas(
                    Sprite::TimerRingAtlas,
                    color::GOLD_400,
                    ring_frame(SPIN_ARC),
                ),
                crate::ui::kit_assets::LowDensity,
                UiTransform::IDENTITY,
                Spin,
                Pickable::IGNORE,
            ));
        })
        .id()
}

fn full_layer() -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(0.0),
        right: Val::Px(0.0),
        top: Val::Px(0.0),
        bottom: Val::Px(0.0),
        ..default()
    }
}

/// A value that has not arrived: a `color.surface.3` bar (or ring outline)
/// that pulses.
#[derive(Component, Clone, Copy, Default)]
pub(crate) struct Skeleton {
    /// Pulse the outline (a ring) instead of the fill (a bar).
    pub ring: bool,
}

/// A skeleton bar `width` × `height` (`radius.sm`).
pub(crate) fn skeleton(width: Val, height: Val) -> impl Bundle {
    (
        Node {
            width,
            height,
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(Val::Px(radius::SM)),
            ..default()
        },
        BackgroundColor(color::SURFACE_3),
        Skeleton { ring: false },
        Pickable::IGNORE,
    )
}

/// A skeleton ring (the progress strip's level badge before the result):
/// `side` circle outlined in `color.surface.3`.
pub(crate) fn skeleton_ring(side: f32) -> impl Bundle {
    (
        Node {
            width: Val::Px(side),
            height: Val::Px(side),
            flex_shrink: 0.0,
            border: UiRect::all(Val::Px(border::FRAME)),
            border_radius: BorderRadius::all(Val::Px(radius::PILL)),
            ..default()
        },
        BorderColor::all(color::SURFACE_3),
        Skeleton { ring: true },
        Pickable::IGNORE,
    )
}

/// Arc frame, visibility, number and alert icon of every status ring whose
/// mode changed; the arc spins while it is indeterminate.
pub(crate) fn paint_status_rings(
    mut commands: Commands,
    rings: Query<(&RingMode, &KitParts, &Children), Changed<RingMode>>,
    mut arcs: Query<(Entity, &mut KitImage, &mut Visibility, &mut UiTransform), With<RingArc>>,
    mut texts: Query<&mut Text>,
    mut icons: Query<&mut Visibility, (Without<RingArc>, Without<Text>)>,
) {
    for (mode, parts, children) in &rings {
        let (frame, arc_shown, spin, number, alert) = match mode {
            RingMode::Countdown { progress, number } => (
                ring_frame(*progress),
                true,
                false,
                number.to_string(),
                false,
            ),
            RingMode::Indeterminate => (ring_frame(SPIN_ARC), true, true, String::new(), false),
            RingMode::Error => (ring_frame(0.0), false, false, String::new(), true),
        };
        for child in children.iter() {
            let Ok((entity, mut image, mut visibility, mut transform)) = arcs.get_mut(child) else {
                continue;
            };
            if image.frame != Some(frame) {
                image.frame = Some(frame);
            }
            let shown = if arc_shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *visibility != shown {
                *visibility = shown;
            }
            if spin {
                commands.entity(entity).insert(Spin);
            } else {
                commands.entity(entity).remove::<Spin>();
                if transform.rotation != Rot2::IDENTITY {
                    transform.rotation = Rot2::IDENTITY;
                }
            }
        }
        if let Some(mut text) = parts.label.and_then(|label| texts.get_mut(label).ok())
            && text.0 != number
        {
            text.0 = number;
        }
        if let Some(mut visibility) = parts.icon.and_then(|icon| icons.get_mut(icon).ok()) {
            let shown = if alert {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *visibility != shown {
                *visibility = shown;
            }
        }
    }
}

/// Turns every spinning arc: one turn per [`SPIN_PERIOD_SECS`], clockwise.
pub(crate) fn spin_arcs(time: Res<Time>, mut arcs: Query<&mut UiTransform, With<Spin>>) {
    let turn = (time.elapsed_secs() / SPIN_PERIOD_SECS).fract();
    let rotation = Rot2::radians(turn * TAU);
    for mut transform in &mut arcs {
        transform.rotation = rotation;
    }
}

/// Skeleton alpha at `t` seconds: 1.0 → 0.5 → 1.0 over the pulse.
pub(crate) fn skeleton_alpha(t: f32) -> f32 {
    0.75 + 0.25 * (t / SKELETON_PULSE_SECS * TAU).cos()
}

/// Pulses every skeleton's fill (bars) or outline (rings).
pub(crate) fn pulse_skeletons(
    time: Res<Time>,
    mut bars: Query<(&Skeleton, &mut BackgroundColor, &mut BorderColor)>,
) {
    let alpha = skeleton_alpha(time.elapsed_secs());
    let tint = color::SURFACE_3.with_alpha(alpha);
    for (skeleton, mut fill, mut outline) in &mut bars {
        if skeleton.ring {
            *outline = BorderColor::all(tint);
        } else {
            fill.0 = tint;
        }
    }
}

/// Every system of this module.
pub(crate) fn add_systems(app: &mut App) {
    app.add_systems(
        Update,
        (paint_status_rings, spin_arcs, pulse_skeletons)
            .chain()
            .in_set(crate::ui::UiSet::Paint)
            .before(crate::ui::kit_assets::resolve_kit_images),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring_app(mode: RingMode) -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<Time>();
        add_systems(&mut app);
        let root = app.world_mut().spawn(Node::default()).id();
        let mut ring = Entity::PLACEHOLDER;
        app.world_mut()
            .commands()
            .entity(root)
            .with_children(|parent| ring = status_ring(parent, mode, RingSize::Medium));
        app.world_mut().flush();
        app.update();
        app.update();
        (app, ring)
    }

    fn arc(app: &mut App) -> (Visibility, Option<usize>, bool) {
        let mut arcs = app
            .world_mut()
            .query_filtered::<(&Visibility, &KitImage, Has<Spin>), With<RingArc>>();
        let (visibility, image, spin) = arcs.single(app.world()).unwrap();
        (*visibility, image.frame, spin)
    }

    #[test]
    fn a_status_ring_counts_down_turns_and_fails() {
        let (mut app, ring) = ring_app(RingMode::Countdown {
            progress: 0.5,
            number: 2,
        });
        assert_eq!(arc(&mut app), (Visibility::Inherited, Some(30), false));
        let parts = *app.world().get::<KitParts>(ring).unwrap();
        let number = |app: &App| {
            app.world()
                .get::<Text>(parts.label.unwrap())
                .unwrap()
                .0
                .clone()
        };
        let alert = |app: &App| *app.world().get::<Visibility>(parts.icon.unwrap()).unwrap();
        assert_eq!(number(&app), "2");
        assert_eq!(alert(&app), Visibility::Hidden);

        *app.world_mut().get_mut::<RingMode>(ring).unwrap() = RingMode::Indeterminate;
        app.update();
        app.update();
        let (visibility, frame, spin) = arc(&mut app);
        assert_eq!(visibility, Visibility::Inherited);
        assert_eq!(frame, Some(ring_frame(SPIN_ARC)));
        assert!(spin, "an indeterminate arc turns");
        assert_eq!(number(&app), "");

        *app.world_mut().get_mut::<RingMode>(ring).unwrap() = RingMode::Error;
        app.update();
        app.update();
        let (visibility, _, spin) = arc(&mut app);
        assert_eq!(
            visibility,
            Visibility::Hidden,
            "an error shows the track only"
        );
        assert!(!spin);
        assert_eq!(alert(&app), Visibility::Inherited);
    }

    #[test]
    fn spinners_turn_once_per_period_and_skeletons_pulse() {
        assert_eq!(skeleton_alpha(0.0), 1.0);
        assert!((skeleton_alpha(SKELETON_PULSE_SECS / 2.0) - 0.5).abs() < 1e-5);
        let mut app = App::new();
        app.init_resource::<Time>().add_systems(Update, spin_arcs);
        let arc = app.world_mut().spawn((UiTransform::IDENTITY, Spin)).id();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(SPIN_PERIOD_SECS / 4.0));
        app.update();
        let rotation = app.world().get::<UiTransform>(arc).unwrap().rotation;
        assert!(
            (rotation.as_radians() - TAU / 4.0).abs() < 1e-3,
            "{rotation:?}"
        );
    }
}
