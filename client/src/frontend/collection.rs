//! Avatar collection: every avatar the player can look at, with a live 3D
//! preview, animation switching and the showcase/loadout choices.

use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use shared::AvatarDefinition;

use super::AppScreen;
use super::card::ProfileCard;
use super::preview::{AvatarPreview, PreviewStatus};
use super::widgets::{self, ButtonKind, MenuButton};
use crate::team::{AvatarThumbnails, TeamSelection};

/// Where an avatar comes from. The collection groups by this so a player can
/// see what they own versus what the community published.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AvatarSource {
    /// Shipped with the game; everyone can play it.
    Roster,
    /// Owned through the Ekza store or the player's own library.
    Owned,
    /// Published by other players; viewable, not always playable.
    Community,
}

impl AvatarSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Roster => "Included",
            Self::Owned => "Owned",
            Self::Community => "Community",
        }
    }
}

pub struct CollectionScreenPlugin;

impl Plugin for CollectionScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppScreen::Collection), spawn_collection)
            .add_systems(
                Update,
                (
                    collection_actions,
                    scroll_collection,
                    drag_to_rotate,
                    refresh_collection_details,
                )
                    .chain()
                    .run_if(in_state(AppScreen::Collection)),
            );
    }
}

#[derive(Component, Clone)]
enum CollectionAction {
    Back,
    Select(String),
    Clip(usize),
    ToggleSpin,
    Showcase,
    Equip,
}

#[derive(Component)]
struct PreviewSurface;

/// The scrolling avatar grid.
#[derive(Component)]
struct CollectionGrid;

#[derive(Component)]
struct ClipRow;

#[derive(Component)]
struct DetailPanel;

/// Every avatar the player can inspect, de-duplicated, owned first.
pub fn collection_entries() -> Vec<(&'static AvatarDefinition, AvatarSource)> {
    let mut entries: Vec<(&'static AvatarDefinition, AvatarSource)> = Vec::new();
    let mut push = |avatar: &'static AvatarDefinition, source: AvatarSource| {
        if !entries
            .iter()
            .any(|(existing, _)| existing.slug == avatar.slug)
        {
            entries.push((avatar, source));
        }
    };
    for avatar in crate::passport::purchased_avatars() {
        push(avatar, AvatarSource::Owned);
    }
    for avatar in crate::passport::library_avatars() {
        push(avatar, AvatarSource::Owned);
    }
    for avatar in crate::passport::default_avatars() {
        push(avatar, AvatarSource::Roster);
    }
    for avatar in crate::passport::community_avatars() {
        push(avatar, AvatarSource::Community);
    }
    for avatar in shared::avatar_roster() {
        push(avatar, AvatarSource::Roster);
    }
    entries
}

