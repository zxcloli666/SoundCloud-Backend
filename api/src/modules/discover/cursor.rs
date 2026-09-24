use base64::Engine;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistCursor {
    pub p: f64,
    #[serde(default)]
    pub p2: f64,
    pub n: String,
    pub id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumCursor {
    pub p: f64,
    #[serde(default)]
    pub p2: f64,
    pub n: String,
    pub id: Uuid,
}

pub fn encode<T: Serialize>(c: &T) -> String {
    let json = serde_json::to_vec(c).expect("cursor serialization");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
}

pub fn decode<T: for<'de> Deserialize<'de>>(s: &str) -> AppResult<T> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .map_err(|_| AppError::bad_request("invalid cursor"))?;
    serde_json::from_slice(&bytes).map_err(|_| AppError::bad_request("invalid cursor"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    fn cursor() -> ArtistCursor {
        ArtistCursor {
            p: 12.5,
            p2: -3.25,
            n: "Boards of Canada".to_owned(),
            id: Uuid::from_u128(42),
        }
    }

    fn refused(raw: &str) -> axum::http::StatusCode {
        match decode::<ArtistCursor>(raw) {
            Ok(_) => panic!("this cursor must be refused: {raw}"),
            Err(error) => error.into_response().status(),
        }
    }

    #[test]
    fn a_cursor_survives_the_round_trip_unchanged() {
        let back: ArtistCursor = decode(&encode(&cursor())).expect("our own cursor decodes");

        assert_eq!(back.p, 12.5);
        assert_eq!(back.p2, -3.25);
        assert_eq!(back.n, "Boards of Canada");
        assert_eq!(back.id, Uuid::from_u128(42));
    }

    #[test]
    fn a_cursor_carries_no_padding_and_survives_a_url() {
        let encoded = encode(&cursor());

        assert!(
            !encoded.contains('='),
            "padding would need escaping in a URL"
        );
        assert!(
            encoded
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "a cursor must be usable as a query value as is: {encoded}"
        );
    }

    #[test]
    fn a_cursor_from_before_the_second_key_still_decodes() {
        let old = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"p":1.5,"n":"Aphex Twin","id":"00000000-0000-0000-0000-00000000002a"}"#);

        let back: ArtistCursor = decode(&old).expect("a cursor issued before p2 must keep working");

        assert_eq!(back.p, 1.5);
        assert_eq!(back.p2, 0.0);
    }

    #[test]
    fn anything_a_client_invents_is_a_bad_request_and_not_a_crash() {
        for raw in [
            "not base64 at all !!",
            "",
            &base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"not json"),
            &base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"p":1.0}"#),
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(br#"{"p":1.0,"n":"x","id":"not-a-uuid"}"#),
            &format!("{}=", encode(&cursor())),
        ] {
            assert_eq!(
                refused(raw),
                axum::http::StatusCode::BAD_REQUEST,
                "a cursor the client made up must be a 400: {raw}"
            );
        }
    }
}
