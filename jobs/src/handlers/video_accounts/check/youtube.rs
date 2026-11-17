use super::{BROWSER_AGENT, Fetch, Verdict, headers, unavailable};

const HOME: &str = "https://www.youtube.com/";
const SESSION_COOKIES: [&str; 3] = ["__Secure-3PSID", "__Secure-1PSID", "SID"];

pub(super) async fn check<F: Fetch>(fetch: &F, cookies_txt: &str) -> Verdict {
    let Some(cookie) = cookie_header(cookies_txt) else {
        return Verdict::Rejected("the file has no Google session cookies".to_owned());
    };
    let request = match headers(&[
        ("cookie", &cookie),
        ("user-agent", BROWSER_AGENT),
        ("accept-language", "en-US,en;q=0.9"),
    ]) {
        Ok(request) => request,
        Err(verdict) => return verdict,
    };
    let page = match fetch.get(HOME, request).await {
        Ok(page) => page,
        Err(error) => return unavailable("YouTube", &error),
    };
    let page = String::from_utf8_lossy(&page);
    if page.contains("\"LOGGED_IN\":true") {
        Verdict::Valid
    } else if page.contains("\"LOGGED_IN\":false") {
        Verdict::Rejected("YouTube shows the account as signed out".to_owned())
    } else {
        Verdict::Unavailable("the YouTube page has no sign-in marker".to_owned())
    }
}

fn cookie_header(netscape: &str) -> Option<String> {
    let mut pairs = Vec::new();
    for line in netscape.lines() {
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let [domain, _, _, _, _, name, value, ..] = fields.as_slice() else {
            continue;
        };
        let domain = domain.trim_start_matches('.');
        if domain.ends_with("youtube.com") || domain.ends_with("google.com") {
            pairs.push((*name, *value));
        }
    }
    let signed_in = pairs.iter().any(|(name, _)| SESSION_COOKIES.contains(name));
    signed_in.then(|| {
        pairs
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    })
}
