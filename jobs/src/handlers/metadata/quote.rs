use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

pub fn quoted(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut written = String::with_capacity(text.len() + 2);
    written.push(quote);
    for character in text.chars() {
        match character {
            '\\' => written.push_str("\\\\"),
            '\t' => written.push_str("\\t"),
            '\n' => written.push_str("\\n"),
            '\r' => written.push_str("\\r"),
            _ if character == quote => {
                written.push('\\');
                written.push(character);
            }
            _ if printable(character) => written.push(character),
            _ => written.push_str(&escaped(character)),
        }
    }
    written.push(quote);
    written
}

pub fn quoted_list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| quoted(item)).collect();
    format!("[{}]", quoted.join(", "))
}

fn printable(character: char) -> bool {
    character == ' '
        || !matches!(
            character.general_category_group(),
            GeneralCategoryGroup::Other | GeneralCategoryGroup::Separator
        )
}

fn escaped(character: char) -> String {
    match u32::from(character) {
        code @ ..=0xff => format!("\\x{code:02x}"),
        code @ ..=0xffff => format!("\\u{code:04x}"),
        code => format!("\\U{code:08x}"),
    }
}
