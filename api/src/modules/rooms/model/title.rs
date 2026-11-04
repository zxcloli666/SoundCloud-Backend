pub const MAX_TITLE_CHARS: usize = 48;

fn invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

pub fn clean_title(raw: &str) -> Option<String> {
    let visible: String = raw
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control() && !invisible(*c))
        .collect();
    let title: String = visible
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect();
    let title = title.trim_end().to_owned();
    (!title.is_empty()).then_some(title)
}
