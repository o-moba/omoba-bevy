//! Deliberate skill inspection, independent of attacks and cast admission.
use bevy::prelude::*;

pub(crate) const HOLD_SECONDS: f64 = 1.5;

#[derive(Resource, Default)]
pub(crate) struct SkillInspection {
    pub(crate) slot: Option<usize>,
    pub(crate) touch: bool,
    held: Option<(u8, usize, f64)>,
}
impl SkillInspection {
    fn advance(&mut self, input: Option<(u8, usize)>, now: f64) {
        self.touch = false;
        self.held = match (input, self.held) {
            (Some((source, slot)), Some((was_source, was_slot, since)))
                if source == was_source && slot == was_slot =>
            {
                Some((source, slot, since))
            }
            (Some((source, slot)), _) => Some((source, slot, now)),
            (None, _) => None,
        };
        self.slot = self
            .held
            .filter(|(_, _, since)| now - since >= HOLD_SECONDS)
            .map(|(_, slot, _)| slot);
    }
}

pub(super) fn update_inspection(
    time: Res<Time>,
    context: Res<crate::input_context::GameplayInputContext>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    pad: Option<Res<crate::gamepad::GamepadControls>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    slots: Query<(&super::hotbar::SkillSlotButton, &Interaction)>,
    mut inspection: ResMut<SkillInspection>,
) {
    if !context.gameplay_allowed() {
        inspection.advance(None, time.elapsed_secs_f64());
        return;
    }
    let pad = pad.filter(|p| p.active);
    if let Some(mobile) = mobile.filter(|m| m.enabled && pad.is_none()) {
        inspection.held = None;
        inspection.slot = mobile.inspected_skill();
        inspection.touch = true;
        return;
    }
    let input = if let Some(pad) = pad {
        pad.aiming_slot.map(|slot| (2, slot))
    } else {
        slots
            .iter()
            .find(|(_, interaction)| **interaction == Interaction::Pressed)
            .map(|(button, _)| (0, button.slot))
            .or_else(|| {
                keys.as_ref().and_then(|keys| {
                    if keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight) {
                        return None;
                    }
                    crate::input_bindings::SKILL_CAST_KEYS
                        .iter()
                        .position(|key| keys.pressed(*key))
                        .map(|slot| (1, slot))
                })
            })
    }
    .filter(|(_, slot)| *slot < 4);
    inspection.advance(input, time.elapsed_secs_f64());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn taps_do_not_accumulate_and_switching_skills_requires_a_fresh_hold() {
        let mut state = SkillInspection::default();
        for start in [0.0, 1.0, 2.0] {
            state.advance(Some((0, 0)), start);
            state.advance(Some((0, 0)), start + 0.2);
            assert_eq!(state.slot, None);
            state.advance(None, start + 0.3);
        }
        state.advance(Some((0, 0)), 3.0);
        state.advance(Some((0, 0)), 4.5);
        assert_eq!(state.slot, Some(0));
        state.advance(Some((0, 1)), 4.6);
        assert_eq!(state.slot, None);
        state.advance(Some((0, 1)), 6.11);
        assert_eq!(state.slot, Some(1));
        state.advance(None, 6.2);
        assert_eq!(state.slot, None);
    }
}
