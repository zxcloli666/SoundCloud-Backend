use serde_json::json;
use url::Url;

use crate::request::MobileRequest;

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

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::request::Kind;

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
