mod qa;
use super::*;
use crate::frontend::widgets::{self as w, ButtonKind};
use bevy::{
    input::{
        keyboard::{Key, KeyboardInput},
        mouse::{MouseScrollUnit, MouseWheel},
    },
    window::PrimaryWindow,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tab {
    Player,
    Enemy,
    Dummy,
    World,
    Animation,
    Damage,
    Presets,
}
impl Tab {
    const ALL: [Self; 7] = [
        Self::Player,
        Self::Enemy,
        Self::Dummy,
        Self::World,
        Self::Animation,
        Self::Damage,
        Self::Presets,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Player => "Hero",
            Self::Enemy => "Enemy",
            Self::Dummy => "Dummy",
            Self::World => "World",
            Self::Animation => "Motion",
            Self::Damage => "Damage",
            Self::Presets => "Presets",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    Level,
    Hp,
    Armor,
    Resistance,
    Move,
    AttackSpeed,
    Damage,
    Rank(usize),
    Aggression,
    Distance,
    DummyHp,
    DummyArmor,
    DummyResistance,
    PresetName,
}
impl Field {
    fn label(self) -> String {
        match self {
            Self::Level => "Level".into(),
            Self::Hp => "Base max HP".into(),
            Self::Armor => "Armor".into(),
            Self::Resistance => "Resistance".into(),
            Self::Move => "Movement multiplier".into(),
            Self::AttackSpeed => "Attack speed multiplier".into(),
            Self::Damage => "Damage multiplier".into(),
            Self::Rank(i) => format!("{} rank", ["Q", "W", "E", "R"][i]),
            Self::Aggression => "Aggression range (m)".into(),
            Self::Distance => "Desired attack distance (m)".into(),
            Self::DummyHp => "Dummy max HP".into(),
            Self::DummyArmor => "Dummy armor".into(),
            Self::DummyResistance => "Dummy resistance".into(),
            Self::PresetName => "Preset name".into(),
        }
    }
    fn value(self, s: &SandboxClient) -> String {
        let a = s.actor_config();
        match self {
            Self::PresetName => s.preset_name.clone(),
            Self::Level => a.level.to_string(),
            Self::Rank(i) => a.ranks[i].to_string(),
            _ => format!(
                "{:.2}",
                match self {
                    Self::Hp => a.max_hp,
                    Self::Armor => a.armor,
                    Self::Resistance => a.resistance,
                    Self::Move => a.move_speed,
                    Self::AttackSpeed => a.attack_speed,
                    Self::Damage => a.damage_multiplier,
                    Self::Aggression => s.config.enemy.aggression_range,
                    Self::Distance => s.config.enemy.attack_distance,
                    Self::DummyHp => s.config.dummy.max_hp,
                    Self::DummyArmor => s.config.dummy.armor,
                    Self::DummyResistance => s.config.dummy.resistance,
                    _ => 0.0,
                }
            ),
        }
    }
    fn set(self, s: &mut SandboxClient, text: &str) -> Result<(), String> {
        if self == Self::PresetName {
            s.preset_name = text.into();
            return Ok(());
        }
        let value: f32 = text
            .parse()
            .map_err(|_| "Enter a finite number".to_string())?;
        if !value.is_finite() {
            return Err("Enter a finite number".into());
        }
        if matches!(self, Self::Level | Self::Rank(_))
            && (value.fract() != 0.0 || !(0.0..=100.0).contains(&value))
        {
            return Err("Enter a whole level or rank".into());
        }
        match self {
            Self::Level => {
                let actor = s.actor_config_mut();
                actor.level = value as u32;
                actor.xp = 0;
            }
            Self::Rank(i) => s.actor_config_mut().ranks[i] = value as u8,
            Self::Hp => s.actor_config_mut().max_hp = value,
            Self::Armor => s.actor_config_mut().armor = value,
            Self::Resistance => s.actor_config_mut().resistance = value,
            Self::Move => s.actor_config_mut().move_speed = value,
            Self::AttackSpeed => s.actor_config_mut().attack_speed = value,
            Self::Damage => s.actor_config_mut().damage_multiplier = value,
            Self::Aggression => s.config.enemy.aggression_range = value,
            Self::Distance => s.config.enemy.attack_distance = value,
            Self::DummyHp => s.config.dummy.max_hp = value,
            Self::DummyArmor => s.config.dummy.armor = value,
            Self::DummyResistance => s.config.dummy.resistance = value,
            Self::PresetName => {}
        }
        s.apply();
        Ok(())
    }
    fn step(self) -> f32 {
        match self {
            Self::Level | Self::Rank(_) => 1.0,
            Self::Hp | Self::DummyHp => 100.0,
            Self::Armor | Self::Resistance | Self::DummyArmor | Self::DummyResistance => 10.0,
            Self::Move | Self::AttackSpeed | Self::Damage => 0.25,
            _ => 1.0,
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct Edit {
    field: Field,
    text: String,
    selected: bool,
}
#[derive(Clone, Copy, Debug)]
enum Toggle {
    God,
    Infinite,
    Cooldowns,
    Unlock,
    Enemy,
    Dummy,
    DummyInfinite,
    DummyMoving,
    Respawn,
    Minions,
    MinionPause,
    Pause,
    Overlay,
    Geometry,
}
impl Toggle {
    fn label(self) -> &'static str {
        match self {
            Self::God => "God mode",
            Self::Infinite => "Infinite mana",
            Self::Cooldowns => "No cooldowns",
            Self::Unlock => "Unlock all skills",
            Self::Enemy => "Enemy enabled",
            Self::Dummy => "Dummy enabled",
            Self::DummyInfinite => "Infinite HP",
            Self::DummyMoving => "Moving dummy",
            Self::Respawn => "Enemy auto-respawn",
            Self::Minions => "Minions enabled",
            Self::MinionPause => "Pause minions",
            Self::Pause => "Pause simulation",
            Self::Overlay => "Combat state overlay",
            Self::Geometry => "Combat ranges / hurtboxes",
        }
    }
    fn value(self, s: &SandboxClient) -> bool {
        match self {
            Self::God => s.actor_config().god_mode,
            Self::Infinite => s.actor_config().infinite_resource,
            Self::Cooldowns => s.actor_config().no_cooldowns,
            Self::Unlock => s.actor_config().unlock_all,
            Self::Enemy => s.config.enemy.enabled,
            Self::Dummy => s.config.dummy.enabled,
            Self::DummyInfinite => s.config.dummy.infinite_hp,
            Self::DummyMoving => s.config.dummy.moving,
            Self::Respawn => s.config.enemy.auto_respawn,
            Self::Minions => s.config.environment.minions,
            Self::MinionPause => s.config.environment.minions_paused,
            Self::Pause => s.config.environment.paused,
            Self::Overlay => s.overlay,
            Self::Geometry => s.geometry,
        }
    }
    fn flip(self, s: &mut SandboxClient) {
        match self {
            Self::God => s.actor_config_mut().god_mode ^= true,
            Self::Infinite => s.actor_config_mut().infinite_resource ^= true,
            Self::Cooldowns => s.actor_config_mut().no_cooldowns ^= true,
            Self::Unlock => s.actor_config_mut().unlock_all ^= true,
            Self::Enemy => s.config.enemy.enabled ^= true,
            Self::Dummy => s.config.dummy.enabled ^= true,
            Self::DummyInfinite => s.config.dummy.infinite_hp ^= true,
            Self::DummyMoving => s.config.dummy.moving ^= true,
            Self::Respawn => s.config.enemy.auto_respawn ^= true,
            Self::Minions => s.config.environment.minions ^= true,
            Self::MinionPause => s.config.environment.minions_paused ^= true,
            Self::Pause => s.config.environment.paused ^= true,
            Self::Overlay => s.overlay ^= true,
            Self::Geometry => s.geometry ^= true,
        }
        if !matches!(self, Self::Overlay | Self::Geometry) {
            s.apply();
        }
    }
}
#[derive(Component, Clone, Debug)]
enum Action {
    Tab(Tab),
    Close,
    Toggle(Toggle),
    Edit(Field),
    Step(Field, f32),
    Hero(shared::HeroClass),
    Avatar(String),
    Stage(u32),
    Xp(i32),
    Command(SandboxCommand),
    Behavior(BotBehavior),
    Speed(f32),
    Item(shared::shop::ItemId),
    ClearItems,
    Teleport,
    Preview(PreviewKind),
    StopPreview,
    Repeat,
    Save,
    Load(String),
    LoadNamed,
    Actor(SandboxActor),
}
#[derive(Component)]
enum Label {
    Field(Field),
    Toggle(Toggle),
    Status,
    Actor,
    Analytics,
    Animations,
}
#[derive(Component)]
struct Root;
#[derive(Component)]
struct Body;
#[derive(Component)]
struct Overlay;

pub(super) fn install(app: &mut App) {
    qa::install(app);
    app.add_systems(
        Update,
        (
            keys,
            actions,
            edit_keys,
            teleport,
            build_panel,
            refresh_labels,
            scroll,
            overlay,
        )
            .chain()
            .in_set(crate::input_context::InputContextSet::Modal)
            .before(crate::pause_menu::toggle_pause_menu),
    )
    .add_systems(PostUpdate, draw_geometry);
}
fn row() -> Node {
    Node {
        width: Val::Percent(100.0),
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        column_gap: Val::Px(6.0),
        row_gap: Val::Px(6.0),
        align_items: AlignItems::Center,
        flex_shrink: 0.0,
        ..default()
    }
}
fn button(parent: &mut ChildSpawnerCommands, label: impl Into<String>, action: Action) {
    parent
        .spawn((
            Button,
            Node {
                min_height: Val::Px(32.0),
                padding: UiRect::axes(Val::Px(9.0), Val::Px(6.0)),
                border_radius: BorderRadius::all(Val::Px(5.0)),
                ..default()
            },
            BackgroundColor(w::TILE),
            w::MenuButton::new(ButtonKind::Secondary),
            action,
        ))
        .with_child(w::label(&label.into(), 13.0, w::IVORY));
}
fn number(parent: &mut ChildSpawnerCommands, field: Field, s: &SandboxClient) {
    parent.spawn(row()).with_children(|r| {
        r.spawn((
            w::label(&field.label(), 13.0, w::MUTED),
            Node {
                width: Val::Px(180.0),
                ..default()
            },
        ));
        button(r, "-", Action::Step(field, -field.step()));
        r.spawn((
            Button,
            Node {
                min_width: Val::Px(84.0),
                min_height: Val::Px(32.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(w::TILE),
            Action::Edit(field),
        ))
        .with_child((
            w::label(&field.value(s), 14.0, w::IVORY),
            Label::Field(field),
        ));
        button(r, "+", Action::Step(field, field.step()));
    });
}
fn toggle(parent: &mut ChildSpawnerCommands, t: Toggle, s: &SandboxClient) {
    let mut node = row();
    node.min_height = Val::Px(32.0);
    node.padding = UiRect::axes(Val::Px(9.0), Val::Px(6.0));
    parent
        .spawn((Button, node, BackgroundColor(w::TILE), Action::Toggle(t)))
        .with_child((
            w::label(
                &format!("{}: {}", t.label(), if t.value(s) { "ON" } else { "OFF" }),
                14.0,
                w::IVORY,
            ),
            Label::Toggle(t),
        ));
}
fn heading(parent: &mut ChildSpawnerCommands, text: &str) {
    parent.spawn(w::label(text, 16.0, w::GOLD));
}
fn build_panel(
    mut commands: Commands,
    mut state: ResMut<SandboxClient>,
    session: Res<crate::net::ClientSession>,
    roots: Query<Entity, With<Root>>,
) {
    if !state.enabled || !state.rebuild || !session.join_confirmed() {
        return;
    }
    state.rebuild = false;
    for e in &roots {
        commands.entity(e).despawn();
    }
    if !state.open {
        return;
    }
    commands.spawn((Node{position_type:PositionType::Absolute,right:Val::Px(14.0),top:Val::Px(60.0),bottom:Val::Px(16.0),width:Val::Px(460.0),max_width:Val::Percent(95.0),flex_direction:FlexDirection::Column,padding:UiRect::all(Val::Px(14.0)),row_gap:Val::Px(10.0),border:UiRect::all(Val::Px(1.0)),border_radius:BorderRadius::all(Val::Px(10.0)),..default()},Root,Name::new("CombatTestPanel"),BackgroundColor(w::PANEL),BorderColor::all(w::GOLD),GlobalZIndex(120))).with_children(|p|{
        p.spawn(row()).with_children(|r|{heading(r,"COMBAT TEST");button(r,"Close [F6]",Action::Close);});
        p.spawn((w::label(&state.status,12.0,w::MUTED),Label::Status));
        p.spawn(row()).with_children(|r|{for t in Tab::ALL{button(r,t.label(),Action::Tab(t));}});
        p.spawn((Node{flex_direction:FlexDirection::Column,flex_grow:1.0,min_height:Val::Px(0.0),overflow:Overflow::scroll_y(),row_gap:Val::Px(9.0),padding:UiRect::right(Val::Px(6.0)),..default()},Body,Name::new("CombatTestBody"),ScrollPosition::default())).with_children(|p|match state.tab{
            Tab::Player|Tab::Enemy=>{
                if state.tab==Tab::Enemy{toggle(p,Toggle::Enemy,&state);}
                heading(p,"Hero and progression");p.spawn((w::label("",13.0,w::IVORY),Label::Actor));
                p.spawn(row()).with_children(|r|for hero in shared::HeroClass::ALL{button(r,hero.display_name(),Action::Hero(hero));});
                p.spawn(row()).with_children(|r|for(level,label)in [(1,"Early"),(5,"Mid"),(10,"Late")]{button(r,label,Action::Stage(level));});
                number(p,Field::Level,&state);
                p.spawn(row()).with_children(|r|{button(r,"-100 XP",Action::Xp(-100));button(r,"+100 XP",Action::Xp(100));});
                for i in 0..4{number(p,Field::Rank(i),&state);}toggle(p,Toggle::Unlock,&state);
                heading(p,"Combat controls");
                for t in [Toggle::God,Toggle::Infinite,Toggle::Cooldowns]{toggle(p,t,&state);}
                for field in [Field::Hp,Field::Armor,Field::Resistance,Field::Move,Field::AttackSpeed,Field::Damage]{number(p,field,&state);}
                p.spawn(row()).with_children(|r|{button(r,"Refill HP + mana",Action::Command(SandboxCommand::Refill{actor:state.actor}));button(r,"Reset cooldowns",Action::Command(SandboxCommand::ResetCooldowns{actor:state.actor}));button(r,"Teleport: pick point",Action::Teleport);button(r,"Reset actor",Action::Command(SandboxCommand::ResetActor{actor:state.actor}));button(r,"Reset both / duel",Action::Command(SandboxCommand::ResetDuel));});
                if state.tab==Tab::Enemy{
                    heading(p,"AI behavior");p.spawn(row()).with_children(|r|for (label,mode) in [("Stand",BotBehavior::Stationary),("Flee",BotBehavior::Flee),("Attack",BotBehavior::Attack),("Fight",BotBehavior::Fight)]{button(r,label,Action::Behavior(mode));});
                    number(p,Field::Aggression,&state);number(p,Field::Distance,&state);toggle(p,Toggle::Respawn,&state);
                }
                p.spawn(row()).with_children(|r|for i in 0..4{button(r,format!("Cast {}",["Q","W","E","R"][i]),Action::Command(SandboxCommand::ForceCast{actor:state.actor,slot:i as u8,target_id:None}));});
                heading(p,"Grant equipment");p.spawn(row()).with_children(|r|{for item in shared::shop::items(){button(r,item.name,Action::Item(item.id));}button(r,"Clear inventory",Action::ClearItems);});
                heading(p,"Shipped appearances");p.spawn(row()).with_children(|r|for avatar in shared::avatar_roster().iter().filter(|a|a.passport.is_none()){button(r,&avatar.slug,Action::Avatar(avatar.slug.clone()));});
            },
            Tab::Dummy=>{
                heading(p,"Training target");for t in [Toggle::Dummy,Toggle::DummyInfinite,Toggle::DummyMoving]{toggle(p,t,&state);}
                for f in [Field::DummyHp,Field::DummyArmor,Field::DummyResistance]{number(p,f,&state);}
                button(p,"Move dummy: pick point",Action::Teleport);button(p,"Reset damage measurements",Action::Command(SandboxCommand::ResetAnalytics));
                p.spawn(w::label("Infinite HP records full mitigated hits. Finite HP records health actually removed. Armor reduces physical hits; resistance reduces skills.",13.0,w::MUTED));
            },
            Tab::World=>{
                heading(p,"Simulation time");p.spawn(row()).with_children(|r|for speed in TIME_SCALES{button(r,format!("{speed}x"),Action::Speed(speed));});
                toggle(p,Toggle::Pause,&state);button(p,"Step one simulation frame",Action::Command(SandboxCommand::FrameStep));
                heading(p,"Match environment");toggle(p,Toggle::Minions,&state);toggle(p,Toggle::MinionPause,&state);button(p,"Spawn one wave",Action::Command(SandboxCommand::SpawnWave));
                toggle(p,Toggle::Overlay,&state);toggle(p,Toggle::Geometry,&state);
                p.spawn(w::label("Network and menu input keep running while combat is paused. F7 pause/resume · F9 frame step.",13.0,w::MUTED));
            },
            Tab::Animation=>{
                heading(p,"Inspect model motion");p.spawn(row()).with_children(|r|{button(r,"Your hero",Action::Actor(SandboxActor::Player));button(r,"Enemy",Action::Actor(SandboxActor::Enemy));});
                p.spawn((w::label("Waiting for loaded animation graph…",13.0,w::MUTED),Label::Animations));
                p.spawn(row()).with_children(|r|for(kind,label)in [(PreviewKind::Idle,"Idle"),(PreviewKind::Run,"Run"),(PreviewKind::Walk,"Walk"),(PreviewKind::Attack,"Attack"),(PreviewKind::Cast,"Skill"),(PreviewKind::Hit,"Hit"),(PreviewKind::Death,"Death")]{button(r,label,Action::Preview(kind));});
                button(p,"Resume combat animation",Action::StopPreview);button(p,"Repeat last action / preview",Action::Repeat);
                p.spawn(row()).with_children(|r|for speed in TIME_SCALES{button(r,format!("{speed}x"),Action::Speed(speed));});toggle(p,Toggle::Pause,&state);button(p,"Step frame",Action::Command(SandboxCommand::FrameStep));
                p.spawn(w::label("Preview changes only the selected model's pose; it does not deal damage or kill the hero. Use Cast in Hero/Enemy to repeat a real skill.",13.0,w::MUTED));
            },
            Tab::Damage=>{
                heading(p,"Confirmed damage");p.spawn(row()).with_children(|r|{button(r,"Dummy",Action::Actor(SandboxActor::Dummy));button(r,"Enemy",Action::Actor(SandboxActor::Enemy));button(r,"Your hero",Action::Actor(SandboxActor::Player));});
                p.spawn((w::label("Deal damage to begin.",14.0,w::IVORY),Label::Analytics));button(p,"Reset meter",Action::Command(SandboxCommand::ResetAnalytics));
                p.spawn(w::label("DPS = confirmed damage / simulation seconds since reset. Pauses do not dilute DPS. Breakdown filters the selected target; source IDs separate each attacker.",13.0,w::MUTED));
            },
            Tab::Presets=>{
                heading(p,"Start from a scenario");for name in presets::BUILTINS{button(p,name,Action::Load(name.into()));}
                p.spawn(w::label("duel: both level 10 · late-game: max build + waves · dps: infinite target · animation: 0.25x",13.0,w::MUTED));
                heading(p,"Save your complete test configuration");p.spawn((Button,row(),BackgroundColor(w::TILE),Action::Edit(Field::PresetName))).with_child((w::label(&state.preset_name,14.0,w::IVORY),Label::Field(Field::PresetName)));
                button(p,"Save named preset",Action::Save);button(p,"Load named preset",Action::LoadNamed);
                p.spawn(w::label(&format!("Files: {}",presets::directory().display()),12.0,w::MUTED));
            }
        });
        p.spawn(w::label("Click a value to type · Enter applies · Esc cancels · F6 closes",11.0,w::MUTED));
    });
}
fn keys(mut keys: ResMut<ButtonInput<KeyCode>>, mut s: ResMut<SandboxClient>) {
    if !s.enabled {
        return;
    }
    if keys.just_pressed(KeyCode::F6) {
        s.open ^= true;
        s.edit = None;
        s.teleport = false;
        s.rebuild = true;
    }
    if keys.just_pressed(KeyCode::Escape) && (s.open || s.teleport) {
        if s.edit.take().is_none() {
            s.open = false;
            s.teleport = false;
            s.rebuild = true;
        }
        keys.clear_just_pressed(KeyCode::Escape);
    }
    if s.edit.is_none() {
        if keys.just_pressed(KeyCode::F7) {
            Toggle::Pause.flip(&mut s);
        }
        if keys.just_pressed(KeyCode::F9) {
            s.submit(SandboxCommand::FrameStep);
        }
    }
}
fn actions(
    mut s: ResMut<SandboxClient>,
    game: Res<GameStateSnapshot>,
    mut network: MessageWriter<NetworkCommand>,
    buttons: Query<(&Interaction, &Action), Changed<Interaction>>,
) {
    if !s.enabled || !s.open {
        return;
    }
    for (_, action) in buttons.iter().filter(|(i, _)| **i == Interaction::Pressed) {
        match action {
            Action::Tab(t) => {
                s.tab = *t;
                s.edit = None;
                s.rebuild = true;
                s.actor = match t {
                    Tab::Enemy => SandboxActor::Enemy,
                    Tab::Dummy | Tab::Damage => SandboxActor::Dummy,
                    _ => SandboxActor::Player,
                };
            }
            Action::Close => {
                s.open = false;
                s.rebuild = true;
                s.edit = None;
            }
            Action::Toggle(t) => t.flip(&mut s),
            Action::Edit(f) => {
                s.edit = Some(Edit {
                    field: *f,
                    text: f.value(&s),
                    selected: true,
                })
            }
            Action::Step(f, delta) => {
                if let Ok(v) = f.value(&s).parse::<f32>() {
                    if let Err(e) = f.set(&mut s, &(v + delta).to_string()) {
                        s.status = e;
                    }
                }
            }
            Action::Hero(hero) => {
                s.actor_config_mut().hero = *hero;
                s.apply();
            }
            Action::Avatar(slug) => {
                s.actor_config_mut().avatar = Some(slug.clone());
                s.apply();
            }
            Action::Stage(level) => {
                let a = s.actor_config_mut();
                a.level = *level;
                a.xp = 0;
                a.ranks = if *level == 10 { [3; 4] } else { [1; 4] };
                s.apply();
            }
            Action::Xp(amount) => {
                let actor = s.actor;
                s.submit(SandboxCommand::AddXp {
                    actor,
                    amount: *amount,
                });
            }
            Action::Command(command) => {
                if matches!(command, SandboxCommand::ForceCast { .. }) {
                    s.preview = None;
                }
                let mut command = command.clone();
                if let SandboxCommand::ForceCast {
                    actor, target_id, ..
                } = &mut command
                {
                    *target_id = game
                        .sandbox
                        .as_ref()
                        .and_then(|g| {
                            g.actors.iter().find(|a| {
                                a.actor
                                    == if *actor == SandboxActor::Enemy {
                                        SandboxActor::Player
                                    } else if g.config.dummy.enabled {
                                        SandboxActor::Dummy
                                    } else {
                                        SandboxActor::Enemy
                                    }
                            })
                        })
                        .map(|a| a.id);
                }
                s.submit(command);
            }
            Action::Behavior(mode) => {
                s.config.enemy.behavior = *mode;
                s.apply();
            }
            Action::Speed(speed) => {
                s.config.environment.time_scale = *speed;
                s.apply();
            }
            Action::Item(item) => {
                let actor = s.actor;
                s.submit(SandboxCommand::GrantItem { actor, item: *item });
            }
            Action::ClearItems => {
                s.actor_config_mut().inventory.clear();
                s.apply();
            }
            Action::Teleport => {
                s.teleport = true;
                s.open = false;
                s.rebuild = true;
                s.status = "Click a walkable point on the map. Esc cancels.".into();
            }
            Action::Preview(kind) => {
                if let Some(id) = game
                    .sandbox
                    .as_ref()
                    .and_then(|g| g.actors.iter().find(|a| a.actor == s.actor))
                    .map(|a| a.id)
                {
                    s.preview_sequence += 1;
                    s.preview = Some(Preview {
                        id,
                        kind: *kind,
                        sequence: s.preview_sequence,
                    });
                    s.last_action = None;
                    s.last_cast = None;
                } else {
                    s.status = "Spawn the selected actor first".into();
                }
            }
            Action::StopPreview => s.preview = None,
            Action::Repeat => {
                if let Some(mut p) = s.preview.clone() {
                    s.preview_sequence += 1;
                    p.sequence = s.preview_sequence;
                    s.preview = Some(p);
                } else if let Some((target, slot)) = s.last_cast {
                    network.write(NetworkCommand::Cast { target, slot });
                } else if let Some(command) = s.last_action.clone() {
                    s.submit(command);
                }
            }
            Action::Actor(actor) => s.actor = *actor,
            Action::Save => {
                let mut config = s.config.clone();
                if let Some(snapshot) = &game.sandbox {
                    for actor in &snapshot.actors {
                        match actor.actor {
                            SandboxActor::Player => config.player.position = actor.position,
                            SandboxActor::Enemy => config.enemy.actor.position = actor.position,
                            SandboxActor::Dummy => config.dummy.position = actor.position,
                        }
                    }
                }
                s.status = match presets::save(&s.preset_name, &config) {
                    Ok(p) => format!("Saved {}", p.display()),
                    Err(e) => e,
                };
            }
            Action::Load(name) => match presets::load(name) {
                Ok(config) => {
                    s.config = config;
                    s.apply();
                    s.submit(SandboxCommand::ResetDuel);
                }
                Err(e) => s.status = e,
            },
            Action::LoadNamed => match presets::load(&s.preset_name) {
                Ok(config) => {
                    s.config = config;
                    s.apply();
                    s.submit(SandboxCommand::ResetDuel);
                }
                Err(e) => s.status = e,
            },
        }
    }
}
fn edit_keys(mut input: MessageReader<KeyboardInput>, mut s: ResMut<SandboxClient>) {
    if !s.enabled || s.edit.is_none() {
        input.clear();
        return;
    }
    for event in input.read().filter(|e| e.state.is_pressed()) {
        match &event.logical_key {
            Key::Enter => {
                if let Some(edit) = s.edit.take() {
                    if let Err(e) = edit.field.set(&mut s, &edit.text) {
                        s.status = e;
                    }
                }
            }
            Key::Escape => s.edit = None,
            Key::Backspace => {
                if let Some(edit) = s.edit.as_mut() {
                    if edit.selected {
                        edit.text.clear();
                        edit.selected = false;
                    } else {
                        edit.text.pop();
                    }
                }
            }
            Key::Character(value) => {
                if let Some(edit) = s.edit.as_mut() {
                    if edit.selected {
                        edit.text.clear();
                        edit.selected = false;
                    }
                    if edit.text.len() + value.len() <= 64 {
                        edit.text.push_str(value);
                    }
                }
            }
            _ => {}
        }
    }
}
fn refresh_labels(
    s: Res<SandboxClient>,
    game: Res<GameStateSnapshot>,
    animations: Res<AnimationReadout>,
    mut labels: Query<(&Label, &mut Text)>,
) {
    if !s.enabled {
        return;
    }
    for (label, mut text) in &mut labels {
        let next = match label {
            Label::Field(f) => s
                .edit
                .as_ref()
                .filter(|e| e.field == *f)
                .map(|e| format!("> {}_", e.text))
                .unwrap_or_else(|| f.value(&s)),
            Label::Toggle(t) => {
                format!("{}: {}", t.label(), if t.value(&s) { "ON" } else { "OFF" })
            }
            Label::Status => s.status.clone(),
            Label::Actor => format!(
                "{} · appearance {}\nRanks {:?} · items {}\nAI {:?}",
                s.actor_config().hero.display_name(),
                s.actor_config()
                    .avatar
                    .as_deref()
                    .unwrap_or("class default"),
                s.actor_config().ranks,
                s.actor_config()
                    .inventory
                    .iter()
                    .map(|i| i.id())
                    .collect::<Vec<_>>()
                    .join(", "),
                s.config.enemy.behavior
            ),
            Label::Analytics => analytics_text(&s, &game),
            Label::Animations => game
                .sandbox
                .as_ref()
                .and_then(|g| g.actors.iter().find(|a| a.actor == s.actor))
                .and_then(|a| animations.0.get(&a.id))
                .map(|(current, available)| {
                    format!(
                        "Current: {current}\nAvailable: {}\nHit without a clip: Attack fallback",
                        available.join(", ")
                    )
                })
                .unwrap_or_else(|| "Spawn the actor and wait for its model to load.".into()),
        };
        if text.0 != next {
            text.0 = next;
        }
    }
}
fn analytics_text(s: &SandboxClient, game: &GameStateSnapshot) -> String {
    let Some(g) = &game.sandbox else {
        return "Waiting for sandbox telemetry".into();
    };
    let Some(target) = g.actors.iter().find(|a| a.actor == s.actor) else {
        return "Selected target is not spawned".into();
    };
    let rows: Vec<_> = g
        .analytics
        .breakdown
        .iter()
        .filter(|b| b.target_id == target.id)
        .collect();
    let total: f64 = rows.iter().map(|b| b.damage).sum();
    let hits: u64 = rows.iter().map(|b| b.hits).sum();
    let mut text = format!(
        "Target {:?} #{}\nDamage {:.1} · {} hits\nDPS {:.2} · window {:.2}s\nLast hit on target: {:.1}\n",
        s.actor,
        target.id,
        total,
        hits,
        total / g.analytics.elapsed_secs.max(0.001),
        g.analytics.elapsed_secs,
        rows.iter()
            .max_by_key(|r| r.last_event_id)
            .map_or(0.0, |r| r.last_hit)
    );
    for row in rows {
        let slot = row
            .slot
            .filter(|s| *s < 4)
            .map(|i| ["Q", "W", "E", "R"][i as usize])
            .unwrap_or("Attack");
        text.push_str(&format!(
            "\n{:?} #{} / {}: {:.1} ({} hits)",
            row.source_kind, row.source_id, slot, row.damage, row.hits
        ));
    }
    text
}
fn scroll(
    mut events: MessageReader<MouseWheel>,
    s: Res<SandboxClient>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<Body>>,
) {
    if !s.open || !s.enabled {
        events.clear();
        return;
    }
    let delta: f32 = events
        .read()
        .map(|e| {
            e.y * if e.unit == MouseScrollUnit::Line {
                32.0
            } else {
                1.0
            }
        })
        .sum();
    for (node, mut pos) in &mut panels {
        let max = ((node.content_size().y - node.size().y) * node.inverse_scale_factor()).max(0.0);
        pos.y = (pos.y - delta).clamp(0.0, max);
    }
}
fn teleport(
    mut s: ResMut<SandboxClient>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<crate::camera::MainCamera>>,
    mode: Res<crate::sprite::PlayerVisualMode>,
) {
    if !s.teleport || !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let (Ok(window), Ok((camera, transform))) = (window.single(), camera.single()) else {
        return;
    };
    let Some(point) = window.cursor_position().and_then(|p| {
        crate::player::viewport_to_simulation_world(camera, transform, p, *mode, 0.0)
    }) else {
        return;
    };
    let actor = s.actor;
    s.submit(SandboxCommand::Teleport {
        actor,
        position: [point.x, point.z],
    });
    s.teleport = false;
    s.open = true;
    s.rebuild = true;
}
fn overlay(
    mut commands: Commands,
    s: Res<SandboxClient>,
    game: Res<GameStateSnapshot>,
    animations: Res<AnimationReadout>,
    players: Query<(
        &crate::net::NetworkPlayerId,
        &crate::combat::CombatStats,
        &Transform,
        &crate::net::PlayerProgression,
    )>,
    mut existing: Query<(Entity, &mut Text), With<Overlay>>,
) {
    if !s.enabled {
        return;
    }
    if !s.overlay && !s.teleport {
        for (e, _) in &existing {
            commands.entity(e).despawn();
        }
        return;
    }
    let mut text = "COMBAT TEST [F6] panel · [F7] pause · [F9] step\n".to_string();
    if s.geometry {
        text.push_str("Red: hurtbox · gold: basic reach · orange: projectile\nQ cyan · W purple · E pink · R green (cast reach)\n");
    }
    if s.teleport {
        text.push_str("SELECT TELEPORT POINT · left click · Esc cancels\n");
    }
    if let Some(g) = &game.sandbox {
        text.push_str(&format!(
            "Simulation {:.2}s · frame {} · {:.2}x {}\n",
            g.simulation_secs,
            g.frame,
            g.config.environment.time_scale,
            if g.config.environment.paused {
                "PAUSED"
            } else {
                ""
            }
        ));
        for a in &g.actors {
            if a.actor != s.actor {
                continue;
            }
            if let Some((_, stats, pose, progress)) = players.iter().find(|(id, ..)| id.0 == a.id) {
                text.push_str(&format!("\n{:?} #{} L{} XP{} · HP {:.0}/{:.0} Mana {:.0}/{:.0}\nDMG {:.1} Armor {:.0} Resist {:.0} Move {:.2}m/s Attack speed {:.2}x\nCD Q/W/E/R {:.1?}\nServer XZ {:.2},{:.2} · client XZ {:.2},{:.2}\nMotion {}\n",a.actor,a.id,progress.level,progress.xp,stats.hp,stats.max_hp,stats.mana,stats.max_mana,a.attack_damage,a.armor,a.resistance,a.move_speed,a.attack_speed,a.cooldowns,a.position[0],a.position[1],pose.translation.x,pose.translation.z,animations.0.get(&a.id).map(|a|a.0.as_str()).unwrap_or("loading")));
            }
        }
    } else {
        text.push_str(&s.status);
    }
    if let Some((_, mut label)) = existing.iter_mut().next() {
        if label.0 != text {
            label.0 = text;
        }
    } else {
        commands.spawn((
            Text::new(text),
            TextFont {
                font_size: 12.0,
                ..default()
            },
            TextColor(w::IVORY),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(14.0),
                top: Val::Px(285.0),
                max_width: Val::Percent(48.0),
                padding: UiRect::all(Val::Px(10.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(w::PANEL),
            GlobalZIndex(90),
            Overlay,
            Name::new("CombatTestTelemetry"),
        ));
    }
}
fn draw_geometry(
    s: Res<SandboxClient>,
    game: Res<GameStateSnapshot>,
    players: Query<(
        &crate::net::NetworkPlayerId,
        &Transform,
        &crate::net::NetworkHeroClass,
        &crate::net::PlayerProgression,
    )>,
    projectiles: Query<&Transform, With<crate::net::NetworkProjectile>>,
    mut gizmos: Gizmos,
) {
    if !s.enabled || !s.geometry {
        return;
    }
    let Some(g) = &game.sandbox else {
        return;
    };
    for transform in &projectiles {
        gizmos.sphere(
            Isometry3d::from_translation(transform.translation),
            shared::PROJECTILE_COLLISION_RADIUS,
            Color::srgb(1.0, 0.45, 0.1),
        );
    }
    for (id, t, class, progress) in &players {
        if !g.actors.iter().any(|a| a.id == id.0) {
            continue;
        }
        let p = t.translation + Vec3::Y * 0.1;
        gizmos.circle(
            Isometry3d::new(p, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            shared::PLAYER_TARGET_RADIUS,
            Color::srgb(1.0, 0.3, 0.3),
        );
        gizmos.circle(
            Isometry3d::new(p, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            shared::basic_attack_for_class(class.0).range,
            Color::srgb(1.0, 0.85, 0.2),
        );
        for slot in shared::SkillSlot::ALL {
            let radius = shared::scaled_cast_range(
                shared::ability_for_class_slot(class.0, slot),
                progress.ranks[slot.index()],
            );
            gizmos.circle(
                Isometry3d::new(
                    p + Vec3::Y * (slot.index() as f32 * 0.02),
                    Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                ),
                radius,
                [
                    Color::srgb(0.1, 0.8, 1.0),
                    Color::srgb(0.7, 0.4, 1.0),
                    Color::srgb(1.0, 0.35, 0.65),
                    Color::srgb(0.3, 1.0, 0.6),
                ][slot.index()],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn field_edit_is_numeric_and_queues_real_authority_request() {
        let mut s = SandboxClient {
            enabled: true,
            ..default()
        };
        assert!(Field::Hp.set(&mut s, "NaN").is_err());
        assert_eq!(s.config.player.max_hp, 100.0);
        assert!(Field::Rank(2).set(&mut s, "1.5").is_err());
        Field::Hp.set(&mut s, "550").unwrap();
        assert_eq!(s.config.player.max_hp, 550.0);
        assert!(
            matches!(s.queue.back(),Some(SandboxCommand::ApplyConfig{config})if config.player.max_hp==550.0)
        );
        s.config.player.xp = 35;
        Field::Level.set(&mut s, "10").unwrap();
        assert_eq!((s.config.player.level, s.config.player.xp), (10, 0));
        s.actor = SandboxActor::Enemy;
        Field::AttackSpeed.set(&mut s, "2.5").unwrap();
        assert_eq!(s.config.enemy.actor.attack_speed, 2.5);
        assert_eq!(s.config.player.attack_speed, 1.0);
    }
    #[test]
    fn target_analytics_ignores_other_targets_and_uses_latest_hit() {
        let s = SandboxClient {
            enabled: true,
            actor: SandboxActor::Dummy,
            ..default()
        };
        let game = GameStateSnapshot {
            sandbox: Some(SandboxSnapshot {
                config: Default::default(),
                ack: None,
                last_request_id: 0,
                actors: vec![ActorTelemetry {
                    actor: SandboxActor::Dummy,
                    id: 9,
                    position: [0.0; 2],
                    hp: 100.0,
                    mana: 100.0,
                    armor: 0.0,
                    resistance: 0.0,
                    move_speed: 0.0,
                    attack_speed: 1.0,
                    attack_damage: 0.0,
                    cooldowns: [0.0; 4],
                    unlocked: [true; 4],
                }],
                analytics: DamageAnalytics {
                    elapsed_secs: 2.0,
                    breakdown: vec![
                        DamageBreakdown {
                            target_id: 9,
                            source_id: 1,
                            hits: 1,
                            damage: 30.0,
                            last_hit: 30.0,
                            last_event_id: 1,
                            ..default()
                        },
                        DamageBreakdown {
                            target_id: 10,
                            damage: 500.0,
                            hits: 2,
                            last_hit: 500.0,
                            last_event_id: 2,
                            ..default()
                        },
                    ],
                    ..default()
                },
                simulation_secs: 2.0,
                frame: 20,
            }),
            ..default()
        };
        let text = analytics_text(&s, &game);
        assert!(text.contains("Damage 30.0"));
        assert!(text.contains("DPS 15.00"));
        assert!(text.contains("Last hit on target: 30.0"));
        assert!(!text.contains("500.0"));
    }
}
