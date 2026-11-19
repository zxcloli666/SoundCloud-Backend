use std::fmt;

use bytes::Bytes;
use serde_json::{Value, json};
use url::Url;
use wreq::Method;

use crate::auth::TOKEN_URL;
use crate::error::MobileError;

pub const API_BASE: &str = "https://api-mobile.soundcloud.com";
pub const GRAPH_URL: &str = "https://graph.soundcloud.com/graphql";

const MOBILE_HOSTS: &[&str] = &["api-mobile.soundcloud.com", "graph.soundcloud.com"];
pub(crate) const JSON: &str = "application/json; charset=utf-8";
const FORM: &str = "application/x-www-form-urlencoded";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Read,
    Write,
    Query,
    Mutation,
    Token,
}

#[derive(Clone)]
enum Target {
    Api(Vec<String>),
    Graph,
    Token,
    Absolute(Url),
}

#[derive(Clone)]
pub(crate) struct Body {
    pub(crate) content_type: &'static str,
    pub(crate) bytes: Bytes,
}

#[derive(Clone)]
pub struct MobileRequest {
    method: Method,
    kind: Kind,
    target: Target,
    query: Vec<(String, String)>,
    body: Option<Body>,
}

impl MobileRequest {
    pub fn get(segments: &[&str]) -> Self {
        Self::api(Method::GET, Kind::Read, segments, None)
    }

    pub fn post(segments: &[&str], body: &Value) -> Self {
        Self::api(Method::POST, Kind::Write, segments, Some(json_body(body)))
    }

    pub fn put(segments: &[&str], body: &Value) -> Self {
        Self::api(Method::PUT, Kind::Write, segments, Some(json_body(body)))
    }

    pub fn delete(segments: &[&str]) -> Self {
        Self::api(Method::DELETE, Kind::Write, segments, None)
    }

    pub fn get_url(url: Url) -> Self {
        Self {
            method: Method::GET,
            kind: Kind::Read,
            target: Target::Absolute(url),
            query: Vec::new(),
            body: None,
        }
    }

    pub fn query(document: &str, variables: Value) -> Self {
        Self::graph(Kind::Query, document, variables)
    }

    pub fn mutation(document: &str, variables: Value) -> Self {
        Self::graph(Kind::Mutation, document, variables)
    }

    pub(crate) fn token(form: String) -> Self {
        Self {
            method: Method::POST,
            kind: Kind::Token,
            target: Target::Token,
            query: Vec::new(),
            body: Some(Body {
                content_type: FORM,
                bytes: Bytes::from(form),
            }),
        }
    }

    pub fn with_query(mut self, name: &str, value: &str) -> Self {
        self.query.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn method(&self) -> &Method {
        &self.method
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn body(&self) -> Option<&[u8]> {
        match self.kind {
            Kind::Token => None,
            _ => self.body.as_ref().map(|body| body.bytes.as_ref()),
        }
    }

    pub(crate) fn payload(&self) -> Option<&Body> {
        self.body.as_ref()
    }

    pub fn url(&self) -> Result<Url, MobileError> {
        let mut url = match &self.target {
            Target::Api(segments) => api_url(segments)?,
            Target::Graph => parse(GRAPH_URL)?,
            Target::Token => parse(TOKEN_URL)?,
            Target::Absolute(url) => mobile_url(url)?,
        };
        if !self.query.is_empty() {
            url.query_pairs_mut().extend_pairs(&self.query);
        }
        Ok(url)
    }

    fn api(method: Method, kind: Kind, segments: &[&str], body: Option<Body>) -> Self {
        Self {
            method,
            kind,
            target: Target::Api(
                segments
                    .iter()
                    .map(|segment| (*segment).to_owned())
                    .collect(),
            ),
            query: Vec::new(),
            body,
        }
    }

    fn graph(kind: Kind, document: &str, variables: Value) -> Self {
        Self {
            method: Method::POST,
            kind,
            target: Target::Graph,
            query: Vec::new(),
            body: Some(json_body(
                &json!({ "query": document, "variables": variables }),
            )),
        }
    }
}

impl fmt::Debug for MobileRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MobileRequest")
            .field("method", &self.method)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

fn json_body(value: &Value) -> Body {
    Body {
        content_type: JSON,
        bytes: Bytes::from(value.to_string()),
    }
}

fn api_url(segments: &[String]) -> Result<Url, MobileError> {
    let mut url = parse(API_BASE)?;
    url.path_segments_mut()
        .map_err(|()| MobileError::Invalid("the mobile API base cannot take a path"))?
        .clear()
        .extend(segments);
    Ok(url)
}

fn mobile_url(url: &Url) -> Result<Url, MobileError> {
    let mobile = url.scheme() == "https"
        && url.port().is_none()
        && url
            .host_str()
            .is_some_and(|host| MOBILE_HOSTS.contains(&host));
    if mobile {
        Ok(url.clone())
    } else {
        Err(MobileError::ForeignHost)
    }
}

fn parse(url: &str) -> Result<Url, MobileError> {
    Url::parse(url).map_err(|_| MobileError::Invalid("a built-in SoundCloud URL does not parse"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_urn_stays_one_path_segment() {
        let url = MobileRequest::get(&["tracks", "soundcloud:tracks:1/../me", "streams"])
            .with_query("secure", "true")
            .url();

        assert_eq!(
            url.ok().map(String::from).as_deref(),
            Some(
                "https://api-mobile.soundcloud.com/tracks/soundcloud:tracks:1%2F..%2Fme/streams?secure=true"
            )
        );
    }

    #[test]
    fn graphql_sends_the_document_and_its_variables() {
        let request = MobileRequest::query("query Me { me { urn } }", json!({ "a": 1 }));
        let body: Option<Value> = request
            .body()
            .and_then(|body| serde_json::from_slice(body).ok());

        assert_eq!(request.kind(), Kind::Query);
        assert_eq!(request.method(), &Method::POST);
        assert_eq!(
            body,
            Some(json!({ "query": "query Me { me { urn } }", "variables": { "a": 1 } }))
        );
        assert_eq!(
            request.url().ok().map(String::from).as_deref(),
            Some(GRAPH_URL)
        );
    }

    #[test]
    fn an_absolute_url_must_point_at_a_mobile_api_host() {
        for foreign in [
            "https://cf-media.sndcdn.com/a.mp3",
            "http://api-mobile.soundcloud.com/me",
            "https://api-mobile.soundcloud.com:8443/me",
            "https://api-mobile.soundcloud.com.example.org/me",
        ] {
            let Ok(url) = Url::parse(foreign) else {
                panic!("{foreign} does not parse");
            };
            assert!(
                matches!(
                    MobileRequest::get_url(url).url(),
                    Err(MobileError::ForeignHost)
                ),
                "{foreign}"
            );
        }

        let Ok(media) = Url::parse("https://api-mobile.soundcloud.com/media/x/y/stream/hls") else {
            panic!("the media url does not parse");
        };
        assert!(MobileRequest::get_url(media).url().is_ok());
    }

    #[test]
    fn a_token_request_never_hands_out_its_form() {
        let request = MobileRequest::token("code=dummy-code".to_owned());

        assert_eq!(request.body(), None);
        assert!(!format!("{request:?}").contains("dummy-code"));
    }
}
