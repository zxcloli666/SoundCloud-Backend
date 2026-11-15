use super::*;

const ASKED: &str = "кто это?";

fn after(replies: &[&str]) -> Readings {
    asked_after(ASKED, replies)
}

fn asked_after(asked: &str, replies: &[&str]) -> Readings {
    let mut readings = Readings::asked(asked);
    for reply in replies {
        readings.hear(reply);
    }
    readings
}

fn heard(readings: &mut Readings, deltas: &[&str]) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = deltas
        .iter()
        .flat_map(|delta| readings.pass(say(delta)))
        .collect();
    pieces.extend(readings.end());
    pieces
}

fn say(text: &str) -> Piece {
    Piece::Say(text.to_owned())
}

fn name(written: &str, spoken: &str) -> Piece {
    Piece::Name {
        written: written.to_owned(),
        spoken: spoken.to_owned(),
    }
}

#[test]
fn text_without_a_known_name_passes_as_it_came() {
    let mut readings = after(&[]);

    assert_eq!(
        heard(&mut readings, &["Включаю Psychosis", ", слушай."]),
        [say("Включаю Psychosis"), say(", слушай.")]
    );
}

#[test]
fn a_name_marked_in_an_earlier_reply_keeps_its_reading() {
    let mut readings = after(&["Это [[Psychosis|Психосис]], трек Nightmare."]);

    assert_eq!(
        heard(
            &mut readings,
            &["Psychosis звучит мощно, а Nightmare тем более."]
        ),
        [
            name("Psychosis", "Психосис"),
            say(" звучит мощно, а Nightmare тем более."),
        ]
    );
}

#[test]
fn a_name_marked_earlier_in_the_same_reply_keeps_its_reading() {
    let mut readings = after(&[]);
    let mut pieces = readings.pass(say("Про "));
    pieces.extend(readings.pass(name("Psychosis", "Психосис")));
    pieces.extend(readings.pass(say(" знаю мало. Трек Psychosis играет.")));
    pieces.extend(readings.end());

    assert_eq!(
        pieces,
        [
            say("Про "),
            name("Psychosis", "Психосис"),
            say(" знаю мало. Трек "),
            name("Psychosis", "Психосис"),
            say(" играет."),
        ]
    );
}

#[test]
fn a_reading_in_letters_the_user_did_not_write_is_not_used() {
    let said = ["Это [[Psychosis|Психосис]] и [[Кино|Kino]]."];
    let mut in_english = asked_after("who is it?", &said);
    let mut in_both = asked_after("кто такие Psychosis?", &said);
    let mut in_digits = asked_after("2 + 2?", &said);

    assert_eq!(
        heard(&mut in_english, &["Psychosis and Кино."]),
        [say("Psychosis and "), name("Кино", "Kino"), say(".")]
    );
    assert_eq!(
        heard(&mut in_both, &["Psychosis и Кино."]),
        [
            name("Psychosis", "Психосис"),
            say(" и "),
            name("Кино", "Kino"),
            say("."),
        ]
    );
    assert_eq!(
        heard(&mut in_digits, &["Psychosis и Кино."]),
        [say("Psychosis и Кино.")]
    );
}

#[test]
fn a_name_split_between_pieces_is_held_until_it_is_whole() {
    let mut readings = after(&["[[Psychosis|Психосис]]"]);

    assert_eq!(readings.pass(say("Слушаем Ps")), [say("Слушаем ")]);
    assert!(readings.pass(say("ycho")).is_empty());
    assert!(readings.pass(say("sis")).is_empty());
    assert_eq!(
        readings.pass(say(" дальше")),
        [name("Psychosis", "Психосис"), say(" дальше")]
    );
    assert!(readings.end().is_empty());
}

#[test]
fn a_held_start_goes_out_when_it_turns_into_another_word() {
    let mut readings = after(&["[[Psychosis|Психосис]]"]);

    assert_eq!(readings.pass(say("Это Psy")), [say("Это ")]);
    assert_eq!(readings.pass(say("trance.")), [say("Psytrance.")]);
}

#[test]
fn the_end_of_the_reply_closes_what_was_held() {
    let mut whole = after(&["[[Psychosis|Психосис]]"]);
    let mut cut = after(&["[[Psychosis|Психосис]]"]);

    assert_eq!(
        heard(&mut whole, &["Играет Psychosis"]),
        [say("Играет "), name("Psychosis", "Психосис")]
    );
    assert_eq!(
        heard(&mut cut, &["Играет Psy"]),
        [say("Играет "), say("Psy")]
    );
}

#[test]
fn a_mark_or_an_action_closes_what_was_held() {
    let mut readings = after(&["[[6ix9ine|Сикс Найн]]"]);

    assert!(readings.pass(say("6ix9ine")).is_empty());
    assert_eq!(
        readings.pass(name("Gummo", "Гаммо")),
        [name("6ix9ine", "Сикс Найн"), name("Gummo", "Гаммо")]
    );
    assert_eq!(readings.pass(say(" и 6ix")), [say(" и ")]);
    assert_eq!(
        readings.pass(Piece::Action(serde_json::Value::Null)),
        [say("6ix"), Piece::Action(serde_json::Value::Null)]
    );
}

#[test]
fn a_longer_word_around_the_name_is_left_alone() {
    let mut readings = after(&["[[Muse|Мьюз]]"]);

    assert_eq!(
        heard(&mut readings, &["Museum, aMuse и Muse2, но Muse."]),
        [
            say("Museum, aMuse и Muse2, но "),
            name("Muse", "Мьюз"),
            say(".")
        ]
    );
}

#[test]
fn the_case_does_not_matter_and_the_text_keeps_its_own() {
    let mut readings = after(&["[[$uicideboy$|Суисайдбойз]] и [[Кино|Кино́]]"]);

    assert_eq!(
        heard(&mut readings, &["$UICIDEBOY$ и кино."]),
        [
            name("$UICIDEBOY$", "Суисайдбойз"),
            say(" и "),
            name("кино", "Кино́"),
            say("."),
        ]
    );
}

#[test]
fn the_newest_reading_of_a_name_wins() {
    let mut readings = after(&["[[hatttes|хаттс]]", "[[Hatttes|хэттес]]"]);

    assert_eq!(
        heard(&mut readings, &["hatttes"]),
        [name("hatttes", "хэттес")]
    );
    readings.pass(name("hatttes", "хаттес"));
    assert_eq!(
        heard(&mut readings, &[" hatttes"]),
        [say(" "), name("hatttes", "хаттес")]
    );
}

#[test]
fn the_longest_known_name_is_taken() {
    let mut readings = after(&["[[Linkin|Линкин]] и [[Linkin Park|Линкин Парк]]"]);

    assert!(readings.pass(say("Linkin ")).is_empty());
    assert_eq!(
        readings.pass(say("Park и Linkin Par")),
        [name("Linkin Park", "Линкин Парк"), say(" и ")]
    );
    assert_eq!(
        readings.pass(say("ty.")),
        [name("Linkin", "Линкин"), say(" Party.")]
    );
}

#[test]
fn only_the_newest_names_are_kept() {
    let marks: Vec<String> = (0..=MOST_KNOWN)
        .map(|at| format!("[[Name{at}|Имя{at}]]"))
        .collect();
    let replies: Vec<&str> = marks.iter().map(String::as_str).collect();
    let mut readings = after(&replies);

    assert_eq!(
        heard(&mut readings, &["Name0 и Name1"]),
        [say("Name0 и "), name("Name1", "Имя1")]
    );
}
