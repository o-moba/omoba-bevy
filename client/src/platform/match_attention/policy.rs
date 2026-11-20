//! Presentation-only mobile attention policy. It never accepts a match or
//! extends a server deadline. Wall time includes device sleep between frames.
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Activity {
    Menus,
    Searching,
    Draft,
    Loading,
    Playing,
}

pub(super) struct Ready<'a> {
    pub id: &'a str,
    pub answered: bool,
    pub remaining_secs: u64,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct Effects {
    pub keep_awake: Option<bool>,
    pub resumed_search: bool,
    pub match_found: bool,
}

#[derive(Default)]
pub(super) struct Attention {
    focused: Option<bool>,
    away_since: Option<Duration>,
    last_update: Option<Duration>,
    awake: bool,
    alerted: Option<String>,
}

impl Attention {
    pub fn update(
        &mut self,
        now: Duration,
        focused: bool,
        activity: Activity,
        ready: Option<Ready<'_>>,
    ) -> Effects {
        let awake = focused && activity != Activity::Menus;
        let mut effects = Effects::default();
        if self.awake != awake {
            self.awake = awake;
            effects.keep_awake = Some(awake);
        }
        if self.focused == Some(true) && !focused {
            self.away_since = Some(now);
        } else if self.focused == Some(false) && focused {
            effects.resumed_search = self.away_since.take().is_some_and(|at| {
                activity == Activity::Searching && now.saturating_sub(at) > Duration::from_secs(15)
            });
        } else if self.focused == Some(true) && focused {
            // Suspension can stop frames before the platform delivers Focused(false).
            effects.resumed_search = activity == Activity::Searching
                && self
                    .last_update
                    .is_some_and(|at| now.saturating_sub(at) > Duration::from_secs(15));
        }
        self.focused = Some(focused);
        self.last_update = Some(now);
        if let Some(ready) = ready
            && focused
            && activity == Activity::Searching
            && !effects.resumed_search
            && !ready.answered
            && ready.remaining_secs > 0
            && self.alerted.as_deref() != Some(ready.id)
        {
            self.alerted = Some(ready.id.to_owned());
            effects.match_found = true;
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready(id: &str) -> Ready<'_> {
        Ready {
            id,
            answered: false,
            remaining_secs: 15,
        }
    }
    #[test]
    fn only_foreground_search_draft_loading_and_play_keep_the_screen_awake() {
        let mut state = Attention::default();
        let now = Duration::ZERO;
        assert_eq!(
            state.update(now, true, Activity::Menus, None),
            Effects::default()
        );
        for activity in [
            Activity::Searching,
            Activity::Draft,
            Activity::Loading,
            Activity::Playing,
        ] {
            assert_eq!(
                state.update(now, true, activity, None).keep_awake,
                Some(true)
            );
            assert_eq!(state.update(now, true, activity, None).keep_awake, None);
            assert_eq!(
                state.update(now, false, activity, None).keep_awake,
                Some(false)
            );
        }
        state.update(now, true, Activity::Searching, None);
        assert_eq!(
            state.update(now, true, Activity::Menus, None).keep_awake,
            Some(false)
        );
    }
    #[test]
    fn a_wall_time_gap_is_reported_once_for_a_resumed_search_only() {
        for (activity, seconds, expected) in [
            (Activity::Searching, 15, false),
            (Activity::Searching, 16, true),
            (Activity::Menus, 60, false),
            (Activity::Draft, 60, false),
        ] {
            let mut state = Attention::default();
            state.update(Duration::ZERO, true, activity, None);
            state.update(Duration::from_secs(1), false, activity, None);
            let now = Duration::from_secs(1 + seconds);
            assert_eq!(
                state.update(now, true, activity, None).resumed_search,
                expected
            );
            assert!(!state.update(now, true, activity, None).resumed_search);
        }
    }
    #[test]
    fn the_first_foreground_frame_is_not_a_resume_and_clock_rollback_is_safe() {
        let mut state = Attention::default();
        state.update(Duration::ZERO, false, Activity::Searching, None);
        assert!(
            !state
                .update(Duration::from_secs(100), true, Activity::Searching, None)
                .resumed_search
        );
        state.update(Duration::from_secs(100), false, Activity::Searching, None);
        assert!(
            !state
                .update(Duration::ZERO, true, Activity::Searching, None)
                .resumed_search
        );
    }
    #[test]
    fn suspended_frames_refresh_before_alerting_a_buffered_ready_check() {
        let mut state = Attention::default();
        state.update(Duration::ZERO, true, Activity::Searching, None);
        let effects = state.update(
            Duration::from_secs(16),
            true,
            Activity::Searching,
            Some(ready("buffered")),
        );
        assert!(effects.resumed_search);
        assert!(!effects.match_found);
        // The adapter discards the buffered view. A fresh live reply may alert.
        assert!(
            state
                .update(
                    Duration::from_secs(17),
                    true,
                    Activity::Searching,
                    Some(ready("fresh")),
                )
                .match_found
        );
        assert!(
            !state
                .update(
                    Duration::from_secs(17),
                    true,
                    Activity::Searching,
                    Some(ready("fresh")),
                )
                .resumed_search
        );
    }
    #[test]
    fn each_live_unanswered_check_alerts_once_even_after_a_transient_view_gap() {
        let mut state = Attention::default();
        let now = Duration::ZERO;
        assert!(
            state
                .update(now, true, Activity::Searching, Some(ready("one")))
                .match_found
        );
        assert!(
            !state
                .update(now, true, Activity::Searching, Some(ready("one")))
                .match_found
        );
        state.update(now, true, Activity::Searching, None);
        assert!(
            !state
                .update(now, true, Activity::Searching, Some(ready("one")))
                .match_found
        );
        assert!(
            state
                .update(now, true, Activity::Searching, Some(ready("two")))
                .match_found
        );
    }
    #[test]
    fn background_checks_alert_on_resume_only_if_still_live_and_unanswered() {
        for (answered, remaining_secs, expected) in
            [(false, 1, true), (true, 1, false), (false, 0, false)]
        {
            let mut state = Attention::default();
            assert!(
                !state
                    .update(
                        Duration::ZERO,
                        false,
                        Activity::Searching,
                        Some(ready("one"))
                    )
                    .match_found
            );
            assert_eq!(
                state
                    .update(
                        Duration::from_secs(20),
                        true,
                        Activity::Searching,
                        Some(Ready {
                            id: "one",
                            answered,
                            remaining_secs
                        })
                    )
                    .match_found,
                expected
            );
        }
    }
}
