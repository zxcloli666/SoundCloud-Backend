use std::fmt;
use std::time::Duration;

use bytes::Bytes;
use redact::Secret;
use url::Url;
use wreq::header::{
    ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, RANGE, USER_AGENT,
};
use wreq::{Client, Method, redirect, retry};

use crate::app::MobileApp;
use crate::auth::client_authorization;
use crate::error::MobileError;
use crate::plan::UserPlan;
use crate::request::{JSON, Kind, MobileRequest};
use crate::transport::{self, Egress, Prepared, Route};

pub const TLS_PROFILE: &str = "okhttp_5";

const BODY_LIMIT: usize = 8 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const UDID: &str = "udid";
const APP_VERSION: &str = "app-version";

pub struct MobileClient {
    app: MobileApp,
    http: Client,
    egress: Egress,
}

#[derive(Clone)]
pub struct Caller {
    pub device_id: String,
    pub access_token: Option<Secret<String>>,
}

pub struct MobileReply {
    pub status: u16,
    pub plan: Option<UserPlan>,
    pub location: Option<Url>,
    pub body: Bytes,
}

impl Caller {
    pub fn device(device_id: &str) -> Self {
        Self {
            device_id: device_id.to_owned(),
            access_token: None,
        }
    }

    pub fn user(device_id: &str, access_token: Secret<String>) -> Self {
        Self {
            device_id: device_id.to_owned(),
            access_token: Some(access_token),
        }
    }
}

impl MobileReply {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

impl MobileClient {
    pub fn new(app: MobileApp, egress: Egress) -> Result<Self, MobileError> {
        let http = sc_fingerprint::builder(Some(TLS_PROFILE))
            .no_proxy()
            .redirect(redirect::Policy::none())
            .retry(retry::Policy::never())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| MobileError::Client(error.without_uri().to_string()))?;
        Ok(Self { app, http, egress })
    }

    pub fn app(&self) -> &MobileApp {
        &self.app
    }

    pub async fn call(
        &self,
        caller: &Caller,
        request: &MobileRequest,
    ) -> Result<MobileReply, MobileError> {
        let prepared = prepare(&self.app, caller, request)?;
        let first = self.send(&prepared, self.egress.first()).await;
        if !worth_another_send(&first) {
            return first;
        }
        let Some(route) = second_route(request.kind(), &self.egress) else {
            return first;
        };
        tracing::debug!(
            kind = ?request.kind(),
            direct = (route == Route::Direct),
            "mobile request failed in transport or with 5xx, sending it once more"
        );
        self.send(&prepared, route).await
    }

    pub async fn first_bytes(&self, url: &Url, limit: usize) -> Result<MobileReply, MobileError> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, header_value(&self.app.user_agent)?);
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        headers.insert(
            RANGE,
            header_value(&format!("bytes=0-{}", limit.saturating_sub(1)))?,
        );
        let prepared = Prepared {
            method: Method::GET,
            target: url.clone(),
            headers,
            body: None,
        };
        let received = transport::send(&self.http, &prepared, self.egress.first(), limit).await?;
        Ok(received.reply)
    }

    async fn send(
        &self,
        prepared: &Prepared,
        route: Route<'_>,
    ) -> Result<MobileReply, MobileError> {
        let received = transport::send(&self.http, prepared, route, BODY_LIMIT).await?;
        if received.cut {
            return Err(MobileError::TooLarge(BODY_LIMIT));
        }
        Ok(received.reply)
    }
}

fn prepare(
    app: &MobileApp,
    caller: &Caller,
    request: &MobileRequest,
) -> Result<Prepared, MobileError> {
    let mut target = request.url()?;
    let mut headers = HeaderMap::new();
    if request.kind() == Kind::Token {
        headers.insert(
            AUTHORIZATION,
            secret_value(client_authorization(app).expose())?,
        );
    } else {
        target
            .query_pairs_mut()
            .append_pair("client_id", &app.client_id);
        if let Some(token) = &caller.access_token {
            headers.insert(
                AUTHORIZATION,
                secret_value(&format!("OAuth {}", token.expose()))?,
            );
        }
    }
    headers.insert(UDID, secret_value(&caller.device_id)?);
    headers.insert(APP_VERSION, header_value(&app.app_version)?);
    headers.insert(USER_AGENT, header_value(&app.user_agent)?);
    headers.insert(ACCEPT, HeaderValue::from_static(JSON));
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    let body = request.payload().map(|payload| {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(payload.content_type));
        payload.bytes.clone()
    });
    Ok(Prepared {
        method: request.method().clone(),
        target,
        headers,
        body,
    })
}

fn second_route(kind: Kind, egress: &Egress) -> Option<Route<'_>> {
    match kind {
        Kind::Read => egress.direct_fallback(),
        Kind::Query => Some(egress.first()),
        Kind::Write | Kind::Mutation | Kind::Token => None,
    }
}

fn worth_another_send(result: &Result<MobileReply, MobileError>) -> bool {
    match result {
        Ok(reply) => reply.status >= 500,
        Err(error) => error.is_transport(),
    }
}

fn header_value(value: &str) -> Result<HeaderValue, MobileError> {
    HeaderValue::from_str(value).map_err(|_| MobileError::Invalid("a header value is not valid"))
}

fn secret_value(value: &str) -> Result<HeaderValue, MobileError> {
    let mut value = header_value(value)?;
    value.set_sensitive(true);
    Ok(value)
}

impl fmt::Debug for Caller {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Caller")
            .field("signed_in", &self.access_token.is_some())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for MobileReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MobileReply")
            .field("status", &self.status)
            .field("plan", &self.plan)
            .field("redirected", &self.location.is_some())
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
