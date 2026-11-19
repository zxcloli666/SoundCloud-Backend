use std::path::{Path, PathBuf};

use sc_mobile::MobileApp;
use url::Url;

pub const USAGE: &str = "usage: mobile_probe --profile <file> --out <dir> \
[--track <urn>] [--goplus-track <urn>] [--reaction-track <urn>] [--comment-track <urn>] \
[--comment-via apiv1|mobile] [--delete-comment <comment urn>] [--actions-track <urn>] [--comment-urn <urn>] \
[--follow-user <urn>] [--playlist-probe] [--proxy <url>]";

const TRACK_PREFIX: &str = "soundcloud:tracks:";
const USER_PREFIX: &str = "soundcloud:users:";
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
    pub actions_track: Option<String>,
    pub comment_urn: Option<String>,
    pub follow_user: Option<String>,
    pub playlist_probe: bool,
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
    let mut actions_track = None;
    let mut comment_urn_arg = None;
    let mut follow_user = None;
    let mut playlist_probe = false;
    let mut proxy = None;
    let mut rest = arguments.iter();
    while let Some(flag) = rest.next() {
        if flag == "--playlist-probe" {
            if playlist_probe {
                return Err(format!("{flag} is given twice"));
            }
            playlist_probe = true;
            continue;
        }
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
            "--actions-track" => actions_track.replace(track_urn(flag, value)?).is_some(),
            "--comment-urn" => comment_urn_arg.replace(comment_urn(flag, value)?).is_some(),
            "--follow-user" => follow_user.replace(user_urn(flag, value)?).is_some(),
            "--proxy" => proxy
                .replace(Url::parse(value).map_err(|_| "--proxy is not a URL".to_owned())?)
                .is_some(),
            unknown => return Err(format!("unknown flag {unknown}")),
        };
        if slot_taken {
            return Err(format!("{flag} is given twice"));
        }
    }
    if comment_urn_arg.is_some() && actions_track.is_none() {
        return Err("--comment-urn needs --actions-track as the parent track".to_owned());
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
        actions_track,
        comment_urn: comment_urn_arg,
        follow_user,
        playlist_probe,
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

fn user_urn(flag: &str, value: &str) -> Result<String, String> {
    let id = value.strip_prefix(USER_PREFIX).unwrap_or_default();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("{flag} needs a user urn like soundcloud:users:123"));
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
    fn the_action_flags_fill_their_slots() {
        let Ok(args) = parse(&strings(&[
            "--profile",
            "p",
            "--out",
            "/out",
            "--actions-track",
            "soundcloud:tracks:1",
            "--comment-urn",
            "soundcloud:comments:2",
            "--follow-user",
            "soundcloud:users:3",
            "--playlist-probe",
        ])) else {
            panic!("valid flags are refused");
        };

        assert_eq!(args.actions_track.as_deref(), Some("soundcloud:tracks:1"));
        assert_eq!(args.comment_urn.as_deref(), Some("soundcloud:comments:2"));
        assert_eq!(args.follow_user.as_deref(), Some("soundcloud:users:3"));
        assert!(args.playlist_probe);

        let Ok(bare) = parse(&strings(&["--profile", "p", "--out", "/out"])) else {
            panic!("bare flags are refused");
        };
        assert!(!bare.playlist_probe);
    }

    #[test]
    fn bad_action_flags_are_refused() {
        for extra in [
            &["--follow-user", "soundcloud:users:x"][..],
            &["--follow-user", "soundcloud:tracks:1"],
            &["--actions-track", "5"],
            &["--comment-urn", "soundcloud:comments:2"],
            &["--playlist-probe", "--playlist-probe"],
        ] {
            let mut arguments = strings(&["--profile", "p", "--out", "/out"]);
            arguments.extend(strings(extra));
            assert!(parse(&arguments).is_err(), "{extra:?}");
        }
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
