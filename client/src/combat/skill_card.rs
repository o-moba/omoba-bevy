//! What an ability does, on demand (`omoba-ui/handoff/screens/skill-description.md`):
//! the desktop ability tooltip and the phone hold card share one content
//! model ([`SkillCardView`]) and one card ([`spawn_skill_card`]): title row
//! (keycap, name, rank pips or lock), availability, a two-line description,
//! mana and cooldown, and the live status (desktop) or the touch hint (phone).
// i18n-strict
use bevy::prelude::*;
use shared::{HeroClass, SkillSlot, ability_for_class_slot, scaled_mana_cost};

use crate::i18n::{Locale, data, tr, trf};
use crate::net::PlayerProgression;
use crate::ui::{
    kit_assets::{Icon, KitImage, Sprite},
    theme::{self, Form, TextStyle},
    tokens::{Metric, TextRole, border, color, radius, size, space},
    widgets::{game, icon_node},
};

/// Card size (`skill-description.md`: width 280, height 148 on both profiles).
pub(crate) const CARD: Vec2 = Vec2::new(280.0, 148.0);
/// Card padding 12 / 10 and the line heights of § Content.
const PADDING: (f32, f32) = (space::S12, space::S8 + border::FRAME);
const TITLE_H: f32 = 20.0;
const LINE_H: f32 = 16.0;
const DESCRIPTION_H: f32 = 40.0;
/// Description: `type.body` at the tooltip component size, 20 px lines.
const DESCRIPTION_TEXT: Metric = Metric::new(14.0, 14.0);

/// Tone of the desktop status line.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum StatusTone {
    #[default]
    Ready,
    Warning,
    Danger,
    Secondary,
    Muted,
}

impl StatusTone {
    fn color(self) -> Color {
        match self {
            StatusTone::Ready => color::TEXT_ACCENT,
            StatusTone::Warning => color::STATE_WARNING,
            StatusTone::Danger => color::TEXT_DANGER,
            StatusTone::Secondary => color::TEXT_SECONDARY,
            StatusTone::Muted => color::TEXT_MUTED,
        }
    }
}

/// One card's content (§ Content). The owner writes it; [`paint_skill_card`]
/// renders it and re-renders on a language change.
#[derive(Component, Clone, PartialEq, Debug, Default)]
pub(crate) struct SkillCardView {
    pub visible: bool,
    /// Desktop keycap (`Q`, or the pad binding).
    pub key: Option<String>,
    pub class: Option<HeroClass>,
    pub slot: usize,
    /// Rank 1–3 (rank-1 values while locked).
    pub rank: u8,
    pub locked: bool,
    pub mana: u32,
    pub no_mana: bool,
    /// Cooldown duration in seconds (one decimal).
    pub cooldown: f32,
    /// Desktop: the live cast status; hidden while dead.
    pub status: Option<(String, StatusTone)>,
    /// Phone: the release/cast hint as the last line.
    pub hint: bool,
}

impl SkillCardView {
    /// The card of `slot` for a class and progression (`rank` = the learned
    /// rank, at least 1).
    pub(crate) fn of(
        class: HeroClass,
        progression: &PlayerProgression,
        slot: usize,
        mana: f32,
        cooldown: f32,
    ) -> Self {
        let definition = ability_for_class_slot(class, SkillSlot::ALL[slot]);
        let rank = progression.ranks[slot].clamp(1, shared::MAX_ABILITY_RANK);
        let cost = scaled_mana_cost(definition, rank);
        Self {
            visible: true,
            key: None,
            class: Some(class),
            slot,
            rank,
            locked: !progression.unlocked()[slot],
            mana: cost.round() as u32,
            no_mana: mana < cost,
            cooldown,
            status: None,
            hint: false,
        }
    }
}

/// `combat.skill.cooldown` with one decimal (`4.0s cooldown`).
#[cfg(test)]
pub(crate) fn cooldown_line(seconds: f32) -> String {
    trf(
        "combat.skill.cooldown",
        &[("seconds", &format!("{seconds:.1}"))],
    )
}

