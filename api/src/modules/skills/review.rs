use super::converse::Said;
use super::phrases::{Phrases, wants_help};

const DEMO_TRACK: &str = "Night Drive, исполнитель Demo Artist";

const NOW_PLAYING: &[&str] = &[
    "что играет",
    "что сейчас играет",
    "что за трек",
    "что за песня",
    "какой трек",
    "какая песня",
    "что это за",
];
const PAUSE: &[&str] = &["пауз", "стоп", "останови", "выключи"];
const RESUME: &[&str] = &["продолж", "возобнов", "сними с паузы", "сними паузу"];
const NEXT: &[&str] = &["следующ", "дальше", "переключи", "пропусти"];
const PREVIOUS: &[&str] = &["предыдущ", "назад", "прошл"];
const LOUDER: &[&str] = &["громче", "прибавь", "увеличь громкость"];
const QUIETER: &[&str] = &["тише", "убавь", "уменьши громкость"];
const PLAY: &[&str] = &["включи", "поставь", "играй", "запусти", "сыграй"];

#[derive(Debug, PartialEq, Eq)]
enum Intent {
    Help,
    NowPlaying,
    Pause,
    Resume,
    Next,
    Previous,
    Louder,
    Quieter,
    Volume(u32),
    Play(String),
    Other,
}

fn words(utterance: &str) -> String {
    let spaced: String = utterance
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_any(text: &str, keys: &[&str]) -> bool {
    keys.iter().any(|key| text.contains(key))
}

fn volume_level(text: &str) -> Option<u32> {
    if !text.contains("громкост") {
        return None;
    }
    text.split_whitespace()
        .find_map(|word| word.parse::<u32>().ok())
        .map(|level| level.min(100))
}

fn play_target(utterance: &str) -> Option<String> {
    let cleaned = words(utterance);
    let mut parts = cleaned.split_whitespace();
    let first = parts.next()?.to_lowercase();
    if !PLAY.iter().any(|verb| first.starts_with(verb)) {
        return None;
    }
    Some(parts.collect::<Vec<_>>().join(" "))
}

fn intent(utterance: &str) -> Intent {
    let text = words(utterance).to_lowercase();
    if wants_help(utterance) {
        return Intent::Help;
    }
    if has_any(&text, NOW_PLAYING) {
        return Intent::NowPlaying;
    }
    if let Some(level) = volume_level(&text) {
        return Intent::Volume(level);
    }
    if has_any(&text, RESUME) {
        return Intent::Resume;
    }
    if has_any(&text, PAUSE) {
        return Intent::Pause;
    }
    if has_any(&text, NEXT) {
        return Intent::Next;
    }
    if has_any(&text, PREVIOUS) {
        return Intent::Previous;
    }
    if has_any(&text, LOUDER) {
        return Intent::Louder;
    }
    if has_any(&text, QUIETER) {
        return Intent::Quieter;
    }
    match play_target(utterance) {
        Some(target) => Intent::Play(target),
        None => Intent::Other,
    }
}

pub fn answer(utterance: &str, phrases: &Phrases<'_>) -> Said {
    let text = match intent(utterance) {
        Intent::Help => phrases.help(),
        Intent::NowPlaying => format!("Сейчас играет {DEMO_TRACK}."),
        Intent::Pause => "Ставлю на паузу.".to_owned(),
        Intent::Resume => "Продолжаю.".to_owned(),
        Intent::Next => "Включаю следующий трек.".to_owned(),
        Intent::Previous => "Включаю предыдущий трек.".to_owned(),
        Intent::Louder => "Делаю погромче.".to_owned(),
        Intent::Quieter => "Делаю потише.".to_owned(),
        Intent::Volume(level) => format!("Громкость {level}."),
        Intent::Play(target) if target.is_empty() || target.to_lowercase() == "музыку" => {
            "Включаю музыку на компьютере.".to_owned()
        }
        Intent::Play(target) => format!("Включаю {target} на компьютере."),
        Intent::Other => phrases.done().to_owned(),
    };
    Said::plain(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moderator_phrases_get_canned_answers() {
        let cases = [
            ("Включи Земфиру", Intent::Play("Земфиру".into())),
            ("поставь музыку", Intent::Play("музыку".into())),
            ("пауза", Intent::Pause),
            ("поставь на паузу", Intent::Pause),
            ("продолжи", Intent::Resume),
            ("сними с паузы", Intent::Resume),
            ("включи следующий трек", Intent::Next),
            ("дальше", Intent::Next),
            ("предыдущий трек", Intent::Previous),
            ("сделай громче", Intent::Louder),
            ("потише", Intent::Quieter),
            ("громкость 40", Intent::Volume(40)),
            ("поставь громкость 500", Intent::Volume(100)),
            ("что сейчас играет?", Intent::NowPlaying),
            ("что ты умеешь", Intent::Help),
            ("расскажи анекдот", Intent::Other),
        ];
        for (utterance, expected) in cases {
            assert_eq!(intent(utterance), expected, "{utterance}");
        }
    }

    #[test]
    fn answers_name_what_was_asked() {
        let phrases = Phrases { name: "Лоля" };
        assert_eq!(
            answer("включи Земфиру", &phrases).text,
            "Включаю Земфиру на компьютере."
        );
        assert_eq!(answer("пауза", &phrases).speech, "Ставлю на паузу.");
        assert!(answer("что играет", &phrases).text.contains("Night Drive"));
        assert_eq!(answer("громкость 30", &phrases).text, "Громкость 30.");
        assert_eq!(answer("абв", &phrases).text, "Готово.");
        assert!(answer("помощь", &phrases).text.starts_with("Я Лоля"));
    }
}
