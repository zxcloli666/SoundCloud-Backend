use sc_mobile::{MobileError, MobileReply};

const CHALLENGE_HOST: &str = "captcha-delivery.com";

pub const NOTE: &str = "datadome";

pub fn is_datadome(outcome: &Result<MobileReply, MobileError>) -> bool {
    matches!(
        outcome,
        Ok(reply) if reply.status == 403
            && String::from_utf8_lossy(&reply.body).contains(CHALLENGE_HOST)
    )
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    fn reply(status: u16, body: &'static str) -> Result<MobileReply, MobileError> {
        Ok(MobileReply {
            status,
            plan: None,
            location: None,
            body: Bytes::from_static(body.as_bytes()),
        })
    }

    #[test]
    fn a_403_with_the_challenge_link_is_datadome() {
        let page = r#"{"url":"https://geo.captcha-delivery.com/interstitial/?cid=1"}"#;

        assert!(is_datadome(&reply(403, page)));
        assert!(!is_datadome(&reply(403, r#"{"error":"forbidden"}"#)));
        assert!(!is_datadome(&reply(200, page)));
        assert!(!is_datadome(&Err(MobileError::Random)));
    }
}
