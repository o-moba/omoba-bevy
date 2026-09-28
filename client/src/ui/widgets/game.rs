//! Game-facing kit components: progress bars, the ability button, item
//! slots and shop cards, hero tiles and portraits, scoreboard rows, the
//! timer ring and the HUD plates (`omoba-ui/handoff/components/{bars,
//! ability-button,item-slot,hero-tile,scoreboard-row,timer-ring,
//! hud-plate}.md`). They are built here so the HUD and screen steps only
//! compose them; state lives in small view components (`BarValue`,
//! `AbilityView`, `TimerRing`) that the owner updates.
// i18n-strict
// Kit parts that screen steps adopt; the kit gallery (`qa` feature) uses all of them.
#![cfg_attr(not(feature = "qa"), allow(dead_code))]
use bevy::prelude::*;

use super::{ButtonStyle, KitParts, KitSkin, NoSlab, button_bundle, icon_node};
use crate::i18n::UiLabel;
use crate::ui::{
    TestId,
    action::UiActionT,
    kit_assets::{CoverImage, Icon, KitImage, Sprite},
    theme::{self, ButtonKind, ButtonState, Form, TextStyle},
    tokens::{Metric, TextRole, border, color, motion, radius, size, space},
};

/// Abilities shrink to 0.94 while pressed (`ability-button.md`).
pub(crate) const ABILITY_PRESS_SCALE: f32 = 0.94;

/// Roster or other art loaded by path, cover-cropped into its node.
#[derive(Component, Clone, PartialEq, Debug)]
pub(crate) struct Art {
    pub path: String,
}

/// Loads [`Art`] into the node's `ImageNode`.
pub(crate) fn resolve_art(
    mut commands: Commands,
    assets: Option<Res<AssetServer>>,
    art: Query<(Entity, &Art), Changed<Art>>,
) {
    let Some(assets) = assets else { return };
    for (entity, art) in &art {
        commands.entity(entity).insert((
            ImageNode::new(assets.load(art.path.clone())).with_mode(NodeImageMode::Stretch),
            CoverImage { anchor_y: 0.0 },
        ));
    }
}

/// A circular art node `side` px (portraits, list-row leading).
pub(crate) fn round_art(path: String, side: f32) -> impl Bundle {
    (
        Node {
            width: Val::Px(side),
            height: Val::Px(side),
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(Val::Px(radius::PILL)),
            ..default()
        },
        BackgroundColor(color::SURFACE_3),
        Art { path },
        Pickable::IGNORE,
    )
}

// --- Progress bars ---

/// Which bar (`bars.md`): height and fill token.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BarKind {
    HpSelf,
    HpAlly,
    HpEnemy,
    Mana,
    Xp,
    Loading,
}

impl BarKind {
    pub(crate) fn height(self, form: Form) -> f32 {
        match self {
            BarKind::HpSelf | BarKind::HpAlly | BarKind::HpEnemy => size::BAR_HP_HUD.at(form),
            BarKind::Mana => size::BAR_MANA_HUD,
            BarKind::Xp => size::BAR_XP,
            BarKind::Loading => size::BAR_LOADING,
        }
    }

    pub(crate) fn fill(self) -> Color {
        match self {
            BarKind::HpSelf => color::BAR_HP_SELF,
            BarKind::HpAlly => color::BAR_HP_ALLY,
            BarKind::HpEnemy => color::BAR_HP_ENEMY,
            BarKind::Mana => color::BAR_MANA,
            BarKind::Xp => color::BAR_XP,
            BarKind::Loading => color::BAR_LOADING,
        }
    }

    fn is_hp(self) -> bool {
        matches!(self, BarKind::HpSelf | BarKind::HpAlly | BarKind::HpEnemy)
    }
}

/// A bar's value; `respawn` marks the dead state: `Some(seconds)` shows the
/// countdown (`kit.bar.respawn`), `Some(0)` a dead bar without one
/// (`hud.target.defeated`: the match does not replicate respawn seconds).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub(crate) struct BarValue {
    pub current: f32,
    pub max: f32,
    pub respawn: Option<u32>,
}

impl BarValue {
    pub(crate) fn fraction(&self) -> f32 {
        if self.max <= 0.0 {
            0.0
        } else {
            (self.current / self.max).clamp(0.0, 1.0)
        }
    }
}

/// Which bar a [`bar`] root is (its fill rule and value format).
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct BarKindTag(pub BarKind);

/// The fill of a bar at a fraction: the player's own HP turns
/// `color.state.warning` at ≤ 50 % and `color.bar.hp.enemy` at ≤ 25 %
/// (hud.md § States, Low HP); every other bar keeps its token.
pub(crate) fn bar_fill_at(kind: BarKind, fraction: f32) -> Color {
    match kind {
        BarKind::HpSelf if fraction <= LOW_HP_DANGER => color::BAR_HP_ENEMY,
        BarKind::HpSelf if fraction <= LOW_HP_WARNING => color::STATE_WARNING,
        kind => kind.fill(),
    }
}

/// Low-HP thresholds of the player's HP bar.
pub(crate) const LOW_HP_WARNING: f32 = 0.5;
pub(crate) const LOW_HP_DANGER: f32 = 0.25;

/// The damage trail: the fraction it shows now and where it started.
#[derive(Component, Clone, Copy, Default)]
pub(crate) struct DamageTrail {
    shown: f32,
    from: f32,
    elapsed: f32,
}

/// A progress bar: track (`color.bar.track`, 1 px black edge, `radius.sm`),
/// fill, HP damage trail and an optional value (`type.number_sm`, centred on
/// HP, right of the mana bar). `width` is the track width.
pub(crate) fn bar(
    parent: &mut ChildSpawnerCommands,
    kind: BarKind,
    value: BarValue,
    width: Val,
    form: Form,
    show_value: bool,
) -> Entity {
    bar_parts(parent, kind, value, width, form, show_value).0
}

