pub(crate) const MIN_NAME_LENGTH: usize = 4;

pub(crate) fn name_length(normalized: &str) -> usize {
    normalized
        .chars()
        .map(|c| match c {
            '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{9FFF}' | '\u{20000}'..='\u{3134F}' => 2,
            _ => 1,
        })
        .sum()
}
