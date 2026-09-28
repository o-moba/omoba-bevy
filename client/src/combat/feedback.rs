// i18n-strict
use bevy::prelude::*;

/// One visible action message, replaced in place and expired after three seconds.
#[derive(Resource, Default)]
pub(crate) struct ActionFeedback {
    pub text: String,
    pub(super) remaining: f32,
}

impl ActionFeedback {
    pub(crate) fn push_line(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.remaining = 3.0;
    }
}

#[derive(Component)]
pub(super) struct ActionFeedbackText;

pub(super) fn update_action_feedback(
    time: Res<Time>,
    mut feedback: ResMut<ActionFeedback>,
    mut labels: Query<(&mut Text, &mut Node), With<ActionFeedbackText>>,
) {
    feedback.remaining = (feedback.remaining - time.delta_secs()).max(0.0);
    if feedback.remaining == 0.0 {
        feedback.text.clear();
    }
    for (mut label, mut node) in &mut labels {
        label.0.clone_from(&feedback.text);
        node.display = if feedback.text.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
    }
}
