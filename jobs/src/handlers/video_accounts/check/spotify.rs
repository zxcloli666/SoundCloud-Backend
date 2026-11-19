use serde::Deserialize;

use super::{BROWSER_AGENT, Fetch, Verdict, headers, totp, unavailable};

const SERVER_TIME: &str = "https://open.spotify.com/api/server-time";
const TOKEN: &str = "https://open.spotify.com/api/token";

pub struct SpotifyKey {
    pub(super) version: u32,
    pub(super) secret: Vec<u8>,
}

impl SpotifyKey {
    pub fn new(version: u32, cipher: &[u8]) -> Self {
        let digits: String = cipher
            .iter()
            .enumerate()
            .map(|(index, byte)| (byte ^ ((index % 33) as u8 + 9)).to_string())
            .collect();
        Self {
            version,
            secret: digits.into_bytes(),
        }
    }
}

#[derive(Deserialize)]
struct ServerTime {
    #[serde(rename = "serverTime")]
    seconds: u64,
}

#[derive(Deserialize)]
struct Token {
    #[serde(rename = "accessToken", default)]
    access_token: String,
    #[serde(rename = "isAnonymous", default)]
    anonymous: bool,
    #[serde(rename = "accessTokenExpirationTimestampMs", default)]
    expires_ms: u64,
}

pub(crate) struct Issued {
    pub(crate) token: String,
    pub(crate) expires_ms: u64,
}

pub(super) async fn check<F: Fetch>(fetch: &F, key: &SpotifyKey, sp_dc: &str) -> Verdict {
    match access_token(fetch, key, sp_dc).await {
        Ok(_) => Verdict::Valid,
        Err(verdict) => verdict,
    }
}

pub(crate) async fn access_token<F: Fetch>(
    fetch: &F,
    key: &SpotifyKey,
    sp_dc: &str,
) -> Result<Issued, Verdict> {
    if sp_dc.is_empty() {
        return Err(Verdict::Rejected("sp_dc is empty".to_owned()));
    }
    let clock = fetch_json::<ServerTime>(fetch, "Spotify server-time", SERVER_TIME, "").await?;
    let Some(code) = totp::code(&key.secret, clock.seconds) else {
        return Err(Verdict::Unavailable(
            "the Spotify TOTP key is unusable".to_owned(),
        ));
    };
    let url = format!(
        "{TOKEN}?reason=init&productType=web-player&totp={code}&totpServer={code}&totpVer={}",
        key.version
    );
    let cookie = format!("sp_dc={sp_dc}");
    let token = fetch_json::<Token>(fetch, "Spotify token", &url, &cookie).await?;
    if token.access_token.is_empty() {
        return Err(Verdict::Unavailable(
            "Spotify returned no access token".to_owned(),
        ));
    }
    if token.anonymous {
        return Err(Verdict::Rejected(
            "Spotify treats sp_dc as signed out".to_owned(),
        ));
    }
    Ok(Issued {
        token: token.access_token,
        expires_ms: token.expires_ms,
    })
}

async fn fetch_json<T: serde::de::DeserializeOwned>(
    fetch: &impl Fetch,
    step: &str,
    url: &str,
    cookie: &str,
) -> Result<T, Verdict> {
    let mut pairs = vec![
        ("user-agent", BROWSER_AGENT),
        ("accept", "application/json"),
        ("app-platform", "WebPlayer"),
    ];
    if !cookie.is_empty() {
        pairs.push(("cookie", cookie));
    }
    let request = headers(&pairs)?;
    let body = fetch
        .get(url, request)
        .await
        .map_err(|error| unavailable(step, &error))?;
    serde_json::from_slice(&body)
        .map_err(|_| Verdict::Unavailable(format!("{step} answered with an unexpected body")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_the_decimal_text_of_the_unmasked_cipher() {
        let key = SpotifyKey::new(61, &[9 ^ 40, 10 ^ 34]);

        assert_eq!(key.secret, b"4034");
    }
}