/// The parts of a card.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Part {
    Key,
    Name,
    Pips,
    Lock,
    Availability,
    Description,
    ManaWord(Side),
    ManaValue,
    CooldownWord(Side),
    CooldownValue,
    Status,
    Hint,
}

/// Word before / after a template's number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Side {
    Before,
    After,
}

/// The words of a one-placeholder template around its number: `"{mana}
/// mana"` → `("", " mana")`, `"冷却 {seconds} 秒"` → `("冷却 ", " 秒")`.
fn around(key: &'static str, placeholder: &str) -> (String, String) {
    let template = tr(key);
    let token = format!("{{{placeholder}}}");
    match template.split_once(token.as_str()) {
        Some((before, after)) => (before.to_owned(), after.to_owned()),
        None => (String::new(), template.to_owned()),
    }
}

/// Spawns a hidden card (the owner places it and fills its view).
pub(crate) fn spawn_skill_card(commands: &mut Commands, form: Form, bundle: impl Bundle) -> Entity {
    let line = |height: f32| Node {
        height: Val::Px(height),
        column_gap: Val::Px(space::S4),
        align_items: AlignItems::Center,
        flex_shrink: 0.0,
        ..default()
    };
    let caption = || theme::styled_text(TextStyle::keep_case(TextRole::Caption));
    let caption_semibold = || {
        theme::styled_text(
            TextStyle::keep_case(TextRole::Label).sized(TextRole::Caption.style().size),
        )
    };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(CARD.x),
                height: Val::Px(CARD.y),
                padding: UiRect::axes(Val::Px(PADDING.0), Val::Px(PADDING.1)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(space::S4),
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::all(Val::Px(radius::MD)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
            BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)),
            SkillCardView::default(),
            GlobalZIndex(4700),
            Pickable::IGNORE,
            bundle,
        ))
        .with_children(|card| {
            card.spawn(line(TITLE_H)).with_children(|title| {
                if form == Form::Desktop {
                    title
                        .spawn((
                            Node {
                                min_width: Val::Px(game::ABILITY_BADGE),
                                height: Val::Px(game::ABILITY_BADGE),
                                padding: UiRect::horizontal(Val::Px(space::S4)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(border::HAIRLINE)),
                                border_radius: BorderRadius::all(Val::Px(radius::SM)),
                                ..default()
                            },
                            BackgroundColor(color::SURFACE_1_OPAQUE),
                            BorderColor::all(color::GOLD_600),
                        ))
                        .with_child((
                            Text::new(""),
                            theme::styled_text(
                                TextStyle::keep_case(TextRole::Label)
                                    .sized(TextRole::NumberSm.style().size),
                            ),
                            TextColor(color::TEXT_GOLD),
                            Part::Key,
                        ));
                }
                title.spawn((
                    Text::new(""),
                    theme::styled_text(TextStyle::keep_case(TextRole::Label)),
                    TextColor(color::TEXT_PRIMARY),
                    TextLayout::new_with_no_wrap(),
                    Node {
                        flex_grow: 1.0,
                        min_width: Val::Px(0.0),
                        overflow: Overflow::clip_x(),
                        ..default()
                    },
                    Part::Name,
                ));
                title
                    .spawn((
                        Node {
                            column_gap: Val::Px(game::ABILITY_PIP_GAP),
                            ..default()
                        },
                        Part::Pips,
                    ))
                    .with_children(|pips| {
                        for _ in 0..game::MAX_RANK {
                            pips.spawn((
                                Node {
                                    width: Val::Px(game::ABILITY_PIP),
                                    height: Val::Px(game::ABILITY_PIP),
                                    ..default()
                                },
                                KitImage::sprite(Sprite::Pip, color::SURFACE_3),
                            ));
                        }
                    });
                title.spawn((
                    icon_node(Icon::NavLock, size::ICON_SM, color::TEXT_MUTED),
                    Part::Lock,
                ));
            });
            card.spawn(line(LINE_H)).with_child((
                Text::new(""),
                caption(),
                TextColor(color::TEXT_GOLD),
                Part::Availability,
            ));
            card.spawn((
                Text::new(""),
                theme::styled_text(TextStyle::keep_case(TextRole::Body).sized(DESCRIPTION_TEXT)),
                TextColor(color::TEXT_SECONDARY),
                bevy::text::LineHeight::Px(space::S16 + space::S4),
                Node {
                    height: Val::Px(DESCRIPTION_H),
                    overflow: Overflow::clip(),
                    flex_shrink: 0.0,
                    ..default()
                },
                Part::Description,
            ));
            // The template words carry their own spaces (`{mana} mana`,
            // `{seconds}s cooldown`, `冷却 {seconds} 秒`): no gap.
            card.spawn(Node {
                column_gap: Val::Px(0.0),
                align_items: AlignItems::Baseline,
                ..line(LINE_H)
            })
            .with_children(|stats| {
                stats.spawn((
                    Text::new(""),
                    caption(),
                    TextColor(color::TEXT_SECONDARY),
                    Part::ManaWord(Side::Before),
                ));
                stats.spawn((
                    Text::new(""),
                    theme::role_text(TextRole::NumberSm),
                    TextColor(color::BAR_MANA),
                    Part::ManaValue,
                ));
                stats.spawn((
                    Text::new(""),
                    caption(),
                    TextColor(color::TEXT_SECONDARY),
                    Part::ManaWord(Side::After),
                ));
                stats.spawn((
                    Text::new("·"),
                    caption(),
                    TextColor(color::TEXT_MUTED),
                    Node {
                        margin: UiRect::horizontal(Val::Px(space::S4)),
                        ..default()
                    },
                ));
                stats.spawn((
                    Text::new(""),
                    caption(),
                    TextColor(color::TEXT_SECONDARY),
                    Part::CooldownWord(Side::Before),
                ));
                stats.spawn((
                    Text::new(""),
                    theme::role_text(TextRole::NumberSm),
                    TextColor(color::TEXT_PRIMARY),
                    Part::CooldownValue,
                ));
                stats.spawn((
                    Text::new(""),
                    caption(),
                    TextColor(color::TEXT_SECONDARY),
                    Part::CooldownWord(Side::After),
                ));
            });
            card.spawn(line(LINE_H)).with_child((
                Text::new(""),
                caption_semibold(),
                TextColor(color::TEXT_ACCENT),
                Part::Status,
            ));
            card.spawn(line(LINE_H)).with_child((
                Text::new(tr("combat.skill.hint_touch")),
                caption(),
                TextColor(color::TEXT_MUTED),
                Part::Hint,
            ));
        })
        .id()
}

