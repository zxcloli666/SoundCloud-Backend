use crate::client::MobileReply;

pub(crate) const CAPTCHA: &str = "captcha";

const CAPTCHA_HOST: &[u8] = b"captcha-delivery.com";
const CAPTCHA_STATUS: u16 = 403;

impl MobileReply {
    pub fn is_captcha(&self) -> bool {
        self.status == CAPTCHA_STATUS
            && self
                .body
                .windows(CAPTCHA_HOST.len())
                .any(|window| window == CAPTCHA_HOST)
    }

    pub fn is_redirect(&self) -> bool {
        (300..400).contains(&self.status) && self.location.is_some()
    }

    pub fn status_class(&self) -> &'static str {
        if self.is_captcha() {
            return CAPTCHA;
        }
        class_of(self.status)
    }
}

pub(crate) fn class_of(status: u16) -> &'static str {
    match status {
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use url::Url;

    use super::*;

    fn reply(status: u16, location: Option<&str>, body: &'static [u8]) -> MobileReply {
        MobileReply {
            status,
            plan: None,
            location: location.and_then(|url| Url::parse(url).ok()),
            body: Bytes::from_static(body),
        }
    }

    #[test]
    fn a_datadome_interstitial_is_a_captcha_and_a_plain_403_is_not() {
        let challenged = reply(
            403,
            None,
            br#"{"url":"https://geo.captcha-delivery.com/interstitial/?initialCid=x"}"#,
        );
        let refused = reply(403, None, br#"{"error":"forbidden"}"#);

        assert!(challenged.is_captcha());
        assert_eq!(challenged.status_class(), "captcha");
        assert!(!refused.is_captcha());
        assert_eq!(refused.status_class(), "4xx");
    }

    #[test]
    fn a_redirect_needs_a_location() {
        assert!(reply(302, Some("https://cf-media.sndcdn.com/a.mp3"), b"").is_redirect());
        assert!(!reply(302, None, b"").is_redirect());
        assert!(!reply(200, Some("https://cf-media.sndcdn.com/a.mp3"), b"").is_redirect());
        assert_eq!(reply(302, None, b"").status_class(), "3xx");
        assert_eq!(reply(503, None, b"").status_class(), "5xx");
    }
}
