const MAX_IMAGE_CHARS: usize = 512;
const IMAGE_HOST: &str = "sndcdn.com";

pub fn artwork_url(raw: &str) -> Option<String> {
    if raw.len() > MAX_IMAGE_CHARS {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    let host = url.host_str()?;
    let trusted = host == IMAGE_HOST
        || host
            .strip_suffix(IMAGE_HOST)
            .is_some_and(|rest| rest.ends_with('.'));
    (url.scheme() == "https" && trusted).then(|| url.to_string())
}
