use std::collections::HashSet;

use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

use super::phrases::REUPLOAD_MARKERS;

pub fn casefold(text: &str) -> String {
    caseless::default_case_fold_str(text)
}

pub fn unique(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(casefold(item)))
        .collect()
}

pub fn is_reupload_channel(uploader: &str) -> bool {
    let folded = casefold(uploader);
    REUPLOAD_MARKERS
        .iter()
        .any(|marker| folded.contains(marker))
}

pub fn words(text: &str) -> Vec<String> {
    let folded = casefold(&text.nfkc().collect::<String>());
    let plain = unidecode::unidecode(&folded).to_ascii_lowercase();
    word_spans(&plain)
        .into_iter()
        .filter_map(|(start, end)| plain.get(start..end))
        .map(str::to_owned)
        .collect()
}

pub fn find_name(text: &str, name: &str) -> Option<String> {
    let name: String = name.nfkc().collect();
    let needle: Vec<String> = folded_words(&name)
        .into_iter()
        .map(|(_, _, word)| word)
        .collect();
    if needle.is_empty() {
        return None;
    }
    let plain: String = text.nfkc().collect();
    let spans = folded_words(&plain);
    let window = spans
        .windows(needle.len())
        .find(|window| window.iter().map(|(_, _, word)| word).eq(needle.iter()))?;
    let start = window.first()?.0;
    let end = window.last()?.1;
    plain.get(start..end).map(str::to_owned)
}

fn folded_words(text: &str) -> Vec<(usize, usize, String)> {
    word_spans(text)
        .into_iter()
        .filter_map(|(start, end)| Some((start, end, casefold(text.get(start..end)?))))
        .collect()
}

fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut open = None;
    for (at, character) in text.char_indices() {
        match (is_word(character), open) {
            (true, None) => open = Some(at),
            (false, Some(start)) => {
                spans.push((start, at));
                open = None;
            }
            _ => {}
        }
    }
    if let Some(start) = open {
        spans.push((start, text.len()));
    }
    spans
}

fn is_word(character: char) -> bool {
    character == '_'
        || matches!(
            character.general_category_group(),
            GeneralCategoryGroup::Letter | GeneralCategoryGroup::Number
        )
}