/// Renders every card whose view (or the language) changed.
#[allow(clippy::type_complexity)]
pub(crate) fn paint_skill_card(
    locale: Option<Res<Locale>>,
    cards: Query<(Entity, Ref<SkillCardView>)>,
    mut roots: Query<&mut Node, With<SkillCardView>>,
    children: Query<&Children>,
    mut parts: Query<(
        &Part,
        Option<&mut Text>,
        Option<&mut TextColor>,
        Option<&ChildOf>,
    )>,
    mut nodes: Query<&mut Node, Without<SkillCardView>>,
    mut pips: Query<&mut KitImage>,
) {
    let relabel = crate::i18n::locale_changed(&locale);
    for (card, view) in &cards {
        if let Ok(mut node) = roots.get_mut(card) {
            let display = if view.visible {
                Display::Flex
            } else {
                Display::None
            };
            if node.display != display {
                node.display = display;
            }
        }
        if !(view.is_changed() || relabel) || !view.visible {
            continue;
        }
        let Some(class) = view.class else { continue };
        let definition = ability_for_class_slot(class, SkillSlot::ALL[view.slot]);
        let (mana_before, mana_after) = around("combat.skill.mana", "mana");
        let (cool_before, cool_after) = around("combat.skill.cooldown", "seconds");
        for entity in children.iter_descendants(card) {
            let Ok((part, text, ink, parent)) = parts.get_mut(entity) else {
                continue;
            };
            let part = *part;
            let value: Option<String> = match part {
                Part::Key => Some(view.key.clone().unwrap_or_default()),
                Part::Name => Some(data::ability_name(definition).to_owned()),
                Part::Availability => Some(if view.locked {
                    trf(
                        "touch.skill.unlocks",
                        &[("level", &shared::SLOT_UNLOCK_LEVELS[view.slot])],
                    )
                } else {
                    trf("touch.skill.rank", &[("rank", &view.rank)])
                }),
                Part::Description => Some(data::ability_desc(definition).to_owned()),
                Part::ManaWord(Side::Before) => Some(mana_before.clone()),
                Part::ManaWord(Side::After) => Some(mana_after.clone()),
                Part::ManaValue => Some(view.mana.to_string()),
                Part::CooldownWord(Side::Before) => Some(cool_before.clone()),
                Part::CooldownWord(Side::After) => Some(cool_after.clone()),
                Part::CooldownValue => Some(format!("{:.1}", view.cooldown)),
                Part::Status => Some(
                    view.status
                        .as_ref()
                        .map_or_else(String::new, |(line, _)| line.clone()),
                ),
                Part::Hint => Some(tr("combat.skill.hint_touch").to_owned()),
                Part::Pips | Part::Lock => None,
            };
            if let (Some(value), Some(mut text)) = (value, text) {
                if text.0 != value {
                    text.0 = value;
                }
            }
            if let Some(mut ink) = ink {
                let next = match part {
                    Part::Availability if view.locked => Some(color::TEXT_MUTED),
                    Part::Availability => Some(color::TEXT_GOLD),
                    Part::ManaValue if view.no_mana => Some(color::TEXT_DANGER),
                    Part::ManaValue => Some(color::BAR_MANA),
                    Part::Status => view.status.as_ref().map(|(_, tone)| tone.color()),
                    _ => None,
                };
                if let Some(next) = next {
                    if ink.0 != next {
                        ink.0 = next;
                    }
                }
            }
            // Line visibility: pips or the lock, status (desktop) or hint.
            let shown = match part {
                Part::Pips => Some(!view.locked),
                Part::Lock => Some(view.locked),
                Part::Status => Some(view.status.is_some()),
                Part::Hint => Some(view.hint),
                _ => None,
            };
            if let Some(shown) = shown {
                let target = match part {
                    Part::Status | Part::Hint => parent.map(|parent| parent.parent()),
                    _ => Some(entity),
                };
                if let Some(Ok(mut node)) = target.map(|target| nodes.get_mut(target)) {
                    let display = if shown { Display::Flex } else { Display::None };
                    if node.display != display {
                        node.display = display;
                    }
                }
            }
            if part == Part::Pips {
                if let Ok(row) = children.get(entity) {
                    for (index, pip) in row.iter().enumerate() {
                        if let Ok(mut image) = pips.get_mut(pip) {
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
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_split_around_their_number_in_both_languages() {
        assert_eq!(
            around("combat.skill.mana", "mana"),
            (String::new(), " mana".into())
        );
        assert_eq!(
            around("combat.skill.cooldown", "seconds"),
            (String::new(), "s cooldown".into())
        );
        assert_eq!(cooldown_line(4.0), "4.0s cooldown");
        let zh = crate::i18n::LocaleId::parse("zh-Hans").unwrap();
        assert_eq!(
            crate::i18n::trf_in("combat.skill.cooldown", zh, &[("seconds", &"4.0")]),
            "冷却 4.0 秒"
        );
    }

    #[test]
    fn a_card_takes_the_rank_cost_lock_and_mana_state_of_its_slot() {
        let progression = PlayerProgression {
            level: 2,
            ranks: [2, 1, 1, 1],
            ..default()
        };
        let q = SkillCardView::of(HeroClass::Warden, &progression, 0, 100.0, 1.5);
        assert_eq!((q.rank, q.locked, q.no_mana), (2, false, false));
        let r = SkillCardView::of(HeroClass::Warden, &progression, 3, 0.0, 9.0);
        assert!(r.locked && r.no_mana);
        assert_eq!(r.rank, 1, "locked slots show rank-1 values");
    }
}
