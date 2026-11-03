const TEXT_LIMIT: usize = 1000;

pub fn clip(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= TEXT_LIMIT {
        return trimmed.to_owned();
    }
    let cut: String = trimmed.chars().take(TEXT_LIMIT - 1).collect();
    format!("{}…", cut.trim_end())
}

pub fn spoken_digits(text: &str) -> String {
    let mut digits = String::new();
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        if token.chars().all(|c| c.is_ascii_digit()) {
            digits.push_str(token);
            continue;
        }
        let word = match token.to_lowercase().as_str() {
            "ноль" | "нуль" | "ноля" => '0',
            "один" | "одна" | "раз" => '1',
            "два" | "две" => '2',
            "три" => '3',
            "четыре" => '4',
            "пять" => '5',
            "шесть" => '6',
            "семь" => '7',
            "восемь" => '8',
            "девять" => '9',
            _ => continue,
        };
        digits.push(word);
    }
    digits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_collected_from_numbers_and_words() {
        assert_eq!(spoken_digits("код 123 456"), "123456");
        assert_eq!(spoken_digits("1 2 3 4 5 6"), "123456");
        assert_eq!(spoken_digits("один два три четыре пять шесть"), "123456");
        assert_eq!(spoken_digits("ноль пять 7 8 девять два"), "057892");
        assert_eq!(spoken_digits("привет"), "");
    }

    #[test]
    fn long_text_is_clipped_with_an_ellipsis() {
        let long = "а".repeat(1500);
        let clipped = clip(&long);
        assert_eq!(clipped.chars().count(), 1000);
        assert!(clipped.ends_with('…'));
        assert_eq!(clip("  коротко "), "коротко");
    }
}
