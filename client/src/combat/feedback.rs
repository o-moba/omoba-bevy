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

pub(super) fn adapt_mobile_combat_feedback(
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut feedback: Query<(&mut Node, &mut TextFont), With<ActionFeedbackText>>,
) {
    let Some(mobile) = mobile.filter(|mobile| mobile.enabled) else {
        return;
    };
    for (mut node, mut font) in &mut feedback {
        let width = (mobile.viewport.x * 0.38).min(340.0);
        node.left = Val::Px((mobile.viewport.x - width) * 0.5);
        node.bottom = Val::Px(mobile.safe.bottom + 84.0 * mobile.scale());
        node.max_width = Val::Px(width);
        font.font_size = 13.0 * mobile.scale();
    }
}