/// [`bar`] and its parts (`fill`, `label`, `knob` = damage trail), for an
/// owner that names or recolours them.
pub(crate) fn bar_parts(
    parent: &mut ChildSpawnerCommands,
    kind: BarKind,
    value: BarValue,
    width: Val,
    form: Form,
    show_value: bool,
) -> (Entity, KitParts) {
    let mut parts = KitParts::default();
    let mut root = parent.spawn((
        Node {
            width,
            column_gap: Val::Px(space::S8),
            align_items: AlignItems::Center,
            ..default()
        },
        value,
        BarKindTag(kind),
    ));
    root.with_children(|root| {
        parts.track = Some(
            root.spawn((
                Node {
                    flex_grow: 1.0,
                    height: Val::Px(kind.height(form)),
                    border: UiRect::all(Val::Px(border::HAIRLINE)),
                    border_radius: BorderRadius::all(Val::Px(radius::SM)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(theme::perceptual(color::BAR_TRACK)),
                BorderColor::all(color::SCRIM.with_alpha(1.0)),
                Pickable::IGNORE,
            ))
            .with_children(|track| {
                let fill_node = |width: f32| Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    width: Val::Percent(width * 100.0),
                    ..default()
                };
                if kind.is_hp() {
                    parts.knob = Some(
                        track
                            .spawn((
                                fill_node(value.fraction()),
                                BackgroundColor(theme::perceptual(color::BAR_DAMAGE_TRAIL)),
                                DamageTrail {
                                    shown: value.fraction(),
                                    from: value.fraction(),
                                    elapsed: 0.0,
                                },
                            ))
                            .id(),
                    );
                }
                parts.fill = Some(
                    track
                        .spawn((
                            fill_node(value.fraction()),
                            BackgroundColor(bar_fill_at(kind, value.fraction())),
                        ))
                        .id(),
                );
                if show_value && kind.is_hp() {
                    parts.label = Some(
                        track
                            .spawn((
                                Text::new(bar_text(kind, &value)),
                                theme::role_text(TextRole::NumberSm),
                                TextColor(color::TEXT_PRIMARY),
                                TextShadow::default(),
                            ))
                            .id(),
                    );
                }
            })
            .id(),
        );
        if show_value && kind == BarKind::Mana && form == Form::Desktop {
            parts.label = Some(
                root.spawn((
                    Text::new(bar_text(kind, &value)),
                    theme::role_text(TextRole::NumberSm),
                    TextColor(color::TEXT_SECONDARY),
                ))
                .id(),
            );
        }
    });
    (root.insert(parts).id(), parts)
}

fn bar_text(kind: BarKind, value: &BarValue) -> String {
    match value.respawn {
        Some(0) if kind == BarKind::Mana => String::new(),
        Some(0) => crate::i18n::tr("hud.target.defeated").to_owned(),
        Some(seconds) => crate::i18n::trf("kit.bar.respawn", &[("seconds", &seconds)]),
        // hud.md: HP `{hp} / {max}`, mana `{mana}/{max}` (numbers only).
        None if kind == BarKind::Mana => format!("{:.0}/{:.0}", value.current, value.max),
        None => format!("{:.0} / {:.0}", value.current, value.max),
    }
}

/// Fill, trail and text of every bar whose value changed; the trail shrinks
/// to the fill over `motion.duration.bar_trail`.
pub(crate) fn paint_bars(
    time: Res<Time>,
    bars: Query<(Ref<BarValue>, &KitParts, Option<&BarKindTag>)>,
    mut fills: Query<(&mut Node, Option<&mut DamageTrail>, &mut BackgroundColor)>,
    mut texts: Query<(&mut Text, &mut TextColor)>,
) {
    let duration = motion::DURATION_BAR_TRAIL.as_secs_f32();
    for (value, parts, kind) in &bars {
        let kind = kind.map_or(BarKind::HpSelf, |kind| kind.0);
        let fraction = if value.respawn.is_some() {
            0.0
        } else {
            value.fraction()
        };
        if value.is_changed() {
            if let Some((mut node, _, mut fill)) =
                parts.fill.and_then(|fill| fills.get_mut(fill).ok())
            {
                let width = Val::Percent(fraction * 100.0);
                if node.width != width {
                    node.width = width;
                }
                let next = bar_fill_at(kind, fraction);
                if fill.0 != next {
                    fill.0 = next;
                }
            }
            if let Some((mut text, mut ink)) =
                parts.label.and_then(|label| texts.get_mut(label).ok())
            {
                let next = bar_text(kind, &value);
                if text.0 != next {
                    text.0 = next;
                }
                let next_ink = if value.respawn.is_some() {
                    color::TEXT_DISABLED
                } else {
                    color::TEXT_PRIMARY
                };
                if ink.0 != next_ink {
                    ink.0 = next_ink;
                }
            }
        }
        if let Some((mut node, Some(mut trail), _)) =
            parts.knob.and_then(|trail| fills.get_mut(trail).ok())
        {
            if fraction > trail.shown {
                trail.shown = fraction;
                trail.from = fraction;
                trail.elapsed = 0.0;
            } else if fraction < trail.shown {
                if trail.elapsed == 0.0 {
                    trail.from = trail.shown;
                }
                trail.elapsed = (trail.elapsed + time.delta_secs()).min(duration);
                let t = motion::EASING_STANDARD.ease(trail.elapsed / duration);
                trail.shown = trail.from + (fraction - trail.from) * t;
                if trail.elapsed >= duration {
                    trail.shown = fraction;
                    trail.elapsed = 0.0;
                }
            }
            let width = Val::Percent(trail.shown * 100.0);
            if node.width != width {
                node.width = width;
            }
        }
    }
}

// --- Ability button ---

/// What an ability button shows; the HUD owns it (`ability-button.md`).
#[derive(Component, Clone, PartialEq, Debug)]
pub(crate) struct AbilityView {
    /// Ability id for the skill atlas (`skill_icons`); `None` shows `icon`.
    pub ability: Option<&'static str>,
    pub icon: Icon,
    /// Key badge (desktop), e.g. `Q`.
    pub key: Option<&'static str>,
    pub cost: Option<u32>,
    pub rank: u8,
    /// `(remaining, total)` seconds.
    pub cooldown: Option<(f32, f32)>,
    /// Veiled (`color.locked.overlay`): a slot below its unlock level, or
    /// every slot while the hero is dead (then without a lock or level).
    pub locked: bool,
    /// The level the slot unlocks at, shown on the locked veil (`Lv 6`).
    pub unlock_level: Option<u8>,
    pub no_mana: bool,
    /// Show the rank pips (desktop only).
    pub pips: bool,
}

/// Maximum ability rank (`MAX_ABILITY_RANK`).
pub(crate) const MAX_RANK: u8 = shared::MAX_ABILITY_RANK;
/// Anatomy of `ability-button.md`.
pub(crate) const ABILITY_ART_INSET: f32 = 3.0;
pub(crate) const ABILITY_BADGE: f32 = 20.0;
pub(crate) const ABILITY_COST: f32 = 18.0;
pub(crate) const ABILITY_PIP: f32 = 6.0;
pub(crate) const ABILITY_PIP_GAP: f32 = 3.0;
/// Pips sit under the cost pill: their top is 18 px below the circle
/// (hud-desktop redline: slot top 610, pips 692).
pub(crate) const ABILITY_PIP_TOP: f32 = ABILITY_COST;
/// Locked veil caption (`touch.ability.locked`): `type.caption` semibold.
pub(crate) const LOCKED_LABEL: Metric = Metric::new(12.0, 12.0);
pub(crate) const ABILITY_UPGRADE: f32 = 22.0;
pub(crate) const ABILITY_UPGRADE_GLYPH: f32 = 14.0;
/// Cooldown seconds: `type.number_lg` at 26 px.
pub(crate) const COOLDOWN_TEXT: Metric = Metric::new(26.0, 26.0);

/// Sub-parts of an ability button.
#[derive(Component, Clone, Copy)]
pub(crate) struct AbilityParts {
    art: Entity,
    sweep: Entity,
    seconds: Entity,
    veil: Entity,
    veil_label: Entity,
    /// The key badge's text (the HUD writes pad glyphs into it).
    pub(crate) key: Option<Entity>,
    cost: Option<Entity>,
    pips: Option<Entity>,
    flash: Entity,
}

/// When the cooldown last ended (for the ready flash).
#[derive(Component, Default)]
pub(crate) struct ReadyFlash {
    elapsed: Option<f32>,
    cooling: bool,
}

/// A circular ability button `side` px (64; phone attack 96, utility 48):
/// art clipped to a circle, gold rim, cooldown sweep and seconds, key badge,
/// cost pill, rank pips, locked veil, ready flash. `ButtonStyle::selected`
/// is aiming (rim glow).
pub(crate) fn ability_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    view: AbilityView,
    side: f32,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    let root = parent
        .spawn((
            button_bundle(ability_node(side), ButtonKind::Skill, action, id.into()),
            KitSkin::Ability,
            NoSlab,
        ))
        .id();
    fill_ability(parent, root, view, side)
}

/// The same face without a button: the phone combat group, whose input is
/// the touch layer's (`mobile_controls`), not the kit recognizer's. The owner
/// shows the aiming glow and the held rim through [`AbilityFace`].
pub(crate) fn ability_face(
    parent: &mut ChildSpawnerCommands,
    view: AbilityView,
    side: f32,
    bundle: impl Bundle,
) -> Entity {
    let root = parent
        .spawn((ability_node(side), Pickable::IGNORE, bundle))
        .id();
    fill_ability(parent, root, view, side)
}

fn ability_node(side: f32) -> Node {
    Node {
        width: Val::Px(side),
        height: Val::Px(side),
        flex_shrink: 0.0,
        border_radius: BorderRadius::all(Val::Px(radius::PILL)),
        ..default()
    }
}

/// Aiming glow and held rim of an [`ability_face`] (a kit button paints
/// both from its interaction and `ButtonStyle::selected` instead).
#[derive(Component, Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct AbilityFace {
    pub glow: bool,
    pub held: bool,
}

fn fill_ability(
    parent: &mut ChildSpawnerCommands,
    root: Entity,
    view: AbilityView,
    side: f32,
) -> Entity {
    let mut parts = KitParts::default();
    let mut ability = None;
    let mut commands = parent.commands();
    let mut button = commands.entity(root);
    button.insert(ReadyFlash::default());
    button.with_children(|button| {
        let full = || Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(Val::Px(radius::PILL)),
            ..default()
        };
        let inset = |by: f32| Node {
            left: Val::Px(by),
            right: Val::Px(by),
            top: Val::Px(by),
            bottom: Val::Px(by),
            ..full()
        };
        parts.fill = Some(
            button
                .spawn((
                    Node {
                        display: Display::None,
                        ..full()
                    },
                    BoxShadow::new(
                        color::GOLD_400.with_alpha(0.45),
                        Val::Px(0.0),
                        Val::Px(0.0),
                        Val::Px(space::S2),
                        Val::Px(space::S8),
                    ),
                    Pickable::IGNORE,
                ))
                .id(),
        );
        let art = button
            .spawn((
                inset(ABILITY_ART_INSET),
                BackgroundColor(color::SURFACE_1_OPAQUE),
                AbilityArt(view.ability),
                Pickable::IGNORE,
            ))
            .with_children(|art| {
                if view.ability.is_none() {
                    art.spawn(icon_node(view.icon, side * 0.4, color::TEXT_PRIMARY));
                }
            })
            .id();
        let cooling = view.cooldown.is_some_and(|(remaining, _)| remaining > 0.0);
        let initially = |visible: bool| {
            if visible {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            }
        };
        let sweep = button
            .spawn((
                // No corner radius: Bevy clips an atlas cell's rounded rect
                // against the atlas offset, which hides it; the sprite is a
                // disc already.
                Node {
                    border_radius: BorderRadius::ZERO,
                    ..inset(ABILITY_ART_INSET)
                },
                KitImage::atlas(
                    Sprite::CooldownSweepAtlas,
                    theme::perceptual(color::COOLDOWN_OVERLAY),
                    view.cooldown
                        .map_or(0, |(remaining, total)| cooldown_frame(remaining, total)),
                ),
                initially(cooling),
                Pickable::IGNORE,
            ))
            .id();
        let mut veil_label = Entity::PLACEHOLDER;
        let veil = button
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(space::S2),
                    ..inset(ABILITY_ART_INSET)
                },
                BackgroundColor(theme::perceptual(color::LOCKED_OVERLAY)),
                Visibility::Hidden,
                Pickable::IGNORE,
            ))
            .with_children(|veil| {
                veil.spawn(icon_node(Icon::NavLock, size::ICON_MD, color::TEXT_MUTED));
                veil_label = veil
                    .spawn((
                        Text::new(locked_label(view.unlock_level)),
                        theme::styled_text(
                            TextStyle::keep_case(TextRole::Label).sized(LOCKED_LABEL),
                        ),
                        TextColor(color::TEXT_SECONDARY),
                        TextLayout::new_with_no_wrap(),
                    ))
                    .id();
            })
            .id();
        let flash = button
            .spawn((
                full(),
                KitImage::sprite(Sprite::DiscSoft, color::GOLD_400.with_alpha(0.0)),
                Pickable::IGNORE,
            ))
            .id();
        parts.track = Some(
            button
                .spawn((
                    full(),
                    KitImage::sprite(Sprite::RingCircle, color::GOLD_500),
                    Pickable::IGNORE,
                ))
                .id(),
        );
        button.spawn((
            Node {
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                ..inset(border::FRAME)
            },
            BorderColor::all(color::GOLD_700),
            Pickable::IGNORE,
        ));
        let mut seconds = Entity::PLACEHOLDER;
        button
            .spawn((full(), Pickable::IGNORE))
            .with_children(|centre| {
                seconds = centre
                    .spawn((
                        Text::new(
                            view.cooldown.map_or_else(String::new, |(remaining, _)| {
                                cooldown_text(remaining)
                            }),
                        ),
                        theme::styled_text(TextStyle::new(TextRole::NumberLg).sized(COOLDOWN_TEXT)),
                        TextColor(color::TEXT_PRIMARY),
                        TextShadow::default(),
                        initially(cooling),
                    ))
                    .id();
            });
        let key = view.key.map(|key| {
            let mut text = Entity::PLACEHOLDER;
            button
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(-border::FRAME),
                        top: Val::Px(-border::FRAME),
                        min_width: Val::Px(ABILITY_BADGE),
                        height: Val::Px(ABILITY_BADGE),
                        padding: UiRect::horizontal(Val::Px(space::S4)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(border::HAIRLINE)),
                        border_radius: BorderRadius::all(Val::Px(radius::SM)),
                        ..default()
                    },
                    BackgroundColor(color::SURFACE_1_OPAQUE),
                    BorderColor::all(color::GOLD_600),
                    Pickable::IGNORE,
                ))
                .with_children(|badge| {
                    text = badge
                        .spawn((
                            Text::new(key),
                            // `type.number_sm` size in the semibold body
                            // face: key letters (Q W E R) read as letters,
                            // not Barlow digits.
                            theme::styled_text(
                                TextStyle::keep_case(TextRole::Label)
                                    .sized(TextRole::NumberSm.style().size),
                            ),
                            TextColor(color::TEXT_GOLD),
                            AbilityKey,
                        ))
                        .id();
                });
            text
        });
        let cost = view.cost.map(|cost| {
            // A full-width row centres the pill, which hugs its number.
            button
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        bottom: Val::Px(-(ABILITY_COST / 2.0)),
                        left: Val::Px(0.0),
                        right: Val::Px(0.0),
                        justify_content: JustifyContent::Center,
                        display: if view.locked {
                            Display::None
                        } else {
                            Display::Flex
                        },
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_child((
                    Node {
                        min_width: Val::Px(ABILITY_COST),
                        height: Val::Px(ABILITY_COST),
                        padding: UiRect::horizontal(Val::Px(space::S4)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(border::HAIRLINE)),
                        border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                        ..default()
                    },
                    BackgroundColor(color::SURFACE_1_OPAQUE),
                    BorderColor::all(color::BORDER_SUBTLE),
                    Pickable::IGNORE,
                    children![(
                        Text::new(cost.to_string()),
                        theme::role_text(TextRole::NumberSm),
                        TextColor(color::BAR_MANA),
                        AbilityCost,
                    )],
                ))
                .id()
        });
        let pips = view.pips.then(|| {
            button
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(side + ABILITY_PIP_TOP),
                        left: Val::Px(0.0),
                        right: Val::Px(0.0),
                        column_gap: Val::Px(ABILITY_PIP_GAP),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|pips| {
                    for _ in 0..MAX_RANK {
                        pips.spawn((
                            Node {
                                width: Val::Px(ABILITY_PIP),
                                height: Val::Px(ABILITY_PIP),
                                ..default()
                            },
                            KitImage::sprite(Sprite::Pip, color::SURFACE_3),
                            Pickable::IGNORE,
                        ));
                    }
                })
                .id()
        });
        ability = Some(AbilityParts {
            art,
            sweep,
            seconds,
            veil,
            veil_label,
            key,
            cost,
            pips,
            flash,
        });
    });
    button.insert((parts, view));
    if let Some(ability) = ability {
        button.insert(ability);
    }
    root
}