fn spawn_collection(
    mut commands: Commands,
    mut preview: ResMut<AvatarPreview>,
    card: Res<ProfileCard>,
    selection: Res<TeamSelection>,
    thumbnails: Res<AvatarThumbnails>,
) {
    let phone = crate::platform::ui_profile() == crate::platform::UiProfile::Mobile;
    let entries = collection_entries();
    // Open on something: the current loadout avatar, else the showcase, else
    // the first entry.
    let initial = selection
        .avatar
        .clone()
        .or_else(|| card.showcase_avatar.clone())
        .or_else(|| entries.first().map(|(avatar, _)| avatar.slug.clone()));
    if let Some(slug) = initial.as_deref() {
        preview.show(slug);
        // The collection is where a player inspects a model: turn it.
        preview.auto_spin = true;
    }
    let image = preview.image.clone();
    commands
        .spawn(widgets::screen_root(
            AppScreen::Collection,
            "CollectionScreen",
        ))
        .with_children(|root| {
            root.spawn(Node {
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|header| {
                header
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(2.0),
                        ..default()
                    })
                    .with_children(|title| {
                        title.spawn(widgets::heading("Avatars", 30.0));
                        title.spawn(widgets::label(
                            "Your heroes, your identity · Drag a model to inspect it",
                            13.0,
                            widgets::MUTED,
                        ));
                    });
                widgets::button(
                    header,
                    "Back",
                    ButtonKind::Secondary,
                    CollectionAction::Back,
                    "CollectionBack",
                );
            });

            root.spawn(Node {
                flex_grow: 1.0,
                column_gap: Val::Px(20.0),
                min_height: Val::Px(0.0),
                ..default()
            })
            .with_children(|body| {
                // Grid of avatars.
                body.spawn((
                    Node {
                        width: Val::Percent(50.0),
                        padding: UiRect::all(Val::Px(12.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        align_content: AlignContent::FlexStart,
                        column_gap: Val::Px(10.0),
                        row_gap: Val::Px(10.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                    BackgroundColor(widgets::PANEL),
                    BorderColor::all(widgets::PANEL_EDGE),
                    CollectionGrid,
                    Name::new("CollectionGrid"),
                ))
                .with_children(|grid| {
                    for (avatar, source) in &entries {
                        spawn_avatar_tile(
                            grid,
                            avatar,
                            *source,
                            initial.as_deref() == Some(avatar.slug.as_str()),
                            &thumbnails,
                        );
                    }
                });

                // Preview column.
                body.spawn((
                    Node {
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(8.0),
                        padding: UiRect::all(Val::Px(12.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        min_width: Val::Px(0.0),
                        ..default()
                    },
                    BackgroundColor(widgets::PANEL),
                    BorderColor::all(widgets::PANEL_EDGE),
                    Name::new("CollectionPreview"),
                ))
                .with_children(|column| {
                    column.spawn((
                        Button,
                        ImageNode::new(image),
                        Node {
                            // Height-driven so the clip buttons and the
                            // avatar details stay on screen on a short window.
                            height: Val::Vh(if phone { 28.0 } else { 43.0 }),
                            max_height: Val::Px(420.0),
                            aspect_ratio: Some(0.742),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(12.0)),
                            ..default()
                        },
                        BorderColor::all(widgets::PANEL_EDGE),
                        PreviewSurface,
                        Name::new("AvatarPreviewSurface"),
                    ));
                    column.spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            flex_wrap: FlexWrap::Wrap,
                            justify_content: JustifyContent::Center,
                            column_gap: Val::Px(6.0),
                            row_gap: Val::Px(6.0),
                            ..default()
                        },
                        ClipRow,
                        Name::new("AvatarClipRow"),
                    ));
                    column.spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            row_gap: Val::Px(8.0),
                            ..default()
                        },
                        DetailPanel,
                        Name::new("AvatarDetailPanel"),
                    ));
                });
            });
        });
}

