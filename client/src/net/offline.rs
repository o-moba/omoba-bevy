//! Socket-free practice, using the normal client snapshot/render/input pipeline.
//! This deliberately has no career backend, matchmaking, rewards, or persistence.
use super::session::ClientSession;
use super::transport::NetThreadSignal;
use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};
use shared::{
    protocol::JoinRejection,
    wire::{ClientPacket, ServerPacket},
};
pub(super) const ADDRESS: &str = "offline-practice";

pub(super) fn shipped_avatar(slug: Option<&str>) -> bool {
    slug.is_none_or(|s| {
        omoba_passport::avatars::avatar_roster()
            .iter()
            .any(|a| a.slug == s && a.passport.is_none())
    })
}

#[derive(Resource)]
pub(super) struct LocalPractice {
    commands: Receiver<ClientPacket>,
    snapshots: Sender<ServerPacket>,
    _signals: Sender<NetThreadSignal>,
    simulation: common::offline::PracticeSession,
}
impl LocalPractice {
    pub(super) fn new(
        commands: Receiver<ClientPacket>,
        snapshots: Sender<ServerPacket>,
        signals: Sender<NetThreadSignal>,
    ) -> Self {
        Self {
            commands,
            snapshots,
            _signals: signals,
            simulation: common::offline::PracticeSession::new(std::time::Instant::now()),
        }
    }
}

pub(super) fn step(practice: Option<ResMut<LocalPractice>>, time: Res<Time>) {
    let Some(mut practice) = practice else {
        return;
    };
    while let Ok(packet) = practice.commands.try_recv() {
        if let ClientPacket::Join { avatar, .. } = &packet
            && !shipped_avatar(avatar.as_deref())
        {
            practice
                .simulation
                .reject_join(JoinRejection::AvatarNotAuthorized);
            continue;
        }
        practice.simulation.command(packet);
    }
    practice.simulation.advance(time.delta_secs().min(0.1));
    let snapshot = practice.simulation.snapshot();
    let _ = practice.snapshots.try_send(snapshot);
}