/// The locked veil's caption: `touch.ability.locked` (`Lv {level}`).
fn locked_label(level: Option<u8>) -> String {
    level.map_or_else(String::new, |level| {
        crate::i18n::trf("touch.ability.locked", &[("level", &level)])
    })
}

/// The key letter of an ability's badge (the HUD rewrites it to pad glyphs).
#[derive(Component)]
pub(crate) struct AbilityKey;

/// The number in an ability's cost pill.
#[derive(Component)]
pub(crate) struct AbilityCost;

/// The level-up "+" disc (`ButtonKind::SkillUpgrade`): 22 px, top-right of
/// an ability (−4/−6), `color.emerald.400` with a 2 px `gold.400` rim.
pub(crate) fn ability_upgrade<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    ability_side: f32,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    let mut parts = KitParts::default();
    let mut button = parent.spawn((
        button_bundle(
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(ability_side - ABILITY_UPGRADE + space::S4),
                top: Val::Px(-(space::S4 + border::FRAME)),
                width: Val::Px(ABILITY_UPGRADE),
                height: Val::Px(ABILITY_UPGRADE),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(border::FRAME)),
                border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                ..default()
            },
            ButtonKind::SkillUpgrade,
            action,
            id.into(),
        ),
        NoSlab,
    ));
    button.insert(BorderColor::all(color::GOLD_400));
    button.with_children(|button| {
        parts.icon = Some(
            button
                .spawn(icon_node(
                    Icon::NavPlus,
                    ABILITY_UPGRADE_GLYPH,
                    color::TEXT_ON_PRIMARY,
                ))
                .id(),
        );
    });
    button.insert(parts).id()
}

