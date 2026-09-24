pub fn cyrillic_to_latin(value: &str) -> Option<String> {
    if !value.chars().any(is_cyrillic) {
        return None;
    }
    let mut latin = String::with_capacity(value.len() * 2);
    for character in value.chars() {
        match romanize(character) {
            Some(replacement) => latin.push_str(replacement),
            None => latin.push(character),
        }
    }
    Some(latin)
}

fn is_cyrillic(character: char) -> bool {
    matches!(character, '\u{0400}'..='\u{04FF}')
}

fn romanize(character: char) -> Option<&'static str> {
    let lowercase = character.to_lowercase().next().unwrap_or(character);
    let latin = match lowercase {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'ґ' => "g",
        'д' => "d",
        'е' => "e",
        'ё' => "e",
        'є' => "ye",
        'ж' => "zh",
        'з' => "z",
        'и' => "i",
        'і' => "i",
        'ї' => "yi",
        'й' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "kh",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "shch",
        'ъ' => "",
        'ы' => "y",
        'ь' => "",
        'э' => "e",
        'ю' => "yu",
        'я' => "ya",
        _ => return None,
    };
    Some(latin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_input_has_no_transliteration() {
        assert_eq!(cyrillic_to_latin("cold"), None);
    }

    #[test]
    fn russian_title_matches_the_common_genius_spelling() {
        assert_eq!(cyrillic_to_latin("холод").as_deref(), Some("kholod"));
        assert_eq!(
            cyrillic_to_latin("без шансов").as_deref(),
            Some("bez shansov")
        );
        assert_eq!(cyrillic_to_latin("Щука").as_deref(), Some("shchuka"));
    }

    #[test]
    fn soft_signs_disappear_and_latin_parts_survive() {
        assert_eq!(
            cyrillic_to_latin("День (part 2)").as_deref(),
            Some("den (part 2)")
        );
    }
}
