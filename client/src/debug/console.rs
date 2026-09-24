use bevy::prelude::*;
use std::collections::VecDeque;

const CONSOLE_WIDTH: f32 = 360.0;
const CONSOLE_HEIGHT: f32 = 140.0;
const CONSOLE_MARGIN: f32 = 12.0;
const CONSOLE_MAX_LINES: usize = 8;

/// Set `OMOBA_DEBUG_UI=1` to show the on-screen debug console and admin hotkeys (TASK-16 AC5).
pub const DEBUG_UI_ENV_VAR: &str = "OMOBA_DEBUG_UI";

/// Parses env-style values for [`DEBUG_UI_ENV_VAR`]; used by tests without mutating process env.
pub fn parse_debug_ui_flag(raw: &str) -> bool {
    raw == "1" || raw.eq_ignore_ascii_case("true")
}

fn debug_ui_enabled_from_env() -> bool {
    std::env::var(DEBUG_UI_ENV_VAR)
        .ok()
        .as_deref()
        .is_some_and(parse_debug_ui_flag)
}

#[derive(Resource)]
pub struct DebugConsole {
    lines: VecDeque<String>,
    dirty: bool,
    /// When false, the HUD console is hidden and `push_line` is a no-op (normal play default).
    pub ui_enabled: bool,
}

impl Default for DebugConsole {
    fn default() -> Self {
        Self {
            lines: VecDeque::new(),
            dirty: false,
            ui_enabled: debug_ui_enabled_from_env(),
        }
    }
}

impl DebugConsole {
    pub fn push_line(&mut self, line: impl Into<String>) {
        if !self.ui_enabled {
            return;
        }
        let line = line.into();
        if self.lines.len() == CONSOLE_MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        self.dirty = true;
    }

    fn text(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub struct DebugConsolePlugin;

impl Plugin for DebugConsolePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugConsole>()
            .add_systems(Startup, setup_debug_console_ui)
            .add_systems(Update, update_debug_console_ui);
    }
}

#[derive(Component)]
struct DebugConsoleRoot;

#[derive(Component)]
struct DebugConsoleText;

fn setup_debug_console_ui(mut commands: Commands, console: Res<DebugConsole>) {
    let visibility = if console.ui_enabled {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(CONSOLE_MARGIN),
                bottom: Val::Px(CONSOLE_MARGIN),
                width: Val::Px(CONSOLE_WIDTH),
                height: Val::Px(CONSOLE_HEIGHT),
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::BLACK),
            visibility,
            DebugConsoleRoot,
            Name::new("DebugConsole"),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(""),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor::WHITE,
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                DebugConsoleText,
            ));
        });
}

fn update_debug_console_ui(
    mut console: ResMut<DebugConsole>,
    mut text_query: Query<&mut Text, With<DebugConsoleText>>,
    mut root_query: Query<&mut Visibility, With<DebugConsoleRoot>>,
) {
    if let Ok(mut root_vis) = root_query.single_mut() {
        *root_vis = if console.ui_enabled {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    if !console.ui_enabled || !console.dirty {
        return;
    }
    if let Ok(mut text) = text_query.single_mut() {
        text.0 = console.text();
    }
    console.dirty = false;
}

#[cfg(test)]
mod tests {
    use super::parse_debug_ui_flag;

    #[test]
    fn parse_debug_ui_flag_accepts_one_and_true() {
        assert!(parse_debug_ui_flag("1"));
        assert!(parse_debug_ui_flag("true"));
        assert!(parse_debug_ui_flag("TRUE"));
    }

    #[test]
    fn parse_debug_ui_flag_rejects_other_values() {
        assert!(!parse_debug_ui_flag(""));
        assert!(!parse_debug_ui_flag("0"));
        assert!(!parse_debug_ui_flag("yes"));
        assert!(!parse_debug_ui_flag("false"));
    }
}
