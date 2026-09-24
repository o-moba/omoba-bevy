//! Cosmetic edge mist. Deliberately does not change team vision or network visibility.
use crate::net::{GameState, GameStateSnapshot};
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    ui::FocusPolicy,
};

pub(crate) struct BattlefieldAtmospherePlugin;
impl Plugin for BattlefieldAtmospherePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, sync_visibility);
    }
}
#[derive(Component)]
struct BattlefieldMist;

fn opacity(uv: Vec2) -> f32 {
    let p = (uv - Vec2::splat(0.5)) * 2.0;
    let radius = (p * Vec2::new(0.92, 1.1)).length();
    let t = ((radius - 0.64) / 0.78).clamp(0.0, 1.0);
    0.48 * t * t * (3.0 - 2.0 * t)
}
fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    const SIDE: u32 = 256;
    let mut data = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let uv = (Vec2::new(x as f32, y as f32) + Vec2::splat(0.5)) / SIDE as f32;
            data.extend_from_slice(&[9, 25, 32, (opacity(uv) * 255.0).round() as u8]);
        }
    }
    let texture = images.add(Image::new(
        Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ));
    commands.spawn((
        Name::new("Battlefield edge mist"),
        BattlefieldMist,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            display: Display::None,
            ..default()
        },
        ImageNode::new(texture),
        GlobalZIndex(-100),
        FocusPolicy::Pass,
        Pickable::IGNORE,
    ));
}
fn sync_visibility(
    game: Option<Res<GameStateSnapshot>>,
    mut mist: Query<&mut Node, With<BattlefieldMist>>,
) {
    let visible = game.is_some_and(|g| matches!(g.state, GameState::Running));
    for mut node in &mut mist {
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_mist_is_soft_symmetric_and_center_is_clear() {
        assert_eq!(opacity(Vec2::splat(0.5)), 0.0);
        assert_eq!(opacity(Vec2::new(0.7, 0.5)), 0.0);
        for i in 0..100 {
            let u = i as f32 / 99.0;
            assert!(
                (opacity(Vec2::new(u, 0.2)) - opacity(Vec2::new(1.0 - u, 0.8))).abs() < 0.00001
            );
            assert!((0.0..=0.48).contains(&opacity(Vec2::new(u, 0.0))));
        }
        assert!(opacity(Vec2::ZERO) > 0.45);
    }
    #[test]
    fn overlay_is_single_noninteractive_full_viewport_and_match_scoped() {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>()
            .init_resource::<GameStateSnapshot>()
            .add_plugins(BattlefieldAtmospherePlugin);
        app.update();
        let entity = app
            .world_mut()
            .query_filtered::<Entity, With<BattlefieldMist>>()
            .single(app.world())
            .unwrap();
        let node = app.world().get::<Node>(entity).unwrap();
        assert_eq!(node.width, Val::Percent(100.0));
        assert_eq!(node.display, Display::None);
        assert_eq!(app.world().get::<Pickable>(entity), Some(&Pickable::IGNORE));
        assert_eq!(
            *app.world().get::<FocusPolicy>(entity).unwrap(),
            FocusPolicy::Pass
        );
        assert!(app.world().get::<GlobalZIndex>(entity).unwrap().0 < 0);
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Running;
        app.update();
        assert_eq!(
            app.world().get::<Node>(entity).unwrap().display,
            Display::Flex
        );
        app.world_mut().resource_mut::<GameStateSnapshot>().state = GameState::Lobby;
        app.update();
        assert_eq!(
            app.world().get::<Node>(entity).unwrap().display,
            Display::None
        );
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<BattlefieldMist>>()
                .iter(app.world())
                .count(),
            1
        );
    }
}
