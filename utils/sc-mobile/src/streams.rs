use serde_json::{Value, json};
use url::Url;

use crate::client::MobileReply;
use crate::request::MobileRequest;

const PREVIEW_SEGMENT: &str = "preview";

pub const TRANSCODINGS_QUERY: &str = "query TrackTranscodings($urns: [ID!]!) {
  tracks(urns: $urns) {
    urn
    fullDuration
    snipDuration
    authorization {
      monetizable
      snipped
      syncable
      subMidTier
      subHighTier
      policy
      blocked
      monetizationModel
    }
    transcodings {
      url
      uuid
      preset
      durationMs
      format {
        protocol
        mimeType
      }
      quality
      snipped
      type
    }
  }
}";

pub fn hls(track_urn: &str) -> MobileRequest {
    MobileRequest::get(&["tracks", track_urn, "streams", "hls"])
        .with_query("can_snip", "false")
        .with_query("secure", "true")
}

pub fn https(track_urn: &str) -> MobileRequest {
    MobileRequest::get(&["tracks", track_urn, "streams", "https"])
}

pub fn transcodings(track_urns: &[&str]) -> MobileRequest {
    MobileRequest::query(TRANSCODINGS_QUERY, json!({ "urns": track_urns }))
}

pub fn media(url: Url) -> MobileRequest {
    MobileRequest::get_url(url)
}

pub fn stream_url(reply: &MobileReply) -> Option<Url> {
    let url = if reply.is_redirect() {
        reply.location.clone()
    } else if reply.is_success() {
        serde_json::from_slice::<Value>(&reply.body)
            .ok()?
            .get("url")
            .and_then(Value::as_str)
            .and_then(|url| Url::parse(url).ok())
    } else {
        None
    }?;
    let preview = url
        .path_segments()
        .is_some_and(|mut segments| segments.any(|segment| segment == PREVIEW_SEGMENT));
    (url.scheme() == "https" && !preview).then_some(url)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::request::Kind;

    fn reply(status: u16, location: Option<&str>, body: &'static [u8]) -> MobileReply {
        MobileReply {
            status,
            plan: None,
            location: location.and_then(|url| Url::parse(url).ok()),
            body: Bytes::from_static(body),
        }
    }

    #[test]
    fn a_302_to_the_cdn_is_the_stream_url() {
        let hls = reply(
            302,
            Some(
                "https://cf-hls-media.sndcdn.com/playlist/a.128.mp3/playlist.m3u8?Policy=p&Signature=s",
            ),
            b"",
        );

        assert_eq!(
            stream_url(&hls).as_ref().and_then(Url::host_str),
            Some("cf-hls-media.sndcdn.com")
        );
    }

    #[test]
    fn a_json_url_also_resolves_and_a_refusal_or_preview_does_not() {
        let json = reply(200, None, br#"{"url":"https://cf-media.sndcdn.com/a.mp3"}"#);
        let missing = reply(404, None, b"{}");
        let preview = reply(
            302,
            Some("https://cf-preview-media.sndcdn.com/preview/0/30/a.128.mp3"),
            b"",
        );
        let plain = reply(302, Some("http://cf-media.sndcdn.com/a.mp3"), b"");

        assert!(stream_url(&json).is_some());
        assert!(stream_url(&missing).is_none());
        assert!(stream_url(&preview).is_none());
        assert!(stream_url(&plain).is_none());
    }

    #[test]
    fn the_hls_stream_asks_for_the_full_secure_stream() {
        let url = hls("soundcloud:tracks:1").url().ok().map(String::from);

        assert_eq!(
            url.as_deref(),
            Some(
                "https://api-mobile.soundcloud.com/tracks/soundcloud:tracks:1/streams/hls?can_snip=false&secure=true"
            )
        );
    }

    #[test]
    fn transcodings_are_a_retryable_query_by_urn() {
        let request = transcodings(&["soundcloud:tracks:1"]);
        let body: Option<Value> = request
            .body()
            .and_then(|body| serde_json::from_slice(body).ok());

        assert_eq!(request.kind(), Kind::Query);
        assert_eq!(
            body.as_ref()
                .and_then(|body| body.pointer("/variables/urns/0")),
            Some(&json!("soundcloud:tracks:1"))
        );
        assert!(TRANSCODINGS_QUERY.starts_with("query TrackTranscodings("));
    }
}
