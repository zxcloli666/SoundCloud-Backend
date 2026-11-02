use std::borrow::Cow;
use std::io;

use crate::MASK;

const SECRET_WORDS: [&str; 10] = [
    "token",
    "secret",
    "passw",
    "key",
    "auth",
    "session",
    "client_id",
    "credential",
    "cookie",
    "signature",
];

pub fn secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_WORDS.iter().any(|word| lower.contains(word))
}

pub fn text(input: &str) -> Cow<'_, str> {
    if input.len() < 16 && !input.contains([':', '=', '/', '?', ' ', ',']) {
        return Cow::Borrowed(input);
    }
    let without_uri = strip_uri_note(input);
    let mut out = String::with_capacity(without_uri.len());
    let mut hide_next = false;
    let mut rest = without_uri.as_str();
    while !rest.is_empty() {
        let space = rest.find(|c: char| c.is_whitespace() || c == ',');
        let (word, tail) = match space {
            Some(at) => rest.split_at(at),
            None => (rest, ""),
        };
        out.push_str(&word_text(word, &mut hide_next));
        let gap = tail.len()
            - tail
                .trim_start_matches(|c: char| c.is_whitespace() || c == ',')
                .len();
        out.push_str(&tail[..gap]);
        rest = &tail[gap..];
    }
    if out == input {
        Cow::Borrowed(input)
    } else {
        Cow::Owned(out)
    }
}

fn strip_uri_note(input: &str) -> String {
    let Some(start) = input.find(" for uri (") else {
        return input.to_owned();
    };
    let after = &input[start + " for uri (".len()..];
    match after.find(')') {
        Some(end) => format!("{}{}", &input[..start], &after[end + 1..]),
        None => input[..start].to_owned(),
    }
}

fn word_text(word: &str, hide_next: &mut bool) -> String {
    let lead = word.len() - word.trim_start_matches(is_wrapper).len();
    let core_end = word.trim_end_matches(is_wrapper).len().max(lead);
    let (head, core, tail) = (&word[..lead], &word[lead..core_end], &word[core_end..]);
    format!("{head}{}{tail}", core_text(core, hide_next))
}

fn is_wrapper(character: char) -> bool {
    matches!(
        character,
        '(' | ')' | '"' | '\'' | '<' | '>' | '[' | ']' | '{' | '}' | ',' | ';' | '.'
    )
}

fn core_text(core: &str, hide_next: &mut bool) -> String {
    if core.is_empty() {
        return String::new();
    }
    let lower = core.to_ascii_lowercase();
    if matches!(lower.as_str(), "bearer" | "oauth" | "basic") {
        *hide_next = true;
        return core.to_owned();
    }
    if *hide_next {
        *hide_next = false;
        return MASK.to_owned();
    }
    if let Some(at) = core.find("://") {
        return url_text(core, at);
    }
    if let Some(at) = core.find(['=', ':']) {
        let (key, value) = core.split_at(at);
        let separator = &value[..1];
        let value = &value[1..];
        let key_name = key.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if key_name.ends_with("request_id")
            || key_name.ends_with("trace_id")
            || key_name == "oauth_app_id"
        {
            return core.to_owned();
        }
        if secret_key(key_name) {
            if value.is_empty() {
                *hide_next = true;
                return core.to_owned();
            }
            let quote = &value[..value.len() - value.trim_start_matches(is_wrapper).len()];
            return format!("{key}{separator}{quote}{MASK}");
        }
        if value.starts_with(':') {
            return core.to_owned();
        }
        return format!("{key}{separator}{}", word_text(value, hide_next));
    }
    if secret_shaped(core) {
        MASK.to_owned()
    } else {
        core.to_owned()
    }
}