/// The skill atlas art of an ability button, cropped to its cell.
#[derive(Component, Clone, Copy)]
pub(crate) struct AbilityArt(pub(crate) Option<&'static str>);

pub(crate) fn resolve_ability_art(
    mut commands: Commands,
    assets: Option<Res<AssetServer>>,
    textures: Res<Assets<Image>>,
    mut arts: Query<(Entity, &AbilityArt, Option<&mut ImageNode>)>,
) {
    let Some(assets) = assets else { return };
    for (entity, art, image) in &mut arts {
        let Some(ability) = art.0 else { continue };
        match image {
            None => {
                commands.entity(entity).insert(
                    ImageNode::new(assets.load(crate::skill_icons::ATLAS_PATH))
                        .with_mode(NodeImageMode::Stretch),
                );
            }
            Some(mut image) if image.rect.is_none() => {
                if let Some(texture) = textures.get(&image.image) {
                    image.rect = crate::skill_icons::icon_rect(ability, texture.size().as_vec2());
                }
            }
            Some(_) => {}
        }
    }
}

/// Cooldown sweep and seconds, locked veil and its level, cost colour, art
/// dimming, rank pips, the ready flash of every ability button, and the
/// aiming glow / held rim of an [`ability_face`].
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn paint_abilities(
    time: Res<Time>,
    locale: Option<Res<crate::i18n::Locale>>,
    mut abilities: Query<(
        Ref<AbilityView>,
        &AbilityParts,
        &mut ReadyFlash,
        Option<&AbilityFace>,
        Option<&KitParts>,
    )>,
    mut images: Query<&mut KitImage>,
    mut visibility: Query<&mut Visibility>,
    mut texts: Query<(&mut Text, &mut TextColor)>,
    mut art: Query<(&mut AbilityArt, Option<&mut ImageNode>)>,
    mut nodes: Query<&mut Node>,
    children: Query<&Children>,
) {
    let relabel = crate::i18n::locale_changed(&locale);
    for (view, parts, mut flash, face, kit) in &mut abilities {
        // The art follows the view's ability (a class change, or a view
        // spawned before the class was known); the fallback glyph hides.
        if view.is_changed() {
            if let Ok((mut ability, image)) = art.get_mut(parts.art) {
                if ability.0 != view.ability {
                    ability.0 = view.ability;
                    if let Some(mut image) = image {
                        image.rect = None;
                    }
                }
            }
            if let Ok(glyphs) = children.get(parts.art) {
                for glyph in glyphs.iter() {
                    if let Ok(mut current) = visibility.get_mut(glyph) {
                        let next = if view.ability.is_some() {
                            Visibility::Hidden
                        } else {
                            Visibility::Inherited
                        };
                        if *current != next {
                            *current = next;
                        }
                    }
                }
            }
        }
        let cooling = view.cooldown.is_some_and(|(remaining, _)| remaining > 0.0);
        let show = |visible: bool| {
            if visible {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            }
        };
        if let Ok(mut sweep) = images.get_mut(parts.sweep) {
            let frame = view
                .cooldown
                .map_or(0, |(remaining, total)| cooldown_frame(remaining, total));
            if sweep.frame != Some(frame) {
                sweep.frame = Some(frame);
            }
        }
        for (entity, visible) in [
            (parts.sweep, cooling),
            (parts.seconds, cooling),
            (parts.veil, view.locked),
        ] {
            if let Ok(mut current) = visibility.get_mut(entity) {
                let next = show(visible);
                if *current != next {
                    *current = next;
                }
            }
        }
        if let (Some((remaining, _)), Ok((mut text, _))) =
            (view.cooldown, texts.get_mut(parts.seconds))
        {
            let next = cooldown_text(remaining);
            if cooling && text.0 != next {
                text.0 = next;
            }
        }
        if view.is_changed() || relabel {
            if let Ok((mut text, _)) = texts.get_mut(parts.veil_label) {
                let next = locked_label(view.unlock_level);
                if text.0 != next {
                    text.0 = next;
                }
            }
            // A veil without an unlock level (the dead state) has no lock.
            if let Some(lock) = children
                .get(parts.veil)
                .ok()
                .and_then(|veil| veil.first().copied())
            {
                if let Ok(mut current) = visibility.get_mut(lock) {
                    let next = if view.unlock_level.is_some() {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    };
                    if *current != next {
                        *current = next;
                    }
                }
            }
        }
        if let Some(cost) = parts.cost {
            if let Ok(mut node) = nodes.get_mut(cost) {
                // A locked slot has no price yet (hud.md: `Lv 6` only).
                let display = if view.locked {
                    Display::None
                } else {
                    Display::Flex
                };
                if node.display != display {
                    node.display = display;
                }
            }
            let pill = children.get(cost).ok().and_then(|row| row.first().copied());
            let number = pill
                .and_then(|pill| children.get(pill).ok())
                .and_then(|pill| pill.first().copied());
            if let Some(Ok((mut text, mut ink))) = number.map(|number| texts.get_mut(number)) {
                let next = if view.no_mana {
                    color::TEXT_DANGER
                } else {
                    color::BAR_MANA
                };
                if ink.0 != next {
                    ink.0 = next;
                }
                if let Some(value) = view.cost {
                    let value = value.to_string();
                    if text.0 != value {
                        text.0 = value;
                    }
                }
            }
        }
        if let Ok((_, Some(mut image))) = art.get_mut(parts.art) {
            // Bevy UI has no desaturation; unaffordable art is dimmed.
            let next = if view.no_mana || view.locked {
                color::TEXT_SECONDARY
            } else {
                Color::WHITE
            };
            if image.color != next {
                image.color = next;
            }
        }
        if let Some(pips) = parts.pips.and_then(|pips| children.get(pips).ok()) {
            for (index, pip) in pips.iter().enumerate() {
                if let Ok(mut image) = images.get_mut(pip) {
                    let next = if (index as u8) < view.rank {
                        color::GOLD_400
                    } else {
                        color::SURFACE_3
                    };
                    if image.tint != next {
                        image.tint = next;
                    }
                }
            }
        }
        if let (Some(face), Some(kit)) = (face, kit) {
            if let Some(mut rim) = kit.track.and_then(|rim| images.get_mut(rim).ok()) {
                let tint = if face.held {
                    color::GOLD_300
                } else {
                    color::GOLD_500
                };
                if rim.tint != tint {
                    rim.tint = tint;
                }
            }
            if let Some(mut glow) = kit.fill.and_then(|glow| nodes.get_mut(glow).ok()) {
                let display = if face.glow || face.held {
                    Display::Flex
                } else {
                    Display::None
                };
                if glow.display != display {
                    glow.display = display;
                }
            }
        }
        // Ready flash: alpha 0 → 0.6 → 0 when a cooldown ends.
        if flash.cooling && !cooling {
            flash.elapsed = Some(0.0);
        }
        flash.cooling = cooling;
        let duration = motion::DURATION_COOLDOWN_READY_FLASH.as_secs_f32();
        let alpha = match flash.elapsed {
            Some(elapsed) if elapsed < duration => {
                flash.elapsed = Some(elapsed + time.delta_secs());
                let t = elapsed / duration;
                READY_FLASH_ALPHA * (1.0 - (2.0 * t - 1.0).abs())
            }
            _ => {
                flash.elapsed = None;
                0.0
            }
        };
        if let Ok(mut glow) = images.get_mut(parts.flash) {
            let next = color::GOLD_400.with_alpha(alpha);
            if glow.tint != next {
                glow.tint = next;
            }
        }
    }
}

/// Peak alpha of the ready flash.
pub(crate) const READY_FLASH_ALPHA: f32 = 0.6;

/// The sweep atlas cell: frame 0 full disc, 59 empty.
pub(crate) fn cooldown_frame(remaining: f32, total: f32) -> usize {
    if total <= 0.0 {
        return Sprite::CooldownSweepAtlas
            .atlas(false)
            .map_or(0, |grid| grid.frames as usize - 1);
    }
    let last = Sprite::CooldownSweepAtlas
        .atlas(false)
        .map_or(59, |grid| grid.frames - 1) as f32;
    ((1.0 - (remaining / total).clamp(0.0, 1.0)) * last).round() as usize
}

/// Seconds as shown on the sweep: one decimal below 1 s.
pub(crate) fn cooldown_text(remaining: f32) -> String {
    if remaining < 1.0 {
        format!("{remaining:.1}")
    } else {
        format!("{:.0}", remaining.ceil())
    }
}

// --- Item slot and shop card ---

/// Inventory slot corner radius (`item-slot.md`: 6) and icon size (34).
pub(crate) const ITEM_SLOT_RADIUS: f32 = 6.0;
pub(crate) const ITEM_ICON: f32 = 34.0;

/// An inventory slot (plain node on the HUD): 40 / 44, `color.surface.0`,
/// subtle border, the item's 32 px glyph tinted `gold.400`, optional stack
/// count bottom-right.
pub(crate) fn item_slot(
    parent: &mut ChildSpawnerCommands,
    icon: Option<Icon>,
    count: Option<u32>,
    form: Form,
) -> Entity {
    let side = size::ITEM_SLOT.at(form);
    parent
        .spawn((
            Node {
                width: Val::Px(side),
                height: Val::Px(side),
                flex_shrink: 0.0,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::all(Val::Px(ITEM_SLOT_RADIUS)),
                ..default()
            },
            BackgroundColor(color::SURFACE_0),
            BorderColor::all(color::BORDER_SUBTLE),
        ))
        .with_children(|slot| {
            if let Some(icon) = icon {
                slot.spawn(icon_node(
                    icon,
                    ITEM_ICON.min(side - space::S4),
                    color::GOLD_400,
                ));
            }
            if let Some(count) = count {
                slot.spawn((
                    Text::new(count.to_string()),
                    theme::role_text(TextRole::NumberSm),
                    TextColor(color::TEXT_PRIMARY),
                    TextShadow::default(),
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(space::S2),
                        bottom: Val::Px(0.0),
                        ..default()
                    },
                ));
            }
        })
        .id()
}

/// An inventory slot as a shop control (`ButtonKind::ShopItem`, selected =
/// owned): the slot look with the item card's states.
pub(crate) fn item_slot_button<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    icon: Icon,
    form: Form,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    let side = size::ITEM_SLOT.at(form);
    let mut parts = KitParts::default();
    let mut button = parent.spawn((
        button_bundle(
            Node {
                width: Val::Px(side),
                height: Val::Px(side),
                flex_shrink: 0.0,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::all(Val::Px(ITEM_SLOT_RADIUS)),
                ..default()
            },
            ButtonKind::ShopItem,
            action,
            id.into(),
        ),
        KitSkin::ShopCard,
        UiTransform::IDENTITY,
    ));
    button.with_children(|slot| {
        parts.icon = Some(
            slot.spawn(icon_node(
                icon,
                ITEM_ICON.min(side - space::S4),
                color::GOLD_400,
            ))
            .id(),
        );
    });
    button.insert(parts).id()
}

/// The 32 px glyph of a catalogue item.
pub(crate) fn item_icon(item: shared::shop::ItemId) -> Icon {
    use shared::shop::ItemId;
    match item {
        ItemId::EmberBlade => Icon::ItemEmberBlade32,
        ItemId::SwiftGrip => Icon::ItemSwiftGrip32,
        ItemId::TrailBoots => Icon::ItemTrailBoots32,
        ItemId::VitalityGem => Icon::ItemVitalityGem32,
        ItemId::FocusCharm => Icon::ItemFocusCharm32,
        ItemId::GuardianCrest => Icon::ItemGuardianCrest32,
    }
}

/// The 24 px glyph of a hero class.
pub(crate) fn class_icon(class: shared::HeroClass) -> Icon {
    use shared::HeroClass;
    match class {
        HeroClass::Warrior => Icon::ClassWarrior,
        HeroClass::Mage => Icon::ClassMage,
        HeroClass::Ranger => Icon::ClassRanger,
        HeroClass::Cleric => Icon::ClassCleric,
        HeroClass::Warden => Icon::ClassWarden,
    }
}

/// Shop card width (`item-slot.md`: 176 desktop grid of 3, 156 phone).
pub(crate) const SHOP_CARD_W: Metric = Metric::new(176.0, 156.0);

/// What a shop card shows.
pub(crate) struct ShopCard {
    pub item: shared::shop::ItemId,
    pub price: u32,
    pub owned: bool,
    pub recommended: bool,
    /// Gold still missing (unaffordable: the card is disabled).
    pub missing: Option<u32>,
}

/// A shop item card (`ButtonKind::ShopItem`, selected = owned): icon slot,
/// name (`type.label`), price (`type.number` gold with `hud/gold`), effect
/// line (`type.caption` muted), recommended star and inner gold line,
/// "Need N more gold" when unaffordable.
pub(crate) fn shop_card<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    card: ShopCard,
    form: Form,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    let mut parts = KitParts::default();
    let mut button = parent.spawn((
        button_bundle(
            Node {
                width: Val::Px(SHOP_CARD_W.at(form)),
                padding: UiRect::axes(Val::Px(space::S12), Val::Px(space::S8 + border::FRAME)),
                column_gap: Val::Px(space::S12),
                align_items: AlignItems::FlexStart,
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::all(Val::Px(radius::MD)),
                ..default()
            },
            ButtonKind::ShopItem,
            action,
            id.into(),
        ),
        KitSkin::ShopCard,
    ));
    button.insert(ButtonStyle {
        kind: ButtonKind::ShopItem,
        selected: card.owned,
    });
    button.with_children(|card_node| {
        if card.recommended {
            card_node.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    border: UiRect::all(Val::Px(border::HAIRLINE)),
                    border_radius: BorderRadius::all(Val::Px(radius::MD - border::HAIRLINE)),
                    ..default()
                },
                BorderColor::all(color::GOLD_600),
                Pickable::IGNORE,
            ));
            card_node.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(space::S8),
                    top: Val::Px(space::S8),
                    ..default()
                },
                children![icon_node(Icon::NavStar, size::ICON_SM, color::TEXT_GOLD)],
            ));
        }
        item_slot(card_node, Some(item_icon(card.item)), None, Form::Desktop);
        card_node
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                min_width: Val::Px(0.0),
                flex_grow: 1.0,
                overflow: Overflow::clip_x(),
                ..default()
            })
            .with_children(|text| {
                parts.label = Some(
                    text.spawn((
                        Text::new(crate::i18n::data::item_name(card.item)),
                        theme::styled_text(TextStyle::keep_case(TextRole::Label)),
                        TextColor(color::TEXT_PRIMARY),
                        TextLayout::new_with_no_wrap(),
                    ))
                    .id(),
                );
                text.spawn(Node {
                    column_gap: Val::Px(space::S4),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|price| {
                    price.spawn(icon_node(Icon::HudGold, size::ICON_SM, color::TEXT_GOLD));
                    price.spawn((
                        Text::new(card.price.to_string()),
                        theme::role_text(TextRole::Number),
                        TextColor(color::TEXT_GOLD),
                    ));
                });
                let (line, ink) = match card.missing {
                    Some(gold) => (
                        crate::i18n::trf("shop.reason.need_gold", &[("gold", &gold)]),
                        color::TEXT_DANGER,
                    ),
                    None => (
                        crate::i18n::data::item_desc(card.item).to_owned(),
                        color::TEXT_MUTED,
                    ),
                };
                parts.extra[0] = Some(
                    text.spawn((
                        Text::new(line),
                        theme::styled_text(TextStyle::keep_case(TextRole::Caption)),
                        TextColor(ink),
                        TextLayout::new_with_no_wrap(),
                    ))
                    .id(),
                );
            });
    });
    let entity = button.insert(parts).id();
    if card.missing.is_some() {
        parent
            .commands()
            .entity(entity)
            .insert(crate::ui::Pressable {
                disabled: true,
                ..default()
            });
    }
    entity
}

