const MAX_LENGTH: usize = 64;

pub const HEADER: &str = "x-request-id";

pub fn accept_or_mint(incoming: Option<&str>) -> String {
    match incoming.map(str::trim) {
        Some(value) if is_safe(value) => value.to_owned(),
        _ => uuid::Uuid::now_v7().simple().to_string(),
    }
}

fn is_safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caller_that_brings_its_own_id_keeps_it() {
        assert_eq!(
            accept_or_mint(Some("desktop-42_abc")),
            "desktop-42_abc".to_owned()
        );
        assert_eq!(accept_or_mint(Some("  trimmed  ")), "trimmed".to_owned());
    }

    #[test]
    fn a_caller_without_an_id_gets_one() {
        let minted = accept_or_mint(None);

        assert_eq!(minted.len(), 32, "a compact uuid, saw {minted}");
        assert!(is_safe(&minted));
        assert_ne!(minted, accept_or_mint(None), "each request gets its own");
    }

    #[test]
    fn nothing_a_caller_sends_can_be_written_into_a_log_line() {
        for injected in [
            "id with spaces",
            "id\nlevel=ERROR message=\"forged\"",
            "id\"quoted\"",
            "id\u{1b}[31m",
            "id;rm -rf /",
            "",
            "   ",
        ] {
            let accepted = accept_or_mint(Some(injected));
            assert_ne!(
                accepted, injected,
                "a log line must not carry {injected:?} verbatim"
            );
            assert!(is_safe(&accepted));
        }
    }

    #[test]
    fn an_absurdly_long_id_is_replaced_rather_than_truncated() {
        let long = "a".repeat(MAX_LENGTH + 1);

        let accepted = accept_or_mint(Some(&long));

        assert_ne!(accepted, long);
        assert_eq!(
            accept_or_mint(Some(&"a".repeat(MAX_LENGTH))),
            "a".repeat(MAX_LENGTH),
            "exactly at the limit is still the caller's own id"
        );
    }
}