fn spawn_avatar_tile(
    grid: &mut ChildSpawnerCommands,
    avatar: &AvatarDefinition,
    source: AvatarSource,
    selected: bool,
    thumbnails: &AvatarThumbnails,
) {
    let playable = crate::passport::can_select(avatar);
    let menu = MenuButton::tile(selected);
    grid.spawn((
        Button,
        Node {
            width: Val::Px(
                if crate::platform::ui_profile() == crate::platform::UiProfile::Mobile {
                    136.0
                } else {
                    112.0
                },
            ),
            height: Val::Px(
                if crate::platform::ui_profile() == crate::platform::UiProfile::Mobile {
                    180.0
                } else {
                    146.0
                },
            ),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::FlexStart,
            padding: UiRect::all(Val::Px(6.0)),
            row_gap: Val::Px(4.0),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            ..default()
        },
        BackgroundColor(menu.idle_color()),
        menu,
        CollectionAction::Select(avatar.slug.clone()),
        Name::new(format!("CollectionTile-{}", avatar.slug)),
    ))
    .with_children(|tile| {
        let mut thumb = tile.spawn((
            Node {
                width: Val::Px(84.0),
                height: Val::Px(84.0),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(widgets::PANEL),
        ));
        if let Some(image) = thumbnails.0.get(&avatar.slug) {
            thumb.insert(ImageNode::new(image.clone()));
        } else {
            thumb
                .insert(Node {
                    width: Val::Px(84.0),
                    height: Val::Px(84.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                })
                .with_children(|portrait| {
                    let initials: String = avatar
                        .slug
                        .split('-')
                        .filter_map(|part| part.chars().next())
                        .take(2)
                        .flat_map(char::to_uppercase)
                        .collect();
                    portrait.spawn(widgets::heading(&initials, 24.0));
                });
        }
        tile.spawn(widgets::label(&avatar.display_name, 12.0, widgets::IVORY));
        tile.spawn(widgets::label(
            if playable {
                source.label()
            } else {
                "View only"
            },
            10.5,
            if playable {
                widgets::MUTED
            } else {
                widgets::GOLD
            },
        ));
    });
}

#[allow(clippy::too_many_arguments)]
fn collection_actions(
    mut preview: ResMut<AvatarPreview>,
    mut card: ResMut<ProfileCard>,
    mut selection: ResMut<TeamSelection>,
    mut next: ResMut<NextState<AppScreen>>,
    buttons: Query<(&Interaction, &CollectionAction), Changed<Interaction>>,
    mut tiles: Query<(&CollectionAction, &mut MenuButton)>,
) {
    let mut picked: Option<String> = None;
    for (interaction, action) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            CollectionAction::Back => next.set(AppScreen::Home),
            CollectionAction::Select(slug) => {
                preview.show(slug);
                picked = Some(slug.clone());
            }
            CollectionAction::Clip(index) => {
                if *index < preview.clips.len() {
                    preview.selected = *index;
                }
            }
            CollectionAction::ToggleSpin => preview.auto_spin = !preview.auto_spin,
            CollectionAction::Showcase => {
                card.showcase_avatar = preview.slug.clone();
            }
            CollectionAction::Equip => {
                if preview
                    .slug
                    .as_deref()
                    .and_then(shared::avatar_definition)
                    .is_some_and(crate::passport::can_select)
                {
                    selection.avatar = preview.slug.clone();
                }
            }
        }
    }
    let Some(picked) = picked else {
        return;
    };
    for (action, mut button) in &mut tiles {
        if let CollectionAction::Select(slug) = action {
            let selected = *slug == picked;
            if button.selected != selected {
                button.selected = selected;
            }
        }
    }
}

/// Bevy lays a scroll container out but does not move it: without this the
/// avatars below the fold could be seen clipped and never reached.
fn scroll_collection(
    mut wheel: MessageReader<MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    mut grids: Query<(&ComputedNode, &mut ScrollPosition), With<CollectionGrid>>,
) {
    let mut delta = wheel
        .read()
        .map(|event| {
            -event.y
                * if event.unit == MouseScrollUnit::Line {
                    48.0
                } else {
                    1.0
                }
        })
        .sum::<f32>();
    if keys.just_pressed(KeyCode::PageDown) {
        delta += 300.0;
    }
    if keys.just_pressed(KeyCode::PageUp) {
        delta -= 300.0;
    }
    if delta == 0.0 {
        return;
    }
    for (computed, mut position) in &mut grids {
        position.y = scrolled(position.y, delta, computed);
    }
}

/// New scroll offset, kept inside what the container can actually show.
fn scrolled(current: f32, delta: f32, computed: &ComputedNode) -> f32 {
    let maximum = ((computed.content_size().y - computed.size().y)
        * computed.inverse_scale_factor())
    .max(0.0);
    (current + delta).clamp(0.0, maximum)
}

/// Dragging across the preview turns the model; releasing keeps the angle.
fn drag_to_rotate(
    mut preview: ResMut<AvatarPreview>,
    mut motion: MessageReader<MouseMotion>,
    surfaces: Query<&Interaction, With<PreviewSurface>>,
) {
    let dragging = surfaces
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed);
    let delta: f32 = motion.read().map(|event| event.delta.x).sum();
    if !dragging || delta == 0.0 {
        return;
    }
    preview.auto_spin = false;
    preview.yaw -= delta * 0.012;
}

