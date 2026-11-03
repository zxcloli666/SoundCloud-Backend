use rand::Rng;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const PREFIX: &str = "SCD-";
const BODY_LEN: usize = 8;

pub fn generate(slot: u8) -> String {
    let mut rng = rand::thread_rng();
    let random: String = (1..BODY_LEN)
        .map(|_| char::from(ALPHABET[rng.gen_range(0..ALPHABET.len())]))
        .collect();
    format!(
        "{PREFIX}{}{random}",
        char::from(ALPHABET[usize::from(slot)])
    )
}

pub fn normalize(input: &str) -> Option<String> {
    let upper = input.trim().to_ascii_uppercase();
    let body = upper.strip_prefix(PREFIX)?;
    if body.len() != BODY_LEN {
        return None;
    }
    let body: String = body
        .chars()
        .map(|c| match c {
            'O' => '0',
            'I' | 'L' => '1',
            other => other,
        })
        .collect();
    body.bytes()
        .all(|byte| ALPHABET.contains(&byte))
        .then(|| format!("{PREFIX}{body}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_code_survives_normalize() {
        for _ in 0..200 {
            let code = generate(0);
            assert_eq!(normalize(&code), Some(code.clone()));
            assert!(!code[4..].contains(['I', 'L', 'O', 'U']));
        }
    }

    #[test]
    fn each_host_slot_mints_codes_the_other_slots_never_do() {
        for _ in 0..200 {
            assert!(generate(0).starts_with("SCD-0"));
            assert!(generate(1).starts_with("SCD-1"));
            assert!(generate(31).starts_with("SCD-Z"));
        }
    }

    #[test]
    fn lowercase_and_lookalikes_are_accepted() {
        assert_eq!(normalize("scd-7f3k92qa").as_deref(), Some("SCD-7F3K92QA"));
        assert_eq!(normalize("SCD-OIL00000").as_deref(), Some("SCD-01100000"));
    }

    #[test]
    fn junk_is_refused() {
        for bad in [
            "",
            "SCD-",
            "SCD-1234567",
            "SCD-123456789",
            "XYZ-12345678",
            "SCD-1234567U",
        ] {
            assert_eq!(normalize(bad), None, "{bad}");
        }
    }
}
