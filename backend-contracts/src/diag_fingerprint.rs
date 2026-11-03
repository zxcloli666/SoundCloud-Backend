use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::diag::ErrorKind;

const FRAMES: usize = 3;

pub fn fingerprint(
    kind: ErrorKind,
    message: &str,
    stack: Option<&str>,
    context: &BTreeMap<String, String>,
) -> String {
    let context = |key: &str| context.get(key).map_or("", String::as_str);
    let basis = match kind {
        ErrorKind::Request => format!(
            "request|{}|{}|{}",
            context("service"),
            numbered(context("path")),
            context("class")
        ),
        ErrorKind::Playback => format!("playback|{}|{}", context("reason"), context("quality")),
        kind => format!(
            "{}|{}|{}",
            kind.as_str(),
            numbered(message),
            top_frames(stack.unwrap_or(""))
        ),
    };
    hex::encode(&Sha256::digest(basis.as_bytes())[..8])
}

fn numbered(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_digits = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            if !in_digits {
                out.push('#');
            }
            in_digits = true;
        } else {
            in_digits = false;
            out.push(ch);
        }
    }
    out
}

fn top_frames(stack: &str) -> String {
    stack
        .lines()
        .filter(|line| line.contains(".js") || line.trim_start().starts_with("at "))
        .take(FRAMES)
        .map(|line| unhashed(line.trim()))
        .collect::<Vec<_>>()
        .join("|")
}

fn unhashed(frame: &str) -> String {
    let mut out = String::with_capacity(frame.len());
    let mut rest = frame;
    while let Some(at) = rest.find(".js") {
        let before = &rest[..at];
        let name_start = before
            .rfind(['/', '(', ' ', '@'])
            .map_or(0, |index| index + 1);
        let name = &before[name_start..];
        let stem = name.rsplit_once('-').map_or(name, |(stem, _)| stem);
        out.push_str(&before[..name_start]);
        out.push_str(stem);
        out.push_str(".js");
        rest = &rest[at + 3..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHROME: &str = "TypeError: Cannot read properties of undefined (reading 'title')\n    at Object.render (http://tauri.localhost/assets/index-AbC123xy.js:10:2045)\n    at Zo (https://tauri.localhost/assets/vendor-Zz9Q.js:1:2)\n    at tauri://localhost/assets/Library-k3Jd9.js:77:12\n    at deep (x.js:1:1)";
    const WEBKIT: &str = "render@tauri://localhost/assets/index-AbC123xy.js:10:20\ngo@tauri://localhost/assets/player-Q1w2e3.js:5:6\nnot a frame\n@tauri://localhost/assets/x-1.js:3:4\nlast@tauri://localhost/assets/y-2.js:3:4";

    fn context(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn the_values_match_what_desktop_computes_today() {
        let none = BTreeMap::new();
        let cases = [
            (
                fingerprint(
                    ErrorKind::Render,
                    "Cannot read properties of undefined (reading 'title')",
                    Some(CHROME),
                    &none,
                ),
                "96e8f0ff9c0b4522",
            ),
            (
                fingerprint(
                    ErrorKind::Render,
                    "Track 12345 failed after 30 tries",
                    Some(CHROME),
                    &none,
                ),
                "b405b01a295e0226",
            ),
            (
                fingerprint(
                    ErrorKind::Unhandled,
                    "ResizeObserver 2 loop",
                    Some(WEBKIT),
                    &none,
                ),
                "0dd256efdeb5ed1b",
            ),
            (
                fingerprint(ErrorKind::Rejection, "Network error", None, &none),
                "95b9a0be5ec22dc7",
            ),
            (
                fingerprint(
                    ErrorKind::Panic,
                    "thread 'main' panicked at src/audio.rs:123:9",
                    None,
                    &context(&[("x", "y")]),
                ),
                "a58430ab9d1b8ba0",
            ),
            (
                fingerprint(
                    ErrorKind::Request,
                    "whatever",
                    None,
                    &context(&[
                        ("service", "api"),
                        ("path", "/tracks/123/stream?quality=hq"),
                        ("class", "EdgeUnreachableError"),
                    ]),
                ),
                "f7353d7ad26f46a2",
            ),
            (
                fingerprint(
                    ErrorKind::Playback,
                    "a message",
                    None,
                    &context(&[("reason", "no_stream"), ("quality", "hq")]),
                ),
                "76987a02f6e21b7d",
            ),
            (
                fingerprint(ErrorKind::Request, "x", None, &none),
                "4f71db1efa5217ba",
            ),
            (
                fingerprint(
                    ErrorKind::Render,
                    "Ошибка 42 трек",
                    Some("at a (/assets/ü-1.js:1:1)"),
                    &none,
                ),
                "303759e825d4316b",
            ),
        ];

        for (got, want) in cases {
            assert_eq!(got, want);
        }
    }

    #[test]
    fn digits_asset_hashes_and_deep_frames_do_not_change_it() {
        let none = BTreeMap::new();
        let rebuilt = CHROME
            .replace("AbC123xy", "QqQ999zz")
            .replace("x.js:1:1", "y.js:9:9");

        assert_eq!(
            fingerprint(ErrorKind::Render, "track 1 failed", Some(CHROME), &none),
            fingerprint(ErrorKind::Render, "track 987 failed", Some(&rebuilt), &none)
        );
        assert_ne!(
            fingerprint(ErrorKind::Render, "track 1 failed", Some(CHROME), &none),
            fingerprint(ErrorKind::Unhandled, "track 1 failed", Some(CHROME), &none)
        );
    }
}