// --- Hero tile and portrait ---

/// Hero tile anatomy (`hero-tile.md`): art 88 high, name line, 3 px source
/// edge; the tile is `size.hero_tile.*` wide and 20 px taller.
pub(crate) const HERO_TILE_EXTRA_H: f32 = space::S16 + space::S4;
pub(crate) const HERO_ART_H: Metric = Metric::new(88.0, 68.0);
pub(crate) const SOURCE_EDGE: f32 = 3.0;

/// Where an avatar comes from: the tile's bottom edge colour.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AvatarSource {
    Included,
    Studio,
    Owned,
    Supporter,
}

impl AvatarSource {
    pub(crate) fn color(self) -> Color {
        match self {
            AvatarSource::Included => color::SOURCE_INCLUDED,
            AvatarSource::Studio => color::SOURCE_STUDIO,
            AvatarSource::Owned => color::SOURCE_OWNED,
            AvatarSource::Supporter => color::SOURCE_SUPPORTER,
        }
    }
}

/// A hero tile (`ButtonKind::Tile`): roster art cover-cropped (or the class
/// icon on `color.surface.3` when there is no art, never initials), the name
/// in `type.caption` semibold and the source edge. Disabled = not unlocked:
/// art dimmed and a lock.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hero_tile<T: UiActionT>(
    parent: &mut ChildSpawnerCommands,
    art: Option<String>,
    class: shared::HeroClass,
    name: impl UiLabel,
    source: AvatarSource,
    form: Form,
    action: T,
    id: impl Into<TestId>,
) -> Entity {
    let width = size::HERO_TILE.at(form);
    let mut parts = KitParts::default();
    let mut button = parent.spawn((
        button_bundle(
            Node {
                width: Val::Px(width),
                height: Val::Px(width + HERO_TILE_EXTRA_H),
                flex_direction: FlexDirection::Column,
                flex_shrink: 0.0,
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::all(Val::Px(radius::MD)),
                overflow: Overflow::clip(),
                ..default()
            },
            ButtonKind::Tile,
            action,
            id.into(),
        ),
        KitSkin::HeroTile,
        UiTransform::IDENTITY,
    ));
    button.with_children(|tile| {
        let art_node = Node {
            width: Val::Percent(100.0),
            height: Val::Px(HERO_ART_H.at(form)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        };
        let art_entity = match art {
            Some(path) => tile
                .spawn((
                    art_node,
                    BackgroundColor(color::SURFACE_3),
                    Art { path },
                    Pickable::IGNORE,
                ))
                .id(),
            None => tile
                .spawn((
                    art_node,
                    BackgroundColor(color::SURFACE_3),
                    Pickable::IGNORE,
                    children![icon_node(
                        class_icon(class),
                        size::ICON_XL,
                        color::TEXT_MUTED
                    )],
                ))
                .id(),
        };
        parts.icon = None;
        parts.knob = Some(art_entity);
        tile.spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(SOURCE_EDGE),
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(source.color()),
            Pickable::IGNORE,
        ));
        parts.label = Some(
            tile.spawn((
                name.into_text(),
                theme::styled_text(
                    TextStyle::keep_case(TextRole::Label).sized(TextRole::Caption.style().size),
                ),
                TextColor(color::TEXT_PRIMARY),
                TextLayout::new(Justify::Center, LineBreak::NoWrap),
                Node {
                    flex_grow: 1.0,
                    align_self: AlignSelf::Center,
                    margin: UiRect::vertical(Val::Auto),
                    ..default()
                },
            ))
            .id(),
        );
        parts.fill = Some(
            tile.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(space::S4),
                    top: Val::Px(space::S4),
                    display: Display::None,
                    ..default()
                },
                Pickable::IGNORE,
                children![icon_node(Icon::NavLock, size::ICON_SM, color::TEXT_PRIMARY)],
            ))
            .id(),
        );
    });
    button.insert(parts).id()
}

/// Selected glow and border width, disabled art dimming and lock.
pub(crate) fn paint_hero_tile(
    selected: bool,
    state: ButtonState,
    parts: &KitParts,
    nodes: &mut Query<
        (
            &mut Node,
            Option<&mut BackgroundColor>,
            Option<&mut BorderColor>,
            Option<&mut super::controls::Slide>,
        ),
        (Without<KitSkin>, Without<TextColor>),
    >,
) {
    if let Some((mut lock, ..)) = parts.fill.and_then(|lock| nodes.get_mut(lock).ok()) {
        let display = if state == ButtonState::Disabled {
            Display::Flex
        } else {
            Display::None
        };
        if lock.display != display {
            lock.display = display;
        }
    }
    let _ = selected;
}

/// Border width, glow and art tint of hero tiles (the parts the generic
/// painter does not reach: `Node::border`, `BoxShadow` and the art
/// `ImageNode`, which arrives once the art loads). Runs every frame; it only
/// writes on a difference.
#[allow(clippy::type_complexity)]
pub(crate) fn paint_hero_tile_frames(
    mut commands: Commands,
    mut tiles: Query<(
        Entity,
        &KitSkin,
        &ButtonStyle,
        &KitParts,
        &Interaction,
        &crate::ui::Pressable,
        Option<&super::PreviewState>,
        &mut Node,
        Has<BoxShadow>,
    )>,
    mut art: Query<&mut ImageNode>,
) {
    for (entity, skin, style, parts, interaction, pressable, preview, mut node, glowing) in
        &mut tiles
    {
        if *skin != KitSkin::HeroTile {
            continue;
        }
        let state = super::kit_state(*interaction, pressable, preview);
        let width = if style.selected {
            border::FRAME
        } else {
            border::HAIRLINE
        };
        let next = UiRect::all(Val::Px(width));
        if node.border != next {
            node.border = next;
        }
        if style.selected && !glowing {
            commands.entity(entity).insert(BoxShadow::new(
                color::GOLD_400.with_alpha(0.35),
                Val::Px(0.0),
                Val::Px(0.0),
                Val::Px(0.0),
                Val::Px(space::S8),
            ));
        } else if !style.selected && glowing {
            commands.entity(entity).remove::<BoxShadow>();
        }
        if let Some(mut image) = parts
            .knob
            .and_then(|art_entity| art.get_mut(art_entity).ok())
        {
            let tint = if state == ButtonState::Disabled {
                color::TEXT_DISABLED
            } else {
                Color::WHITE
            };
            if image.color != tint {
                image.color = tint;
            }
        }
    }
}

/// Portrait level disc (`hero-tile.md`: 22, bottom-left).
pub(crate) const LEVEL_DISC: f32 = 22.0;
/// XP ring thickness.
pub(crate) const XP_RING: f32 = 3.0;

/// A portrait with its XP ring and level disc (plain node): `side` circle
/// (`size.portrait.md` 56), art inset 3, ring `color.bar.xp` over
/// `color.surface.3`, level `type.number_sm` gold.
pub(crate) fn portrait(
    parent: &mut ChildSpawnerCommands,
    art: String,
    level: u32,
    xp: f32,
    side: f32,
) -> Entity {
    parent
        .spawn(Node {
            width: Val::Px(side),
            height: Val::Px(side),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|portrait| {
            timer_ring_layers(portrait, xp, color::SURFACE_3, color::BAR_XP);
            portrait.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(XP_RING),
                    top: Val::Px(XP_RING),
                    ..default()
                },
                children![round_art(art, side - 2.0 * XP_RING)],
            ));
            portrait
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(-space::S2),
                        bottom: Val::Px(-space::S2),
                        width: Val::Px(LEVEL_DISC),
                        height: Val::Px(LEVEL_DISC),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(border::HAIRLINE)),
                        border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                        ..default()
                    },
                    BackgroundColor(color::SURFACE_1_OPAQUE),
                    BorderColor::all(color::GOLD_500),
                ))
                .with_child((
                    Text::new(level.to_string()),
                    theme::role_text(TextRole::NumberSm),
                    TextColor(color::TEXT_GOLD),
                ));
        })
        .id()
}

/// Anatomy of a live portrait (`hud.md`, `target-hero.md`): circle `side`,
/// XP ring (player status) or a rim, level disc `disc` at `disc_at` (left,
/// bottom offsets from the circle), fallback icon size.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct PortraitSpec {
    pub side: f32,
    pub ring: bool,
    pub disc: f32,
    pub disc_at: Vec2,
    pub icon: f32,
}

