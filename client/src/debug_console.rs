use bevy::prelude::*;
use std::collections::VecDeque;

const CONSOLE_WIDTH: f32 = 360.0;
const CONSOLE_HEIGHT: f32 = 140.0;
const CONSOLE_MARGIN: f32 = 12.0;
const CONSOLE_MAX_LINES: usize = 8;

#[derive(Resource)]
pub struct DebugConsole {
    lines: VecDeque<String>,
    dirty: bool,
}

impl Default for DebugConsole {
    fn default() -> Self {
        Self {
            lines: VecDeque::new(),
            dirty: false,
        }
    }
}

impl DebugConsole {
    pub fn push_line(&mut self, line: impl Into<String>) {
        let line = line.into();
        if self.lines.len() == CONSOLE_MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        self.dirty = true;
    }

    fn text(&self) -> String {
        self
            .lines
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
struct DebugConsoleText;

fn setup_debug_console_ui(mut commands: Commands) {
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
) {
    if !console.dirty {
        return;
    }
    if let Ok(mut text) = text_query.get_single_mut() {
        text.0 = console.text();
    }
    console.dirty = false;
}
