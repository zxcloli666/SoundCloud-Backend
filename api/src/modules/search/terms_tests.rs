use super::terms::{Shape, TermRow, Terms, build, quote};

fn token(ord: i16, lexeme: &str) -> TermRow {
    TermRow {
        ord,
        lexeme: lexeme.to_owned(),
        word: None,
        ndoc: None,
        kind: None,
    }
}

fn alt(ord: i16, lexeme: &str, word: &str, ndoc: i32, kind: &str) -> TermRow {
    TermRow {
        ord,
        lexeme: lexeme.to_owned(),
        word: Some(word.to_owned()),
        ndoc: Some(ndoc),
        kind: Some(kind.to_owned()),
    }
}

fn known(ord: i16, word: &str, ndoc: i32) -> TermRow {
    alt(ord, word, word, ndoc, "same")
}

fn built(rows: &[TermRow], shape: Shape) -> Terms {
    build(rows, shape).expect("terms")
}

#[test]
fn every_operand_is_a_quoted_lexeme() {
    assert_eq!(quote("it's"), "'it''s'");
    assert_eq!(quote("back\\slash"), "'back\\\\slash'");
    assert_eq!(quote("a:*"), "'a:*'");
    assert_eq!(quote("&|!()"), "'&|!()'");
}

#[test]
fn one_group_is_strict_only() {
    let terms = built(&[known(1, "umbrella", 10)], Shape::Tracks);
    assert_eq!(terms.strict, "'umbrella'");
    assert_eq!(terms.loose, None);
    assert!(!terms.variant);
}

#[test]
fn two_groups_loosen_to_either() {
    let terms = built(
        &[known(1, "blinding", 3), known(2, "lights", 9)],
        Shape::Tracks,
    );
    assert_eq!(terms.strict, "'blinding' & 'lights'");
    assert_eq!(terms.loose.as_deref(), Some("'blinding' | 'lights'"));
}

#[test]
fn three_groups_loosen_to_any_two() {
    let terms = built(
        &[
            known(1, "nothing", 4),
            known(2, "else", 4),
            known(3, "matters", 4),
        ],
        Shape::Tracks,
    );
    assert_eq!(terms.strict, "'nothing' & 'else' & 'matters'");
    assert_eq!(
        terms.loose.as_deref(),
        Some("('else' & 'matters') | ('nothing' & 'matters') | ('nothing' & 'else')")
    );
}

#[test]
fn six_groups_keep_every_group_and_drop_one_per_loose_branch() {
    let rows: Vec<TermRow> = ["aa", "bb", "cc", "dd", "ee", "ff"]
        .iter()
        .zip(1..)
        .map(|(word, ord)| known(ord, word, 5))
        .collect();
    let terms = built(&rows, Shape::Tracks);
    assert_eq!(terms.strict.matches(" & ").count(), 5);
    assert_eq!(
        terms.loose.as_deref().map(|l| l.matches(" | ").count()),
        Some(5)
    );
}

#[test]
fn corrections_and_completions_join_the_group() {
    let rows = [
        alt(1, "metalica", "metallica", 40, "near"),
        known(2, "nothing", 30),
        alt(3, "els", "else", 20, "prefix"),
        alt(3, "els", "elsewhere", 2, "prefix"),
    ];
    let terms = built(&rows, Shape::Tracks);
    assert_eq!(
        terms.strict,
        "('metalica' | 'metallica') & 'nothing' & ('els' | 'else' | 'elsewhere')"
    );
}

#[test]
fn translit_twins_are_alternatives() {
    let rows = [
        alt(1, "kukla", "кукла", 3, "same"),
        alt(2, "kolduna", "колдуна", 3, "same"),
    ];
    assert_eq!(
        built(&rows, Shape::Tracks).strict,
        "('kukla' | 'кукла') & ('kolduna' | 'колдуна')"
    );
}

#[test]
fn one_letter_noise_and_unknown_tokens_are_not_required() {
    let rows = [
        known(1, "rihanna", 9),
        token(2, "x"),
        known(3, "umbrella", 9),
        known(4, "official", 900),
        token(5, "zzqv"),
    ];
    let terms = built(&rows, Shape::Tracks);
    assert_eq!(terms.strict, "'rihanna' & 'umbrella'");
}

#[test]
fn entities_keep_an_unknown_word_when_another_word_is_listed() {
    let rows = [
        known(1, "john", 40),
        token(2, "smithson"),
        token(3, "x"),
        known(4, "official", 900),
    ];
    let terms = built(&rows, Shape::Entities);
    assert_eq!(terms.strict, "'john' & 'smithson'");
    assert_eq!(terms.loose.as_deref(), Some("'john' | 'smithson'"));
}

#[test]
fn entities_with_no_listed_word_keep_only_the_longest_token() {
    let rows = [token(1, "qq"), token(2, "zzqvxw")];
    assert_eq!(built(&rows, Shape::Entities).strict, "'zzqvxw'");
}

#[test]
fn when_everything_is_dropped_the_longest_token_stays() {
    let rows = [token(1, "qq"), token(2, "zzqvxw"), known(3, "video", 9)];
    assert_eq!(built(&rows, Shape::Tracks).strict, "'zzqvxw'");
}

#[test]
fn a_variant_word_in_the_query_is_detected() {
    let rows = [
        known(1, "umbrella", 9),
        known(2, "sped", 3),
        known(3, "up", 9),
    ];
    assert!(built(&rows, Shape::Tracks).variant);
    let rows = [known(1, "umbrella", 9), known(2, "remix", 3)];
    assert!(built(&rows, Shape::Tracks).variant);
    let rows = [known(1, "umbrella", 9), known(2, "up", 3)];
    assert!(!built(&rows, Shape::Tracks).variant);
}

#[test]
fn lyrics_keep_the_four_rarest_groups_in_query_order() {
    let rows = [
        known(1, "forever", 50),
        known(2, "trusting", 2),
        known(3, "who", 900),
        known(4, "we", 950),
        known(5, "are", 990),
        token(6, "harte"),
        alt(7, "hart", "heart", 40, "near"),
    ];
    let terms = built(&rows, Shape::Lyrics);
    assert_eq!(
        terms.strict,
        "'forever' & 'trusting' & 'who' & ('hart' | 'heart')"
    );
    assert_eq!(
        terms.loose.as_deref().map(|l| l.matches(" | (").count()),
        Some(3)
    );
}

#[test]
fn lyrics_with_two_groups_have_no_loose_form() {
    let terms = built(&[known(1, "purple", 5), known(2, "rain", 5)], Shape::Lyrics);
    assert_eq!(terms.loose, None);
}

#[test]
fn no_tokens_is_no_terms() {
    assert_eq!(build(&[], Shape::Tracks), None);
}