fn url_text(core: &str, scheme_end: usize) -> String {
    let start = core[..scheme_end]
        .rfind(|c: char| !c.is_ascii_alphanumeric())
        .map_or(0, |at| at + 1);
    let after = scheme_end + 3;
    let end = core[after..]
        .find(['"', '\'', ')', '<', '>', '}', ']'])
        .map_or(core.len(), |at| after + at);
    let (prefix, url, suffix) = (&core[..start], &core[start..end], &core[end..]);
    let scheme = &url[..scheme_end - start];
    let rest = &url[scheme_end - start + 3..];
    let cut = rest.find(['?', '#']).unwrap_or(rest.len());
    let location = &rest[..cut];
    let (authority, path) = match location.find('/') {
        Some(at) => location.split_at(at),
        None => (location, ""),
    };
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let path = path
        .split('/')
        .map(|segment| {
            if secret_shaped(segment) || share_token(segment) {
                MASK
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("{prefix}{scheme}://{host}{path}{suffix}")
}

fn share_token(segment: &str) -> bool {
    segment.len() > 3
        && segment.starts_with("s-")
        && segment[2..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
}

fn secret_shaped(word: &str) -> bool {
    if word.starts_with("eyJ") && word.contains('.') && word.len() > 20 {
        return true;
    }
    if word.len() < 24 || is_uuid(word) {
        return false;
    }
    let plain = word
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    plain
        && word.bytes().any(|byte| byte.is_ascii_digit())
        && word.bytes().any(|byte| byte.is_ascii_alphabetic())
}

fn is_uuid(word: &str) -> bool {
    word.len() == 36
        && word.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

pub struct Scrubbing<W>(W);

impl<W: io::Write> Scrubbing<W> {
    pub fn new(inner: W) -> Self {
        Self(inner)
    }
}

impl<W: io::Write> io::Write for Scrubbing<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let line = String::from_utf8_lossy(buf);
        self.0.write_all(text(&line).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT_ID: &str = "aB3dE5gH7jK9mN1pQ3sT5vX7zC9eF1hJ";
    const OAUTH: &str = "2-291234-123456789-abcDEF0123456";

    fn forbidden(value: &str) -> bool {
        value.contains('?') || value.contains(CLIENT_ID) || value.contains(OAUTH)
    }

    #[test]
    fn a_wreq_error_loses_its_uri_and_query() {
        let message = format!(
            "error sending request for uri (https://api-v2.soundcloud.com/tracks/1?client_id={CLIENT_ID}&limit=5): connection reset"
        );

        let cleaned = text(&message);

        assert!(!forbidden(&cleaned), "{cleaned}");
        assert!(cleaned.contains("connection reset"));
    }

    #[test]
    fn urls_keep_host_and_path_but_never_the_query_or_userinfo() {
        let message = format!(
            "GET https://user:pass@api.soundcloud.com/playlists/9?secret_token=s-AbC12&client_id={CLIENT_ID} failed"
        );
        let cleaned = text(&message);

        assert_eq!(cleaned, "GET https://api.soundcloud.com/playlists/9 failed");
    }

    #[test]
    fn secret_shaped_values_are_masked_wherever_they_sit() {
        for message in [
            format!("rejected {CLIENT_ID} twice"),
            format!("Authorization: OAuth {OAUTH}"),
            format!("Bearer {OAUTH}"),
            format!("client_id={CLIENT_ID}"),
            format!("{{\"client_id\":\"{CLIENT_ID}\"}}"),
            format!("oauth_token: {OAUTH}"),
            "https://soundcloud.com/artist/track/s-AbCdE12".to_owned(),
        ] {
            let cleaned = text(&message);
            assert!(!forbidden(&cleaned), "{cleaned}");
            assert!(!cleaned.contains("s-AbCdE12"), "{cleaned}");
        }
    }

    #[test]
    fn ordinary_messages_stay_as_they_are() {
        for message in [
            "database error: pool timed out",
            "request failed",
            "550e8400-e29b-41d4-a716-446655440000 not found",
            "modules::auth::service refused",
            "GET /tracks/{id}",
        ] {
            assert!(matches!(text(message), Cow::Borrowed(_)), "{message}");
        }
    }

    #[test]
    fn a_written_log_line_loses_its_query_secrets_and_ids() {
        let mut sink = Vec::new();
        let mut writer = Scrubbing::new(&mut sink);
        let plain = format!(
            "2026-10-31T10:00:00Z  WARN api::sc: send failed for uri (https://api-v2.soundcloud.com/tracks/1?client_id={CLIENT_ID}): reset\n"
        );
        let json = format!(
            "{{\"level\":\"WARN\",\"fields\":{{\"message\":\"x\",\"client_id\":\"{CLIENT_ID}\"}},\"spans\":[{{\"request_id\":\"4bf92f3577b3da6a3ce929d0e0e4736f\"}}],\"url\":\"https://h/p?oauth_token={OAUTH}\"}}\n"
        );

        io::Write::write_all(&mut writer, plain.as_bytes()).expect("write");
        io::Write::write_all(&mut writer, json.as_bytes()).expect("write");

        let written = String::from_utf8(sink).expect("utf8");
        assert!(!forbidden(&written), "{written}");
        assert!(written.contains("reset"));
        assert!(written.contains("\"message\":\"x\""));
        assert!(written.contains("4bf92f3577b3da6a3ce929d0e0e4736f"));
    }

    #[test]
    fn a_masked_json_field_keeps_the_line_valid_json_and_an_app_id_stays_readable() {
        let app = "019a4f2e-7c1d-7b3a-9e55-1d2c3b4a5f60";
        let line = format!(
            "{{\"fields\":{{\"message\":\"paused\",\"oauth_app_id\":\"{app}\",\"client_id\":\"{CLIENT_ID}\",\"seconds\":300}}}}"
        );

        let cleaned = text(&line);

        assert!(!forbidden(&cleaned), "{cleaned}");
        let parsed: serde_json::Value = serde_json::from_str(&cleaned).expect("still json");
        assert_eq!(parsed["fields"]["oauth_app_id"], app);
        assert_eq!(parsed["fields"]["client_id"], MASK);
        assert_eq!(parsed["fields"]["seconds"], 300);
    }
}
