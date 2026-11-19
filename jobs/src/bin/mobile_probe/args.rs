use std::path::{Path, PathBuf};

use sc_mobile::MobileApp;
use url::Url;

pub const USAGE: &str = "usage: mobile_probe --profile <file> --out <dir> \
[--track <urn>] [--goplus-track <urn>] [--reaction-track <urn>] [--comment-track <urn>] \
[--comment-via apiv1|mobile] [--delete-comment <comment urn>] [--proxy <url>]";

const TRACK_PREFIX: &str = "soundcloud:tracks:";
const COMMENT_PREFIX: &str = "soundcloud:comments:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentVia {
    Mobile,
    Apiv1,
}

pub struct Args {
    pub profile: PathBuf,
    pub out: PathBuf,
    pub track: Option<String>,
    pub goplus_track: Option<String>,
    pub reaction_track: Option<String>,
    pub comment_track: Option<String>,
    pub comment_via: CommentVia,
    pub delete_comment: Option<String>,
    pub proxy: Option<Url>,
}

pub fn parse(arguments: &[String]) -> Result<Args, String> {
    let mut profile = None;
    let mut out = None;
    let mut track = None;
    let mut goplus_track = None;
    let mut reaction_track = None;
    let mut comment_track = None;
    let mut comment_via = None;
    let mut delete_comment = None;
    let mut proxy = None;
    let mut rest = arguments.iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let slot_taken = match flag.as_str() {
            "--profile" => profile.replace(PathBuf::from(value)).is_some(),
            "--out" => out.replace(PathBuf::from(value)).is_some(),
            "--track" => track.replace(track_urn(flag, value)?).is_some(),
            "--goplus-track" => goplus_track.replace(track_urn(flag, value)?).is_some(),
            "--reaction-track" => reaction_track.replace(track_urn(flag, value)?).is_some(),
            "--comment-track" => comment_track.replace(track_urn(flag, value)?).is_some(),
            "--comment-via" => comment_via.replace(via(value)?).is_some(),
            "--delete-comment" => delete_comment.replace(comment_urn(flag, value)?).is_some(),
            "--proxy" => proxy
                .replace(Url::parse(value).map_err(|_| "--proxy is not a URL".to_owned())?)
                .is_some(),
            unknown => return Err(format!("unknown flag {unknown}")),
        };
        if slot_taken {
            return Err(format!("{flag} is given twice"));
        }
    }
    Ok(Args {
        profile: profile.ok_or("--profile is required")?,
        out: out.ok_or("--out is required")?,
        track,
        goplus_track,
        reaction_track,
        comment_track,
        comment_via: comment_via.unwrap_or(CommentVia::Mobile),
        delete_comment,
        proxy,
    })
}

pub fn load_profile(path: &Path) -> Result<MobileApp, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("the profile {} cannot be read: {error}", path.display()))?;
    let app: MobileApp = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "the profile is not a JSON object with client_id, client_secret, app_id, app_version \
             and user_agent strings (line {}, column {})",
            error.line(),
            error.column()
        )
    })?;
    let filled = [
        app.client_id.as_str(),
        app.client_secret.expose(),
        app.app_id.as_str(),
        app.app_version.as_str(),
        app.user_agent.as_str(),
    ]
    .iter()
    .all(|value| !value.trim().is_empty());
    if !filled {
        return Err("the profile has an empty field".to_owned());
    }
    Ok(app)
}

fn via(value: &str) -> Result<CommentVia, String> {
    match value {
        "mobile" => Ok(CommentVia::Mobile),
        "apiv1" => Ok(CommentVia::Apiv1),
        _ => Err("--comment-via is apiv1 or mobile".to_owned()),
    }
}

fn track_urn(flag: &str, value: &str) -> Result<String, String> {
    let id = value.strip_prefix(TRACK_PREFIX).unwrap_or_default();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "{flag} needs a track urn like soundcloud:tracks:123"
        ));
    }
    Ok(value.to_owned())
}

fn comment_urn(flag: &str, value: &str) -> Result<String, String> {
    let id = value.strip_prefix(COMMENT_PREFIX).unwrap_or_default();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "{flag} needs a comment urn like soundcloud:comments:123"
        ));
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn the_flags_fill_their_slots() {
        let Ok(args) = parse(&strings(&[
            "--profile",
            "/run/profile.json",
            "--out",
            "/out",
            "--track",
            "soundcloud:tracks:1",
            "--proxy",
            "http://proxy:8080",
        ])) else {
            panic!("valid flags are refused");
        };

        assert_eq!(args.profile, PathBuf::from("/run/profile.json"));
        assert_eq!(args.track.as_deref(), Some("soundcloud:tracks:1"));
        assert_eq!(args.goplus_track, None);
        assert_eq!(args.comment_via, CommentVia::Mobile);
        assert_eq!(args.delete_comment, None);
        assert!(args.proxy.is_some());
    }

    #[test]
    fn the_comment_flags_fill_their_slots() {
        let Ok(args) = parse(&strings(&[
            "--profile",
            "p",
            "--out",
            "/out",
            "--comment-track",
            "soundcloud:tracks:1",
            "--comment-via",
            "apiv1",
            "--delete-comment",
            "soundcloud:comments:42",
        ])) else {
            panic!("valid flags are refused");
        };

        assert_eq!(args.comment_via, CommentVia::Apiv1);
        assert_eq!(
            args.delete_comment.as_deref(),
            Some("soundcloud:comments:42")
        );
    }

    #[test]
    fn bad_flags_are_refused() {
        for arguments in [
            strings(&["--out", "/out"]),
            strings(&["--profile", "p", "--out", "/out", "--track", "123"]),
            strings(&["--profile", "p", "--out", "/out", "--track"]),
            strings(&["--profile", "p", "--profile", "q", "--out", "/out"]),
            strings(&["--profile", "p", "--out", "/out", "--token", "x"]),
            strings(&["--profile", "p", "--out", "/out", "--comment-via", "web"]),
            strings(&["--profile", "p", "--out", "/out", "--delete-comment", "42"]),
            strings(&[
                "--profile",
                "p",
                "--out",
                "/out",
                "--delete-comment",
                "soundcloud:tracks:42",
            ]),
            strings(&[
                "--profile",
                "p",
                "--out",
                "/out",
                "--delete-comment",
                "soundcloud:comments:4x",
            ]),
            strings(&[
                "--profile",
                "p",
                "--out",
                "/out",
                "--comment-via",
                "apiv1",
                "--comment-via",
                "mobile",
            ]),
        ] {
            assert!(parse(&arguments).is_err(), "{arguments:?}");
        }
    }
}
