//! Avatar collection: every avatar the player can look at, with a live 3D
//! preview, animation switching and the showcase/loadout choices.

use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use omoba_passport::avatars::AvatarDefinition;

use super::AppScreen;
use super::card::ProfileCard;
use super::preview::{AvatarPreview, PreviewStatus};
use super::widgets::{self, ButtonKind, MenuButton};
use crate::team::{AvatarThumbnails, TeamSelection};

pub use crate::passport::AvatarCatalogueSource as AvatarSource;

pub struct CollectionScreenPlugin;

impl Plugin for CollectionScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CollectionDrag>()
            .add_systems(OnEnter(AppScreen::Collection), spawn_collection)
            .add_systems(OnExit(AppScreen::Collection), clear_collection_drag)
            .add_systems(
                Update,
                (
                    refresh_collection_catalogue,
                    refresh_connection_labels,
                    drag_to_rotate,
                    collection_actions,
                    scroll_collection,
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
    Refresh,
    ConnectAccount,
    ConnectWallet,
}

#[derive(Component)]
struct PreviewSurface;

/// The scrolling avatar grid.
#[derive(Component)]
struct CollectionGrid;

#[derive(Component)]
struct CatalogueRevision(u64);

#[derive(Component)]
struct ClipRow;

#[derive(Component)]
struct DetailPanel;

/// Same ordering and approved membership as the pre-match picker.
pub fn collection_entries() -> Vec<(AvatarDefinition, AvatarSource)> {
    crate::passport::avatar_catalogue()
        .entries
        .into_iter()
        .map(|entry| (entry.avatar, entry.source))
        .collect()
}

fn ensure_thumbnails(asset_server: &AssetServer, thumbnails: &mut AvatarThumbnails) {
    for entry in crate::passport::avatar_catalogue().entries {
        if let Some(path) = crate::passport::thumbnail_asset_path(&entry.avatar) {
            thumbnails
                .0
                .insert(entry.avatar.slug.clone(), asset_server.load(path));
        }
    }
}

fn spawn_catalogue_grid(
    grid: &mut ChildSpawnerCommands,
    catalogue: &crate::passport::AvatarCatalogue,
    selected: Option<&str>,
    thumbnails: &AvatarThumbnails,
    phone: bool,
) {
    grid.spawn(widgets::heading("Included heroes", 17.0));
    for (defaults, title) in [(true, ""), (false, "Ekza Studio · Library")] {
        if !defaults {
            grid.spawn((
                widgets::heading(title, 17.0),
                Name::new("CollectionStudioHeading"),
            ));
            grid.spawn((
                widgets::label(catalogue.status.label(), 12.0, widgets::MUTED),
                Name::new("CollectionStudioStatus"),
            ));
            grid.spawn(Node {
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(6.0),
                row_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|row| {
                widgets::button(
                    row,
                    "Refresh",
                    ButtonKind::Secondary,
                    CollectionAction::Refresh,
                    "CollectionRefresh",
                );
                widgets::button(
                    row,
                    if crate::passport::account_connected() {
                        "Sign out of Ekza"
                    } else {
                        "Connect Ekza"
                    },
                    ButtonKind::Secondary,
                    CollectionAction::ConnectAccount,
                    "CollectionConnectAccount",
                );
            });
            grid.spawn((
                widgets::label(
                    &crate::passport::avatar_account_status_line(),
                    11.0,
                    widgets::MUTED,
                ),
                Name::new("CollectionAccountStatus"),
            ));
        }
        grid.spawn(Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            column_gap: Val::Px(10.0),
            row_gap: Val::Px(10.0),
            ..default()
        })
        .with_children(|tiles| {
            for entry in &catalogue.entries {
                if (entry.source == AvatarSource::Default) == defaults {
                    spawn_avatar_tile(
                        tiles,
                        &entry.avatar,
                        entry.source,
                        selected == Some(entry.avatar.slug.as_str()),
                        thumbnails,
                        phone,
                    );
                }
            }
        });
    }
    // Preserve existing paid-avatar pairing as an optional action, below the
    // primary account/free library path.
    {
        widgets::button(
            grid,
            "Connect wallet (optional)",
            ButtonKind::Secondary,
            CollectionAction::ConnectWallet,
            "CollectionConnectWallet",
        );
        grid.spawn((
            widgets::label(&crate::passport::wallet_status_line(), 11.0, widgets::MUTED),
            Name::new("CollectionWalletStatus"),
        ));
    }
}

fn refresh_connection_labels(mut labels: Query<(&Name, &mut Text)>) {
    let account = crate::passport::avatar_account_status_line();
    let wallet = crate::passport::wallet_status_line();
    let account_button = crate::passport::account_button_label();
    let wallet_button = crate::passport::wallet_button_label();
    let catalogue = omoba_passport::store::catalogue_status();
    for (name, mut text) in &mut labels {
        let value = match name.as_str() {
            "CollectionAccountStatus" => account.as_str(),
            "CollectionWalletStatus" => wallet.as_str(),
            "CollectionStudioStatus" => catalogue.label(),
            "CollectionConnectAccountLabel" => account_button,
            "CollectionConnectWalletLabel" => wallet_button,
            _ => continue,
        };
        if text.0 != value {
            text.0 = value.to_owned();
        }
    }
}

fn refresh_collection_catalogue(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut thumbnails: ResMut<AvatarThumbnails>,
    preview: Res<AvatarPreview>,
    mut grids: Query<(Entity, &mut CatalogueRevision), With<CollectionGrid>>,
    platform: Res<crate::ui::UiPlatform>,
) {
    crate::passport::poll_account();
    crate::passport::poll_wallet();
    let catalogue = crate::passport::avatar_catalogue();
    for (grid, mut revision) in &mut grids {
        if revision.0 == catalogue.revision {
            continue;
        }
        ensure_thumbnails(&asset_server, &mut thumbnails);
        revision.0 = catalogue.revision;
        // Keep the scrolling entity and selected preview; only replace its
        // contents. A same-count roster replacement therefore preserves offset.
        commands
            .entity(grid)
            .despawn_related::<Children>()
            .with_children(|grid| {
                spawn_catalogue_grid(
                    grid,
                    &catalogue,
                    preview.slug.as_deref(),
                    &thumbnails,
                    platform.is_mobile(),
                );
            });
    }
}

fn spawn_collection(
    mut commands: Commands,
    mut preview: ResMut<AvatarPreview>,
    card: Res<ProfileCard>,
    selection: Res<TeamSelection>,
    mut thumbnails: ResMut<AvatarThumbnails>,
    asset_server: Res<AssetServer>,
    platform: Res<crate::ui::UiPlatform>,
) {
    ensure_thumbnails(&asset_server, &mut thumbnails);
    let catalogue = crate::passport::avatar_catalogue();
    let phone = platform.is_mobile();
    let entries = collection_entries();
    // Open on something: the current loadout avatar, else the showcase, else
    // the first entry.
    let initial = selection
        .avatar
        .clone()
        .filter(|slug| entries.iter().any(|(avatar, _)| avatar.slug == *slug))
        .or_else(|| {
            card.showcase_avatar
                .clone()
                .filter(|slug| entries.iter().any(|(avatar, _)| avatar.slug == *slug))
        })
        .or_else(|| entries.first().map(|(avatar, _)| avatar.slug.clone()));
    if let Some(slug) = initial.as_deref() {
        preview.show_portrait(slug);
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
                        flex_direction: FlexDirection::Column,
                        flex_shrink: 0.0,
                        align_content: AlignContent::FlexStart,
                        column_gap: Val::Px(10.0),
                        row_gap: Val::Px(10.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                    BackgroundColor(widgets::PANEL),
                    BorderColor::all(widgets::PANEL_EDGE),
                    CollectionGrid,
                    CatalogueRevision(catalogue.revision),
                    Name::new("CollectionGrid"),
                ))
                .with_children(|grid| {
                    spawn_catalogue_grid(grid, &catalogue, initial.as_deref(), &thumbnails, phone);
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
    phone: bool,
) {
    let playable = crate::passport::can_select(avatar);
    let menu = MenuButton::tile(selected);
    grid.spawn((
        Button,
        Node {
            width: Val::Px(if phone { 136.0 } else { 112.0 }),
            height: Val::Px(if phone { 180.0 } else { 146.0 }),
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

fn collection_actions(
    mut preview: ResMut<AvatarPreview>,
    mut card: ResMut<ProfileCard>,
    mut selection: ResMut<TeamSelection>,
    mut next: ResMut<NextState<AppScreen>>,
    drag: Res<CollectionDrag>,
    buttons: Query<(&Interaction, &CollectionAction), Changed<Interaction>>,
    mut tiles: Query<(&CollectionAction, &mut MenuButton)>,
) {
    if drag.block_actions {
        return;
    }
    let mut picked: Option<String> = None;
    for (interaction, action) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            CollectionAction::Back => next.set(AppScreen::Home),
            CollectionAction::Refresh => crate::passport::refresh_avatar_catalogue(),
            CollectionAction::ConnectAccount => crate::passport::connect_account(),
            CollectionAction::ConnectWallet => crate::passport::connect(),
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
                    .and_then(omoba_passport::avatars::avatar_definition)
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

#[derive(Resource, Default)]
struct CollectionDrag {
    owner: Option<DragOwner>,
    // Captured before action dispatch and retained through release/cancel.
    block_actions: bool,
}

#[derive(Clone, Copy)]
enum DragOwner {
    Mouse,
    PreviewTouch {
        id: u64,
        previous: Vec2,
    },
    GridTouch {
        id: u64,
        previous: Vec2,
        moved: bool,
    },
}

fn clear_collection_drag(mut drag: ResMut<CollectionDrag>) {
    drag.owner = None;
    drag.block_actions = false;
}

fn rotate_delta(preview: &mut AvatarPreview, delta_x: f32) {
    if delta_x != 0.0 {
        preview.auto_spin = false;
        // A rightward drag pulls the front of the model toward the right.
        preview.yaw += delta_x * 0.012;
    }
}

fn surface_rect(node: &ComputedNode, transform: &UiGlobalTransform, window_scale: f32) -> Rect {
    // TouchInput/cursor positions are logical window pixels. UiScale is already
    // applied to the rendered node; undo only display DPI, not menu scaling.
    Rect::from_center_size(
        transform.translation / window_scale,
        node.size() * transform.to_scale_angle_translation().0.abs() / window_scale,
    )
}

/// The first pointer owns the gesture until release/cancel, including outside
/// the preview. Unrelated touches never steal it or turn the avatar.
fn drag_to_rotate(
    mut preview: ResMut<AvatarPreview>,
    mut drag: ResMut<CollectionDrag>,
    mut motion: MessageReader<MouseMotion>,
    mut touches: MessageReader<TouchInput>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    surfaces: Query<(&ComputedNode, &UiGlobalTransform), With<PreviewSurface>>,
    mut grids: Query<
        (&ComputedNode, &UiGlobalTransform, &mut ScrollPosition),
        With<CollectionGrid>,
    >,
) {
    drag.block_actions = matches!(
        drag.owner,
        Some(
            DragOwner::Mouse
                | DragOwner::PreviewTouch { .. }
                | DragOwner::GridTouch { moved: true, .. }
        )
    );
    let delta_x: f32 = motion.read().map(|event| event.delta.x).sum();
    let Some(window) = windows.single().ok().filter(|window| window.focused) else {
        touches.clear();
        drag.owner = None;
        drag.block_actions = true;
        return;
    };
    let mut touched = false;
    for event in touches.read() {
        touched = true;
        match event.phase {
            TouchPhase::Started if drag.owner.is_none() => {
                if surfaces.iter().any(|(node, transform)| {
                    surface_rect(node, transform, window.scale_factor()).contains(event.position)
                }) {
                    drag.owner = Some(DragOwner::PreviewTouch {
                        id: event.id,
                        previous: event.position,
                    });
                    drag.block_actions = true;
                } else if grids.iter().any(|(node, transform, _)| {
                    surface_rect(node, transform, window.scale_factor()).contains(event.position)
                }) {
                    drag.owner = Some(DragOwner::GridTouch {
                        id: event.id,
                        previous: event.position,
                        moved: false,
                    });
                }
            }
            TouchPhase::Moved => match drag.owner {
                Some(DragOwner::PreviewTouch { id, previous }) if id == event.id => {
                    rotate_delta(&mut preview, event.position.x - previous.x);
                    drag.owner = Some(DragOwner::PreviewTouch {
                        id,
                        previous: event.position,
                    });
                }
                Some(DragOwner::GridTouch {
                    id,
                    previous,
                    moved,
                }) if id == event.id => {
                    let moved = moved || event.position.distance(previous) > 4.0;
                    drag.block_actions |= moved;
                    for (node, _, mut scroll) in &mut grids {
                        scroll.y = scrolled(scroll.y, previous.y - event.position.y, node);
                    }
                    drag.owner = Some(DragOwner::GridTouch {
                        id,
                        previous: event.position,
                        moved,
                    });
                }
                _ => {}
            },
            TouchPhase::Ended | TouchPhase::Canceled => {
                if matches!(drag.owner, Some(DragOwner::PreviewTouch { id, .. } | DragOwner::GridTouch { id, .. }) if id == event.id)
                {
                    drag.owner = None;
                }
            }
            _ => {}
        }
    }
    if !touched
        && drag.owner.is_none()
        && mouse.just_pressed(MouseButton::Left)
        && window.cursor_position().is_some_and(|point| {
            surfaces.iter().any(|(node, transform)| {
                surface_rect(node, transform, window.scale_factor()).contains(point)
            })
        })
    {
        drag.owner = Some(DragOwner::Mouse);
        drag.block_actions = true;
    }
    if matches!(drag.owner, Some(DragOwner::Mouse)) {
        if mouse.pressed(MouseButton::Left) {
            rotate_delta(&mut preview, delta_x);
        } else {
            drag.owner = None;
        }
    }
}

/// Keeps the clip buttons and the detail panel in step with the loaded model.
fn refresh_collection_details(
    mut commands: Commands,
    preview: Res<AvatarPreview>,
    card: Res<ProfileCard>,
    selection: Res<TeamSelection>,
    clip_row: Query<Entity, With<ClipRow>>,
    detail: Query<(Entity, Ref<DetailPanel>)>,
    platform: Res<crate::ui::UiPlatform>,
    mut last: Local<
        Option<(
            Option<String>,
            PreviewStatus,
            usize,
            usize,
            bool,
            bool,
            bool,
            bool,
            u64,
        )>,
    >,
) {
    let catalogue = crate::passport::avatar_catalogue();
    let definition = catalogue
        .entries
        .iter()
        .find(|entry| Some(entry.avatar.slug.as_str()) == preview.slug.as_deref())
        .map(|entry| &entry.avatar)
        .or_else(|| {
            preview
                .slug
                .as_deref()
                .and_then(omoba_passport::avatars::avatar_definition)
        });
    let playable = definition.is_some_and(crate::passport::can_select);
    let current = (
        preview.slug.clone(),
        preview.status,
        preview.clips.len(),
        preview.selected,
        preview.auto_spin,
        card.showcase_avatar == preview.slug,
        selection.avatar == preview.slug,
        playable,
        catalogue.revision,
    );
    if last.as_ref() == Some(&current) && detail.iter().all(|(_, panel)| !panel.is_added()) {
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
                PreviewStatus::Unavailable => {
                    row.spawn(widgets::label(
                        "Model unavailable · refresh Studio to retry",
                        13.0,
                        widgets::GOLD,
                    ));
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
                        widgets::compact_tile(
                            row,
                            &label,
                            index == selected,
                            CollectionAction::Clip(index),
                            &format!("AvatarClip-{index}"),
                            platform.is_mobile(),
                        );
                    }
                }
            }
            widgets::compact_tile(
                row,
                if spinning { "Stop spin" } else { "Auto spin" },
                spinning,
                CollectionAction::ToggleSpin,
                "AvatarAutoSpin",
                platform.is_mobile(),
            );
        });
    }

    if let Ok((panel, _)) = detail.single() {
        commands.entity(panel).despawn_related::<Children>();
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

    fn drag_app() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Image>>()
            .init_resource::<AvatarPreview>()
            .init_resource::<CollectionDrag>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<TeamSelection>()
            .init_resource::<ProfileCard>()
            .init_resource::<NextState<AppScreen>>()
            .add_message::<MouseMotion>()
            .add_message::<TouchInput>()
            .add_systems(Update, (drag_to_rotate, collection_actions).chain());
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        app.world_mut().spawn((
            PreviewSurface,
            ComputedNode {
                size: Vec2::splat(100.0),
                ..default()
            },
            UiGlobalTransform::from_translation(Vec2::splat(100.0)),
        ));
        (app, window)
    }

    fn touch(app: &mut App, window: Entity, id: u64, phase: TouchPhase, position: Vec2) {
        app.world_mut().write_message(TouchInput {
            window,
            id,
            phase,
            position,
            force: None,
        });
        app.update();
    }

    #[test]
    fn scaled_phone_menu_uses_visible_window_coordinates_for_touch_hits() {
        let (mut app, window) = drag_app();
        let surface = app
            .world_mut()
            .query_filtered::<Entity, With<PreviewSurface>>()
            .single(app.world())
            .unwrap();
        app.world_mut().entity_mut(surface).insert((
            ComputedNode {
                size: Vec2::new(81.0, 109.0),
                inverse_scale_factor: 1.0 / 0.61,
                ..default()
            },
            UiGlobalTransform::from_translation(Vec2::new(630.5, 133.5)),
        ));
        let initial = app.world().resource::<AvatarPreview>().yaw;
        touch(
            &mut app,
            window,
            1,
            TouchPhase::Started,
            Vec2::new(630.5, 133.5),
        );
        touch(
            &mut app,
            window,
            1,
            TouchPhase::Moved,
            Vec2::new(694.5, 133.5),
        );
        assert!((app.world().resource::<AvatarPreview>().yaw - initial - 0.768).abs() < 0.001);
    }

    #[test]
    fn preview_touch_keeps_owner_outside_and_ignores_second_finger_until_cancel() {
        let (mut app, window) = drag_app();
        let initial = app.world().resource::<AvatarPreview>().yaw;
        touch(&mut app, window, 1, TouchPhase::Started, Vec2::splat(100.0));
        touch(&mut app, window, 2, TouchPhase::Started, Vec2::splat(110.0));
        touch(&mut app, window, 2, TouchPhase::Moved, Vec2::splat(140.0));
        assert_eq!(app.world().resource::<AvatarPreview>().yaw, initial);
        touch(
            &mut app,
            window,
            1,
            TouchPhase::Moved,
            Vec2::new(300.0, 100.0),
        );
        let turned = app.world().resource::<AvatarPreview>().yaw;
        assert!(
            (turned - initial - 2.4).abs() < 0.001,
            "rightward drag must increase yaw"
        );
        touch(&mut app, window, 2, TouchPhase::Ended, Vec2::splat(140.0));
        assert!(app.world().resource::<CollectionDrag>().owner.is_some());
        touch(
            &mut app,
            window,
            1,
            TouchPhase::Canceled,
            Vec2::new(300.0, 100.0),
        );
        assert!(app.world().resource::<CollectionDrag>().owner.is_none());
        touch(&mut app, window, 1, TouchPhase::Moved, Vec2::splat(100.0));
        assert_eq!(app.world().resource::<AvatarPreview>().yaw, turned);
    }

    #[test]
    fn same_frame_preview_touch_suppresses_unrelated_actions_in_the_real_chain() {
        let (mut app, window) = drag_app();
        app.world_mut()
            .resource_mut::<AvatarPreview>()
            .show("selected-preview");
        app.world_mut()
            .resource_mut::<ProfileCard>()
            .showcase_avatar = None;
        app.world_mut()
            .spawn((CollectionAction::Showcase, Interaction::Pressed));
        app.world_mut().write_message(TouchInput {
            window,
            id: 1,
            phase: TouchPhase::Started,
            position: Vec2::splat(100.0),
            force: None,
        });
        app.world_mut().write_message(TouchInput {
            window,
            id: 2,
            phase: TouchPhase::Started,
            position: Vec2::new(500.0, 100.0),
            force: None,
        });
        app.update();
        assert!(
            app.world()
                .resource::<ProfileCard>()
                .showcase_avatar
                .is_none()
        );
        assert!(app.world().resource::<CollectionDrag>().block_actions);
        touch(&mut app, window, 1, TouchPhase::Ended, Vec2::splat(100.0));
        assert!(
            app.world().resource::<CollectionDrag>().block_actions,
            "release frame must not dispatch an unrelated pressed action"
        );
    }

    #[test]
    fn an_ordinary_grid_tap_still_selects_an_avatar() {
        let (mut app, window) = drag_app();
        app.world_mut().spawn((
            CollectionGrid,
            ScrollPosition::default(),
            ComputedNode {
                size: Vec2::splat(100.0),
                ..default()
            },
            UiGlobalTransform::from_translation(Vec2::new(300.0, 100.0)),
        ));
        app.world_mut().spawn((
            CollectionAction::Select("grid-choice".into()),
            Interaction::Pressed,
            MenuButton::tile(false),
        ));
        touch(
            &mut app,
            window,
            1,
            TouchPhase::Started,
            Vec2::new(300.0, 100.0),
        );
        assert_eq!(
            app.world().resource::<AvatarPreview>().slug.as_deref(),
            Some("grid-choice")
        );
        assert!(!app.world().resource::<CollectionDrag>().block_actions);
    }

    #[test]
    fn mouse_drag_retains_ownership_and_clears_on_focus_loss() {
        let (mut app, window) = drag_app();
        app.world_mut()
            .entity_mut(window)
            .get_mut::<Window>()
            .unwrap()
            .set_cursor_position(Some(Vec2::splat(100.0)));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        app.world_mut()
            .entity_mut(window)
            .get_mut::<Window>()
            .unwrap()
            .set_cursor_position(Some(Vec2::new(500.0, 100.0)));
        app.world_mut().write_message(MouseMotion {
            delta: Vec2::new(10.0, 0.0),
        });
        app.update();
        let yaw = app.world().resource::<AvatarPreview>().yaw;
        assert!((yaw - std::f32::consts::PI - 0.12).abs() < 0.001);
        app.world_mut()
            .entity_mut(window)
            .get_mut::<Window>()
            .unwrap()
            .focused = false;
        app.update();
        assert!(app.world().resource::<CollectionDrag>().owner.is_none());
    }

    #[test]
    fn catalogue_rebuild_preserves_selection_scroll_and_loads_direct_entry_thumbnails() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .init_resource::<AvatarPreview>()
            .init_resource::<AvatarThumbnails>()
            .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
            .add_systems(Update, refresh_collection_catalogue);
        let selected = omoba_passport::avatars::avatar_roster()[0].slug.clone();
        app.world_mut()
            .resource_mut::<AvatarPreview>()
            .show_portrait(&selected);
        let grid = app
            .world_mut()
            .spawn((
                CollectionGrid,
                CatalogueRevision(u64::MAX),
                Node::default(),
                ScrollPosition(Vec2::new(0.0, 120.0)),
            ))
            .id();
        app.update();
        assert_eq!(app.world().get::<ScrollPosition>(grid).unwrap().y, 120.0);
        assert_eq!(
            app.world().resource::<AvatarPreview>().slug.as_deref(),
            Some(selected.as_str())
        );
        assert_eq!(
            app.world().get::<CatalogueRevision>(grid).unwrap().0,
            crate::passport::avatar_catalogue().revision
        );
        assert!(!app.world().get::<Children>(grid).unwrap().is_empty());
        for avatar in crate::passport::default_avatars() {
            if avatar.thumbnail.is_some() {
                assert!(
                    app.world()
                        .resource::<AvatarThumbnails>()
                        .0
                        .contains_key(&avatar.slug)
                );
            }
        }
    }

    #[test]
    fn connection_status_changes_do_not_replace_pressed_controls_or_scroll() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .init_resource::<AvatarPreview>()
            .init_resource::<AvatarThumbnails>()
            .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
            .add_systems(
                Update,
                (refresh_collection_catalogue, refresh_connection_labels).chain(),
            );
        let grid = app
            .world_mut()
            .spawn((
                CollectionGrid,
                CatalogueRevision(u64::MAX),
                Node::default(),
                ScrollPosition(Vec2::new(0.0, 120.0)),
            ))
            .id();
        app.update();
        let button = app
            .world_mut()
            .query::<(Entity, &Name)>()
            .iter(app.world())
            .find(|(_, name)| name.as_str() == "CollectionConnectAccount")
            .unwrap()
            .0;
        let label = app
            .world_mut()
            .query::<(Entity, &Name)>()
            .iter(app.world())
            .find(|(_, name)| name.as_str() == "CollectionAccountStatus")
            .unwrap()
            .0;
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.world_mut().get_mut::<Text>(label).unwrap().0 = "Previous connection state".into();
        let children: Vec<_> = app.world().get::<Children>(grid).unwrap().iter().collect();
        app.update();
        assert_eq!(
            *app.world().get::<Interaction>(button).unwrap(),
            Interaction::Pressed
        );
        assert_ne!(
            app.world().get::<Text>(label).unwrap().0,
            "Previous connection state"
        );
        assert_eq!(
            app.world()
                .get::<Children>(grid)
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            children
        );
        assert_eq!(app.world().get::<ScrollPosition>(grid).unwrap().y, 120.0);
    }

    #[test]
    fn defaults_precede_the_studio_library() {
        let entries = collection_entries();
        let mut studio = false;
        for (_, source) in entries {
            if source != AvatarSource::Default {
                studio = true;
            }
            assert!(!studio || source != AvatarSource::Default);
        }
    }

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