/// What a live portrait shows; the HUD owns it and [`paint_portraits`]
/// follows every change.
#[derive(Component, Clone, PartialEq, Debug)]
pub(crate) struct PortraitView {
    /// Avatar thumbnail path; `None` shows `fallback` on `color.surface.3`.
    pub art: Option<String>,
    pub fallback: Icon,
    /// Level disc; hidden when `None`.
    pub level: Option<u32>,
    /// XP ring progress `0..=1` (portraits spawned with a ring).
    pub xp: f32,
    /// Dead / disconnected: art and icon dimmed (Bevy UI cannot desaturate).
    pub grey: bool,
    /// Boss or base: `border.frame` `color.gold.500` rim instead of the
    /// hairline `color.gold.600`.
    pub strong_rim: bool,
}

/// The parts [`paint_portraits`] repaints.
#[derive(Component, Clone, Copy)]
pub(crate) struct PortraitParts {
    art: Entity,
    icon: Entity,
    disc: Entity,
    level: Entity,
    ring: bool,
}

/// A portrait that follows a [`PortraitView`]: the art (or class / kind
/// icon), an optional XP ring, a level disc.
pub(crate) fn live_portrait(
    parent: &mut ChildSpawnerCommands,
    view: PortraitView,
    spec: PortraitSpec,
    bundle: impl Bundle,
) -> Entity {
    let mut parts = None;
    let mut root = parent.spawn((
        Node {
            width: Val::Px(spec.side),
            height: Val::Px(spec.side),
            flex_shrink: 0.0,
            ..default()
        },
        Pickable::IGNORE,
        bundle,
    ));
    root.with_children(|portrait| {
        if spec.ring {
            timer_ring_layers(portrait, view.xp, color::SURFACE_3, color::BAR_XP);
        }
        let inset = if spec.ring { XP_RING } else { 0.0 };
        let mut icon = Entity::PLACEHOLDER;
        let art = portrait
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(inset),
                    top: Val::Px(inset),
                    width: Val::Px(spec.side - 2.0 * inset),
                    height: Val::Px(spec.side - 2.0 * inset),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(if spec.ring { 0.0 } else { border::HAIRLINE })),
                    border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(color::SURFACE_3),
                BorderColor::all(color::GOLD_600),
                Pickable::IGNORE,
            ))
            .with_children(|art| {
                icon = art
                    .spawn(icon_node(view.fallback, spec.icon, color::TEXT_GOLD))
                    .id();
            })
            .id();
        let mut level = Entity::PLACEHOLDER;
        let disc = portrait
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(spec.disc_at.x),
                    bottom: Val::Px(spec.disc_at.y),
                    width: Val::Px(spec.disc),
                    height: Val::Px(spec.disc),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(border::HAIRLINE)),
                    border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(color::SURFACE_1_OPAQUE),
                BorderColor::all(color::GOLD_500),
                Pickable::IGNORE,
            ))
            .with_children(|disc| {
                level = disc
                    .spawn((
                        Text::new(""),
                        theme::role_text(TextRole::NumberSm),
                        TextColor(color::TEXT_GOLD),
                    ))
                    .id();
            })
            .id();
        parts = Some(PortraitParts {
            art,
            icon,
            disc,
            level,
            ring: spec.ring,
        });
    });
    root.insert(view);
    if let Some(parts) = parts {
        root.insert(parts);
    }
    root.id()
}

/// Art, fallback icon, level disc, XP arc, dimming and rim of every live
/// portrait (every frame, writing only differences: the thumbnail's
/// `ImageNode` arrives after the art loads).
#[allow(clippy::type_complexity)]
pub(crate) fn paint_portraits(
    mut commands: Commands,
    portraits: Query<(Ref<PortraitView>, &PortraitParts, &Children)>,
    arts: Query<Option<&Art>>,
    mut images: Query<&mut ImageNode>,
    mut kit_images: Query<&mut KitImage>,
    mut nodes: Query<(&mut Node, Option<&mut BorderColor>)>,
    mut texts: Query<&mut Text>,
    arcs: Query<(), With<RingArc>>,
) {
    for (view, parts, children) in &portraits {
        if view.is_changed() {
            let wanted = view.art.as_ref();
            let current = arts.get(parts.art).ok().flatten().map(|art| &art.path);
            if wanted != current {
                match wanted {
                    Some(path) => {
                        commands
                            .entity(parts.art)
                            .insert(Art { path: path.clone() });
                    }
                    None => {
                        commands
                            .entity(parts.art)
                            .remove::<(Art, ImageNode, CoverImage)>();
                    }
                }
            }
            if let Ok(mut text) = texts.get_mut(parts.level) {
                let next = view
                    .level
                    .map_or_else(String::new, |level| level.to_string());
                if text.0 != next {
                    text.0 = next;
                }
            }
            if let Ok((mut node, _)) = nodes.get_mut(parts.disc) {
                let display = if view.level.is_some() {
                    Display::Flex
                } else {
                    Display::None
                };
                if node.display != display {
                    node.display = display;
                }
            }
            if let Ok((mut node, border)) = nodes.get_mut(parts.art) {
                if !parts.ring {
                    let (width, rim) = if view.strong_rim {
                        (border::FRAME, color::GOLD_500)
                    } else {
                        (border::HAIRLINE, color::GOLD_600)
                    };
                    let next = UiRect::all(Val::Px(width));
                    if node.border != next {
                        node.border = next;
                    }
                    if let Some(mut border) = border {
                        let next = BorderColor::all(rim);
                        if *border != next {
                            *border = next;
                        }
                    }
                }
            }
            if parts.ring {
                for child in children.iter() {
                    if arcs.contains(child) {
                        if let Ok(mut arc) = kit_images.get_mut(child) {
                            let frame = Some(ring_frame(view.xp));
                            if arc.frame != frame {
                                arc.frame = frame;
                            }
                        }
                    }
                }
            }
        }
        let has_art = view.art.is_some();
        if let Ok((mut node, _)) = nodes.get_mut(parts.icon) {
            let display = if has_art {
                Display::None
            } else {
                Display::Flex
            };
            if node.display != display {
                node.display = display;
            }
        }
        if let Ok(mut icon) = kit_images.get_mut(parts.icon) {
            let tint = if view.grey {
                color::TEXT_DISABLED
            } else {
                color::TEXT_GOLD
            };
            let source = KitImage::icon(view.fallback, tint);
            if *icon != source {
                *icon = source;
            }
        }
        if let Ok(mut image) = images.get_mut(parts.art) {
            let next = if view.grey {
                color::TEXT_DISABLED
            } else {
                Color::WHITE
            };
            if image.color != next {
                image.color = next;
            }
        }
    }
}

// --- Timer ring ---

/// A timer ring's progress (`0..=1`, from 12 o'clock clockwise) and
/// whether it is in its warning phase (countdown ≤ 5 s).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub(crate) struct TimerRing {
    pub progress: f32,
    pub warning: bool,
}

/// The two atlas layers of a ring (track = full frame, live arc = progress).
fn timer_ring_layers(parent: &mut ChildSpawnerCommands, progress: f32, track: Color, arc: Color) {
    let full = Node {
        position_type: PositionType::Absolute,
        left: Val::Px(0.0),
        right: Val::Px(0.0),
        top: Val::Px(0.0),
        bottom: Val::Px(0.0),
        ..default()
    };
    parent.spawn((
        full.clone(),
        KitImage::atlas(Sprite::TimerRingAtlas, track, ring_frame(1.0)),
        crate::ui::kit_assets::LowDensity,
        Pickable::IGNORE,
    ));
    parent.spawn((
        full,
        KitImage::atlas(Sprite::TimerRingAtlas, arc, ring_frame(progress)),
        crate::ui::kit_assets::LowDensity,
        RingArc,
        Pickable::IGNORE,
    ));
}

/// The live arc layer of a ring.
#[derive(Component)]
pub(crate) struct RingArc;

/// Timer-ring atlas cell for a progress (frame 0 empty, 59 full).
pub(crate) fn ring_frame(progress: f32) -> usize {
    let last = Sprite::TimerRingAtlas
        .atlas(false)
        .map_or(59, |grid| grid.frames - 1) as f32;
    (progress.clamp(0.0, 1.0) * last).round() as usize
}

/// Ring sizes (`timer-ring.md`): lg 160 (ring 8, `type.number_xl`), md 96
/// (ring 6, `type.number_lg`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RingSize {
    Large,
    Medium,
}