/// Keeps the clip buttons and the detail panel in step with the loaded model.
fn refresh_collection_details(
    mut commands: Commands,
    preview: Res<AvatarPreview>,
    card: Res<ProfileCard>,
    selection: Res<TeamSelection>,
    clip_row: Query<Entity, With<ClipRow>>,
    detail: Query<Entity, With<DetailPanel>>,
    mut last: Local<
        Option<(
            Option<String>,
            PreviewStatus,
            usize,
            usize,
            bool,
            bool,
            bool,
        )>,
    >,
) {
    let playable = preview
        .slug
        .as_deref()
        .and_then(shared::avatar_definition)
        .is_some_and(crate::passport::can_select);
    let current = (
        preview.slug.clone(),
        preview.status,
        preview.clips.len(),
        preview.selected,
        preview.auto_spin,
        card.showcase_avatar == preview.slug,
        selection.avatar == preview.slug,
    );
    if last.as_ref() == Some(&current) {
        return;
    }
    *last = Some(current);

    if let Ok(row) = clip_row.single() {
        commands.entity(row).despawn_related::<Children>();
        let clips: Vec<(usize, String)> = preview
            .clips
            .iter()
            .enumerate()
            .map(|(index, clip)| (index, AvatarPreview::clip_label(&clip.name)))
            .collect();
        let status = preview.status;
        let selected = preview.selected;
        let spinning = preview.auto_spin;
        commands.entity(row).with_children(|row| {
            match status {
                PreviewStatus::Empty => {
                    row.spawn(widgets::label("Pick an avatar", 13.0, widgets::MUTED));
                }
                PreviewStatus::Loading => {
                    row.spawn(widgets::label("Loading model…", 13.0, widgets::MUTED));
                }
                PreviewStatus::NoAnimations => {
                    row.spawn(widgets::label(
                        "This avatar ships without animation clips",
                        13.0,
                        widgets::GOLD,
                    ));
                }
                PreviewStatus::Ready => {
                    for (index, label) in clips {
                        widgets::tile(
                            row,
                            &label,
                            index == selected,
                            CollectionAction::Clip(index),
                            &format!("AvatarClip-{index}"),
                        );
                    }
                }
            }
            widgets::tile(
                row,
                if spinning { "Stop spin" } else { "Auto spin" },
                spinning,
                CollectionAction::ToggleSpin,
                "AvatarAutoSpin",
            );
        });
    }

    if let Ok(panel) = detail.single() {
        commands.entity(panel).despawn_related::<Children>();
        let definition = preview.slug.as_deref().and_then(shared::avatar_definition);
        let is_showcase = card.showcase_avatar == preview.slug;
        let is_equipped = selection.avatar == preview.slug;
        commands.entity(panel).with_children(|panel| {
            if let Some(definition) = definition {
                panel.spawn(widgets::heading(&definition.display_name, 20.0));
                panel.spawn(widgets::label(
                    &format!(
                        "{} · {}",
                        definition.collection,
                        definition.author.as_deref().unwrap_or("unknown author")
                    ),
                    12.0,
                    widgets::MUTED,
                ));
                panel.spawn(widgets::label(&definition.license, 11.5, widgets::MUTED));
            }
            panel
                .spawn(Node {
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|row| {
                    widgets::tile(
                        row,
                        if is_showcase {
                            "On your card"
                        } else {
                            "Put on card"
                        },
                        is_showcase,
                        CollectionAction::Showcase,
                        "AvatarShowcase",
                    );
                    if playable {
                        widgets::tile(
                            row,
                            if is_equipped {
                                "Selected"
                            } else {
                                "Play as this"
                            },
                            is_equipped,
                            CollectionAction::Equip,
                            "AvatarEquip",
                        );
                    } else {
                        row.spawn(widgets::label(
                            "Not unlocked for matches",
                            12.0,
                            widgets::GOLD,
                        ));
                    }
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_has_no_duplicate_slugs() {
        let entries = collection_entries();
        let mut slugs: Vec<&str> = entries
            .iter()
            .map(|(avatar, _)| avatar.slug.as_str())
            .collect();
        slugs.sort_unstable();
        let before = slugs.len();
        slugs.dedup();
        assert_eq!(
            before,
            slugs.len(),
            "the collection must not repeat avatars"
        );
    }

    #[test]
    fn shipped_avatars_are_listed() {
        assert!(
            !collection_entries().is_empty(),
            "the shipped roster must be visible in the collection"
        );
    }
}
