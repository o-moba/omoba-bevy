//! Match-found feedback and mobile idle policy. Server acceptance is always
//! an explicit action in the searching UI, independent of these effects.
// i18n-strict
use bevy::{prelude::*, window::PrimaryWindow};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    career::CareerClient,
    frontend::AppScreen,
    game_audio::{AudioCue, AudioCueRequest},
    match_service::MatchServiceClient,
    ui::widgets::surfaces::{ToastKind, ToastRequest},
};
mod policy;

pub(super) fn install(app: &mut App) {
    app.add_message::<AudioCueRequest>()
        .add_message::<ToastRequest>()
        .add_systems(Update, update.after(crate::net::SessionReactions));
}

fn update(
    windows: Query<&Window, With<PrimaryWindow>>,
    screen: Option<Res<State<AppScreen>>>,
    mut flow: Option<ResMut<MatchServiceClient>>,
    mut career: Option<ResMut<CareerClient>>,
    mut state: Local<policy::Attention>,
    mut audio: MessageWriter<AudioCueRequest>,
    mut toasts: MessageWriter<ToastRequest>,
) {
    let focused = windows.single().is_ok_and(|w| w.focused);
    let activity = match screen.as_deref().map(State::get) {
        Some(AppScreen::Searching) => policy::Activity::Searching,
        Some(AppScreen::Draft) => policy::Activity::Draft,
        Some(AppScreen::Loading) => policy::Activity::Loading,
        Some(AppScreen::InMatch) => policy::Activity::Playing,
        _ => policy::Activity::Menus,
    };
    let ready = flow
        .as_ref()
        .zip(career.as_ref())
        .and_then(|(flow, career)| {
            let (id, answered) = flow.ready_check(&career.view)?;
            let shared::match_service::MatchServiceView::ReadyCheck { remaining_secs, .. } =
                career.view.match_service.as_ref()?
            else {
                return None;
            };
            Some(policy::Ready {
                id,
                answered,
                remaining_secs: *remaining_secs,
            })
        });
    let effects = state.update(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default(),
        focused,
        activity,
        ready,
    );
    if let Some(enabled) = effects.keep_awake {
        keep_awake(enabled);
    }
    if effects.match_found {
        audio.write(AudioCueRequest(AudioCue::UiConfirm));
        match_haptic();
    }
    if effects.resumed_search
        && flow
            .as_mut()
            .is_some_and(|flow| flow.refresh_after_resume())
    {
        // Buffered pre-sleep countdowns are not proof of a live ready check.
        // drive_matching retries the same signed search on the next frame.
        if let Some(career) = career.as_mut() {
            career.view.match_service = None;
        }
        toasts.write(ToastRequest {
            kind: ToastKind::Info,
            text: crate::i18n::tr("searching.status.resumed").to_owned(),
        });
    }
}

fn keep_awake(enabled: bool) {
    #[cfg(target_os = "android")]
    if let Some(app) = bevy::android::ANDROID_APP.get() {
        use bevy::android::android_activity::WindowManagerFlags as Flags;
        let (add, remove) = if enabled {
            (Flags::KEEP_SCREEN_ON, Flags::empty())
        } else {
            (Flags::empty(), Flags::KEEP_SCREEN_ON)
        };
        app.set_window_flags(add, remove);
    }
    #[cfg(target_os = "ios")]
    {
        unsafe extern "C" {
            fn omoba_attention_keep_awake(enabled: i32);
        }
        unsafe { omoba_attention_keep_awake(i32::from(enabled)) };
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let _ = enabled;
}

fn match_haptic() {
    #[cfg(target_os = "ios")]
    {
        unsafe extern "C" {
            fn omoba_attention_match_found();
        }
        unsafe { omoba_attention_match_found() };
    }
    // The Android JNI/permission addition is a separate owner-reviewed patch.
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feedback_requires_a_current_unanswered_check_and_never_accepts_it() {
        use shared::match_service::{MatchPreference, MatchServiceView};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .insert_state(AppScreen::Searching)
            .init_resource::<CareerClient>()
            .init_resource::<MatchServiceClient>();
        install(&mut app);
        let window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        app.world_mut().resource_mut::<MatchServiceClient>().active = true;
        let mut career = app.world_mut().resource_mut::<CareerClient>();
        career.view.match_service_request_id = Some(0);
        career.view.match_service = Some(MatchServiceView::ReadyCheck {
            check_id: "first".into(),
            preference: MatchPreference::Ranked,
            accepted: false,
            accepted_count: 1,
            needed: 10,
            remaining_secs: 15,
        });
        app.update();
        let cues: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<AudioCueRequest>>()
            .drain()
            .collect();
        assert!(matches!(&cues[..], [AudioCueRequest(AudioCue::UiConfirm)]));
        assert_eq!(
            app.world()
                .resource::<MatchServiceClient>()
                .ready_check(&app.world().resource::<CareerClient>().view),
            Some(("first", false))
        );
        app.update();
        assert!(
            app.world()
                .resource::<Messages<AudioCueRequest>>()
                .is_empty()
        );
        app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
        if let Some(MatchServiceView::ReadyCheck {
            check_id,
            remaining_secs,
            ..
        }) = app
            .world_mut()
            .resource_mut::<CareerClient>()
            .view
            .match_service
            .as_mut()
        {
            *check_id = "expired-in-background".into();
            *remaining_secs = 0;
        }
        app.update();
        app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
        app.update();
        assert!(
            app.world()
                .resource::<Messages<AudioCueRequest>>()
                .is_empty()
        );
    }
}
