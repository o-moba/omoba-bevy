//! Read-only community sculpture presentation. No replicated combat components.
use bevy::{gltf::Gltf, prelude::*};
use omoba_passport::sponsorship::{self, Ready};

pub(crate) mod info;

pub struct SponsorshipPlugin;
#[derive(Resource, Default)]
struct Desired(Option<Ready>);
#[derive(Component)]
pub(crate) struct Sculpture {
    pub(crate) identity: String,
    gltf: Handle<Gltf>,
    graph: Option<(
        Handle<AnimationGraph>,
        bevy::animation::graph::AnimationNodeIndex,
    )>,
}
#[derive(Component)]
struct SculptureAnimation;
impl Plugin for SponsorshipPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(info::SculptureInfoPlugin)
            .init_resource::<Desired>()
            .add_systems(
                Update,
                (poll, reconcile, animation_library, bind_animation)
                    .chain()
                    .run_if(crate::legal::bound_and_accepted)
                    .run_if(|| crate::account_region::features().ekza)
                    .before(crate::input_context::InputContextSet::Modal),
            );
    }
}
fn poll(mut desired: ResMut<Desired>, mode: Res<crate::sprite::PlayerVisualMode>) {
    let ready = sponsorship::poll();
    desired.0 = if *mode == crate::sprite::PlayerVisualMode::Models3d {
        ready
    } else {
        None
    };
}
fn reconcile(
    mut commands: Commands,
    desired: Res<Desired>,
    assets: Res<AssetServer>,
    roots: Query<(Entity, &Sculpture)>,
) {
    let key = desired
        .0
        .as_ref()
        .map(|r| format!("{}:{}", r.placement.id, r.placement.rendition_sha256));
    let mut exists = false;
    for (entity, root) in &roots {
        if !exists && key.as_deref() == Some(&root.identity) {
            exists = true;
        } else {
            commands.entity(entity).despawn();
        }
    }
    if exists {
        return;
    }
    let (Some(ready), Some(identity)) = (&desired.0, key) else {
        return;
    };
    let origin = Vec3::from_array(shared::sponsorship::slot_position());
    let gltf = assets.load(ready.asset_path.clone());
    commands
        .spawn((
            Sculpture {
                identity,
                gltf,
                graph: None,
            },
            Transform::from_translation(origin),
            Visibility::default(),
            Name::new("Community sculpture (Registry approved)"),
        ))
        .with_children(|parent| {
            parent.spawn((
                WorldAssetRoot(assets.load(format!("{}#Scene0", ready.asset_path))),
                Transform::from_translation(Vec3::from_array(ready.fit.offset))
                    .with_scale(Vec3::splat(ready.fit.scale)),
                Visibility::default(),
                Name::new("Community sculpture model"),
            ));
        });
}
fn animation_library(
    mut roots: Query<&mut Sculpture>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    for mut root in &mut roots {
        if root.graph.is_some() {
            continue;
        }
        let Some(gltf) = gltfs.get(&root.gltf) else {
            continue;
        };
        // The byte validator allows at most one bounded rotation-only clip.
        let Some(clip) = gltf.animations.first() else {
            continue;
        };
        let (graph, index) = AnimationGraph::from_clip(clip.clone());
        root.graph = Some((graphs.add(graph), index));
    }
}
fn bind_animation(
    mut commands: Commands,
    roots: Query<&Sculpture>,
    parents: Query<&ChildOf>,
    mut players: Query<(Entity, &mut AnimationPlayer), Without<SculptureAnimation>>,
) {
    for (entity, mut player) in &mut players {
        let mut ancestor = entity;
        for _ in 0..66 {
            if let Ok(root) = roots.get(ancestor) {
                if let Some((graph, index)) = &root.graph {
                    player.stop_all();
                    player.play(*index).repeat();
                    commands
                        .entity(entity)
                        .insert((SculptureAnimation, AnimationGraphHandle(graph.clone())));
                }
                break;
            }
            let Ok(parent) = parents.get(ancestor) else {
                break;
            };
            ancestor = parent.parent();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_authorization_despawns_scene_tree_and_duplicates() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_resource::<Desired>()
            .add_systems(Update, reconcile);
        app.world_mut()
            .spawn((
                Sculpture {
                    identity: "old".into(),
                    gltf: Handle::default(),
                    graph: None,
                },
                Transform::default(),
            ))
            .with_children(|p| {
                p.spawn(Name::new("old child"));
            });
        app.update();
        let mut q = app.world_mut().query::<&Sculpture>();
        assert_eq!(q.iter(app.world()).count(), 0);
        let mut q = app.world_mut().query::<&Name>();
        assert_eq!(q.iter(app.world()).count(), 0);
    }
    #[test]
    fn repeated_authorization_keeps_only_one_root_and_revocation_removes_children() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_resource::<Desired>()
            .add_systems(Update, reconcile);
        let placement = omoba_passport::sponsorship::Placement {
            id: "placement".into(),
            slot_id: "opposite-dragon".into(),
            asset_id: "asset".into(),
            revision_id: "revision".into(),
            rendition_sha256: "hash".into(),
            rendition_bytes: 1,
            rendition_url: "https://registry.test".into(),
            title: "Cat".into(),
            credit: "Artist".into(),
            description: String::new(),
            starts_at: String::new(),
            expires_at: String::new(),
        };
        app.world_mut().resource_mut::<Desired>().0 = Some(Ready {
            placement,
            asset_path: "unused.glb".into(),
            fit: omoba_passport::sponsorship::model::ModelFit {
                scale: 1.,
                offset: [0.; 3],
            },
        });
        for _ in 0..2 {
            app.world_mut()
                .spawn((
                    Sculpture {
                        identity: "placement:hash".into(),
                        gltf: Handle::default(),
                        graph: None,
                    },
                    Transform::default(),
                ))
                .with_children(|p| {
                    p.spawn(Name::new("child"));
                });
        }
        app.update();
        app.update();
        let mut q = app.world_mut().query::<&Sculpture>();
        assert_eq!(q.iter(app.world()).count(), 1);
        let mut q = app.world_mut().query::<&Name>();
        assert_eq!(q.iter(app.world()).count(), 1);
        app.world_mut().resource_mut::<Desired>().0 = None;
        app.update();
        let mut q = app.world_mut().query::<&Name>();
        assert_eq!(q.iter(app.world()).count(), 0);
    }
}
