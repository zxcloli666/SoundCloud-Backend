pub fn playlist_secret(permalink: &str) -> Option<String> {
    match segments(permalink)?.as_slice() {
        [_, "sets", _, secret] => valid(secret),
        _ => None,
    }
}

pub fn track_secret(permalink: &str) -> Option<String> {
    match segments(permalink)?.as_slice() {
        [_, slug, secret] if *slug != "sets" => valid(secret),
        _ => None,
    }
}

fn segments(permalink: &str) -> Option<Vec<&str>> {
    let path = permalink.split(['?', '#']).next()?;
    let path = path.split_once("soundcloud.com/")?.1;
    Some(path.split('/').filter(|part| !part.is_empty()).collect())
}

fn valid(secret: &str) -> Option<String> {
    (secret.len() > 2
        && secret.starts_with("s-")
        && secret
            .chars()
            .all(|symbol| symbol.is_ascii_alphanumeric() || symbol == '-'))
    .then(|| secret.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playlist_secret_is_the_segment_after_its_slug() {
        assert_eq!(
            playlist_secret("https://soundcloud.com/user/sets/mix/s-Ab12Cd?si=x"),
            Some("s-Ab12Cd".to_owned())
        );
        assert_eq!(
            playlist_secret("https://soundcloud.com/user/sets/mix/s-Ab12Cd/"),
            Some("s-Ab12Cd".to_owned())
        );
        assert_eq!(
            playlist_secret("https://soundcloud.com/user/sets/s-mix"),
            None
        );
        assert_eq!(
            playlist_secret("https://soundcloud.com/user/sets/mix"),
            None
        );
        assert_eq!(
            playlist_secret("https://soundcloud.com/s-user/sets/mix"),
            None
        );
    }

    #[test]
    fn a_track_secret_is_the_segment_after_its_slug() {
        assert_eq!(
            track_secret("https://soundcloud.com/user/song/s-Ab12Cd?utm_medium=api"),
            Some("s-Ab12Cd".to_owned())
        );
        assert_eq!(track_secret("https://soundcloud.com/user/s-song"), None);
        assert_eq!(track_secret("https://soundcloud.com/user/sets/s-mix"), None);
        assert_eq!(track_secret("https://soundcloud.com/user/song"), None);
    }
}
