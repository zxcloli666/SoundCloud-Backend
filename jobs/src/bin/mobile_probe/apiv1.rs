use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use redact::Secret;
use sc_mobile::{MobileError, MobileReply, UserPlan};
use serde_json::{Value, json};
use url::Url;
use wreq::header::{ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_TYPE, HeaderValue, LOCATION};
use wreq::{Client, Method, redirect, retry};

pub const API_URL: &str = "https://api.soundcloud.com";

const JSON: &str = "application/json; charset=utf-8";
const TARGET_HEADER: &str = "x-target";
const PAGE_LIMIT: &str = "200";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Apiv1 {
    http: Client,
    proxy: Option<Url>,
}

pub struct Apiv1Request {
    pub method: Method,
    segments: Vec<String>,
    query: Vec<(&'static str, &'static str)>,
    pub body: Option<Value>,
}

impl Apiv1Request {
    pub fn post_comment(track_urn: &str, body: &str, track_time_ms: u64) -> Self {
        Self {
            method: Method::POST,
            segments: segments(&["tracks", track_urn, "comments"]),
            query: Vec::new(),
            body: Some(json!({ "comment": { "body": body, "timestamp": track_time_ms } })),
        }
    }

    pub fn list_comments(track_urn: &str) -> Self {
        Self {
            method: Method::GET,
            segments: segments(&["tracks", track_urn, "comments"]),
            query: vec![("limit", PAGE_LIMIT)],
            body: None,
        }
    }

    pub fn get_comment(comment_urn: &str) -> Self {
        Self {
            method: Method::GET,
            segments: segments(&["comments", comment_urn]),
            query: Vec::new(),
            body: None,
        }
    }

    pub fn url(&self) -> Result<Url, MobileError> {
        let mut url = Url::parse(API_URL)
            .map_err(|_| MobileError::Invalid("the public API base does not parse"))?;
        url.path_segments_mut()
            .map_err(|()| MobileError::Invalid("the public API base cannot take a path"))?
            .clear()
            .extend(&self.segments);
        if !self.query.is_empty() {
            url.query_pairs_mut().extend_pairs(&self.query);
        }
        Ok(url)
    }
}

impl Apiv1 {
    pub fn new(proxy: Option<Url>) -> Result<Self, MobileError> {
        let http = sc_fingerprint::builder(None)
            .no_proxy()
            .redirect(redirect::Policy::none())
            .retry(retry::Policy::never())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| MobileError::Client(error.without_uri().to_string()))?;
        Ok(Self { http, proxy })
    }

    pub async fn call(
        &self,
        token: &Secret<String>,
        request: &Apiv1Request,
    ) -> Result<MobileReply, MobileError> {
        let target = request.url()?;
        let mut authorization = HeaderValue::from_str(&format!("OAuth {}", token.expose()))
            .map_err(|_| MobileError::Invalid("the access token is not a header value"))?;
        authorization.set_sensitive(true);
        let builder = match &self.proxy {
            Some(proxy) => self
                .http
                .request(request.method.clone(), proxy.as_str())
                .header(TARGET_HEADER, STANDARD.encode(target.as_str())),
            None => self.http.request(request.method.clone(), target.as_str()),
        }
        .header(AUTHORIZATION, authorization)
        .header(ACCEPT, JSON)
        .header(ACCEPT_ENCODING, "identity");
        let builder = match &request.body {
            Some(body) => builder.header(CONTENT_TYPE, JSON).body(body.to_string()),
            None => builder,
        };
        let response = builder.send().await.map_err(transport)?;
        let status = response.status().as_u16();
        let plan = UserPlan::from_headers(response.headers());
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| target.join(value).ok());
        let body = response.bytes().await.map_err(transport)?;
        Ok(MobileReply {
            status,
            plan,
            location,
            body,
        })
    }
}

fn segments(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn transport(error: wreq::Error) -> MobileError {
    MobileError::Transport(error.without_uri().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comment_is_posted_to_the_public_api_in_the_product_shape() {
        let post = Apiv1Request::post_comment("soundcloud:tracks:1", "probe", 0);
        let Ok(url) = post.url() else {
            panic!("the url does not build");
        };

        assert_eq!(post.method, Method::POST);
        assert_eq!(url.host_str(), Some("api.soundcloud.com"));
        assert_eq!(url.path(), "/tracks/soundcloud:tracks:1/comments");
        assert_eq!(url.query(), None);
        assert_eq!(
            post.body,
            Some(json!({ "comment": { "body": "probe", "timestamp": 0 } }))
        );
    }

    #[test]
    fn reads_stay_on_the_public_host_and_a_urn_stays_one_segment() {
        let list = Apiv1Request::list_comments("soundcloud:tracks:1/../me");
        let single = Apiv1Request::get_comment("soundcloud:comments:9");

        assert_eq!(
            list.url().ok().map(String::from).as_deref(),
            Some(
                "https://api.soundcloud.com/tracks/soundcloud:tracks:1%2F..%2Fme/comments?limit=200"
            )
        );
        assert_eq!(list.method, Method::GET);
        assert_eq!(list.body, None);
        assert_eq!(
            single.url().ok().map(String::from).as_deref(),
            Some("https://api.soundcloud.com/comments/soundcloud:comments:9")
        );
    }
}
