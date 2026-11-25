//! Authored match chat in Russian, English and Simplified Chinese.
use shared::{
    bot_personas::BotPersonaId,
    social::{BotChatTranslation, normalize_chat_text},
};

use super::Cue;

type Phrase = [&'static str; 3];

struct Voice {
    kickoff: Phrase,
    kill: [Phrase; 2],
    end: Phrase,
}

impl Voice {
    const fn new(kickoff: Phrase, first: Phrase, second: Phrase, end: Phrase) -> Self {
        Self {
            kickoff,
            kill: [first, second],
            end,
        }
    }
}

// Keep the three translations for each cue together.
#[rustfmt::skip]
fn voice(persona: BotPersonaId) -> Voice {
    use BotPersonaId::*;
    match persona {
        Sever => Voice::new(
            ["Всем удачи.", "Good luck, everyone.", "各位好运。"],
            ["{name}, чётко.", "{name}, clean.", "{name}，漂亮。"],
            ["Хорошо сыграно, {name}.", "Well played, {name}.", "{name}，打得好。"],
            ["Хороший матч.", "Good match.", "打得不错。"],
        ),
        Rikoshet => Voice::new(
            ["Погнали, удачи всем", "Let's go, good luck all", "开打啦，各位好运"],
            ["Красава, {name}!", "Nice one, {name}!", "{name}，太帅了！"],
            ["{name}, вот это момент", "{name}, what a play", "{name}，这波真精彩"],
            ["гг, бодро вышло", "gg, that was a good one", "gg，这局真带劲"],
        ),
        Myata => Voice::new(
            ["Привет всем, хорошей игры!", "Hi all, have a good game!", "大家好，玩得开心！"],
            ["Классно вышло, {name}!", "Lovely play, {name}!", "{name}，打得真棒！"],
            ["{name}, это было красиво ✨", "{name}, that was lovely ✨", "{name}，打得漂亮 ✨"],
            ["Спасибо за игру!", "Thanks for the game!", "谢谢大家陪我打这局！"],
        ),
        Buran => Voice::new(
            ["Здорова! Поехали", "Hey! Let's get going", "大家好！开打吧"],
            ["Мощно, {name}!", "Big play, {name}!", "{name}，这波够猛！"],
            ["{name}, вот это сыграл!", "{name}, what a play!", "{name}，这波打得好！"],
            ["Вот это заруба. гг", "What a scrap. gg", "打得痛快。gg"],
        ),
        Iskra => Voice::new(
            ["Всем гл ✌", "gl all ✌", "各位好运 ✌"],
            ["{name}, найс 🔥", "nice, {name} 🔥", "{name}，厉害 🔥"],
            ["{name}, жёстко!", "That was sick, {name}!", "{name}，太秀了！"],
            ["гг вп ✌", "gg wp ✌", "gg，打得好 ✌"],
        ),
        Sova => Voice::new(
            ["Удачной игры, ребята", "Have a good game, folks", "各位，玩得开心"],
            ["Вовремя, {name}.", "Good timing, {name}.", "{name}，时机抓得好。"],
            ["{name}, здорово поймал момент.", "{name}, great timing.", "{name}，机会抓得漂亮。"],
            ["Всем спасибо, хорошая игра.", "Thanks for a good game, all.", "打得不错，谢谢大家。"],
        ),
        Kaktus => Voice::new(
            ["ну, всем гл", "well, gl all", "好，各位好运"],
            ["{name}, уважаю", "respect, {name}", "{name}，服了"],
            ["{name}, хорошо зашло", "that worked out nicely, {name}", "{name}，这波成了"],
            ["гг, до встречи", "gg, see you around", "gg，下次见"],
        ),
        Kometa => Voice::new(
            ["Всем хорошего матча! ☄", "Have a great match, everyone! ☄", "祝大家这局打得开心！☄"],
            ["Огонь, {name}!", "On fire, {name}!", "{name}，状态火热！"],
            ["Ух, {name}, мощно!", "Whoa, {name}, huge play!", "哇，{name}，这波猛！"],
            ["Спасибо за матч, было весело!", "Thanks for the match, that was fun!", "这局打得很开心，谢谢大家！"],
        ),
        Fonar => Voice::new(
            ["Привет, удачи на линиях", "Hey, good luck in lane", "大家好，对线加油"],
            ["{name}, хорошо получилось", "Nicely done, {name}", "{name}，这波打得不错"],
            ["{name}, красиво получилось", "Smooth play, {name}", "{name}，打得真顺"],
            ["Спасибо, ребята. Хорошая игра", "Thanks, folks. Good game", "谢谢各位，打得不错"],
        ),
        Tuman => Voice::new(
            ["всем гл", "gl all", "各位好运"],
            ["найс, {name}", "nice, {name}", "漂亮，{name}"],
            ["{name}, чисто", "clean, {name}", "{name}，干净利落"],
            ["гг", "gg", "gg"],
        ),
        Yantar => Voice::new(
            ["Спокойной игры, всем удачи", "Take it easy and good luck", "放轻松，各位好运"],
            ["Уверенно, {name}.", "Confident play, {name}.", "{name}，打得很稳。"],
            ["Собранно сыграно, {name}.", "Composed play, {name}.", "{name}，这波很沉着。"],
            ["Спасибо за хороший матч.", "Thanks for a good match.", "谢谢各位，这局打得挺好。"],
        ),
        Bambuk => Voice::new(
            ["Привет. Играем спокойно", "Hey. Let's take it easy", "大家好，慢慢打"],
            ["Как по маслу, {name}", "Smooth as butter, {name}", "{name}，这波真顺"],
            ["Мягко, {name}. Красиво.", "Nice and smooth, {name}.", "{name}，这波打得真轻松。"],
            ["Увидимся. Хорошая игра", "Good game. See you around", "打得不错，下次见"],
        ),
        Prizma => Voice::new(
            ["Всем привет! Посмотрим, как пойдёт", "Hi all! Let's see how this goes", "大家好！看看这局会怎么打"],
            ["Ух ты, {name}!", "Oh wow, {name}!", "哇，{name}！"],
            ["{name}, интересный ход!", "{name}, clever move!", "{name}，这招有意思！"],
            ["Интересная вышла игра. Спасибо!", "That was an interesting game. Thanks!", "这局挺有意思，谢谢大家！"],
        ),
        Volna => Voice::new(
            ["На связи. Удачи всем!", "Hey. Good luck, all!", "来了，各位加油！"],
            ["{name}, хорошо поймал момент", "Good timing, {name}", "{name}，这波时机抓得好"],
            ["Хорошо пошло, {name}!", "Nice and steady, {name}!", "{name}，打得挺顺！"],
            ["Хорошо поиграли, всем спасибо", "That was a good one, thanks all", "这局玩得不错，谢谢大家"],
        ),
        Shpil => Voice::new(
            ["Удачи. Собрались и в бой", "Good luck. Focus up", "好运。集中精神，开打"],
            ["Точно, {name}.", "Right on target, {name}.", "{name}，打得准。"],
            ["{name}, чёткий ход.", "{name}, sharp play.", "{name}，这一下很利落。"],
            ["Хорошо сыграли. Спасибо.", "Well played. Thanks.", "打得好，谢谢各位。"],
        ),
        Klyukva => Voice::new(
            ["Залетели! Всем удачи", "We're in! Good luck, all", "进来啦！各位加油"],
            ["Опа, {name}! Красиво", "Ooh, {name}! Nice", "哟，{name}！漂亮"],
            ["{name}, вот это сюрприз!", "{name}, didn't see that coming!", "{name}，没想到啊！"],
            ["гг, веселуха была", "gg, that was a blast", "gg，这局真好玩"],
        ),
        Kremen => Voice::new(
            ["Удачи.", "Good luck.", "好运。"],
            ["{name}, сильно.", "{name}, solid.", "{name}，够强。"],
            ["Хорошо, {name}.", "Good one, {name}.", "不错，{name}。"],
            ["Сыграно. гг.", "Well played. gg.", "打得好。gg。"],
        ),
        Runa => Voice::new(
            ["Хорошей игры. Держим голову холодной", "Good game, all. Keep cool", "各位玩得开心，稳住心态"],
            ["{name}, отличная выдержка.", "{name}, great patience.", "{name}，真沉得住气。"],
            ["Хладнокровно, {name}.", "Cool under pressure, {name}.", "{name}，打得真冷静。"],
            ["Спасибо. Интересный матч.", "Thanks. Interesting match.", "谢谢各位，这局有意思。"],
        ),
        Shum => Voice::new(
            ["О, погнали! Всем гл!", "Oh, let's go! gl all!", "哦，开打！各位好运！"],
            ["{name}!! Вот это да!", "{name}!! No way!", "{name}！！太牛了！"],
            ["КРАСИВО, {name}!", "BEAUTIFUL, {name}!", "漂亮啊，{name}！"],
            ["Вот это матч! гг!", "What a game! gg!", "这局真过瘾！gg！"],
        ),
        Plomba => Voice::new(
            ["Так. Всем удачи.", "Right. Good luck.", "好。各位好运。"],
            ["{name}, зачёт.", "{name}, gets the stamp of approval.", "{name}，给你个好评。"],
            ["{name}, принято.", "{name}, noted.", "{name}，这波记下了。"],
            ["Матч закрыт. Спасибо.", "Match closed. Thanks.", "本局结束。谢谢。"],
        ),
        Vetka => Voice::new(
            ["О, привет всем! Удачи :)", "Oh hi, everyone! Good luck :)", "嘿，大家好！好运 :)"],
            ["Отлично, {name} :)", "Great play, {name} :)", "太棒了，{name} :)"],
            ["Здорово вышло, {name}!", "Nicely done, {name}!", "{name}，这波打得真好！"],
            ["До встречи! Спасибо за игру :)", "See you! Thanks for the game :)", "下次见！谢谢大家 :)"],
        ),
        Kvarts => Voice::new(
            ["Доброй игры всем.", "A good game to you all.", "祝各位游戏愉快。"],
            ["Точный момент, {name}.", "Just the right moment, {name}.", "{name}，时机恰到好处。"],
            ["Грамотно, {name}.", "Smart play, {name}.", "{name}，打得有章法。"],
            ["Достойно сыграно. гг.", "That's a well-played game. gg.", "各位打得不错。gg。"],
        ),
        Sirena => Voice::new(
            ["Всем удачи, шоу начинается!", "Good luck, the show begins!", "各位好运，好戏开场！"],
            ["Браво, {name}!", "Bravo, {name}!", "{name}，精彩！"],
            ["{name}, аплодирую!", "{name}, take a bow!", "{name}，给你鼓掌！"],
            ["Спасибо всем, отличный матч!", "Thanks, all. Fantastic match!", "谢谢各位，这局太精彩了！"],
        ),
        Krater => Voice::new(
            ["В бой! Всем удачи", "Into the fight! Good luck all", "开战！各位好运"],
            ["Бах! {name}, мощно.", "Boom! Big play, {name}.", "砰！{name}，这波猛。"],
            ["Есть! Сильно, {name}.", "That's it! Huge, {name}.", "成了！{name}，厉害。"],
            ["Жаркий матч. гг.", "Heated match. gg.", "打得真激烈。gg。"],
        ),
        Zefir => Voice::new(
            ["Приветик, хорошей игры всем!", "Hey hey, have a fun game!", "嗨嗨，大家玩得开心！"],
            ["Ура, {name}, получилось!", "Woo, you got it, {name}!", "好耶，{name}，打成了！"],
            ["Супер, {name} ✨", "Awesome, {name} ✨", "棒极了，{name} ✨"],
            ["гг! Спасибо за эту игру!", "gg! Thanks for that game!", "gg！这局谢谢啦！"],
        ),
        Nyra => Voice::new(
            ["Хей. Всем удачи, будет интересно", "Hey. Good luck, this'll be fun", "嘿，各位好运，这局有得玩"],
            ["Хитро, {name} ;)", "Sneaky, {name} ;)", "够机灵的，{name} ;)"],
            ["{name}, вот это поворот!", "{name}, what a twist!", "{name}，这反转可以啊！"],
            ["гг, славно повеселились", "gg, that was good fun", "gg，这局玩得挺尽兴"],
        ),
        Yasha => Voice::new(
            ["Всем привет, удачной катки", "Hey all, have a good one", "大伙儿好，这局顺利"],
            ["Нормально так, {name}.", "Not bad at all, {name}.", "{name}，这波挺行啊。"],
            ["Хорошая работа, {name}.", "Good work, {name}.", "{name}，干得不错。"],
            ["Всем спасибо за катку.", "Thanks for the game, folks.", "谢谢大伙儿陪打这局。"],
        ),
        Kobalt => Voice::new(
            ["Всем удачи. Сосредоточились.", "Good luck. Time to focus.", "各位好运。集中注意力。"],
            ["Чистое исполнение, {name}.", "Clean execution, {name}.", "{name}，操作利落。"],
            ["Точно в цель, {name}.", "Spot on, {name}.", "{name}，这下正中目标。"],
            ["Матч завершён. Хорошо сыграно.", "Match complete. Well played.", "比赛结束。打得不错。"],
        ),
        Sol => Voice::new(
            ["Так, поехали. Удачи!", "Right, let's go. Good luck!", "好，开打。好运！"],
            ["Ну вот, {name}, отлично.", "There it is, {name}. Nice.", "就是这样，{name}。漂亮。"],
            ["{name}, вот это темп.", "{name}, great pace.", "{name}，这节奏真快。"],
            ["гг, приятно сыграли", "gg, pleasure playing", "gg，这局打得挺开心"],
        ),
        Smola => Voice::new(
            ["Погнали, хорошей игры.", "Let's go. Have a good game.", "开打吧，大家玩得开心。"],
            ["Ловко, {name}.", "Slick, {name}.", "{name}，这招灵活。"],
            ["{name}, уверенно и красиво.", "{name}, confident and clean.", "{name}，打得稳又漂亮。"],
            ["гг, хорошая катка.", "gg, good round.", "gg，好局。"],
        ),
    }
}

pub(super) fn phrase(
    persona: BotPersonaId,
    cue: &Cue<'_>,
    seed: u64,
) -> Option<(String, BotChatTranslation)> {
    let voice = voice(persona);
    let (lines, nickname) = match cue {
        Cue::Kickoff => (voice.kickoff, ""),
        Cue::HumanKill { nickname, .. } => (voice.kill[(seed % 2) as usize], *nickname),
        Cue::End { .. } => (voice.end, ""),
    };
    let [ru, en, zh_hans] =
        lines.map(|line| normalize_chat_text(&line.replace("{name}", nickname)));
    Some((
        ru.ok()?,
        BotChatTranslation {
            en: en.ok()?,
            zh_hans: zh_hans.ok()?,
        },
    ))
}