/// A timer ring: track `color.surface.3`, arc `color.gold.400` (warning
/// `color.state.warning`), centre disc `color.surface.1.opaque`, `time` and
/// an optional numbers-only caption (`type.number_sm` muted).
pub(crate) fn timer_ring(
    parent: &mut ChildSpawnerCommands,
    ring: TimerRing,
    ring_size: RingSize,
    time: String,
    caption: Option<String>,
) -> Entity {
    let (side, thickness, role) = match ring_size {
        RingSize::Large => (size::TIMER_RING_LG, space::S8, TextRole::NumberXl),
        RingSize::Medium => (
            size::TIMER_RING_MD,
            space::S4 + border::FRAME,
            TextRole::NumberLg,
        ),
    };
    parent
        .spawn((
            Node {
                width: Val::Px(side),
                height: Val::Px(side),
                flex_shrink: 0.0,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            ring,
        ))
        .with_children(|root| {
            timer_ring_layers(root, ring.progress, color::SURFACE_3, ring_color(ring));
            root.spawn((
                Node {
                    width: Val::Px(side - 2.0 * thickness - border::FRAME),
                    height: Val::Px(side - 2.0 * thickness - border::FRAME),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                    ..default()
                },
                BackgroundColor(color::SURFACE_1_OPAQUE),
            ))
            .with_children(|disc| {
                disc.spawn((
                    Text::new(time),
                    theme::role_text(role),
                    TextColor(color::TEXT_PRIMARY),
                ));
                if let Some(caption) = caption {
                    disc.spawn((
                        Text::new(caption),
                        theme::role_text(TextRole::NumberSm),
                        TextColor(color::TEXT_MUTED),
                    ));
                }
            });
        })
        .id()
}

fn ring_color(ring: TimerRing) -> Color {
    if ring.warning {
        color::STATE_WARNING
    } else {
        color::GOLD_400
    }
}

/// Moves each ring's arc to its progress.
pub(crate) fn paint_timer_rings(
    rings: Query<(&TimerRing, &Children), Changed<TimerRing>>,
    mut arcs: Query<&mut KitImage, With<RingArc>>,
) {
    for (ring, children) in &rings {
        for child in children.iter() {
            if let Ok(mut arc) = arcs.get_mut(child) {
                let frame = Some(ring_frame(ring.progress));
                if arc.frame != frame {
                    arc.frame = frame;
                }
                let tint = ring_color(*ring);
                if arc.tint != tint {
                    arc.tint = tint;
                }
            }
        }
    }
}

// --- Scoreboard row ---

/// One scoreboard line (`scoreboard-row.md`).
pub(crate) struct ScoreRow {
    pub team: crate::domain::Team,
    pub art: String,
    pub name: String,
    pub class: String,
    pub level: u32,
    pub kda: (u32, u32, u32),
    pub cs: u32,
    pub gold: u32,
    pub items: Vec<Option<Icon>>,
    pub own: bool,
    pub disconnected: bool,
}

/// Column widths of the desktop row: portrait 40 · name 1fr · level 32 ·
/// K/D/A 72 · CS 48 · gold 56 · items 6 × 20 (gap 2).
pub(crate) const SCORE_COLUMNS: [f32; 4] = [32.0, 72.0, 48.0, 56.0];
pub(crate) const SCORE_ROW_H: Metric = Metric::new(48.0, size::TOUCH_MIN);
pub(crate) const SCORE_PORTRAIT: Metric = Metric::new(size::PORTRAIT_SM, 32.0);
pub(crate) const SCORE_ITEM: f32 = 20.0;
pub(crate) const SCORE_EDGE: f32 = 3.0;
pub(crate) const SCORE_RADIUS: f32 = 6.0;

/// A scoreboard player row: team edge, portrait, name + class, level,
/// K/D/A, CS, gold, six item cells; own row selected, disconnected dimmed
/// with a muted "off" badge. Phone drops CS and items.
pub(crate) fn scoreboard_row(
    parent: &mut ChildSpawnerCommands,
    row: ScoreRow,
    form: Form,
) -> Entity {
    let team = match row.team {
        crate::domain::Team::Green => color::TEAM_GREEN,
        crate::domain::Team::Blue => color::TEAM_BLUE,
    };
    let ink = if row.disconnected {
        color::TEXT_DISABLED
    } else {
        color::TEXT_PRIMARY
    };
    let (fill, edge) = if row.own {
        (color::SURFACE_SELECTED, color::GOLD_600)
    } else {
        (color::SURFACE_2, Color::NONE)
    };
    parent
        .spawn((
            Node {
                height: Val::Px(SCORE_ROW_H.at(form)),
                width: Val::Percent(100.0),
                padding: UiRect::new(
                    Val::Px(space::S8 + border::FRAME),
                    Val::Px(space::S8 + border::FRAME),
                    Val::Px(0.0),
                    Val::Px(0.0),
                ),
                column_gap: Val::Px(space::S8),
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::new(
                    Val::Px(0.0),
                    Val::Px(SCORE_RADIUS),
                    Val::Px(SCORE_RADIUS),
                    Val::Px(0.0),
                ),
                ..default()
            },
            BackgroundColor(fill),
            BorderColor::all(edge),
        ))
        .with_children(|line| {
            line.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    width: Val::Px(SCORE_EDGE),
                    ..default()
                },
                BackgroundColor(team),
            ));
            line.spawn(round_art(row.art.clone(), SCORE_PORTRAIT.at(form)));
            line.spawn(Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                min_width: Val::Px(0.0),
                overflow: Overflow::clip_x(),
                ..default()
            })
            .with_children(|name| {
                name.spawn(Node {
                    column_gap: Val::Px(space::S8),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|top| {
                    top.spawn((
                        Text::new(row.name.clone()),
                        theme::styled_text(TextStyle::keep_case(TextRole::Label)),
                        TextColor(ink),
                        TextLayout::new_with_no_wrap(),
                    ));
                    if row.disconnected {
                        surfaces_badge_off(top);
                    }
                });
                name.spawn((
                    Text::new(row.class.clone()),
                    theme::styled_text(TextStyle::keep_case(TextRole::Caption)),
                    TextColor(if row.disconnected {
                        ink
                    } else {
                        color::TEXT_MUTED
                    }),
                ));
            });
            let numbers = [
                row.level.to_string(),
                format!("{}/{}/{}", row.kda.0, row.kda.1, row.kda.2),
                row.cs.to_string(),
                group_digits(row.gold),
            ];
            for (index, (value, width)) in numbers.into_iter().zip(SCORE_COLUMNS).enumerate() {
                if form == Form::Phone && index == 2 {
                    continue;
                }
                let gold = index == 3 && !row.disconnected;
                line.spawn((
                    Text::new(value),
                    theme::role_text(TextRole::Number),
                    TextColor(if gold { color::TEXT_GOLD } else { ink }),
                    TextLayout::new_with_justify(Justify::Right),
                    Node {
                        width: Val::Px(width),
                        flex_shrink: 0.0,
                        ..default()
                    },
                ));
            }
            if form == Form::Desktop {
                line.spawn(Node {
                    column_gap: Val::Px(space::S2),
                    flex_shrink: 0.0,
                    ..default()
                })
                .with_children(|items| {
                    for slot in row
                        .items
                        .iter()
                        .copied()
                        .chain(std::iter::repeat(None))
                        .take(6)
                    {
                        items
                            .spawn((
                                Node {
                                    width: Val::Px(SCORE_ITEM),
                                    height: Val::Px(SCORE_ITEM),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    border: UiRect::all(Val::Px(border::HAIRLINE)),
                                    border_radius: BorderRadius::all(Val::Px(radius::SM)),
                                    ..default()
                                },
                                BackgroundColor(color::SURFACE_0),
                                BorderColor::all(color::BORDER_SUBTLE),
                            ))
                            .with_children(|cell| {
                                if let Some(icon) = slot {
                                    cell.spawn(icon_node(icon, size::ICON_SM, color::GOLD_400));
                                }
                            });
                    }
                });
            }
        })
        .id()
}

fn surfaces_badge_off(parent: &mut ChildSpawnerCommands) {
    super::surfaces::badge(
        parent,
        crate::i18n::Localized::new("kit.badge.off"),
        super::surfaces::BadgeKind::Muted,
        false,
    );
}

/// `7401` → `7 401` (thin grouping, as the scoreboard sheet shows).
pub(crate) fn group_digits(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(digit);
    }
    out
}

// --- HUD plates ---

/// HUD plate padding (`hud-plate.md`: 8 / 10).
pub(crate) const PLATE_PADDING: (f32, f32) = (space::S8, space::S8 + border::FRAME);

/// A HUD plate: glass fill (`strong` under text), hairline gold,
/// `radius.md`, padding 8 / 10.
pub(crate) fn hud_plate(strong: bool) -> impl Bundle {
    (
        Node {
            padding: UiRect::axes(Val::Px(PLATE_PADDING.1), Val::Px(PLATE_PADDING.0)),
            column_gap: Val::Px(space::S12),
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            ..default()
        },
        BackgroundColor(if strong {
            theme::perceptual(color::SURFACE_GLASS_STRONG)
        } else {
            theme::perceptual(color::SURFACE_GLASS)
        }),
        BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)),
    )
}

/// The minimap frame: `size.minimap.*` square, 2 px `gold.500` rim and a
/// 1 px `gold.700` inner line, no ornament.
pub(crate) fn minimap_frame(parent: &mut ChildSpawnerCommands, form: Form) -> Entity {
    let side = size::MINIMAP.at(form);
    parent
        .spawn((
            Node {
                width: Val::Px(side),
                height: Val::Px(side),
                border: UiRect::all(Val::Px(border::FRAME)),
                border_radius: BorderRadius::all(Val::Px(radius::SM)),
                ..default()
            },
            BackgroundColor(color::SURFACE_1),
            BorderColor::all(color::GOLD_500),
        ))
        .with_child((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                ..default()
            },
            BorderColor::all(color::GOLD_700),
            Pickable::IGNORE,
        ))
        .id()
}

/// Player status plate: portrait with XP ring, HP and mana bars, gold.
#[allow(clippy::too_many_arguments)]
pub(crate) fn player_status(
    parent: &mut ChildSpawnerCommands,
    art: String,
    level: u32,
    xp: f32,
    hp: BarValue,
    mana: BarValue,
    gold: u32,
    form: Form,
) -> Entity {
    parent
        .spawn(hud_plate(true))
        .with_children(|plate| {
            portrait(plate, art, level, xp, size::PORTRAIT_MD);
            plate
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(space::S4),
                    width: Val::Px(size::TARGET_FRAME_WIDTH.at(form) - size::PORTRAIT_MD),
                    ..default()
                })
                .with_children(|column| {
                    bar(column, BarKind::HpSelf, hp, Val::Percent(100.0), form, true);
                    bar(
                        column,
                        BarKind::Mana,
                        mana,
                        Val::Percent(100.0),
                        form,
                        false,
                    );
                    column
                        .spawn(Node {
                            column_gap: Val::Px(space::S4),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|gold_line| {
                            gold_line.spawn(icon_node(
                                Icon::HudGold,
                                size::ICON_SM,
                                color::TEXT_GOLD,
                            ));
                            gold_line.spawn((
                                Text::new(gold.to_string()),
                                theme::role_text(TextRole::Number),
                                TextColor(color::TEXT_GOLD),
                            ));
                        });
                });
        })
        .id()
}