#[derive(Component)]
pub(super) struct PracticeBanner;
/// The offline-practice banner (`VARIANTS.md` `offline-practice`, R5.6):
/// desktop = a muted badge atop the buff-chip column (the chips move down
/// by `match_hud::PRACTICE_BADGE_SHIFT`); phone = a toast in the
/// action-feedback slot for `motion.duration.toast_hold` at match start.
pub(super) fn setup_banner(
    mut commands: Commands,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
) {
    use crate::ui::tokens::{TextRole, border, color, radius, space};
    let phone = mobile.as_ref().is_some_and(|mobile| mobile.enabled);
    let (region, fill, ink, corner, padding) = if phone {
        (
            crate::hud_layout::HudRegion::ActionFeedback,
            crate::ui::theme::perceptual(color::SURFACE_GLASS_STRONG),
            color::TEXT_PRIMARY,
            radius::MD,
            UiRect::axes(Val::Px(space::S12), Val::Px(space::S8)),
        )
    } else {
        (
            crate::hud_layout::HudRegion::PracticeBadge,
            crate::ui::theme::perceptual(color::SURFACE_GLASS_STRONG),
            color::TEXT_SECONDARY,
            radius::PILL,
            UiRect::axes(Val::Px(space::S8), Val::Px(space::S4)),
        )
    };
    commands.spawn((
        crate::i18n::Localized::with_args(
            "net.practice.banner",
            [("level", &common::offline::START_LEVEL)],
        )
        .into_text(),
        crate::ui::theme::styled_text(
            crate::ui::theme::TextStyle::keep_case(TextRole::Label)
                .sized(TextRole::Caption.style().size),
        ),
        TextColor(ink),
        TextLayout::no_wrap(),
        BackgroundColor(fill),
        BorderColor::all(if phone {
            crate::ui::theme::perceptual(color::BORDER_HAIRLINE)
        } else {
            Color::NONE
        }),
        Node {
            position_type: PositionType::Absolute,
            padding,
            border: UiRect::all(Val::Px(if phone { border::HAIRLINE } else { 0.0 })),
            border_radius: BorderRadius::all(Val::Px(corner)),
            display: Display::None,
            ..default()
        },
        UiTransform::IDENTITY,
        region,
        ZIndex(22),
        Pickable::IGNORE,
        PracticeBanner,
        Name::new("OfflinePracticeBanner"),
    ));
}
pub(super) fn sync_banner(
    time: Res<Time>,
    session: Res<ClientSession>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut shown_for: Local<Option<f32>>,
    mut banners: Query<&mut Node, With<PracticeBanner>>,
) {
    let practice = session.is_offline()
        && screen.is_some_and(|s| *s.get() == crate::frontend::AppScreen::InMatch);
    let phone = mobile.as_ref().is_some_and(|mobile| mobile.enabled);
    // Phone: a start-of-match toast, held `motion.duration.toast_hold`.
    let visible = if phone {
        let hold = crate::ui::tokens::motion::DURATION_TOAST_HOLD.as_secs_f32();
        *shown_for = if practice {
            Some(shown_for.map_or(0.0, |elapsed| elapsed + time.delta_secs()))
        } else {
            None
        };
        shown_for.is_some_and(|elapsed| elapsed < hold)
    } else {
        practice
    };
    for mut node in &mut banners {
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::transport::{NetworkChannels, spawn_network_transport};
    use crate::persistence::ResolvedServerAddressForPrefs;
    use shared::wire::GameState;
    #[test]
    fn offline_session_never_reuses_online_channels_or_overwrites_saved_address() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(ClientSession {
            offline_return_addr: Some("127.0.0.1:49999".into()),
            ephemeral_endpoint: true,
            ..default()
        });
        app.insert_resource(ResolvedServerAddressForPrefs("127.0.0.1:49999".into()));
        app.add_systems(
            Startup,
            |mut commands: Commands, mut session: ResMut<ClientSession>| {
                spawn_network_transport(&mut commands, &mut session, ADDRESS.into())
            },
        );
        app.add_systems(Update, step);
        app.update();
        assert!(app.world().contains_resource::<LocalPractice>());
        assert_eq!(
            app.world().resource::<ResolvedServerAddressForPrefs>().0,
            "127.0.0.1:49999"
        );
        let channels = app.world().resource::<NetworkChannels>();
        let snapshot = channels
            .incoming
            .try_recv()
            .expect("local snapshot without a listener or worker thread");
        assert!(matches!(
            snapshot,
            ServerPacket::Snapshot {
                game_state: GameState::Lobby,
                ..
            }
        ));
    }

    #[test]
    fn all_classes_join_through_local_channels_and_unbundled_avatar_is_rejected() {
        for class in shared::HeroClass::ALL {
            for avatar in [None, Some("not-bundled".to_owned())] {
                let mut app = App::new();
                app.add_plugins(MinimalPlugins);
                let (commands, incoming) = crossbeam_channel::unbounded();
                let (outgoing, snapshots) = crossbeam_channel::unbounded();
                let (signals, _) = crossbeam_channel::unbounded();
                app.insert_resource(LocalPractice::new(incoming, outgoing, signals));
                app.add_systems(Update, step);
                commands
                    .send(ClientPacket::Join {
                        handheld: Default::default(),
                        prematch: false,
                        team: shared::map::Team::Green,
                        character: shared::wire::CharacterChoice::Ipfs,
                        hero_class: class,
                        avatar: avatar.clone(),
                        sprite_character: None,
                        session_id: None,
                        passport_ticket: None,
                    })
                    .unwrap();
                app.update();
                let ServerPacket::Snapshot {
                    players,
                    join_error,
                    your_id,
                    ..
                } = snapshots.try_recv().unwrap()
                else {
                    panic!("local snapshot")
                };
                if avatar.is_some() {
                    assert!(players.is_empty());
                    assert_eq!(join_error, Some(JoinRejection::AvatarNotAuthorized));
                } else {
                    assert_eq!(
                        players.iter().find(|p| p.id == your_id).unwrap().hero_class,
                        class
                    );
                    assert_eq!(join_error, None);
                }
            }
        }
    }

    #[test]
    fn offline_purchase_replicates_authoritative_receipt_inventory_and_bonus_once() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let (commands, incoming) = crossbeam_channel::unbounded();
        let (outgoing, snapshots) = crossbeam_channel::unbounded();
        let (signals, _) = crossbeam_channel::unbounded();
        app.insert_resource(LocalPractice::new(incoming, outgoing, signals));
        app.add_systems(Update, step);
        commands
            .send(ClientPacket::Join {
                handheld: Default::default(),
                prematch: false,
                team: shared::map::Team::Green,
                character: shared::wire::CharacterChoice::Ipfs,
                hero_class: shared::HeroClass::Mage,
                avatar: None,
                sprite_character: None,
                session_id: None,
                passport_ticket: None,
            })
            .unwrap();
        app.update();
        let ServerPacket::Snapshot {
            players,
            your_id,
            meta,
            ..
        } = snapshots.try_recv().unwrap()
        else {
            panic!("local snapshot");
        };
        let before = players.iter().find(|p| p.id == your_id).unwrap();
        let item = shared::shop::ItemId::VitalityGem;
        let packet = ClientPacket::BuyItem {
            item_id: item.id().into(),
            request_id: 44,
            match_id: meta.match_id,
            server_epoch: meta.server_epoch,
        };
        commands.send(packet.clone()).unwrap();
        app.update();
        let ServerPacket::Snapshot { players, .. } = snapshots.try_recv().unwrap() else {
            panic!("purchase snapshot");
        };
        let after = players.iter().find(|p| p.id == your_id).unwrap();
        assert_eq!(after.inventory, vec![item]);
        assert_eq!(after.gold, before.gold - shared::shop::item(item).cost);
        assert_eq!(after.max_hp, before.max_hp + 30.0);
        let receipt = after.last_purchase.as_ref().unwrap();
        assert_eq!(receipt.request_id, 44);
        assert!(receipt.error.is_none());
        commands.send(packet).unwrap();
        app.update();
        let ServerPacket::Snapshot { players, .. } = snapshots.try_recv().unwrap() else {
            panic!("repeated purchase snapshot");
        };
        let repeated = players.iter().find(|p| p.id == your_id).unwrap();
        assert_eq!(repeated.gold, after.gold);
        assert_eq!(repeated.inventory, after.inventory);
        assert_eq!(repeated.last_purchase, after.last_purchase);
    }

    #[test]
    fn unknown_or_store_only_avatars_cannot_trigger_download_or_admission() {
        assert!(!shipped_avatar(Some("not-bundled")));
        assert!(shipped_avatar(None));
        for avatar in omoba_passport::avatars::avatar_roster()
            .iter()
            .filter(|a| a.passport.is_some())
        {
            assert!(!shipped_avatar(Some(&avatar.slug)));
        }
    }
}