/// Score strip (hud.md `score-strip`): team kills `type.number_lg` in team
/// colours around a muted colon, a divider, and `edge.kda` (`type.eyebrow`
/// muted) over the own K/D/A (`type.number_sm`). The match has no clock
/// (hud.md § Out of scope), so there is no timer.
pub(crate) fn score_strip(
    parent: &mut ChildSpawnerCommands,
    green: u32,
    blue: u32,
    kda: (u32, u32, u32),
) -> Entity {
    parent
        .spawn(hud_plate(true))
        .with_children(|plate| {
            for (text, ink) in [
                (green.to_string(), color::TEAM_GREEN),
                (":".to_owned(), color::TEXT_MUTED),
                (blue.to_string(), color::TEAM_BLUE),
            ] {
                plate.spawn((
                    Text::new(text),
                    theme::role_text(TextRole::NumberLg),
                    TextColor(ink),
                ));
            }
            plate.spawn((
                Node {
                    width: Val::Px(border::HAIRLINE),
                    height: Val::Px(space::S24),
                    ..default()
                },
                BackgroundColor(color::BORDER_SUBTLE),
            ));
            plate
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|column| {
                    column.spawn((
                        crate::i18n::Localized::new("edge.kda").into_text(),
                        theme::role_text(TextRole::Eyebrow),
                        TextColor(color::TEXT_MUTED),
                    ));
                    column.spawn((
                        Text::new(format!("{}/{}/{}", kda.0, kda.1, kda.2)),
                        theme::role_text(TextRole::NumberSm),
                        TextColor(color::TEXT_PRIMARY),
                    ));
                });
        })
        .id()
}

/// Target frame: `size.target_frame.width.*` wide, portrait 40, name
/// (`type.label`), class icon 16, HP bar in the enemy/ally colour.
#[allow(clippy::too_many_arguments)]
pub(crate) fn target_frame(
    parent: &mut ChildSpawnerCommands,
    art: String,
    name: String,
    class: shared::HeroClass,
    hp: BarValue,
    enemy: bool,
    form: Form,
) -> Entity {
    parent
        .spawn(hud_plate(true))
        .insert(Node {
            width: Val::Px(size::TARGET_FRAME_WIDTH.at(form)),
            padding: UiRect::axes(Val::Px(PLATE_PADDING.1), Val::Px(PLATE_PADDING.0)),
            column_gap: Val::Px(space::S8),
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            ..default()
        })
        .with_children(|plate| {
            plate.spawn(round_art(art, size::PORTRAIT_SM));
            plate
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(space::S4),
                    flex_grow: 1.0,
                    min_width: Val::Px(0.0),
                    ..default()
                })
                .with_children(|column| {
                    column
                        .spawn(Node {
                            column_gap: Val::Px(space::S4),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|title| {
                            title.spawn(icon_node(
                                class_icon(class),
                                size::ICON_SM,
                                color::TEXT_GOLD,
                            ));
                            title.spawn((
                                Text::new(name),
                                theme::styled_text(TextStyle::keep_case(TextRole::Label)),
                                TextColor(color::TEXT_PRIMARY),
                                TextLayout::new_with_no_wrap(),
                            ));
                        });
                    bar(
                        column,
                        if enemy {
                            BarKind::HpEnemy
                        } else {
                            BarKind::HpAlly
                        },
                        hp,
                        Val::Percent(100.0),
                        form,
                        true,
                    );
                });
        })
        .id()
}

/// Every system of this module.
pub(crate) fn add_systems(app: &mut App) {
    app.add_systems(
        Update,
        (
            resolve_art,
            resolve_ability_art,
            paint_bars,
            paint_abilities,
            paint_portraits.before(resolve_art),
            paint_timer_rings,
            paint_hero_tile_frames.after(super::paint_kit),
        )
            .in_set(crate::ui::UiSet::Paint)
            .before(crate::ui::kit_assets::resolve_kit_images),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_and_ring_frames_follow_the_atlas_order() {
        assert_eq!(cooldown_frame(10.0, 10.0), 0, "full disc at the start");
        assert_eq!(cooldown_frame(0.0, 10.0), 59);
        assert_eq!(cooldown_frame(5.0, 10.0), 30);
        assert_eq!(cooldown_text(3.2), "4");
        assert_eq!(cooldown_text(0.46), "0.5");
        assert_eq!(ring_frame(0.0), 0);
        assert_eq!(ring_frame(1.0), 59);
        assert_eq!(group_digits(7401), "7 401");
        assert_eq!(group_digits(120), "120");
        assert_eq!(group_digits(1_234_567), "1 234 567");
    }

    /// hud.md ability states on one kit button: cooldown sweep + seconds,
    /// the ready flash when it ends, a red cost without mana, the locked
    /// veil with its level, the dead veil without a lock, rank pips.
    #[test]
    fn ability_states_paint_sweep_flash_cost_veil_and_pips() {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        enum Noop {
            Press,
        }
        let mut app = App::new();
        app.init_resource::<Time>()
            .add_systems(Update, paint_abilities);
        let root = app.world_mut().spawn(Node::default()).id();
        let view = AbilityView {
            ability: None,
            icon: Icon::HudAttack,
            key: Some("Q"),
            cost: Some(22),
            rank: 2,
            cooldown: Some((3.2, 8.0)),
            locked: false,
            unlock_level: None,
            no_mana: false,
            pips: true,
        };
        app.world_mut()
            .commands()
            .entity(root)
            .with_children(|parent| {
                ability_button(parent, view, 64.0, Noop::Press, "QaAbility");
            });
        app.world_mut().flush();
        app.update();
        let (entity, parts) = app
            .world_mut()
            .query::<(Entity, &AbilityParts)>()
            .single(app.world())
            .map(|(entity, parts)| (entity, *parts))
            .unwrap();
        let visible = |app: &App, part: Entity| {
            *app.world().get::<Visibility>(part).unwrap() != Visibility::Hidden
        };
        assert!(visible(&app, parts.sweep) && visible(&app, parts.seconds));
        assert_eq!(app.world().get::<Text>(parts.seconds).unwrap().0, "4");
        let pips: Vec<Color> = app
            .world()
            .get::<Children>(parts.pips.unwrap())
            .unwrap()
            .iter()
            .map(|pip| app.world().get::<KitImage>(pip).unwrap().tint)
            .collect();
        assert_eq!(pips, [color::GOLD_400, color::GOLD_400, color::SURFACE_3]);
        // Cooldown ends: the flash rises from 0 while the sweep hides.
        app.world_mut()
            .get_mut::<AbilityView>(entity)
            .unwrap()
            .cooldown = None;
        app.update();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(60));
        app.update();
        app.update();
        assert!(!visible(&app, parts.sweep));
        let glow = app
            .world()
            .get::<KitImage>(parts.flash)
            .unwrap()
            .tint
            .alpha();
        assert!(glow > 0.0 && glow <= READY_FLASH_ALPHA, "flash {glow}");
        // No mana: the cost number turns danger.
        app.world_mut()
            .get_mut::<AbilityView>(entity)
            .unwrap()
            .no_mana = true;
        app.update();
        let pill = app.world().get::<Children>(parts.cost.unwrap()).unwrap()[0];
        let number = app.world().get::<Children>(pill).unwrap()[0];
        assert_eq!(
            app.world().get::<TextColor>(number).unwrap().0,
            color::TEXT_DANGER
        );
        // Locked: veil with `Lv 6`; dead: veil without a lock or level.
        {
            let mut view = app.world_mut().get_mut::<AbilityView>(entity).unwrap();
            view.locked = true;
            view.unlock_level = Some(6);
        }
        app.update();
        assert!(visible(&app, parts.veil));
        assert_eq!(app.world().get::<Text>(parts.veil_label).unwrap().0, "Lv 6");
        app.world_mut()
            .get_mut::<AbilityView>(entity)
            .unwrap()
            .unlock_level = None;
        app.update();
        let lock = app.world().get::<Children>(parts.veil).unwrap()[0];
        assert_eq!(
            *app.world().get::<Visibility>(lock).unwrap(),
            Visibility::Hidden
        );
        assert_eq!(app.world().get::<Text>(parts.veil_label).unwrap().0, "");
    }

    #[test]
    fn bars_fill_to_their_fraction_and_trail_after_damage() {
        let mut app = App::new();
        app.init_resource::<Time>().add_systems(Update, paint_bars);
        let root = app.world_mut().spawn(Node::default()).id();
        app.world_mut()
            .commands()
            .entity(root)
            .with_children(|parent| {
                bar(
                    parent,
                    BarKind::HpSelf,
                    BarValue {
                        current: 130.0,
                        max: 203.0,
                        respawn: None,
                    },
                    Val::Px(200.0),
                    Form::Desktop,
                    true,
                );
            });
        app.world_mut().flush();
        app.update();
        let (entity, parts) = app
            .world_mut()
            .query::<(Entity, &KitParts)>()
            .single(app.world())
            .map(|(entity, parts)| (entity, *parts))
            .unwrap();
        let width =
            |app: &App, part: Option<Entity>| app.world().get::<Node>(part.unwrap()).unwrap().width;
        assert_eq!(width(&app, parts.fill), Val::Percent(130.0 / 203.0 * 100.0));
        let label = parts.label.unwrap();
        assert_eq!(app.world().get::<Text>(label).unwrap().0, "130 / 203");
        app.world_mut().get_mut::<BarValue>(entity).unwrap().current = 60.0;
        app.update();
        assert_eq!(width(&app, parts.fill), Val::Percent(60.0 / 203.0 * 100.0));
        let Val::Percent(trail) = width(&app, parts.knob) else {
            panic!("trail width")
        };
        assert!(
            trail > 60.0 / 203.0 * 100.0,
            "the trail lags behind the fill"
        );
        app.world_mut().get_mut::<BarValue>(entity).unwrap().respawn = Some(8);
        app.update();
        assert_eq!(width(&app, parts.fill), Val::Percent(0.0));
        assert_eq!(app.world().get::<Text>(label).unwrap().0, "Respawn 8");
        assert_eq!(
            app.world().get::<TextColor>(label).unwrap().0,
            color::TEXT_DISABLED
        );
    }
}
